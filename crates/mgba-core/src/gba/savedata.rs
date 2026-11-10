// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's cartridge save memory (src/gba/savedata.c).
// Save data lives in memory; the frontend persists it when `dirty` settles.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::memory::{SIZE_EEPROM, SIZE_EEPROM512, SIZE_FLASH1M, SIZE_FLASH512, SIZE_SRAM, SIZE_SRAM512};

pub const FLASH_ERASE_CYCLES: i32 = 30000;
pub const FLASH_PROGRAM_CYCLES: i32 = 650;
pub const EEPROM_SETTLE_CYCLES: i32 = 115000;

pub const DIRT_NEW: u32 = 1;
pub const DIRT_SEEN: u32 = 2;
const CLEANUP_THRESHOLD: u32 = 15;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SavedataType {
    Autodetect,
    ForceNone,
    Sram,
    Flash512,
    Flash1M,
    Eeprom,
    Eeprom512,
    Sram512,
}

impl SavedataType {
    pub fn name(self) -> &'static str {
        match self {
            SavedataType::Autodetect => "Autodetect",
            SavedataType::ForceNone => "None",
            SavedataType::Sram => "SRAM",
            SavedataType::Flash512 => "Flash 512K",
            SavedataType::Flash1M => "Flash 1M",
            SavedataType::Eeprom => "EEPROM 8K",
            SavedataType::Eeprom512 => "EEPROM 512",
            SavedataType::Sram512 => "SRAM 64K",
        }
    }
}

const EEPROM_COMMAND_NULL: u32 = 0;
const EEPROM_COMMAND_PENDING: u32 = 1;
const EEPROM_COMMAND_WRITE: u32 = 2;
const EEPROM_COMMAND_READ_PENDING: u32 = 3;
const EEPROM_COMMAND_READ: u32 = 4;

const FLASH_COMMAND_START: u8 = 0xAA;
const FLASH_COMMAND_CONTINUE: u8 = 0x55;
const FLASH_COMMAND_ERASE_CHIP: u8 = 0x10;
const FLASH_COMMAND_ERASE_SECTOR: u8 = 0x30;
const FLASH_COMMAND_NONE: u32 = 0;
const FLASH_COMMAND_ERASE: u32 = 0x80;
const FLASH_COMMAND_ID: u32 = 0x90;
const FLASH_COMMAND_PROGRAM: u32 = 0xA0;
const FLASH_COMMAND_SWITCH_BANK: u32 = 0xB0;
const FLASH_COMMAND_TERMINATE: u8 = 0xF0;

const FLASH_STATE_RAW: u32 = 0;
const FLASH_STATE_START: u32 = 1;
const FLASH_STATE_CONTINUE: u32 = 2;

const FLASH_BASE_HI: u16 = 0x5555;
const FLASH_BASE_LO: u16 = 0x2AAA;

const FLASH_PANASONIC_MN63F805MNP: u16 = 0x1B32;
const FLASH_SANYO_LE26FV10N1TS: u16 = 0x1362;

/// `struct GBASavedata`
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Savedata {
    pub ty: SavedataType,
    pub data: Vec<u8>,
    pub command: u32,
    pub read_bits_remaining: i8,
    pub read_address: u32,
    pub write_address: u32,
    pub current_bank: usize,
    pub settling: u32,
    pub dirty: u32,
    pub dirt_age: u32,
    pub flash_state: u32,
    /// Existing save loaded before the type was known.
    pub pending: Option<Vec<u8>>,
}

impl Savedata {
    pub fn new() -> Savedata {
        Savedata {
            ty: SavedataType::Autodetect,
            data: Vec::new(),
            command: EEPROM_COMMAND_NULL,
            read_bits_remaining: 0,
            read_address: 0,
            write_address: 0,
            current_bank: 0,
            settling: 0,
            dirty: 0,
            dirt_age: 0,
            flash_state: FLASH_STATE_RAW,
            pending: None,
        }
    }

    /// `GBASavedataReset`
    pub fn reset(&mut self) {
        self.command = EEPROM_COMMAND_NULL;
        self.flash_state = FLASH_STATE_RAW;
    }

