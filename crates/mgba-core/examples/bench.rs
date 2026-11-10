// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Measure raw emulation speed: frames per second without frame limiting.
use mgba_core::Gba;
use std::time::Instant;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom = std::fs::read(&args[1]).unwrap();
    let frames: u32 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(3000);
    let mut gba = Gba::new();
    gba.load_rom(rom);
    gba.reset();
    let mut audio = Vec::new();
    let start = Instant::now();
    for _ in 0..frames {
        gba.run_frame();
        audio.clear();
        gba.drain_audio(&mut audio);
    }
    let secs = start.elapsed().as_secs_f64();
    println!("{frames} frames in {secs:.2}s = {:.0} fps ({:.1}x realtime)", frames as f64 / secs, frames as f64 / secs / 59.7275);
}
