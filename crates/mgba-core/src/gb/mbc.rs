// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's Game Boy memory bank controllers
// (src/gb/mbc.c, src/gb/mbc/mbc.c, src/gb/mbc/licensed.c).
//
// HuC-3, TAMA5, the Pocket Camera and the unlicensed mappers live in
// mbc_extra.rs. GBX files and their mapper variables aren't supported.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::mbc_extra::*;
use super::memory::*;
use super::{crc32, glog, Gb, LogLevel};

pub const MBC_AUTODETECT: i32 = -1;
pub const MBC_NONE: i32 = 0;
pub const MBC1: i32 = 1;
pub const MBC2: i32 = 2;
pub const MBC3: i32 = 3;
pub const MBC5: i32 = 5;
pub const MBC6: i32 = 6;
pub const MBC7: i32 = 7;
pub const MMM01: i32 = 0x10;
pub const HUC1: i32 = 0x11;
pub const HUC3: i32 = 0x12;
pub const POCKETCAM: i32 = 0x13;
pub const TAMA5: i32 = 0x14;
pub const M161: i32 = 0x15;
pub const MBC3_RTC: i32 = 0x103;
pub const MBC5_RUMBLE: i32 = 0x105;
pub const UNL_WISDOM_TREE: i32 = 0x200;
pub const UNL_PKJD: i32 = 0x203;
pub const UNL_NT_OLD_1: i32 = 0x210;
pub const UNL_NT_OLD_2: i32 = 0x211;
pub const UNL_NT_NEW: i32 = 0x212;
pub const UNL_BBD: i32 = 0x220;
pub const UNL_HITEK: i32 = 0x221;
pub const UNL_LI_CHENG: i32 = 0x222;
pub const UNL_GGB81: i32 = 0x223;
pub const UNL_SACHEN_MMC1: i32 = 0x230;
pub const UNL_SACHEN_MMC2: i32 = 0x231;
pub const UNL_SINTAX: i32 = 0x240;