    pub fn is_eeprom(&self) -> bool {
        matches!(self.ty, SavedataType::Eeprom | SavedataType::Eeprom512)
    }

    /// Bytes that make up the save file, if a save type has been detected.
    pub fn data(&self) -> Option<&[u8]> {
        let size = self.size();
        if size == 0 || self.data.len() < size {
            None
        } else {
            Some(&self.data[..size])
        }
    }

    /// `GBASavedataSize`
    pub fn size(&self) -> usize {
        (match self.ty {
            SavedataType::Sram => SIZE_SRAM,
            SavedataType::Flash512 => SIZE_FLASH512,
            SavedataType::Flash1M => SIZE_FLASH1M,
            SavedataType::Eeprom => SIZE_EEPROM,
            SavedataType::Eeprom512 => SIZE_EEPROM512,
            SavedataType::Sram512 => SIZE_SRAM512,
            SavedataType::ForceNone | SavedataType::Autodetect => 0,
        }) as usize
    }

    /// Accept an existing save file. If the type isn't known yet, infer it
    /// from the size the way mGBA's `GBASavedataLoad` does.
    pub fn load(&mut self, bytes: &[u8]) {
        if self.ty == SavedataType::Autodetect {
            match bytes.len() as u32 {
                SIZE_FLASH1M => self.force_type(SavedataType::Flash1M),
                SIZE_FLASH512 => self.force_type(SavedataType::Flash512),
                SIZE_SRAM => self.force_type(SavedataType::Sram),
                SIZE_EEPROM => self.force_type(SavedataType::Eeprom),
                SIZE_EEPROM512 => self.force_type(SavedataType::Eeprom512),
                _ => {
                    self.pending = Some(bytes.to_vec());
                    return;
                }
            }
        }
        self.fill_from(bytes);
    }

    fn fill_from(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(self.data.len());
        self.data[..n].copy_from_slice(&bytes[..n]);
    }

    fn apply_pending(&mut self) {
        if let Some(p) = self.pending.take() {
            self.fill_from(&p);
        }
    }

    /// `GBASavedataForceType`
    pub fn force_type(&mut self, ty: SavedataType) {
        if self.ty == ty {
            return;
        }
        if self.ty != SavedataType::Autodetect {
            let pending = self.pending.take();
            *self = Savedata::new();
            self.pending = pending;
        }
        match ty {
            SavedataType::Flash512 | SavedataType::Flash1M => {
                self.ty = ty;
                self.init_flash();
            }
            SavedataType::Eeprom | SavedataType::Eeprom512 => {
                self.ty = ty;
                self.init_eeprom();
            }
            SavedataType::Sram => self.init_sram(),
            SavedataType::Sram512 => self.init_sram512(),
            SavedataType::ForceNone => self.ty = SavedataType::ForceNone,
            SavedataType::Autodetect => {}
        }
    }

    /// `GBASavedataInitFlash`
    pub fn init_flash(&mut self) {
        if self.ty == SavedataType::Autodetect {
            self.ty = SavedataType::Flash512;
        }
        if self.ty != SavedataType::Flash512 && self.ty != SavedataType::Flash1M {
            return;
        }
        self.data = vec![0xFF; SIZE_FLASH1M as usize];
        self.current_bank = 0;
        self.apply_pending();
    }

    /// `GBASavedataInitEEPROM`
    pub fn init_eeprom(&mut self) {
        if self.ty == SavedataType::Autodetect {
            self.ty = SavedataType::Eeprom512;
        } else if self.ty != SavedataType::Eeprom512 && self.ty != SavedataType::Eeprom {
            return;
        }
        self.data = vec![0xFF; SIZE_EEPROM as usize];
        self.apply_pending();
    }

    /// `GBASavedataInitSRAM`
    pub fn init_sram(&mut self) {
        if self.ty == SavedataType::Autodetect {
            self.ty = SavedataType::Sram;
        } else {
            return;
        }
        self.data = vec![0xFF; SIZE_SRAM as usize];
        self.apply_pending();
    }

