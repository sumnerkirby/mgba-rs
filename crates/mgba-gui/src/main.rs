//! mGBA-rs: a desktop Game Boy Advance and Game Boy (Color) emulator built on the Rust port of mGBA.
//!
//! This Source Code Form is subject to the terms of the Mozilla Public
//! License, v. 2.0. If a copy of the MPL was not distributed with this
//! file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod app;
mod audio;
mod core;
mod gamepad;
mod rewind;

use std::path::PathBuf;

fn main() -> eframe::Result {
    // mgba-rs [ROM] [--link [PLAYER2_ROM]]
    let mut rom = None;
    let mut link: Option<Option<PathBuf>> = None;
    let mut args = std::env::args_os().skip(1).peekable();
    while let Some(arg) = args.next() {
        if arg == "--link" {
            let p2 = args.next_if(|a| !a.to_string_lossy().starts_with("--")).map(PathBuf::from);
            link = Some(p2);
        } else if rom.is_none() {
            rom = Some(PathBuf::from(arg));
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("mGBA-rs")
            .with_app_id("mgba-rs")
            .with_inner_size([720.0, 548.0])
            .with_min_inner_size([240.0, 208.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("mgba-rs", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, rom, link)))))
}
