//! Runs mGBA "cinema" test ROMs (test.gba + config.ini + baseline_NNNN.png)
//! headlessly and compares each frame against the baseline images.
//!
//! Usage: cargo run --release -p mgba-core --example cinema -- <test dir>... [--dump OUT_DIR]

use std::fs;
use std::path::{Path, PathBuf};

use mgba_core::Gba;

fn ini_get(ini: &str, key: &str) -> Option<String> {
    ini.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().to_string())
}

fn load_png(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(fs::File::open(path).ok()?);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let rgb: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
        png::ColorType::Rgb => buf[..info.buffer_size()].to_vec(),
        _ => return None,
    };
    Some((info.width, info.height, rgb))
}

fn save_png(path: &Path, frame: &[u32]) {
    let file = fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(file, 240, 160);
    enc.set_color(png::ColorType::Rgb);
    let mut w = enc.write_header().unwrap();
    let data: Vec<u8> = frame.iter().flat_map(|&p| [p as u8, (p >> 8) as u8, (p >> 16) as u8]).collect();
    w.write_image_data(&data).unwrap();
}

/// Returns (frames compared, frames failed)
fn run_test(dir: &Path, dump: Option<&Path>) -> (usize, usize) {
    let ini = fs::read_to_string(dir.join("config.ini")).unwrap_or_default();
    let skip: usize = ini_get(&ini, "skip").and_then(|v| v.parse().ok()).unwrap_or(0);
    let frames: usize = ini_get(&ini, "frames").and_then(|v| v.parse().ok()).unwrap_or(3600);
    let input = ini_get(&ini, "input").unwrap_or_default();
    let mut inputs: Vec<(usize, u16)> = input
        .split(',')
        .filter_map(|e| e.split_once(':'))
        .map(|(f, k)| (f.parse().unwrap(), u16::from_str_radix(k, 16).unwrap()))
        .collect();
    inputs.reverse();

    let rom = fs::read(dir.join("test.gba")).expect("test.gba");
    let mut gba = Gba::new();
    gba.load_rom(rom);
    gba.reset();
    for _ in 0..skip {
        gba.run_frame();
    }
    let mut failed = 0;
    let mut compared = 0;
    for frame in 0..frames {
        if let Some(&(f, keys)) = inputs.last() {
            if f == frame {
                gba.set_keys(keys);
                inputs.pop();
            }
        }
        gba.run_frame();
        let fb = gba.framebuffer();
        if let Some(out) = dump {
            let name = dir.file_name().unwrap().to_string_lossy();
            save_png(&out.join(format!("{}_{:04}.png", name, frame)), fb);
        }
        let Some((w, h, expected)) = load_png(&dir.join(format!("baseline_{:04}.png", frame))) else {
            continue;
        };
        assert_eq!((w, h), (240, 160));
        compared += 1;
        let mut max = 0i32;
        let mut bad = 0;
        for (i, &p) in fb.iter().enumerate() {
            let got = [p as u8, (p >> 8) as u8, (p >> 16) as u8];
            let exp = &expected[i * 3..i * 3 + 3];
            let d = (0..3).map(|c| (got[c] as i32 - exp[c] as i32).abs()).max().unwrap();
            if d > 0 {
                bad += 1;
                max = max.max(d);
            }
        }
        if bad > 0 {
            failed += 1;
            println!("  frame {frame}: {bad} pixels differ (max delta {max})");
        }
    }
    (compared, failed)
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut dump = None;
    if let Some(i) = args.iter().position(|a| a == "--dump") {
        dump = Some(PathBuf::from(args.remove(i + 1)));
        args.remove(i);
        fs::create_dir_all(dump.as_ref().unwrap()).unwrap();
    }
    let mut total_fail = 0;
    for d in &args {
        let dir = PathBuf::from(d);
        let (compared, failed) = run_test(&dir, dump.as_deref());
        let status = if failed == 0 { "PASS" } else { "FAIL" };
        println!("{status} {} ({}/{} frames match)", dir.display(), compared - failed, compared);
        total_fail += failed;
    }
    std::process::exit(if total_fail == 0 { 0 } else { 1 });
}
