// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Runs a Game Boy ROM headlessly and writes the last frame as a PNG.
//! Handy for checking a cinema test by eye or timing the core.
//!
//! Usage: cargo run --release -p mgba-core --example gb -- <rom> <frames> [out.png] [--model CGB] [--no-borders]

use std::time::Instant;

use mgba_core::gb::model;
use mgba_core::Gb;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: gb <rom> <frames> [out.png] [--model NAME]");
        std::process::exit(1);
    }
    let frames: u32 = args[1].parse().expect("frame count");
    let out = args.get(2).filter(|a| !a.starts_with("--")).cloned();
    let mut gb = Gb::new();
    if let Some(i) = args.iter().position(|a| a == "--model") {
        let m = model::from_name(&args[i + 1]);
        gb.model_config.gb = Some(m);
        gb.model_config.cgb = Some(m);
        gb.model_config.cgb_hybrid = Some(m);
        gb.model_config.sgb = Some(m);
    }
    if args.iter().any(|a| a == "--no-borders") {
        gb.video.sgb_borders = false;
    }
    gb.load_rom(std::fs::read(&args[0]).expect("read ROM"));
    gb.reset();
    let start = Instant::now();
    let mut audio = Vec::new();
    for _ in 0..frames {
        gb.run_frame();
        gb.drain_audio(&mut audio);
        audio.clear();
    }
    let elapsed = start.elapsed();
    println!(
        "{} frames as {} in {:.2?} ({:.0} fps)",
        frames,
        model::name(gb.model),
        elapsed,
        frames as f64 / elapsed.as_secs_f64()
    );
    if let Some(path) = out {
        let file = std::fs::File::create(&path).unwrap();
        let (w, h) = gb.video_size();
        let mut enc = png::Encoder::new(file, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        let mut w = enc.write_header().unwrap();
        let data: Vec<u8> = gb.framebuffer().iter().flat_map(|&p| [p as u8, (p >> 8) as u8, (p >> 16) as u8]).collect();
        w.write_image_data(&data).unwrap();
        println!("wrote {path}");
    }
}
