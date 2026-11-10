// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Exchange save states with mGBA.
//!
//! `cargo run --release -p mgba-core --example mgba_state -- export ROM FRAMES OUT.state [FRAME.rgb]`
//! runs FRAMES frames (BIOS skipped) and writes an mGBA-format state.
//!
//! `cargo run --release -p mgba-core --example mgba_state -- import ROM STATE FRAMES [FRAME.rgb [OUT.state]]`
//! loads an mGBA state (raw or PNG), runs FRAMES more frames and optionally
//! writes the last frame as raw RGB bytes for comparison, and the resulting
//! state (`OUT.state`) in mGBA's format.

use mgba_core::{gb, Gb, Gba};

fn rgb_gba(fb: &[u32]) -> Vec<u8> {
    fb.iter().flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8]).collect()
}

fn rgb_gb(fb: &[u32]) -> Vec<u8> {
    fb.iter().flat_map(|&p| [p as u8, (p >> 8) as u8, (p >> 16) as u8]).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        eprintln!("usage: mgba_state export ROM FRAMES OUT [FRAME.rgb] | import ROM STATE FRAMES [FRAME.rgb]");
        std::process::exit(2);
    }
    let rom = std::fs::read(&args[1]).expect("read ROM");
    if gb::is_rom(&rom) {
        eprintln!("Game Boy states aren't supported in mGBA's format yet");
        std::process::exit(1);
    }
    let mut gba = Box::new(Gba::new());
    gba.load_rom(rom);
    gba.skip_bios_on_reset = true;
    gba.reset();
    let (frames, fb_out) = match args[0].as_str() {
        "export" => {
            let frames: u32 = args[2].parse().expect("frames");
            for _ in 0..frames {
                gba.run_frame();
            }
            std::fs::write(&args[3], gba.save_state_mgba()).expect("write state");
            (0, args.get(4))
        }
        "import" => {
            let state = std::fs::read(&args[2]).expect("read state");
            if let Err(e) = gba.load_state_mgba(&state) {
                eprintln!("load failed: {e}");
                std::process::exit(1);
            }
            (args[3].parse().expect("frames"), args.get(4))
        }
        _ => panic!("unknown command"),
    };
    for _ in 0..frames {
        gba.run_frame();
    }
    if let Some(path) = fb_out {
        std::fs::write(path, rgb_gba(gba.framebuffer())).expect("write frame");
    }
    if args[0] == "import" {
        if let Some(path) = args.get(5) {
            std::fs::write(path, gba.save_state_mgba()).expect("write state");
        }
    }
    let _ = (rgb_gb, Gb::new);
    println!("frame {} pc {:08X}", gba.video.frame_counter, gba.cpu.gprs[15]);
}
