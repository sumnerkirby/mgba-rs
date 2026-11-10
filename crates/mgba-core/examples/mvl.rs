// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Replays mGBA video logs (`test.mvl`) through the software renderer and
//! compares the frames to the cinema baselines, the way mGBA's video-log
//! playback core (src/gba/core.c `_GBAVLP*`, src/gba/extra/proxy.c) does.
//!
//! Usage: cargo run --release -p mgba-core --example mvl -- <test dir>... [--dump OUT_DIR]

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use mgba_core::gba::renderer::SoftwareRenderer;

const DIRTY_FLUSH: u32 = 1;
const DIRTY_SCANLINE: u32 = 2;
const DIRTY_REGISTER: u32 = 3;
const DIRTY_OAM: u32 = 4;
const DIRTY_PALETTE: u32 = 5;
const DIRTY_VRAM: u32 = 6;
const DIRTY_FRAME: u32 = 7;

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn inflate(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data).read_to_end(&mut out).expect("bad zlib data");
    out
}

/// Returns (initial state, channel 0 stream)
fn parse(file: &[u8]) -> (Vec<u8>, Vec<u8>) {
    assert_eq!(&file[..4], b"mVL\0", "not an mVL file");
    let flags = u32le(file, 4);
    let mut pos = 16;
    let mut state = Vec::new();
    let read_block = |pos: &mut usize| -> Option<(u32, u32, Vec<u8>)> {
        if *pos + 16 > file.len() {
            return None;
        }
        let ty = u32le(file, *pos);
        let len = u32le(file, *pos + 4) as usize;
        let ch = u32le(file, *pos + 8);
        let bflags = u32le(file, *pos + 12);
        *pos += 16;
        if ty == 0x784C566D {
            return None;
        }
        let raw = &file[*pos..*pos + len];
        *pos += len;
        let data = if bflags & 1 != 0 { inflate(raw) } else { raw.to_vec() };
        Some((ty, ch, data))
    };
    if flags & 1 != 0 {
        let (ty, _, data) = read_block(&mut pos).expect("missing initial state");
        assert_eq!(ty, 1);
        state = data;
    }
    let mut stream = Vec::new();
    while let Some((ty, ch, data)) = read_block(&mut pos) {
        if ty == 3 && ch == 0 {
            stream.extend_from_slice(&data);
        }
    }
    (state, stream)
}

fn load_png(path: &Path) -> Option<Vec<u8>> {
    let decoder = png::Decoder::new(fs::File::open(path).ok()?);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    Some(match info.color_type {
        png::ColorType::Rgba => buf.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
        _ => buf[..info.buffer_size()].to_vec(),
    })
}

fn save_png(path: &Path, frame: &[u32]) {
    let mut enc = png::Encoder::new(fs::File::create(path).unwrap(), 240, 160);
    enc.set_color(png::ColorType::Rgb);
    let data: Vec<u8> = frame.iter().flat_map(|&p| [p as u8, (p >> 8) as u8, (p >> 16) as u8]).collect();
    enc.write_header().unwrap().write_image_data(&data).unwrap();
}

fn diff(frame: &[u32], expected: &[u8]) -> usize {
    frame
        .iter()
        .enumerate()
        .filter(|(i, &p)| [p as u8, (p >> 8) as u8, (p >> 16) as u8] != expected[i * 3..i * 3 + 3])
        .count()
}