    /// `GBASavedataInitSRAM512`
    pub fn init_sram512(&mut self) {
        if self.ty == SavedataType::Autodetect {
            self.ty = SavedataType::Sram512;
        } else {
            return;
        }
        self.data = vec![0xFF; SIZE_SRAM512 as usize];
        self.apply_pending();
    }

    /// `GBASavedataReadFlash`. `busy` is whether the settle timer is running.
    pub fn read_flash(&mut self, address: u16, busy: bool) -> u8 {
        if self.command == FLASH_COMMAND_ID {
            if self.ty == SavedataType::Flash512 {
                if address < 2 {
                    return (FLASH_PANASONIC_MN63F805MNP >> (address * 8)) as u8;
                }
            } else if self.ty == SavedataType::Flash1M && address < 2 {
                return (FLASH_SANYO_LE26FV10N1TS >> (address * 8)) as u8;
            }
        }
        let v = self.data[self.current_bank + address as usize];
        if busy && (address as u32 >> 12) == self.settling {
            // This should read Q7 XOR data bit 7 (data# polling), Q6 flipping
            // every read (toggle bit), and /Q5 (error bit cleared), but implementing
            // just data# polling is sufficient for games to figure it out
            return (v ^ 0x80) & 0x80;
        }
        v
    }

    /// `GBASavedataWriteFlash`. Returns cycles to schedule the settle timer for, if any.
    pub fn write_flash(&mut self, address: u16, value: u8) -> Option<i32> {
        let mut settle = None;
        match self.flash_state {
            FLASH_STATE_RAW => match self.command {
                FLASH_COMMAND_PROGRAM => {
                    self.dirty |= DIRT_NEW;
                    self.data[self.current_bank + address as usize] = value;
                    self.command = FLASH_COMMAND_NONE;
                    settle = Some(FLASH_PROGRAM_CYCLES);
                }
                FLASH_COMMAND_SWITCH_BANK => {
                    if address == 0 && value < 2 {
                        self.flash_switch_bank(value as usize);
                    }
                    self.command = FLASH_COMMAND_NONE;
                }
                _ => {
                    if address == FLASH_BASE_HI && value == FLASH_COMMAND_START {
                        self.flash_state = FLASH_STATE_START;
                    }
                }
            },
            FLASH_STATE_START => {
                if address == FLASH_BASE_LO && value == FLASH_COMMAND_CONTINUE {
                    self.flash_state = FLASH_STATE_CONTINUE;
                } else {
                    self.flash_state = FLASH_STATE_RAW;
                }
            }
            _ => {
                self.flash_state = FLASH_STATE_RAW;
                if address == FLASH_BASE_HI {
                    match self.command {
                        FLASH_COMMAND_NONE => {
                            let v = value as u32;
                            if matches!(v, FLASH_COMMAND_ERASE | FLASH_COMMAND_ID | FLASH_COMMAND_PROGRAM | FLASH_COMMAND_SWITCH_BANK) {
                                self.command = v;
                            }
                        }
                        FLASH_COMMAND_ERASE => {
                            if value == FLASH_COMMAND_ERASE_CHIP {
                                self.flash_erase();
                            }
                            self.command = FLASH_COMMAND_NONE;
                        }
                        FLASH_COMMAND_ID => {
                            if value == FLASH_COMMAND_TERMINATE {
                                self.command = FLASH_COMMAND_NONE;
                            }
                        }
                        _ => self.command = FLASH_COMMAND_NONE,
                    }
                } else if self.command == FLASH_COMMAND_ERASE && value == FLASH_COMMAND_ERASE_SECTOR {
                    self.flash_erase_sector(address);
                    self.command = FLASH_COMMAND_NONE;
                    settle = Some(FLASH_ERASE_CYCLES);
                }
            }
        }
        settle
    }

    pub(crate) fn flash_switch_bank(&mut self, bank: usize) {
        if bank > 0 && self.ty == SavedataType::Flash512 {
            // Updating flash chip from 512kb to 1Mb
            self.ty = SavedataType::Flash1M;
        }
        self.current_bank = bank << 16;
    }

