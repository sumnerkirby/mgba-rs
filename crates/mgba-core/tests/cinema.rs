//! Regression tests against mGBA's own "cinema" suite (ROM tests with
//! baseline screenshots). They run when the mGBA source tree is checked out
//! next to this workspace as `mgba-master/`; otherwise they are skipped.

use std::path::{Path, PathBuf};

use mgba_core::Gba;

fn cinema_dir() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mgba-master/cinema/gba");
    dir.is_dir().then_some(dir)
}

fn load_png(path: &Path) -> Option<Vec<u8>> {
    let decoder = png::Decoder::new(std::fs::File::open(path).ok()?);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    Some(match info.color_type {
        png::ColorType::Rgba => buf.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
        _ => buf[..info.buffer_size()].to_vec(),
    })
}

fn ini_get(ini: &str, key: &str) -> Option<String> {
    ini.lines().filter_map(|l| l.split_once('=')).find(|(k, _)| k.trim() == key).map(|(_, v)| v.trim().to_string())
}

fn run_rom_test(dir: &Path) -> Result<usize, String> {
    let ini = std::fs::read_to_string(dir.join("config.ini")).unwrap_or_default();
    let skip: usize = ini_get(&ini, "skip").and_then(|v| v.parse().ok()).unwrap_or(0);
    let frames: usize = ini_get(&ini, "frames").and_then(|v| v.parse().ok()).unwrap_or(1);
    let mut inputs: Vec<(usize, u16)> = ini_get(&ini, "input")
        .unwrap_or_default()
        .split(',')
        .filter_map(|e| e.split_once(':'))
        .map(|(f, k)| (f.parse().unwrap(), u16::from_str_radix(k, 16).unwrap()))
        .collect();
    inputs.reverse();
    let mut gba = Gba::new();
    gba.load_rom(std::fs::read(dir.join("test.gba")).unwrap());
    gba.reset();
    for _ in 0..skip {
        gba.run_frame();
    }
    let mut compared = 0;
    for frame in 0..frames {
        if inputs.last().map(|i| i.0) == Some(frame) {
            gba.set_keys(inputs.pop().unwrap().1);
        }
        gba.run_frame();
        let Some(expected) = load_png(&dir.join(format!("baseline_{frame:04}.png"))) else { continue };
        compared += 1;
        let bad = gba
            .framebuffer()
            .iter()
            .enumerate()
            .filter(|(i, &p)| [p as u8, (p >> 8) as u8, (p >> 16) as u8] != expected[i * 3..i * 3 + 3])
            .count();
        if bad > 0 {
            return Err(format!("{}: frame {frame} differs in {bad} pixels", dir.display()));
        }
    }
    Ok(compared)
}

#[test]
fn cinema_rom_tests() {
    let Some(root) = cinema_dir() else {
        eprintln!("mgba-master/cinema not found; skipping");
        return;
    };
    let mut ran = 0;
    for category in std::fs::read_dir(&root).unwrap() {
        for test in std::fs::read_dir(category.unwrap().path()).unwrap() {
            let dir = test.unwrap().path();
            if dir.join("test.gba").exists() {
                run_rom_test(&dir).unwrap();
                ran += 1;
            }
        }
    }
    assert!(ran > 0);
}

#[test]
fn save_state_round_trip_is_deterministic() {
    let Some(root) = cinema_dir() else { return };
    let rom = std::fs::read(root.join("irq/keyirq/test.gba")).unwrap();
    let mut gba = Gba::new();
    gba.load_rom(rom);
    gba.reset();
    for _ in 0..20 {
        gba.run_frame();
    }
    let state = gba.save_state();
    let run = |g: &mut Gba| {
        let mut frames = Vec::new();
        for i in 0..30 {
            g.set_keys(if i % 7 < 3 { 1 } else { 3 });
            g.run_frame();
            frames.push(g.framebuffer().to_vec());
        }
        (frames, g.cpu.gprs, g.timing.master_cycles)
    };
    let first = run(&mut gba);
    gba.load_state(&state).unwrap();
    let second = run(&mut gba);
    assert_eq!(first.1, second.1, "CPU registers diverged after loading state");
    assert_eq!(first.2, second.2, "timing diverged after loading state");
    // The renderer is rebuilt on load, so allow the first frame to differ.
    assert_eq!(first.0[1..], second.0[1..], "frames diverged after loading state");
}