const LOGO_HASH: u32 = 0x46195417;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct Mbc1State {
    pub mode: i32,
    pub multicart_stride: i32,
    pub bank_lo: u8,
    pub bank_hi: u8,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct Mbc6State {
    pub flash_bank0: bool,
    pub flash_bank1: bool,
}

pub const MBC7_STATE_IDLE: u8 = 0;
pub const MBC7_STATE_READ_COMMAND: u8 = 1;
pub const MBC7_STATE_DO: u8 = 2;
pub const MBC7_STATE_EEPROM_EWDS: u8 = 0x10;
pub const MBC7_STATE_EEPROM_WRAL: u8 = 0x11;
pub const MBC7_STATE_EEPROM_ERAL: u8 = 0x12;
pub const MBC7_STATE_EEPROM_EWEN: u8 = 0x13;
pub const MBC7_STATE_EEPROM_WRITE: u8 = 0x14;
pub const MBC7_STATE_EEPROM_READ: u8 = 0x18;
pub const MBC7_STATE_EEPROM_ERASE: u8 = 0x1C;

const MBC7_CS: u8 = 0x80;
const MBC7_CLK: u8 = 0x40;
const MBC7_DI: u8 = 0x02;
const MBC7_DO: u8 = 0x01;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct Mbc7State {
    pub state: u8,
    pub sr: u16,
    pub address: u8,
    pub writable: bool,
    pub sr_bits: i32,
    pub access: u8,
    pub latch: u8,
    pub eeprom: u8,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct Mmm01State {
    pub locked: bool,
    pub current_bank0: i32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct M161State {
    pub locked: bool,
    pub bank: u8,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct NtNewState {
    pub split_mode: bool,
}

/// `union GBMBCState` (as a struct; only the active member is meaningful)
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct MbcState {
    pub mbc1: Mbc1State,
    pub mbc6: Mbc6State,
    pub mbc7: Mbc7State,
    pub mmm01: Mmm01State,
    pub m161: M161State,
    pub nt_new: NtNewState,
    pub huc3: HuC3State,
    pub tama5: Tama5State,
    pub pocket_cam: PocketCamState,
    pub pkjd: PkjdState,
    pub nt_old: NtOldState,
    pub bbd: BbdState,
    pub sachen: SachenState,
    pub sintax: SintaxState,
}

fn to_pow2(x: usize) -> usize {
    if x <= 1 {
        1
    } else {
        x.next_power_of_two()
    }
}

/// Cartridge header field offsets (relative to 0x100, `struct GBCartridge`).
pub mod cart {
    pub const LOGO: usize = 0x04;
    pub const CGB: usize = 0x43;
    pub const SGB: usize = 0x46;
    pub const TYPE: usize = 0x47;
    pub const ROM_SIZE: usize = 0x48;
    pub const RAM_SIZE: usize = 0x49;
    pub const OLD_LICENSEE: usize = 0x4B;
    pub const VERSION: usize = 0x4C;
}

impl Gb {
    /// `GBMBCSwitchBank`
    pub fn mbc_switch_bank(&mut self, bank: i32) {
        let mut bank = bank;
        let mut bank_start = (bank as usize).wrapping_mul(SIZE_CART_BANK0);
        let rom_size = self.memory.rom_size;
        if bank_start + SIZE_CART_BANK0 > rom_size {
            glog!(self, LogLevel::GameError, "Attempting to switch to an invalid ROM bank: {:X}", bank);
            if rom_size == 0 {
                return;
            }
            bank_start &= to_pow2(rom_size) - 1;
            if bank_start + SIZE_CART_BANK0 > rom_size {
                return;
            }
            bank = (bank_start / SIZE_CART_BANK0) as i32;
        }
        self.memory.rom_bank = BankPtr { off: bank_start, flash: false };
        self.memory.current_bank = bank;
        if self.cpu.pc < BASE_VRAM {
            let pc = self.cpu.pc;
            self.set_active_region_impl(pc);
        }
    }

    /// `GBMBCSwitchBank0`
    pub fn mbc_switch_bank0(&mut self, bank: i32) {
        let mut bank = bank;
        let mut bank_start = (bank as usize).wrapping_mul(SIZE_CART_BANK0);
        let rom_size = self.memory.rom_size;
        if bank_start + SIZE_CART_BANK0 > rom_size {
            glog!(self, LogLevel::GameError, "Attempting to switch to an invalid ROM bank: {:X}", bank);
            if rom_size == 0 {
                return;
            }
            bank_start &= to_pow2(rom_size) - 1;
            if bank_start + SIZE_CART_BANK0 > rom_size {
                return;
            }
            bank = (bank_start / SIZE_CART_BANK0) as i32;
        }
        self.memory.rom_base = bank_start;
        self.memory.bios_mapped = false;
        self.memory.current_bank0 = bank;
        if (self.cpu.pc as usize) < SIZE_CART_BANK0 {
            let pc = self.cpu.pc;
            self.set_active_region_impl(pc);
        }
    }

    /// `GBMBCSwitchHalfBank`
    pub fn mbc_switch_half_bank(&mut self, half: i32, bank: i32) {
        let mut bank = bank;
        let mut bank_start = (bank as usize).wrapping_mul(SIZE_CART_HALFBANK);
        let mut is_flash = false;
        if self.memory.mbc_type == MBC6 {
            is_flash = if half != 0 { self.memory.mbc_state.mbc6.flash_bank1 } else { self.memory.mbc_state.mbc6.flash_bank0 };
        }
        if is_flash {
            if bank_start + SIZE_CART_HALFBANK > SIZE_MBC6_FLASH {
                bank_start &= SIZE_MBC6_FLASH - 1;
                bank = (bank_start / SIZE_CART_HALFBANK) as i32;
            }
            bank_start += self.sram_size - SIZE_MBC6_FLASH;
        } else {
            let rom_size = self.memory.rom_size;
            if bank_start + SIZE_CART_HALFBANK > rom_size {
                if rom_size == 0 {
                    return;
                }
                bank_start &= to_pow2(rom_size) - 1;
                if bank_start + SIZE_CART_HALFBANK > rom_size {
                    return;
                }
                bank = (bank_start / SIZE_CART_HALFBANK) as i32;
                if bank == 0 {
                    bank += 1;
                }
            }
        }
        let ptr = BankPtr { off: bank_start, flash: is_flash };
        if half == 0 {
            self.memory.rom_bank = ptr;
            self.memory.current_bank = bank;
        } else {
            self.memory.rom_bank1 = ptr;
            self.memory.current_bank1 = bank;
        }
        if self.cpu.pc < BASE_VRAM {
            let pc = self.cpu.pc;
            self.set_active_region_impl(pc);
        }
    }

    /// `GBMBCSwitchSramBank`
    pub fn mbc_switch_sram_bank(&mut self, bank: i32) {
        let mut bank = bank;
        let mut bank_start = (bank as usize).wrapping_mul(SIZE_EXTERNAL_RAM);
        let sram_size = self.sram_size;
        if bank_start + SIZE_EXTERNAL_RAM > sram_size {
            if sram_size == 0 {
                return;
            }
            if sram_size < SIZE_EXTERNAL_RAM {
                bank = 0;
            } else {
                bank_start &= to_pow2(sram_size) - 1;
                if bank_start + SIZE_EXTERNAL_RAM > sram_size {
                    bank = 0;
                } else {
                    bank = (bank_start / SIZE_EXTERNAL_RAM) as i32;
                }
            }
        }
        // mGBA keeps the (possibly out-of-range) bankStart pointer here.
        self.memory.sram_bank = bank_start;
        self.memory.sram_current_bank = bank;
    }

    /// `GBMBCSwitchSramHalfBank`
    pub fn mbc_switch_sram_half_bank(&mut self, half: i32, bank: i32) {
        let mut bank = bank;
        let mut bank_start = (bank as isize) * SIZE_EXTERNAL_RAM_HALFBANK as isize;
        let sram_size = self.sram_size as isize - SIZE_MBC6_FLASH as isize;
        if bank_start + SIZE_EXTERNAL_RAM_HALFBANK as isize > sram_size {
            if sram_size <= 0 {
                return;
            }
            if sram_size < SIZE_EXTERNAL_RAM_HALFBANK as isize {
                bank = 0;
            } else {
                bank_start &= to_pow2(self.sram_size) as isize - 1;
                if bank_start + SIZE_EXTERNAL_RAM_HALFBANK as isize > self.sram_size as isize {
                    bank = 0;
                } else {
                    bank = (bank_start / SIZE_EXTERNAL_RAM_HALFBANK as isize) as i32;
                }
            }
        }
        if half == 0 {
            self.memory.sram_bank = bank_start as usize;
            self.memory.sram_current_bank = bank;
        } else {
            self.memory.sram_bank1 = bank_start as usize;
            self.memory.current_sram_bank1 = bank;
        }
    }

    /// `_isMulticart`
    fn is_multicart(&self) -> bool {
        let rom = &self.memory.rom;
        let check = |bank: usize| -> bool {
            let start = SIZE_CART_BANK0 * bank;
            rom.len() >= start + 0x200 && is_rom_header(&rom[start..])
        };
        if !check(0x10) {
            return false;
        }
        check(0x20) || check(0x30)
    }

    /// `GBMBCInit`
    pub fn mbc_init(&mut self) {
        let mut sram_size = 0usize;
        let mut cart_type = 0u8;
        if !self.memory.rom.is_empty() && self.memory.rom_size != 0 {
            let rom = self.memory.rom.clone();
            let mut header = 0x100usize;
            let rom_size = self.memory.rom_size;
            if rom_size >= 0x8000 {
                let footer = rom_size - 0x7F00;
                if footer + 0x50 <= rom.len() {
                    let t = rom[footer + cart::TYPE];
                    if crc32(&rom[footer + cart::LOGO..footer + cart::LOGO + 48]) == LOGO_HASH && (0x0B..=0x0D).contains(&t) {
                        header = footer;
                    }
                }
            }
            cart_type = rom.get(header + cart::TYPE).copied().unwrap_or(0);
            sram_size = match rom.get(header + cart::RAM_SIZE).copied().unwrap_or(0) {
                0 => 0,
                3 => 0x8000,
                4 => 0x20000,
                5 => 0x10000,
                _ => 0x2000,
            };

            if self.memory.mbc_type == MBC_AUTODETECT {
                self.memory.mbc_type = detect_unl_mbc(&rom, rom_size);
            }
            if self.memory.mbc_type == MBC_AUTODETECT {
                self.memory.mbc_type = match cart_type {
                    0 | 8 | 9 => MBC_NONE,
                    1..=3 => MBC1,
                    5 | 6 => MBC2,
                    0x0B..=0x0D => MMM01,
                    0x0F | 0x10 => MBC3_RTC,
                    0x11..=0x13 => MBC3,
                    0x19..=0x1B => MBC5,
                    0x1C..=0x1E => MBC5_RUMBLE,
                    0x20 => MBC6,
                    0x22 => MBC7,
                    0xFC => POCKETCAM,
                    0xFD => TAMA5,
                    0xFE => HUC3,
                    0xFF => HUC1,
                    _ => {
                        glog!(self, LogLevel::Warn, "Unknown MBC type: {:02X}", cart_type);
                        MBC5
                    }
                };
            }
        } else {
            self.memory.mbc_type = MBC_NONE;
        }
        let mem = &mut self.memory;
        mem.has_mbc_read = false;
        mem.direct_sram_access = true;
        mem.mbc_read_bank0 = false;
        mem.mbc_read_bank1 = false;
        mem.mbc_read_high = false;
        mem.mbc_write_high = false;
        mem.cart_bus_decay = 4;
        match mem.mbc_type {
            MBC1 => {
                let stride = if mem.rom_size >= SIZE_CART_BANK0 * 0x31 && self.is_multicart() { 4 } else { 5 };
                self.memory.mbc_state.mbc1.multicart_stride = stride;
            }
            MBC2 => {
                mem.has_mbc_read = true;
                mem.direct_sram_access = false;
                sram_size = 0x100;
            }
            MBC6 => {
                mem.has_mbc_read = true;
                mem.direct_sram_access = false;
                if sram_size == 0 {
                    sram_size = SIZE_EXTERNAL_RAM; // Force minimum size for convenience
                }
                sram_size += SIZE_MBC6_FLASH; // Flash is concatenated at the end
            }
            MBC7 => {
                mem.has_mbc_read = true;
                sram_size = 0x100;
            }
            MBC3_RTC => {
                mem.rtc_regs = [0; 5];
            }
            HUC3 => {
                mem.has_mbc_read = true;
            }
            TAMA5 => {
                mem.has_mbc_read = true;
                mem.mbc_state.tama5.rtc_alarm_page[RTC_PAGE] = 1;
                mem.mbc_state.tama5.rtc_free_page0[RTC_PAGE] = 2;
                mem.mbc_state.tama5.rtc_free_page1[RTC_PAGE] = 3;
                sram_size = 0x20;
            }
            POCKETCAM => {
                mem.has_mbc_read = true;
                if sram_size == 0 {
                    sram_size = SIZE_EXTERNAL_RAM; // Force minimum size for convenience
                }
            }
            UNL_PKJD => {
                mem.has_mbc_read = true;
            }
            UNL_BBD | UNL_GGB81 => {
                mem.has_mbc_read = true;
                mem.mbc_read_bank1 = true;
            }
            UNL_HITEK => {
                mem.has_mbc_read = true;
                mem.mbc_state.bbd.data_swap_mode = 7;
                mem.mbc_state.bbd.bank_swap_mode = 7;
                mem.mbc_read_bank1 = true;
            }
            UNL_SACHEN_MMC1 => {
                mem.has_mbc_read = true;
                mem.mbc_read_bank0 = true;
                mem.mbc_read_bank1 = true;
            }
            UNL_SACHEN_MMC2 => {
                mem.has_mbc_read = true;
                mem.mbc_read_bank0 = true;
                mem.mbc_read_bank1 = true;
                mem.mbc_read_high = true;
                mem.mbc_write_high = true;
            }
            UNL_SINTAX => {
                mem.has_mbc_read = true;
                mem.mbc_read_bank1 = true;
            }
            MBC_NONE | MBC3 | MBC5 | MBC5_RUMBLE | MMM01 | HUC1 | M161 | UNL_WISDOM_TREE | UNL_NT_OLD_1
            | UNL_NT_OLD_2 | UNL_NT_NEW | UNL_LI_CHENG => {}
            _ => {
                glog!(self, LogLevel::Warn, "Unknown MBC type: {:02X}", cart_type);
            }
        }

        let now = self.rtc_unix_time();
        let mem = &mut self.memory;
        mem.current_bank = 1;
        mem.sram_current_bank = 0;
        mem.sram_access = false;
        mem.rtc_access = false;
        mem.active_rtc_reg = 0;
        mem.rtc_latched = false;
        mem.rtc_last_latch = now;
        mem.rtc_regs = [0; 5];

        self.resize_sram(sram_size);

        // GBMBCRTCRead / GBMBCHuC3Read / GBMBCTAMA5Read from the save file's suffix
        if !self.rtc_file.is_empty() {
            let file = std::mem::take(&mut self.rtc_file);
            match self.memory.mbc_type {
                MBC3_RTC => self.rtc_load_suffix(&file),
                HUC3 => self.huc3_load_suffix(&file),
                TAMA5 => self.tama5_load_suffix(&file),
                _ => {}
            }
            self.rtc_file = file;
        }
    }

    /// `GBResizeSram` (memory-backed; the frontend owns the save file)
    pub fn resize_sram(&mut self, size: usize) {
        if !self.memory.sram.is_empty() && size <= self.sram_size {
            return;
        }
        if size != 0 {
            let mut new_sram = vec![0xFFu8; size];
            if !self.memory.sram.is_empty() {
                let n = self.sram_size.min(size).min(self.memory.sram.len());
                new_sram[..n].copy_from_slice(&self.memory.sram[..n]);
            } else {
                self.sram_size = size;
            }
            self.memory.sram = new_sram;
        }
        if self.sram_size < size {
            self.sram_size = size;
        }
    }

    /// `GBMBCReset`
    pub(crate) fn mbc_reset(&mut self) {
        let mem = &mut self.memory;
        mem.current_bank0 = 0;
        mem.rom_bank = BankPtr { off: SIZE_CART_BANK0, flash: false };
        mem.cart_bus = 0xFF;
        mem.cart_bus_pc = 0;
        mem.cart_bus_decay = 1;
        mem.last_rumble = 0;
        mem.mbc_state = MbcState::default();
        self.mbc_init();
        match self.memory.mbc_type {
            MBC1 => {
                self.memory.mbc_state.mbc1.mode = 0;
                self.memory.mbc_state.mbc1.bank_lo = 1;
            }
            MBC6 => {
                self.mbc_switch_half_bank(0, 2);
                self.mbc_switch_half_bank(1, 3);
                self.mbc_switch_sram_half_bank(0, 0);
                self.mbc_switch_sram_half_bank(0, 1);
            }
            MMM01 => {
                let banks = (self.memory.rom_size / SIZE_CART_BANK0) as i32;
                self.mbc_switch_bank0(banks - 2);
                self.mbc_switch_bank(banks - 1);
            }
            _ => {}
        }
        self.memory.sram_bank = 0;
    }

    /// `memory->mbcWrite`
    pub(crate) fn mbc_write(&mut self, address: u16, value: u8) {
        match self.memory.mbc_type {
            MBC_NONE => {
                if self.yanked_rom_size == 0 {
                    glog!(self, LogLevel::GameError, "Wrote to invalid MBC");
                }
            }
            HUC3 => self.huc3_write(address, value),
            TAMA5 => self.tama5_write(address, value),
            POCKETCAM => self.pocket_cam_write(address, value),
            UNL_WISDOM_TREE => self.wisdom_tree_write(address, value),
            UNL_PKJD => self.pkjd_write(address, value),
            UNL_NT_OLD_1 => self.nt_old_write(address, value, false),
            UNL_NT_OLD_2 => self.nt_old_write(address, value, true),
            UNL_NT_NEW => self.nt_new_write(address, value),
            UNL_BBD => self.bbd_write(address, value),
            UNL_HITEK => self.hitek_write(address, value),
            UNL_LI_CHENG => self.li_cheng_write(address, value),
            UNL_GGB81 => self.ggb81_write(address, value),
            UNL_SACHEN_MMC1 | UNL_SACHEN_MMC2 => self.sachen_write(address, value),
            UNL_SINTAX => self.sintax_write(address, value),
            MBC1 => self.mbc1_write(address, value),
            MBC2 => self.mbc2_write(address, value),
            MBC3 | MBC3_RTC => self.mbc3_write(address, value),
            MBC6 => self.mbc6_write(address, value),
            MBC7 => self.mbc7_write(address, value),
            MMM01 => self.mmm01_write(address, value),
            HUC1 => self.huc1_write(address, value),
            M161 => self.m161_write(value),
            _ => self.mbc5_write(address, value),
        }
    }

    /// `memory->mbcRead`
    pub(crate) fn mbc_read(&mut self, address: u16) -> u8 {
        match self.memory.mbc_type {
            MBC2 => {
                let mem = &self.memory;
                if !mem.sram_access {
                    return 0xFF;
                }
                let address = address & 0x1FF;
                let shift = (address & 1) * 4;
                (mem.sram_bank_byte((address >> 1) as usize) >> shift) | 0xF0
            }
            MBC6 => {
                let mem = &self.memory;
                if !mem.sram_access {
                    return 0xFF;
                }
                let offset = address as usize & (SIZE_EXTERNAL_RAM_HALFBANK - 1);
                match address >> 12 {
                    0xA => mem.sram_bank_byte(offset),
                    0xB => mem.sram.get(mem.sram_bank1 + offset).copied().unwrap_or(0xFF),
                    _ => 0xFF,
                }
            }
            MBC7 => self.mbc7_read(address),
            HUC3 => self.huc3_read(address),
            TAMA5 => self.tama5_read(address),
            POCKETCAM => self.pocket_cam_read(address),
            UNL_PKJD => self.pkjd_read(address),
            UNL_BBD => self.reordered_read(address, 0),
            UNL_HITEK => self.reordered_read(address, 1),
            UNL_GGB81 => self.reordered_read(address, 2),
            UNL_SACHEN_MMC1 => self.sachen_mmc1_read(address),
            UNL_SACHEN_MMC2 => self.sachen_mmc2_read(address),
            UNL_SINTAX => self.sintax_read(address),
            _ => 0xFF,
        }
    }

    /// `_GBMBC1Update`
    fn mbc1_update(&mut self) {
        let state = self.memory.mbc_state.mbc1;
        let mut bank = state.bank_lo as i32;
        bank &= (1 << state.multicart_stride) - 1;
        bank |= (state.bank_hi as i32) << state.multicart_stride;
        if state.mode != 0 {
            self.mbc_switch_bank0((state.bank_hi as i32) << state.multicart_stride);
            self.mbc_switch_sram_bank(state.bank_hi as i32 & 3);
        } else {
            self.mbc_switch_bank0(0);
            self.mbc_switch_sram_bank(0);
        }
        if state.bank_lo & 0x1F == 0 {
            self.memory.mbc_state.mbc1.bank_lo = self.memory.mbc_state.mbc1.bank_lo.wrapping_add(1);
            bank += 1;
        }
        self.mbc_switch_bank(bank);
    }

    /// `_GBMBC1`
    fn mbc1_write(&mut self, address: u16, value: u8) {
        let bank = value & 0x1F;
        match address >> 13 {
            0x0 => match value & 0xF {
                0 => self.memory.sram_access = false,
                0xA => {
                    self.memory.sram_access = true;
                    let b = self.memory.sram_current_bank;
                    self.mbc_switch_sram_bank(b);
                }
                _ => {}
            },
            0x1 => {
                self.memory.mbc_state.mbc1.bank_lo = bank;
                self.mbc1_update();
            }
            0x2 => {
                self.memory.mbc_state.mbc1.bank_hi = bank & 3;
                self.mbc1_update();
            }
            0x3 => {
                self.memory.mbc_state.mbc1.mode = (value & 1) as i32;
                self.mbc1_update();
            }
            _ => {}
        }
    }

    /// `_GBMBC2`
    fn mbc2_write(&mut self, address: u16, value: u8) {
        let shift = (address & 1) * 4;
        let mut bank = (value & 0xF) as i32;
        match (address & 0xC100) >> 8 {
            0x0 => match value & 0x0F {
                0 => self.memory.sram_access = false,
                0xA => self.memory.sram_access = true,
                _ => {}
            },
            0x1 => {
                if bank == 0 {
                    bank += 1;
                }
                self.mbc_switch_bank(bank);
            }
            0x80..=0x83 => {
                if !self.memory.sram_access {
                    return;
                }
                let address = (address & 0x1FF) as usize;
                let old = self.memory.sram_bank_byte(address >> 1);
                let v = (old & (0xF0u8 >> shift)) | ((value & 0xF) << shift);
                self.memory.sram_bank_set(address >> 1, v);
                self.sram_dirty |= SAVEDATA_DIRT_NEW;
            }
            _ => {}
        }
    }

    /// `_GBMBC3`
    pub(crate) fn mbc3_write(&mut self, address: u16, value: u8) {
        let mut bank = value as i32;
        match address >> 13 {
            0x0 => {
                if value & 0xF == 0xA {
                    self.memory.sram_access = true;
                    let b = self.memory.sram_current_bank;
                    self.mbc_switch_sram_bank(b);
                } else {
                    self.memory.sram_access = false;
                }
            }
            0x1 => {
                if self.memory.rom_size < SIZE_CART_BANK0 * 0x80 {
                    bank &= 0x7F;
                }
                if bank == 0 {
                    bank += 1;
                }
                self.mbc_switch_bank(bank);
            }
            0x2 => {
                bank &= 0xF;
                if bank < 8 {
                    self.mbc_switch_sram_bank(value as i32);
                    self.memory.rtc_access = false;
                } else if bank <= 0xC {
                    self.memory.active_rtc_reg = bank - 8;
                    self.memory.rtc_access = true;
                }
            }
            0x3 => {
                if self.memory.rtc_latched && value == 0 {
                    self.memory.rtc_latched = false;
                } else if !self.memory.rtc_latched && value == 1 {
                    let now = self.rtc_unix_time();
                    let mem = &mut self.memory;
                    latch_rtc(now, &mut mem.rtc_regs, &mut mem.rtc_last_latch);
                    mem.rtc_latched = true;
                }
            }
            _ => {}
        }
    }

    /// `_GBMBC5`
    pub(crate) fn mbc5_write(&mut self, address: u16, value: u8) {
        match address >> 12 {
            0x0 | 0x1 => match value {
                0 => self.memory.sram_access = false,
                0xA => {
                    self.memory.sram_access = true;
                    let b = self.memory.sram_current_bank;
                    self.mbc_switch_sram_bank(b);
                }
                _ => {}
            },
            0x2 => {
                let bank = (self.memory.current_bank & 0x100) | value as i32;
                self.mbc_switch_bank(bank);
            }
            0x3 => {
                let bank = (self.memory.current_bank & 0xFF) | ((value as i32 & 1) << 8);
                self.mbc_switch_bank(bank);
            }
            0x4 | 0x5 => {
                let mut value = value;
                if self.memory.mbc_type == MBC5_RUMBLE {
                    self.rumble = (value >> 3) & 1 != 0;
                    value &= !8;
                }
                self.mbc_switch_sram_bank((value & 0xF) as i32);
            }
            _ => {}
        }
    }

    /// `_GBMBC6`
    fn mbc6_write(&mut self, address: u16, value: u8) {
        let bank = value as i32;
        match address >> 10 {
            0 => match value {
                0 => self.memory.sram_access = false,
                0xA => self.memory.sram_access = true,
                _ => {}
            },
            0x1 => self.mbc_switch_sram_half_bank(0, bank),
            0x2 => self.mbc_switch_sram_half_bank(1, bank),
            0x3 | 0x4 => {} // flash OE/WE writes are unimplemented in mGBA too
            0x8 | 0x9 => self.mbc_switch_half_bank(0, bank),
            0xA | 0xB => {
                self.memory.mbc_state.mbc6.flash_bank0 = value & 0x08 != 0;
                let b = self.memory.current_bank;
                self.mbc_switch_half_bank(0, b);
            }
            0xC | 0xD => self.mbc_switch_half_bank(1, bank),
            0xE | 0xF => {
                self.memory.mbc_state.mbc6.flash_bank1 = value & 0x08 != 0;
                let b = self.memory.current_bank1;
                self.mbc_switch_half_bank(1, b);
            }
            0x28..=0x2B => {
                if self.memory.sram_access {
                    self.memory.sram_bank_set(address as usize & (SIZE_EXTERNAL_RAM_HALFBANK - 1), value);
                    self.sram_dirty |= SAVEDATA_DIRT_NEW;
                }
            }
            0x2C..=0x2F => {
                if self.memory.sram_access {
                    let off = self.memory.sram_bank1 + (address as usize & (SIZE_EXTERNAL_RAM_HALFBANK - 1));
                    if let Some(b) = self.memory.sram.get_mut(off) {
                        *b = value;
                    }
                }
            }
            _ => {}
        }
    }

    /// `_GBMBC7`
    fn mbc7_write(&mut self, address: u16, value: u8) {
        let bank = (value & 0x7F) as i32;
        match address >> 13 {
            0x0 => match value {
                0xA => self.memory.mbc_state.mbc7.access |= 1,
                _ => self.memory.mbc_state.mbc7.access = 0,
            },
            0x1 => self.mbc_switch_bank(bank),
            0x2 => {
                if value == 0x40 {
                    self.memory.mbc_state.mbc7.access |= 2;
                } else {
                    self.memory.mbc_state.mbc7.access &= !2;
                }
            }
            0x5 => {
                self.mbc7_eeprom_write(address, value);
                self.sram_dirty |= SAVEDATA_DIRT_NEW;
            }
            _ => {}
        }
    }

    /// `_GBMBC7Read` (no tilt sensor attached)
    fn mbc7_read(&mut self, address: u16) -> u8 {
        let mbc7 = &self.memory.mbc_state.mbc7;
        if mbc7.access != 3 {
            return 0xFF;
        }
        match address & 0xF0 {
            0x20 | 0x40 => 0xFF,
            0x30 | 0x50 => 7,
            0x60 => 0,
            0x80 => mbc7.eeprom,
            _ => 0xFF,
        }
    }

    /// `_GBMBC7Write`
    fn mbc7_eeprom_write(&mut self, address: u16, value: u8) {
        let mem = &mut self.memory;
        let mbc7 = &mut mem.mbc_state.mbc7;
        if mbc7.access != 3 {
            return;
        }
        match address & 0xF0 {
            0x00 => {
                mbc7.latch = ((value & 0x55) == 0x55) as u8;
                return;
            }
            0x10 => {
                mbc7.latch |= value & 0xAA;
                mbc7.latch = 0;
                return;
            }
            0x80 => {}
            _ => return,
        }
        let old = mbc7.eeprom;
        let mut value = value | MBC7_DO; // Hi-Z
        if old & MBC7_CS == 0 && value & MBC7_CS != 0 {
            mbc7.state = MBC7_STATE_IDLE;
        }
        let sram = &mut mem.sram;
        let sram_set = |sram: &mut Vec<u8>, i: usize, v: u8| {
            if let Some(b) = sram.get_mut(i) {
                *b = v;
            }
        };
        if old & MBC7_CLK == 0 && value & MBC7_CLK != 0 {
            if mbc7.state == MBC7_STATE_READ_COMMAND || mbc7.state == MBC7_STATE_EEPROM_WRITE || mbc7.state == MBC7_STATE_EEPROM_WRAL {
                mbc7.sr <<= 1;
                mbc7.sr |= ((value & MBC7_DI) >> 1) as u16;
                mbc7.sr_bits += 1;
            }
            match mbc7.state {
                MBC7_STATE_IDLE => {
                    if value & MBC7_DI != 0 {
                        mbc7.state = MBC7_STATE_READ_COMMAND;
                        mbc7.sr_bits = 0;
                        mbc7.sr = 0;
                    }
                }
                MBC7_STATE_READ_COMMAND => {
                    if mbc7.sr_bits == 10 {
                        mbc7.state = 0x10 | (mbc7.sr >> 6) as u8;
                        if mbc7.state & 0xC != 0 {
                            mbc7.state &= !0x3;
                        }
                        mbc7.sr_bits = 0;
                        mbc7.address = (mbc7.sr & 0x7F) as u8;
                    }
                }
                MBC7_STATE_DO => {
                    value = (value & !MBC7_DO) | (mbc7.sr >> 15) as u8;
                    mbc7.sr <<= 1;
                    mbc7.sr_bits -= 1;
                    if mbc7.sr_bits == 0 {
                        mbc7.state = MBC7_STATE_IDLE;
                    }
                }
                _ => {}
            }
            let a = mbc7.address as usize * 2;
            match mbc7.state {
                MBC7_STATE_EEPROM_EWEN => {
                    mbc7.writable = true;
                    mbc7.state = MBC7_STATE_IDLE;
                }
                MBC7_STATE_EEPROM_EWDS => {
                    mbc7.writable = false;
                    mbc7.state = MBC7_STATE_IDLE;
                }
                MBC7_STATE_EEPROM_WRITE => {
                    if mbc7.sr_bits == 16 {
                        if mbc7.writable {
                            sram_set(sram, a, (mbc7.sr >> 8) as u8);
                            sram_set(sram, a + 1, mbc7.sr as u8);
                        }
                        mbc7.state = MBC7_STATE_IDLE;
                    }
                }
                MBC7_STATE_EEPROM_ERASE => {
                    if mbc7.writable {
                        sram_set(sram, a, 0xFF);
                        sram_set(sram, a + 1, 0xFF);
                    }
                    mbc7.state = MBC7_STATE_IDLE;
                }
                MBC7_STATE_EEPROM_READ => {
                    mbc7.sr_bits = 16;
                    mbc7.sr = (sram.get(a).copied().unwrap_or(0xFF) as u16) << 8;
                    mbc7.sr |= sram.get(a + 1).copied().unwrap_or(0xFF) as u16;
                    mbc7.state = MBC7_STATE_DO;
                    value &= !MBC7_DO;
                }
                MBC7_STATE_EEPROM_WRAL => {
                    if mbc7.sr_bits == 16 {
                        if mbc7.writable {
                            for i in 0..128 {
                                sram_set(sram, i * 2, (mbc7.sr >> 8) as u8);
                                sram_set(sram, i * 2 + 1, mbc7.sr as u8);
                            }
                        }
                        mbc7.state = MBC7_STATE_IDLE;
                    }
                }
                MBC7_STATE_EEPROM_ERAL => {
                    if mbc7.writable {
                        for i in 0..128 {
                            sram_set(sram, i * 2, 0xFF);
                            sram_set(sram, i * 2 + 1, 0xFF);
                        }
                    }
                    mbc7.state = MBC7_STATE_IDLE;
                }
                _ => {}
            }
        } else if value & MBC7_CS != 0 && old & MBC7_CLK != 0 && value & MBC7_CLK == 0 {
            value = (value & !MBC7_DO) | (old & MBC7_DO);
        }
        mbc7.eeprom = value;
    }

    /// `_GBMMM01`
    fn mmm01_write(&mut self, address: u16, value: u8) {
        if !self.memory.mbc_state.mmm01.locked {
            match address >> 13 {
                0x0 => {
                    self.memory.mbc_state.mmm01.locked = true;
                    let b = self.memory.mbc_state.mmm01.current_bank0;
                    self.mbc_switch_bank0(b);
                }
                0x1 => {
                    let s = &mut self.memory.mbc_state.mmm01;
                    s.current_bank0 &= !0x7F;
                    s.current_bank0 |= value as i32 & 0x7F;
                }
                0x2 => {
                    let s = &mut self.memory.mbc_state.mmm01;
                    s.current_bank0 &= !0x180;
                    s.current_bank0 |= (value as i32 & 0x30) << 3;
                }
                _ => {}
            }
            return;
        }
        match address >> 13 {
            0x0 => {
                if value == 0xA {
                    self.memory.sram_access = true;
                    let b = self.memory.sram_current_bank;
                    self.mbc_switch_sram_bank(b);
                } else {
                    self.memory.sram_access = false;
                }
            }
            0x1 => {
                let b = value as i32 + self.memory.mbc_state.mmm01.current_bank0;
                self.mbc_switch_bank(b);
            }
            0x2 => self.mbc_switch_sram_bank(value as i32),
            _ => {}
        }
    }

    /// `_GBHuC1`
    fn huc1_write(&mut self, address: u16, value: u8) {
        let bank = (value & 0x3F) as i32;
        match address >> 13 {
            0x0 => {
                if value == 0xE {
                    self.memory.sram_access = false;
                } else {
                    self.memory.sram_access = true;
                    let b = self.memory.sram_current_bank;
                    self.mbc_switch_sram_bank(b);
                }
            }
            0x1 => self.mbc_switch_bank(bank),
            0x2 => self.mbc_switch_sram_bank(value as i32),
            _ => {}
        }
    }

    /// `_GBM161`
    fn m161_write(&mut self, value: u8) {
        if self.memory.mbc_state.m161.locked {
            return;
        }
        let bank = value & 0x7;
        self.memory.mbc_state.m161.bank = bank;
        self.memory.mbc_state.m161.locked = true;
        self.mbc_switch_bank0(bank as i32 * 2);
        self.mbc_switch_bank(bank as i32 * 2 + 1);
    }

    /// The RTC suffix mGBA appends to MBC3 saves (`struct GBMBCRTCSaveBuffer`).
    pub(crate) fn rtc_save_suffix(&self) -> Vec<u8> {
        let mut regs = self.memory.rtc_regs;
        let mut last = self.memory.rtc_last_latch;
        latch_rtc(self.rtc_unix_time(), &mut regs, &mut last);
        let mut out = Vec::with_capacity(48);
        for r in regs {
            out.extend_from_slice(&(r as u32).to_le_bytes());
        }
        for r in self.memory.rtc_regs {
            out.extend_from_slice(&(r as u32).to_le_bytes());
        }
        out.extend_from_slice(&(self.memory.rtc_last_latch as u64).to_le_bytes());
        out
    }

    /// `GBMBCRTCRead` from a save file's suffix.
    pub(crate) fn rtc_load_suffix(&mut self, data: &[u8]) {
        if data.len() < 44 {
            return;
        }
        let word = |i: usize| u32::from_le_bytes([data[i * 4], data[i * 4 + 1], data[i * 4 + 2], data[i * 4 + 3]]);
        for i in 0..5 {
            self.memory.rtc_regs[i] = word(5 + i) as u8;
        }
        if data.len() >= 48 {
            self.memory.rtc_last_latch = u64::from_le_bytes(data[40..48].try_into().unwrap()) as i64;
        } else {
            self.memory.rtc_last_latch = word(10) as i64;
        }
    }
}

/// `_detectUnlMBC`
fn detect_unl_mbc(mem: &[u8], size: usize) -> i32 {
    if mem.len() < 0x8000 {
        return MBC_AUTODETECT;
    }
    let cart_type = mem[0x100 + cart::TYPE];
    if cart_type == 0 && is_wisdom_tree(mem, size) {
        return UNL_WISDOM_TREE;
    }
    match crc32(&mem[0x184..0x184 + 0x30]) {
        0x4fdab691 => return UNL_HITEK,
        0xc7d8c1df | 0x6d1ea662 => {
            // Garou; make sure we're not using a "fixed" version
            if mem[0x7FFF] != 0x01 {
                return UNL_BBD;
            }
        }
        0x79f34594 | 0x7e8c539b => return UNL_GGB81, // DATA. / TD-SOFT
        0x20d092e2 | 0xd2b57657 => {
            if cart_type == 0x01 {
                return UNL_LI_CHENG;
            }
            if 0x8000usize.checked_shl(mem[0x100 + cart::ROM_SIZE] as u32).unwrap_or(0) != size {
                return UNL_LI_CHENG;
            }
        }
        0x6c1dcf2d | 0x99e3449d => {
            if mem[0x7FFF] != 0x01 {
                return UNL_SINTAX;
            }
        }
        _ => {}
    }
    if mem[0x104] == 0xCE && mem[0x144] == 0xED && mem[0x114] == 0x66 {
        return UNL_SACHEN_MMC1;
    }
    if mem[0x184] == 0xCE && mem[0x1C4] == 0xED && mem[0x194] == 0x66 {
        return UNL_SACHEN_MMC2;
    }
    MBC_AUTODETECT
}

/// Human-readable name of an MBC type.
pub fn name(mbc: i32) -> &'static str {
    match mbc {
        MBC_NONE => "ROM only",
        MBC1 => "MBC1",
        MBC2 => "MBC2",
        MBC3 => "MBC3",
        MBC3_RTC => "MBC3+RTC",
        MBC5 => "MBC5",
        MBC5_RUMBLE => "MBC5+Rumble",
        MBC6 => "MBC6",
        MBC7 => "MBC7",
        MMM01 => "MMM01",
        HUC1 => "HuC-1",
        HUC3 => "HuC-3",
        POCKETCAM => "Pocket Camera",
        TAMA5 => "TAMA5",
        M161 => "M161",
        UNL_WISDOM_TREE => "Wisdom Tree",
        UNL_PKJD => "PKJD",
        UNL_NT_OLD_1 => "NT (old 1)",
        UNL_NT_OLD_2 => "NT (old 2)",
        UNL_NT_NEW => "NT (new)",
        UNL_BBD => "BBD",
        UNL_HITEK => "Hitek",
        UNL_LI_CHENG => "Li Cheng",
        UNL_GGB81 => "GGB-81",
        UNL_SACHEN_MMC1 => "Sachen MMC1",
        UNL_SACHEN_MMC2 => "Sachen MMC2",
        UNL_SINTAX => "Sintax",
        _ => "Unknown",
    }
}

/// `_GBMBCLatchRTC`
pub fn latch_rtc(now: i64, rtc_regs: &mut [u8; 5], rtc_last_latch: &mut i64) {
    let mut t = now;
    let current_latch = t;
    t -= *rtc_last_latch;
    *rtc_last_latch = current_latch;

    let mut diff: i64 = rtc_regs[0] as i64 + t % 60;
    if diff < 0 {
        diff += 60;
        t -= 60;
    }
    rtc_regs[0] = (diff % 60) as u8;
    t /= 60;
    t += diff / 60;

    diff = rtc_regs[1] as i64 + t % 60;
    if diff < 0 {
        diff += 60;
        t -= 60;
    }
    rtc_regs[1] = (diff % 60) as u8;
    t /= 60;
    t += diff / 60;

    diff = rtc_regs[2] as i64 + t % 24;
    if diff < 0 {
        diff += 24;
        t -= 24;
    }
    rtc_regs[2] = (diff % 24) as u8;
    t /= 24;
    t += diff / 24;

    diff = rtc_regs[3] as i64 + ((rtc_regs[4] as i64 & 1) << 8) + (t & 0x1FF);
    rtc_regs[3] = diff as u8;
    rtc_regs[4] &= 0xFE;
    rtc_regs[4] |= ((diff >> 8) & 1) as u8;
    if diff & 0x200 != 0 {
        rtc_regs[4] |= 0x80;
    }
}

const KNOWN_HEADER: [u8; 4] = [0xCE, 0xED, 0x66, 0x66];
const KNOWN_HEADER_SACHEN: [u8; 4] = [0x7C, 0xE7, 0xC0, 0x00];

/// Header check from `GBIsROM` on a buffer starting at a bank boundary.
fn is_rom_header(mem: &[u8]) -> bool {
    if mem.len() < 0x200 {
        return false;
    }
    let header = &mem[0x100..0x200];
    if header[4..8] == KNOWN_HEADER || header[4..8] == KNOWN_HEADER_SACHEN {
        return true;
    }
    if header[0x04] == KNOWN_HEADER[0] && header[0x44] == KNOWN_HEADER[1] && header[0x14] == KNOWN_HEADER[2] && header[0x54] == KNOWN_HEADER[3] {
        return true;
    }
    header[0x04] == KNOWN_HEADER_SACHEN[0]
        && header[0x44] == KNOWN_HEADER_SACHEN[1]
        && header[0x14] == KNOWN_HEADER_SACHEN[2]
        && header[0x54] == KNOWN_HEADER_SACHEN[3]
}

/// `GBIsROM`
pub fn is_rom(data: &[u8]) -> bool {
    if is_rom_header(data) {
        return true;
    }
    // GBX footer
    if data.len() >= 16 {
        let footer = &data[data.len() - 16..];
        let size = u32::from_be_bytes(footer[0..4].try_into().unwrap());
        let vers = u32::from_be_bytes(footer[4..8].try_into().unwrap());
        if &footer[12..16] == b"GBX!" && size == 0x40 && vers == 1 {
            return true;
        }
    }
    false
}