    fn flash_erase(&mut self) {
        self.dirty |= DIRT_NEW;
        let size = if self.ty == SavedataType::Flash1M { SIZE_FLASH1M } else { SIZE_FLASH512 } as usize;
        self.data[..size].fill(0xFF);
    }

    fn flash_erase_sector(&mut self, sector_start: u16) {
        self.dirty |= DIRT_NEW;
        let size = 0x1000usize;
        self.settling = (sector_start >> 12) as u32;
        let start = self.current_bank + (sector_start as usize & !(size - 1));
        self.data[start..start + size].fill(0xFF);
    }

    fn ensure_eeprom(&mut self, size: u32) {
        if size < SIZE_EEPROM512 {
            return;
        }
        if self.ty == SavedataType::Eeprom {
            return;
        }
        self.ty = SavedataType::Eeprom;
    }

    /// `GBASavedataWriteEEPROM`. Returns true if the settle timer should be (re)started.
    pub fn write_eeprom(&mut self, value: u16, write_size: u32) -> bool {
        match self.command {
            EEPROM_COMMAND_PENDING => {
                self.command <<= 1;
                self.command |= (value & 1) as u32;
                if self.command == EEPROM_COMMAND_WRITE {
                    self.write_address = 0;
                } else {
                    self.read_address = 0;
                }
            }
            EEPROM_COMMAND_WRITE => {
                if write_size > 65 {
                    self.write_address <<= 1;
                    self.write_address |= ((value & 1) as u32) << 6;
                } else if write_size == 1 {
                    self.command = EEPROM_COMMAND_NULL;
                } else if (self.write_address >> 3) < SIZE_EEPROM {
                    self.ensure_eeprom(self.write_address >> 3);
                    let idx = (self.write_address >> 3) as usize;
                    let bit = 0x7 - (self.write_address & 0x7);
                    let mut current = self.data[idx];
                    current &= !(1 << bit);
                    current |= ((value & 1) as u8) << bit;
                    self.dirty |= DIRT_NEW;
                    self.data[idx] = current;
                    self.write_address += 1;
                    return true;
                }
            }
            EEPROM_COMMAND_READ_PENDING => {
                if write_size > 1 {
                    self.read_address <<= 1;
                    if value & 1 != 0 {
                        self.read_address |= 0x40;
                    }
                } else {
                    self.read_bits_remaining = 68;
                    self.command = EEPROM_COMMAND_READ;
                }
            }
            _ => {
                self.command = (value & 1) as u32;
            }
        }
        false
    }

    /// `GBASavedataReadEEPROM`
    pub fn read_eeprom(&mut self, busy: bool) -> u16 {
        if self.command != EEPROM_COMMAND_READ {
            return if !busy { 1 } else { 0 };
        }
        self.read_bits_remaining -= 1;
        if self.read_bits_remaining < 64 {
            let step = 63 - self.read_bits_remaining as u32;
            let address = (self.read_address + step) >> 3;
            self.ensure_eeprom(address);
            if address >= SIZE_EEPROM {
                return 0xFF;
            }
            let data = self.data[address as usize] >> (0x7 - (step & 0x7));
            if self.read_bits_remaining == 0 {
                self.command = EEPROM_COMMAND_NULL;
            }
            return (data & 1) as u16;
        }
        0
    }

    /// `GBASavedataClean`: returns true when a burst of writes has settled.
    pub fn clean(&mut self, frame_count: u32) -> bool {
        if self.dirty & DIRT_NEW != 0 {
            self.dirt_age = frame_count;
            self.dirty &= !DIRT_NEW;
            self.dirty |= DIRT_SEEN;
        } else if self.dirty & DIRT_SEEN != 0 && frame_count.wrapping_sub(self.dirt_age) > CLEANUP_THRESHOLD {
            self.dirty = 0;
            return true;
        }
        false
    }
}

impl Default for Savedata {
    fn default() -> Self {
        Self::new()
    }
}