/// Replays the log; returns the renderer output after each FLUSH-delimited frame.
fn replay(state: &[u8], stream: &[u8]) -> Vec<Vec<u32>> {
    let hw = |off: usize, n: usize| -> Vec<u16> { (0..n).map(|i| u16::from_le_bytes([state[off + i * 2], state[off + i * 2 + 1]])).collect() };
    let io = hw(0x400, 0x200);
    let mut palette = hw(0x800, 0x200);
    let mut oam = hw(0xC00, 0x200);
    let mut vram = hw(0x1000, 0xC000);

    let mut r = SoftwareRenderer::new();
    // GBAVideoDeserialize: palette/OAM stores, then renderer reset
    for i in 0..0x200 {
        r.write_oam(i as u32);
    }
    r.reset();
    r.sync_palette(&palette);
    // GBAIODeserialize: valid video registers go through GBAIOWrite
    let mut a = 0u32;
    while a < 0x56 {
        let valid = a != 0x02 && a != 0x04 && a != 0x06 && a != 0x4E;
        if valid {
            r.write_video_register(a, io[(a >> 1) as usize]);
        }
        a += 2;
    }

    let mut frames = Vec::new();
    let mut pos = 0;
    while pos + 16 <= stream.len() {
        let ty = u32le(stream, pos);
        let address = u32le(stream, pos + 4);
        let value = u32le(stream, pos + 8);
        pos += 16;
        match ty {
            DIRTY_REGISTER => {
                if address < 0x60 {
                    r.write_video_register(address, value as u16);
                }
            }
            DIRTY_PALETTE => {
                if address < 0x400 {
                    palette[(address >> 1) as usize] = value as u16;
                    r.write_palette(address, value as u16);
                }
            }
            DIRTY_OAM => {
                if (address << 1) < 0x400 {
                    oam[address as usize] = value as u16;
                    r.write_oam(address);
                }
            }
            DIRTY_VRAM => {
                let block = &stream[pos..pos + 0x1000];
                pos += 0x1000;
                if address <= 0x18000 - 0x1000 {
                    for i in 0..0x800 {
                        vram[(address as usize >> 1) + i] = u16::from_le_bytes([block[i * 2], block[i * 2 + 1]]);
                    }
                    r.write_vram(address);
                }
            }
            DIRTY_SCANLINE => {
                if address < 160 {
                    r.draw_scanline(address as usize, &vram, &oam);
                }
            }
            DIRTY_FRAME => r.finish_frame(),
            DIRTY_FLUSH => frames.push(r.output().to_vec()),
            _ => {}
        }
    }
    frames
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut dump = None;
    if let Some(i) = args.iter().position(|a| a == "--dump") {
        dump = Some(PathBuf::from(args.remove(i + 1)));
        args.remove(i);
        fs::create_dir_all(dump.as_ref().unwrap()).unwrap();
    }
    let (mut pass, mut fail) = (0, 0);
    for d in &args {
        let dir = PathBuf::from(d);
        let (state, stream) = parse(&fs::read(dir.join("test.mvl")).unwrap());
        let frames = replay(&state, &stream);
        let baselines: Vec<Vec<u8>> = (0..).map_while(|i| load_png(&dir.join(format!("baseline_{i:04}.png")))).collect();
        // Baseline N is the frame drawn during emulated frame N; find the log offset that lines up.
        let best = (0..2usize)
            .map(|off| {
                let bad: Vec<(usize, usize)> = baselines
                    .iter()
                    .enumerate()
                    .filter_map(|(i, b)| frames.get(i + off).map(|f| (i, diff(f, b))))
                    .filter(|(_, d)| *d > 0)
                    .collect();
                (off, bad)
            })
            .min_by_key(|(_, bad)| (bad.len(), bad.iter().map(|b| b.1).sum::<usize>()))
            .unwrap();
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        if let Some(out) = &dump {
            for (i, f) in frames.iter().enumerate().take(baselines.len() + 1) {
                save_png(&out.join(format!("{name}_{i:04}.png")), f);
            }
        }
        if best.1.is_empty() {
            pass += 1;
            println!("PASS {name} ({} frames)", baselines.len());
        } else {
            fail += 1;
            let detail: Vec<String> = best.1.iter().take(6).map(|(i, d)| format!("#{i}:{d}px")).collect();
            println!("FAIL {name}: {}/{} frames differ [{}]", best.1.len(), baselines.len(), detail.join(" "));
        }
    }
    println!("{pass} passed, {fail} failed");
}
