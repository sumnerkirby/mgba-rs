// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Regression tests against mGBA's Game Boy "cinema" suite
//! (`mgba-master/cinema/gb`: blargg, mooneye-gb, acid, samesuite, ...).
//!
//! Each test directory has a ROM and expected screenshots. Settings come from
//! `config.ini` files, inherited from parent directories like mGBA's
//! `cinema-main.c` does: `[testinfo]` gives skip/frames/input/fail, and
//! `[ports.cinema]` gives core settings such as `gb.model`.
//!
//! Results are classified the way mGBA's runner does:
//! - PASS:  every frame matches `baseline_NNNN.png`
//! - XFAIL: the test is marked `fail=1` (mGBA itself fails it) and the output
//!          matches mGBA's own wrong output in `xbaseline_NNNN.png`
//! - XPASS: marked `fail=1` but matches the correct baseline
//! - FAIL:  anything else
//!
//! A faithful port should PASS what mGBA passes and XFAIL what it fails.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mgba_core::gb::model;
use mgba_core::Gb;

fn cinema_dir() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mgba-master/cinema/gb");
    dir.is_dir().then_some(dir)
}

fn load_png(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(std::fs::File::open(path).ok()?);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let rgb = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
        png::ColorType::Rgb => buf[..info.buffer_size()].to_vec(),
        _ => return None,
    };
    Some((info.width, info.height, rgb))
}

/// Parsed `config.ini` sections: section name -> key -> value.
type Ini = HashMap<String, HashMap<String, String>>;

fn parse_ini(text: &str) -> Ini {
    let mut ini = Ini::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_string();
        } else if let Some((k, v)) = line.split_once('=') {
            ini.entry(section.clone()).or_default().insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    ini
}

/// Merge every config.ini from the suite root down to `dir` (children override parents).
fn load_config(root: &Path, dir: &Path) -> Ini {
    let mut chain = vec![];
    let mut cur = dir.to_path_buf();
    loop {
        chain.push(cur.clone());
        if cur == root || !cur.pop() {
            break;
        }
    }
    let mut merged = Ini::new();
    for d in chain.iter().rev() {
        if let Ok(text) = std::fs::read_to_string(d.join("config.ini")) {
            for (section, kv) in parse_ini(&text) {
                merged.entry(section).or_default().extend(kv);
            }
        }
    }
    merged
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Status {
    Pass,
    Xfail,
    Xpass,
    Fail,
}

struct Outcome {
    name: String,
    status: Status,
    detail: String,
}

fn rom_path(dir: &Path) -> Option<PathBuf> {
    ["test.gb", "test.gbc"].iter().map(|f| dir.join(f)).find(|p| p.exists())
}

fn frame_matches(gb: &Gb, expected: &(u32, u32, Vec<u8>)) -> Result<(), String> {
    let (w, h, px) = expected;
    if (*w as usize, *h as usize) != gb.video_size() {
        return Err(format!("baseline is {w}x{h}, output is {:?}", gb.video_size()));
    }
    let bad = gb
        .framebuffer()
        .iter()
        .enumerate()
        .filter(|(i, &p)| [p as u8, (p >> 8) as u8, (p >> 16) as u8] != px[i * 3..i * 3 + 3])
        .count();
    if bad > 0 {
        Err(format!("{bad} pixels differ"))
    } else {
        Ok(())
    }
}

fn run_test(root: &Path, dir: &Path) -> Outcome {
    let name = dir.strip_prefix(root).unwrap_or(dir).display().to_string();
    let ini = load_config(root, dir);
    let empty = HashMap::new();
    let info = ini.get("testinfo").unwrap_or(&empty);
    let port = ini.get("ports.cinema").unwrap_or(&empty);
    let get = |k: &str, d: usize| info.get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    let skip = get("skip", 0);
    let frames = get("frames", 3600);
    let fail = get("fail", 0) != 0;
    let mut inputs: Vec<(usize, u16)> = info
        .get("input")
        .map(|s| s.as_str())
        .unwrap_or("")
        .split(',')
        .filter_map(|e| e.split_once(':'))
        .map(|(f, k)| (f.trim().parse().unwrap(), u16::from_str_radix(k.trim(), 16).unwrap()))
        .collect();
    inputs.reverse();

    let mut gb = Gb::new();
    let pick = |k: &str| port.get(k).map(|v| model::from_name(v)).filter(|&m| m != model::AUTODETECT);
    gb.model_config.gb = pick("gb.model");
    gb.model_config.sgb = pick("sgb.model");
    gb.model_config.cgb = pick("cgb.model");
    gb.model_config.cgb_hybrid = pick("cgb.hybridModel");
    gb.model_config.cgb_sgb = pick("cgb.sgbModel");
    if let Some(v) = port.get("sgb.borders") {
        gb.video.sgb_borders = !matches!(v.as_str(), "0" | "false");
    }
    gb.rtc_fake_epoch_ms = Some(1_200_000_000);
    gb.load_rom(std::fs::read(rom_path(dir).unwrap()).unwrap());
    gb.reset();
    for _ in 0..skip {
        gb.run_frame();
    }

    let mut status = Status::Pass;
    let mut xdiff = false;
    let mut detail = String::new();
    for frame in 0..frames {
        if inputs.last().map(|i| i.0) == Some(frame) {
            gb.set_keys(inputs.pop().unwrap().1);
        }
        gb.run_frame();
        let baseline = load_png(&dir.join(format!("baseline_{frame:04}.png")));
        let xbaseline = load_png(&dir.join(format!("xbaseline_{frame:04}.png")));
        if baseline.is_none() && xbaseline.is_none() && frame > 0 {
            // No more expected frames on disk.
            break;
        }
        let failed = match &baseline {
            Some(b) => match frame_matches(&gb, b) {
                Ok(()) => false,
                Err(e) => {
                    detail = format!("frame {frame}: {e}");
                    true
                }
            },
            None => {
                detail = format!("frame {frame}: no baseline");
                true
            }
        };
        if failed {
            status = Status::Fail;
            if fail {
                match &xbaseline {
                    Some(x) => {
                        if let Err(e) = frame_matches(&gb, x) {
                            detail = format!("frame {frame}: differs from xbaseline too ({e})");
                            xdiff = true;
                        }
                    }
                    // mGBA's runner only fails an expected-failure test when an
                    // xbaseline exists and differs.
                    None => {}
                }
            }
        }
        if frame + 1 >= frames {
            break;
        }
    }
    if fail {
        if status == Status::Fail && !xdiff {
            status = Status::Xfail;
        } else if status == Status::Pass {
            status = Status::Xpass;
        }
    }
    Outcome { name, status, detail }
}

fn collect_tests(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if rom_path(&p).is_some() && !p.join("test.mvl").exists() {
                out.push(p.clone());
            }
            collect_tests(&p, out);
        }
    }
}

