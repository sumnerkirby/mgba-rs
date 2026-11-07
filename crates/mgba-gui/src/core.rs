//! One handle for whichever emulation core runs the loaded game (`mCore` in mGBA).
//!
//! This Source Code Form is subject to the terms of the Mozilla Public
//! License, v. 2.0. If a copy of the MPL was not distributed with this
//! file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;

use mgba_core::gb::{self, model};
use mgba_core::gba::{self, LogLevel};
use mgba_core::{Gb, Gba};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Platform {
    Gba,
    Gb,
}

/// Decide which core a file belongs to: header check first, then the extension.
pub fn detect_platform(path: &Path, data: &[u8]) -> Option<Platform> {
    if gba::is_rom(data) {
        return Some(Platform::Gba);
    }
    if gb::is_rom(data) {
        return Some(Platform::Gb);
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "gba" | "agb" | "bin" | "mb" => Some(Platform::Gba),
        "gb" | "gbc" | "cgb" | "sgb" | "dmg" => Some(Platform::Gb),
        _ => None,
    }
}

pub enum Core {
    Gba(Box<Gba>),
    Gb(Box<Gb>),
}

impl Core {
    pub fn screen_size(&self) -> [usize; 2] {
        match self {
            Core::Gba(_) => [gba::VIDEO_HORIZONTAL_PIXELS, gba::VIDEO_VERTICAL_PIXELS],
            Core::Gb(_) => [gb::VIDEO_HORIZONTAL_PIXELS, gb::VIDEO_VERTICAL_PIXELS],
        }
    }

    /// Frames per second of the emulated system.
    pub fn fps(&self) -> f64 {
        match self {
            // 16777216 Hz / 280896 cycles per frame
            Core::Gba(_) => gba::ARM7TDMI_FREQUENCY as f64 / 280896.0,
            // 70224 CPU cycles per frame at the model's clock (faster on SGB)
            Core::Gb(g) => {
                let freq = if g.model >= model::CGB && g.model != model::AUTODETECT {
                    gb::DMG_SM83_FREQUENCY
                } else {
                    g.frequency()
                };
                freq as f64 / 70224.0
            }
        }
    }

    pub fn reset(&mut self) {
        match self {
            Core::Gba(g) => g.reset(),
            Core::Gb(g) => g.reset(),
        }
    }

    pub fn run_frame(&mut self) {
        match self {
            Core::Gba(g) => g.run_frame(),
            Core::Gb(g) => g.run_frame(),
        }
    }

    /// Keys in GBA KEYINPUT order; the Game Boy ignores L and R.
    pub fn set_keys(&mut self, keys: u16) {
        match self {
            Core::Gba(g) => g.set_keys(keys),
            Core::Gb(g) => g.set_keys(keys),
        }
    }

    /// Pixels as 0x00BBGGRR.
    pub fn framebuffer(&self) -> &[u32] {
        match self {
            Core::Gba(g) => g.framebuffer(),
            Core::Gb(g) => g.framebuffer(),
        }
    }

    pub fn drain_audio(&mut self, out: &mut Vec<[i16; 2]>) {
        match self {
            Core::Gba(g) => g.drain_audio(out),
            Core::Gb(g) => g.drain_audio(out),
        }
    }

    pub fn audio_sample_rate(&self) -> u32 {
        match self {
            Core::Gba(g) => g.audio_sample_rate(),
            Core::Gb(g) => g.audio_sample_rate(),
        }
    }

    pub fn savedata(&self) -> Option<Vec<u8>> {
        match self {
            Core::Gba(g) => g.savedata().map(|d| d.to_vec()),
            Core::Gb(g) => g.savedata(),
        }
    }

    /// Returns true once each time a save write has settled and should be flushed.
    pub fn take_savedata_updated(&mut self) -> bool {
        let flag = match self {
            Core::Gba(g) => &mut g.savedata_updated,
            Core::Gb(g) => &mut g.savedata_updated,
        };
        std::mem::take(flag)
    }

    pub fn set_log_enabled(&mut self, on: bool) {
        match self {
            Core::Gba(g) => g.log_enabled = on,
            Core::Gb(g) => g.log_enabled = on,
        }
    }

    pub fn drain_log(&mut self) -> Vec<(LogLevel, String)> {
        match self {
            Core::Gba(g) => g.drain_log(),
            Core::Gb(g) => g.drain_log(),
        }
    }

    pub fn save_state(&self) -> Vec<u8> {
        match self {
            Core::Gba(g) => g.save_state(),
            Core::Gb(g) => g.save_state(),
        }
    }

    pub fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        match self {
            Core::Gba(g) => g.load_state(data).map_err(|e| e.to_string()),
            Core::Gb(g) => g.load_state(data).map_err(|e| e.to_string()),
        }
    }

    pub fn game_title(&self) -> String {
        match self {
            Core::Gba(g) => g.game_title(),
            Core::Gb(g) => g.game_title(),
        }
    }

    /// Short description of the cartridge hardware for the status bar.
    pub fn cart_info(&self) -> String {
        match self {
            Core::Gba(g) => format!("Save: {}", g.memory.savedata.ty.name()),
            Core::Gb(g) => format!("{} · {}", model::name(g.model), gb::mbc::name(g.memory.mbc_type)),
        }
    }

    pub fn set_skip_bios(&mut self, on: bool) {
        match self {
            Core::Gba(g) => g.skip_bios_on_reset = on,
            Core::Gb(g) => g.skip_bios_on_reset = on,
        }
    }
}
