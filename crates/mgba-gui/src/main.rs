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
    // mgba-rs [ROM] [--link [PLAYER2_ROM]] [-d|--debug] [-g|--gdb [PORT]] [--script FILE.lua]
    let mut start = app::StartOptions::default();
    let mut args = std::env::args_os().skip(1).peekable();
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy().to_string();
        match text.as_str() {
            "--link" => {
                let p2 = args.next_if(|a| !a.to_string_lossy().starts_with('-')).map(PathBuf::from);
                start.link = Some(p2);
            }
            "-d" | "--debug" => start.debug_cli = true,
            "--script" => {
                if let Some(path) = args.next() {
                    start.scripts.push(PathBuf::from(path));
                }
            }
            "-g" | "--gdb" => {
                let port = args.next_if(|a| a.to_string_lossy().parse::<u16>().is_ok());
                start.gdb_port = Some(port.and_then(|p| p.to_string_lossy().parse().ok()).unwrap_or(2345));
            }
            _ if start.rom.is_none() => start.rom = Some(PathBuf::from(arg)),
            _ => {}
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
    eframe::run_native("mgba-rs", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, start)))))
}
