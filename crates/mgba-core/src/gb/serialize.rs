// Save states. mGBA serializes into a fixed C struct (src/gb/serialize.c);
// this port serializes the emulator state directly with serde/bincode instead,
// so states are not interchangeable with mGBA's .ss files.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::Gb;
pub use crate::gba::serialize::StateError;

const MAGIC: &[u8; 8] = b"MGBRSGB\x01";

impl Gb {
    /// Serialize the whole machine (excluding the ROM and boot ROM images).
    pub fn save_state(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(128 * 1024);
        out.extend_from_slice(MAGIC);
        let mut title = [0u8; 4];
        for (i, b) in self.game_title().bytes().take(4).enumerate() {
            title[i] = b;
        }
        out.extend_from_slice(&title);
        out.extend_from_slice(&self.rom_crc32.to_le_bytes());
        bincode::serialize_into(&mut out, self).expect("serializing to memory cannot fail");
        out
    }

    /// Restore a state produced by [`Gb::save_state`] for the currently loaded game.
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), StateError> {
        if data.len() < 16 || &data[..8] != MAGIC {
            return Err(StateError::BadMagic);
        }
        let crc = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
        if crc != self.rom_crc32 {
            return Err(StateError::WrongGame {
                expected: self.game_title(),
                found: String::from_utf8_lossy(&data[8..12]).trim_end_matches('\0').to_string(),
            });
        }
        let mut new: Gb = bincode::deserialize(&data[16..]).map_err(|e| StateError::Corrupt(e.to_string()))?;
        // Carry over what isn't part of the state.
        new.memory.rom = self.memory.rom.clone();
        new.bios = self.bios.take();
        new.cheats = std::mem::take(&mut self.cheats);
        new.sio_driver = self.sio_driver.take();
        new.debugger = self.debugger.take();
        new.log_enabled = self.log_enabled;
        let audio_buffer = std::mem::take(&mut self.audio.buffer);
        *self = new;
        self.audio.buffer = audio_buffer;
        // The link node's state isn't saved; keep its event running so a
        // linked core doesn't stall after loading a state saved unlinked.
        if self.sio_driver.is_some() && !self.timing.is_scheduled(super::GbEvent::SioLockstep) {
            self.schedule(super::GbEvent::SioLockstep, 0);
        }
        Ok(())
    }
}
