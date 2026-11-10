//! One handle for whichever emulation core runs the loaded game (`mCore` in mGBA).
//!
//! This Source Code Form is subject to the terms of the Mozilla Public
//! License, v. 2.0. If a copy of the MPL was not distributed with this
//! file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;
use std::sync::{Arc, Mutex};

use mgba_core::cheats::{CheatDevice, CheatSet};
use mgba_core::gb::{self, model, GbSioLockstep};
use mgba_core::gba::{self, GbaSioLockstepCoordinator, LogLevel};
use mgba_core::lockstep;
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

/// A link cable between cores of one platform (the lockstep half of mGBA's
/// `MultiplayerController`).
pub enum LinkCable {
    Gba(Arc<Mutex<GbaSioLockstepCoordinator>>),
    Gb(Arc<Mutex<GbSioLockstep>>),
}

impl LinkCable {
    pub fn new(platform: Platform) -> LinkCable {
        match platform {
            Platform::Gba => LinkCable::Gba(GbaSioLockstepCoordinator::new()),
            Platform::Gb => LinkCable::Gb(GbSioLockstep::new()),
        }
    }
}

/// Run linked cores for one frame each. They must all be the cable's platform.
pub fn run_linked_frame(cores: &mut [&mut Core]) {
    let ok = if cores.iter().all(|c| matches!(c, Core::Gba(_))) {
        let mut v: Vec<&mut Gba> = cores
            .iter_mut()
            .map(|c| match &mut **c {
                Core::Gba(g) => g.as_mut(),
                Core::Gb(_) => unreachable!(),
            })
            .collect();
        lockstep::run_linked_frame(&mut v)
    } else if cores.iter().all(|c| matches!(c, Core::Gb(_))) {
        let mut v: Vec<&mut Gb> = cores
            .iter_mut()
            .map(|c| match &mut **c {
                Core::Gb(g) => g.as_mut(),
                Core::Gba(_) => unreachable!(),
            })
            .collect();
        lockstep::run_linked_frame(&mut v)
    } else {
        false
    };
    if !ok {
        // The link stalled; keep everyone moving rather than freezing the window.
        for c in cores.iter_mut() {
            c.run_frame();
        }
    }
}

impl Core {
    pub fn platform(&self) -> Platform {
        match self {
            Core::Gba(_) => Platform::Gba,
            Core::Gb(_) => Platform::Gb,
        }
    }

    /// Plug this core into `cable` as player `player` (0-based). Returns false
    /// if the platforms differ or the cable is full.
    pub fn attach_link(&mut self, cable: &LinkCable, player: i32) -> bool {
        match (self, cable) {
            (Core::Gba(g), LinkCable::Gba(c)) => {
                if c.lock().unwrap().attached() >= gba::sio_lockstep::MAX_GBAS {
                    return false;
                }
                g.attach_link(c, Some(player));
                true
            }
            (Core::Gb(g), LinkCable::Gb(l)) => g.attach_link(l),
            _ => false,
        }
    }

    pub fn detach_link(&mut self) {
        match self {
            Core::Gba(g) => g.detach_link(),
            Core::Gb(g) => g.detach_link(),
        }
    }

    pub fn screen_size(&self) -> [usize; 2] {
        match self {
            Core::Gba(_) => [gba::VIDEO_HORIZONTAL_PIXELS, gba::VIDEO_VERTICAL_PIXELS],
            Core::Gb(g) => {
                let (w, h) = g.video_size();
                [w, h]
            }
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

    pub fn cheats(&self) -> &CheatDevice {
        match self {
            Core::Gba(g) => &g.cheats,
            Core::Gb(g) => &g.cheats,
        }
    }

    pub fn cheats_mut(&mut self) -> &mut CheatDevice {
        match self {
            Core::Gba(g) => &mut g.cheats,
            Core::Gb(g) => &mut g.cheats,
        }
    }

    /// An empty cheat set for this system's code formats.
    pub fn new_cheat_set(&self, name: &str) -> CheatSet {
        match self {
            Core::Gba(_) => CheatSet::new_gba(name),
            Core::Gb(_) => CheatSet::new_gb(name),
        }
    }

    /// Load sets from a `.cheats` file (added to any already present).
    pub fn load_cheats(&mut self, text: &str) {
        match self {
            Core::Gba(g) => g.cheats.parse_file(text, CheatSet::new_gba),
            Core::Gb(g) => g.cheats.parse_file(text, CheatSet::new_gb),
        };
    }

    /// Remove a set, undoing any ROM patches it applied.
    pub fn remove_cheat_set(&mut self, index: usize) {
        match self {
            Core::Gba(g) => {
                let mut d = std::mem::take(&mut g.cheats);
                d.remove_set(g.as_mut(), index);
                g.cheats = d;
            }
            Core::Gb(g) => {
                let mut d = std::mem::take(&mut g.cheats);
                d.remove_set(g.as_mut(), index);
                g.cheats = d;
            }
        }
    }
}