/// Run every GB ROM test (video-log tests under `window/` are not ROM tests).
fn run_suite(root: &Path) -> Vec<Outcome> {
    let mut tests = vec![];
    collect_tests(root, &mut tests);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let tests = std::sync::Arc::new(std::sync::Mutex::new(tests.into_iter()));
    let results = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut handles = vec![];
    for _ in 0..threads {
        let tests = tests.clone();
        let results = results.clone();
        let root = root.to_path_buf();
        handles.push(
            std::thread::Builder::new()
                .stack_size(16 << 20)
                .spawn(move || loop {
                    let next = tests.lock().unwrap().next();
                    let Some(dir) = next else { break };
                    let outcome = std::panic::catch_unwind(|| run_test(&root, &dir)).unwrap_or_else(|_| Outcome {
                        name: dir.strip_prefix(&root).unwrap_or(&dir).display().to_string(),
                        status: Status::Fail,
                        detail: "panicked".into(),
                    });
                    results.lock().unwrap().push(outcome);
                })
                .unwrap(),
        );
    }
    for h in handles {
        h.join().unwrap();
    }
    let mut results = std::mem::take(&mut *results.lock().unwrap());
    results.sort_by(|a, b| a.name.cmp(&b.name));
    results
}

/// Tests this port is known not to match mGBA on yet. Keep this list honest:
/// it is printed with the summary, and a test that starts matching should be
/// removed from it.
const KNOWN_MISMATCHES: &[&str] = &[];

#[test]
fn gb_cinema_rom_tests() {
    let Some(root) = cinema_dir() else {
        eprintln!("mgba-master/cinema/gb not found; skipping");
        return;
    };
    let results = run_suite(&root);
    let mut counts: HashMap<Status, usize> = HashMap::new();
    for r in &results {
        *counts.entry(r.status).or_default() += 1;
    }
    let count = |s| counts.get(&s).copied().unwrap_or(0);
    println!("GB cinema: {} tests", results.len());
    println!(
        "  PASS {}  XFAIL {} (fails in mGBA too, matches mGBA's output)  XPASS {}  FAIL {}",
        count(Status::Pass),
        count(Status::Xfail),
        count(Status::Xpass),
        count(Status::Fail)
    );
    let mut unexpected = vec![];
    for r in &results {
        if matches!(r.status, Status::Fail | Status::Xpass) {
            println!("  {:?}: {} ({})", r.status, r.name, r.detail);
            if !KNOWN_MISMATCHES.contains(&r.name.as_str()) {
                unexpected.push(r.name.clone());
            }
        }
    }
    for name in KNOWN_MISMATCHES {
        if let Some(r) = results.iter().find(|r| r.name == *name) {
            if matches!(r.status, Status::Pass | Status::Xfail) {
                println!("  now matching, remove from KNOWN_MISMATCHES: {name}");
            }
        }
    }
    assert!(!results.is_empty());
    assert!(unexpected.is_empty(), "{} tests don't match mGBA: {:?}", unexpected.len(), unexpected);
}

#[test]
fn gb_save_state_round_trip_is_deterministic() {
    let Some(root) = cinema_dir() else { return };
    let rom = std::fs::read(root.join("acid/cgb-acid2/test.gbc")).or_else(|_| std::fs::read(root.join("blargg/cpu_instrs/01-special/test.gb")));
    let Ok(rom) = rom else { return };
    let mut gb = Gb::new();
    gb.load_rom(rom);
    gb.reset();
    for _ in 0..20 {
        gb.run_frame();
    }
    let state = gb.save_state();
    let run = |g: &mut Gb| {
        let mut frames = Vec::new();
        let mut audio = Vec::new();
        for i in 0..30 {
            g.set_keys(if i % 7 < 3 { 1 } else { 0x80 });
            g.run_frame();
            frames.push(g.framebuffer().to_vec());
            g.drain_audio(&mut audio);
        }
        (frames, g.cpu.pc, g.cpu.af(), g.timing.master_cycles)
    };
    let first = run(&mut gb);
    gb.load_state(&state).unwrap();
    gb.audio.buffer.clear();
    let second = run(&mut gb);
    assert_eq!(first.1, second.1, "PC diverged after loading state");
    assert_eq!(first.2, second.2, "AF diverged after loading state");
    assert_eq!(first.3, second.3, "timing diverged after loading state");
    assert_eq!(first.0, second.0, "frames diverged after loading state");
}
