// Save states. mGBA serializes into a fixed C struct (src/gba/serialize.c);
// this port serializes the emulator state directly with serde/bincode instead,
// so states are not interchangeable with mGBA's .ss files.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::regs::*;
use super::Gba;

const MAGIC: &[u8; 8] = b"MGBARS\x00\x01";

#[derive(Debug)]
pub enum StateError {
    BadMagic,
    WrongGame { expected: String, found: String },
    Corrupt(String),
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateError::BadMagic => write!(f, "not a save state for this emulator"),
            StateError::WrongGame { expected, found } => {
                write!(f, "state is for game {found:?}, but {expected:?} is loaded")
            }
            StateError::Corrupt(e) => write!(f, "corrupt save state: {e}"),
        }
    }
}

impl std::error::Error for StateError {}

impl Gba {
    /// Serialize the whole machine (excluding ROM and BIOS images).
    pub fn save_state(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(512 * 1024);
        out.extend_from_slice(MAGIC);
        let code = self.game_code();
        let mut code_bytes = [0u8; 4];
        for (i, b) in code.bytes().take(4).enumerate() {
            code_bytes[i] = b;
        }
        out.extend_from_slice(&code_bytes);
        out.extend_from_slice(&self.rom_crc32.to_le_bytes());
        bincode::serialize_into(&mut out, self).expect("serializing to memory cannot fail");
        out
    }

    /// Restore a state produced by [`Gba::save_state`] for the currently loaded game.
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), StateError> {
        if data.len() < 16 || &data[..8] != MAGIC {
            return Err(StateError::BadMagic);
        }
        let crc = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
        if crc != self.rom_crc32 {
            return Err(StateError::WrongGame {
                expected: self.game_code(),
                found: String::from_utf8_lossy(&data[8..12]).to_string(),
            });
        }
        let mut new: Gba = bincode::deserialize(&data[16..]).map_err(|e| StateError::Corrupt(e.to_string()))?;

        // Carry over what isn't part of the state.
        new.memory.rom = self.memory.rom.clone();
        new.memory.bios = std::mem::take(&mut self.memory.bios);
        new.memory.full_bios = self.memory.full_bios;
        new.log_enabled = self.log_enabled;
        new.cheats = std::mem::take(&mut self.cheats);
        new.sio_driver = self.sio_driver.take();
        new.debugger = self.debugger.take();
        let mut renderer = std::mem::take(&mut self.renderer);
        let audio_buffer = std::mem::take(&mut self.audio.buffer);
        *self = new;
        self.audio.buffer = audio_buffer;

        // Rebuild the renderer from video memory and registers, like GBAVideoAssociateRenderer.
        renderer.reset();
        self.renderer = renderer;
        self.renderer.sync_palette(&self.video.palette);
        let dispcnt = self.memory.io[(DISPCNT >> 1) as usize];
        self.renderer.write_video_register(DISPCNT, dispcnt);
        let stereo = self.memory.io[(STEREOCNT >> 1) as usize];
        self.renderer.write_video_register(STEREOCNT, stereo);
        let mut address = BG0CNT;
        while address < 0x56 {
            if address != 0x4E {
                let v = self.memory.io[(address >> 1) as usize];
                self.renderer.write_video_register(address, v);
            }
            address += 2;
        }
        self.renderer.write_oam(0);
        // The link driver's state isn't saved; keep its event running so a
        // linked core doesn't stall after loading a state saved unlinked.
        if self.sio_driver.is_some() && !self.timing.is_scheduled(crate::timing::EventId::SioLockstep) {
            self.schedule(crate::timing::EventId::SioLockstep, 0);
        }
        Ok(())
    }
}
