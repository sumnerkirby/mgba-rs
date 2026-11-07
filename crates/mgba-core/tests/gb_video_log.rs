//! Replays mGBA's recorded Game Boy video logs (cinema/gb/window/*/test.mvl)
//! through the GB renderer and checks every frame against mGBA's baselines.
//! Skipped when the mGBA source tree isn't next to this workspace.

use std::io::Read;
use std::path::Path;

use mgba_core::Gb;

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Returns (initial state, channel 0 stream).
fn parse(file: &[u8]) -> (Vec<u8>, Vec<u8>) {
    assert_eq!(&file[..4], b"mVL\0", "not an mVL file");
    let mut pos = 16;
    let (mut state, mut stream) = (Vec::new(), Vec::new());
    while pos + 16 <= file.len() {
        let (ty, len, ch, flags) = (u32le(file, pos), u32le(file, pos + 4) as usize, u32le(file, pos + 8), u32le(file, pos + 12));
        pos += 16;
        if ty == 0x784C566D {
            break;
        }
        let raw = &file[pos..pos + len];
        pos += len;
        let data = if flags & 1 != 0 {
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(raw).read_to_end(&mut out).unwrap();
            out
        } else {
            raw.to_vec()
        };
        match (ty, ch) {
            (1, _) => state = data,
            (3, 0) => stream.extend_from_slice(&data),
            _ => {}
        }
    }
    (state, stream)
}

fn load_png(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    let decoder = png::Decoder::new(std::fs::File::open(path).ok()?);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let rgb = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
        _ => buf[..info.buffer_size()].to_vec(),
    };
    Some((info.width as usize, info.height as usize, rgb))
}

/// Replays the log; returns the output after each FLUSH-delimited frame.
fn replay(state: &[u8], stream: &[u8]) -> (Vec<Vec<u32>>, (usize, usize)) {
    let mut gb = Gb::new();
    assert!(gb.load_mvl_state(state));
    let mut frames = Vec::new();
    let mut pos = 0;
    while pos + 16 <= stream.len() {
        let ty = u32le(stream, pos);
        let (address, value, value2) = (u32le(stream, pos + 4), u32le(stream, pos + 8), u32le(stream, pos + 12));
        pos += 16;
        let payload_len = match ty {
            6 => 0x1000,
            9 => value2 as usize,
            _ => 0,
        };
        let payload = &stream[pos..(pos + payload_len).min(stream.len())];
        pos += payload_len;
        if ty == 1 {
            frames.push(gb.framebuffer().to_vec());
        } else {
            gb.mvl_packet(ty, address, value, value2, payload);
        }
    }
    let size = gb.video_size();
    (frames, size)
}

#[test]
fn gb_cinema_video_logs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mgba-master/cinema/gb/window");
    if !root.is_dir() {
        eprintln!("mgba-master/cinema/gb/window not found; skipping");
        return;
    }
    let mut dirs: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().path()).filter(|p| p.join("test.mvl").exists()).collect();
    dirs.sort();
    let mut failures = Vec::new();
    for dir in &dirs {
        let (state, stream) = parse(&std::fs::read(dir.join("test.mvl")).unwrap());
        let (frames, (w, h)) = replay(&state, &stream);
        let baselines: Vec<_> = (0..).map_while(|i| load_png(&dir.join(format!("baseline_{i:04}.png")))).collect();
        let matches = |off: usize| {
            baselines.iter().enumerate().all(|(i, (bw, bh, px))| {
                (*bw, *bh) == (w, h)
                    && frames.get(i + off).is_some_and(|f| {
                        f.iter().enumerate().all(|(j, &p)| [p as u8, (p >> 8) as u8, (p >> 16) as u8] == px[j * 3..j * 3 + 3])
                    })
            })
        };
        let ok = (0..2).any(matches);
        println!("{}: {} ({} frames, {} baselines, {w}x{h})", dir.file_name().unwrap().to_string_lossy(), if ok { "ok" } else { "MISMATCH" }, frames.len(), baselines.len());
        if !ok {
            failures.push(dir.file_name().unwrap().to_string_lossy().to_string());
        }
    }
    assert_eq!(dirs.len(), 10);
    assert!(failures.is_empty(), "GB video log mismatches: {failures:?}");
}
