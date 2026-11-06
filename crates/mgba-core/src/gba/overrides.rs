// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's per-game overrides (src/gba/overrides.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::gpio::{HW_EREADER, HW_GYRO, HW_LIGHT_SENSOR, HW_NONE, HW_NO_OVERRIDE, HW_RTC, HW_RUMBLE, HW_TILT};
use super::savedata::SavedataType;
use super::{Gba, IdleLoopOptimization, IDLE_LOOP_NONE};

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub struct Override {
    pub id: [u8; 4],
    pub savetype: SavedataType,
    pub hardware: u32,
    pub idle_loop: u32,
    pub vba_bug_compat: bool,
}

pub static OVERRIDES: &[Override] = &[
    Override { id: *b"AWRE", savetype: SavedataType::Flash512, hardware: HW_NONE, idle_loop: 0x8038810, vba_bug_compat: false },
    Override { id: *b"AWRP", savetype: SavedataType::Flash512, hardware: HW_NONE, idle_loop: 0x8038810, vba_bug_compat: false },
    Override { id: *b"AW2E", savetype: SavedataType::Flash512, hardware: HW_NONE, idle_loop: 0x8036E08, vba_bug_compat: false },
    Override { id: *b"AW2P", savetype: SavedataType::Flash512, hardware: HW_NONE, idle_loop: 0x803719C, vba_bug_compat: false },
    Override { id: *b"U3IJ", savetype: SavedataType::Eeprom, hardware: HW_RTC | HW_LIGHT_SENSOR, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"U3IE", savetype: SavedataType::Eeprom, hardware: HW_RTC | HW_LIGHT_SENSOR, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"U3IP", savetype: SavedataType::Eeprom, hardware: HW_RTC | HW_LIGHT_SENSOR, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"U32J", savetype: SavedataType::Eeprom, hardware: HW_RTC | HW_LIGHT_SENSOR, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"U32E", savetype: SavedataType::Eeprom, hardware: HW_RTC | HW_LIGHT_SENSOR, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"U32P", savetype: SavedataType::Eeprom, hardware: HW_RTC | HW_LIGHT_SENSOR, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AC8J", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AC8E", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AC8P", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BDKJ", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"ALGP", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"ALFJ", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"ALFE", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"ALFP", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BDBE", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BDBP", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"V49J", savetype: SavedataType::Sram, hardware: HW_RUMBLE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"V49E", savetype: SavedataType::Sram, hardware: HW_RUMBLE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"V49P", savetype: SavedataType::Sram, hardware: HW_RUMBLE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"PEAJ", savetype: SavedataType::Flash1M, hardware: HW_EREADER, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"PSAJ", savetype: SavedataType::Flash1M, hardware: HW_EREADER, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"PSAE", savetype: SavedataType::Flash1M, hardware: HW_EREADER, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AFXE", savetype: SavedataType::Flash512, hardware: HW_NONE, idle_loop: 0x8000428, vba_bug_compat: false },
    Override { id: *b"BFTJ", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"2GBP", savetype: SavedataType::Sram, hardware: HW_RUMBLE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AI2E", savetype: SavedataType::ForceNone, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AI2P", savetype: SavedataType::ForceNone, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BGWJ", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AGFE", savetype: SavedataType::Flash512, hardware: HW_NONE, idle_loop: 0x801353A, vba_bug_compat: false },
    Override { id: *b"KHPJ", savetype: SavedataType::Eeprom, hardware: HW_TILT, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BLJJ", savetype: SavedataType::Flash512, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BLJK", savetype: SavedataType::Flash512, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BLVJ", savetype: SavedataType::Flash512, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AREE", savetype: SavedataType::Sram, hardware: HW_NONE, idle_loop: 0x800032E, vba_bug_compat: false },
    Override { id: *b"AZCE", savetype: SavedataType::Sram, hardware: HW_NONE, idle_loop: 0x80004E8, vba_bug_compat: false },
    Override { id: *b"BSME", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: 0x8000290, vba_bug_compat: false },
    Override { id: *b"AXVJ", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AXVE", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AXVP", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AXVI", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AXVS", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AXVD", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AXVF", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AXPJ", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AXPE", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AXPP", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AXPI", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AXPS", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AXPD", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AXPF", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BPEJ", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: 0x80008C6, vba_bug_compat: false },
    Override { id: *b"BPEE", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: 0x80008C6, vba_bug_compat: false },
    Override { id: *b"BPEP", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: 0x80008C6, vba_bug_compat: false },
    Override { id: *b"BPEI", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: 0x80008C6, vba_bug_compat: false },
    Override { id: *b"BPES", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: 0x80008C6, vba_bug_compat: false },
    Override { id: *b"BPED", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: 0x80008C6, vba_bug_compat: false },
    Override { id: *b"BPEF", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: 0x80008C6, vba_bug_compat: false },
    Override { id: *b"B24E", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"B24P", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BPRJ", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BPRE", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BPRP", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BPRI", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BPRS", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BPRD", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BPRF", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BPGJ", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BPGE", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BPGP", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BPGI", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BPGS", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BPGD", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BPGF", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BR4J", savetype: SavedataType::Flash512, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AR8E", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AROP", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BKAJ", savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"U33J", savetype: SavedataType::Eeprom, hardware: HW_RTC | HW_LIGHT_SENSOR, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"ASLE", savetype: SavedataType::ForceNone, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"ASLF", savetype: SavedataType::ForceNone, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"AA2J", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: 0x800052E, vba_bug_compat: false },
    Override { id: *b"AA2E", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: 0x800052E, vba_bug_compat: false },
    Override { id: *b"AA2P", savetype: SavedataType::Autodetect, hardware: HW_NONE, idle_loop: 0x800052E, vba_bug_compat: false },
    Override { id: *b"A3AJ", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: 0x8002B9C, vba_bug_compat: false },
    Override { id: *b"A3AE", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: 0x8002B9C, vba_bug_compat: false },
    Override { id: *b"A3AP", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: 0x8002B9C, vba_bug_compat: false },
    Override { id: *b"AX4J", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: 0x800072A, vba_bug_compat: false },
    Override { id: *b"AX4E", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: 0x800072A, vba_bug_compat: false },
    Override { id: *b"AX4P", savetype: SavedataType::Flash1M, hardware: HW_NONE, idle_loop: 0x800072A, vba_bug_compat: false },
    Override { id: *b"ALUE", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"ALUP", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"A2YE", savetype: SavedataType::ForceNone, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"BUHJ", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"RZWJ", savetype: SavedataType::Sram, hardware: HW_RUMBLE | HW_GYRO, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"RZWE", savetype: SavedataType::Sram, hardware: HW_RUMBLE | HW_GYRO, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"RZWP", savetype: SavedataType::Sram, hardware: HW_RUMBLE | HW_GYRO, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"KYGJ", savetype: SavedataType::Eeprom, hardware: HW_TILT, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"KYGE", savetype: SavedataType::Eeprom, hardware: HW_TILT, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"KYGP", savetype: SavedataType::Eeprom, hardware: HW_TILT, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
    Override { id: *b"TCHK", savetype: SavedataType::Eeprom, hardware: HW_NONE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false },
];

const POKEMON_TABLE: [u32; 17] = [
    // Emerald
    0x4881F3F8, 0x8C4D3108, 0x1F1C08FB, 0x34C9DF89, 0xA3FDCCB1, 0xA0AEC80A,
    // FireRed
    0x1A81EEDF, 0x3B2056E9, 0x5DC668F6, 0x73A72167, 0x84EE4776, 0x9F08064E, 0xBB640DF7, 0xDD88761C,
    // Ruby
    0x61641576, 0xAEAC73E6, 0xF0815EE7,
];

/// `GBAOverrideFind` (built-in table only)
pub fn find(id: [u8; 4]) -> Option<Override> {
    if let Some(o) = OVERRIDES.iter().find(|o| o.id == id) {
        return Some(*o);
    }
    if id[0] == b'F' {
        // Classic NES Series
        return Some(Override { id, savetype: SavedataType::Eeprom, hardware: HW_NO_OVERRIDE, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: false });
    }
    None
}

impl Gba {
    /// `GBAOverrideApply`
    pub fn apply_override(&mut self, o: &Override) {
        if o.savetype != SavedataType::Autodetect {
            self.memory.savedata.force_type(o.savetype);
        }
        self.vba_bug_compat = o.vba_bug_compat;
        if o.hardware != HW_NO_OVERRIDE {
            self.memory.hw.clear();
            if o.hardware & HW_RTC != 0 {
                self.memory.hw.init_rtc();
            }
            self.memory.hw.devices |= o.hardware & (HW_GYRO | HW_RUMBLE | HW_LIGHT_SENSOR | HW_TILT);
        }
        if o.idle_loop != IDLE_LOOP_NONE {
            self.idle_loop = o.idle_loop;
            if self.idle_optimization == IdleLoopOptimization::Detect {
                self.idle_optimization = IdleLoopOptimization::Remove;
            }
        }
    }

    /// `GBAOverrideApplyDefaults`
    pub(crate) fn apply_overrides(&mut self) {
        if !self.has_rom || self.memory.rom.len() < 0x120 {
            return;
        }
        if let Some(o) = self.forced_override {
            self.apply_override(&o);
            return;
        }
        let rom = &self.memory.rom;
        let id = [rom[0xAC], rom[0xAD], rom[0xAE], rom[0xAF]];
        let mut is_pokemon = false;
        is_pokemon |= &rom[0x108..0x108 + 19] == b"pokemon red version";
        is_pokemon |= &rom[0x108..0x108 + 23] == b"pokemon emerald version";
        is_pokemon |= &rom[0xAC..0xB0] == b"AXVE";
        let is_known_pokemon = is_pokemon && POKEMON_TABLE.contains(&self.rom_crc32);
        if is_pokemon && !is_known_pokemon {
            // Enable FLASH1M and RTC on Pokémon ROM hacks
            let o = Override { id, savetype: SavedataType::Flash1M, hardware: HW_RTC, idle_loop: IDLE_LOOP_NONE, vba_bug_compat: true };
            self.apply_override(&o);
        } else if let Some(o) = find(id) {
            self.apply_override(&o);
        }
    }
}

#[allow(dead_code)]
const _UNUSED: u32 = HW_NONE;
