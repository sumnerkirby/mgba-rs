// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's Game Boy memory map (src/gb/memory.c).
//
// mGBA keeps raw pointers into the ROM/SRAM/WRAM buffers for the current
// banks; here they are offsets into the owning buffers.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use super::mbc::{self, MbcState};
use super::{io, model, Gb, GbEvent};

pub const BASE_CART_BANK1: u16 = 0x4000;
pub const BASE_VRAM: u16 = 0x8000;
pub const BASE_OAM: u16 = 0xFE00;
pub const BASE_UNUSABLE: u16 = 0xFEA0;
pub const BASE_IO: u16 = 0xFF00;
pub const BASE_HRAM: u16 = 0xFF80;
pub const BASE_IE: u16 = 0xFFFF;

pub const SIZE_CART_BANK0: usize = 0x4000;
pub const SIZE_CART_HALFBANK: usize = 0x2000;
pub const SIZE_CART_MAX: usize = 0x800000;
pub const SIZE_VRAM: usize = 0x4000;
pub const SIZE_VRAM_BANK0: usize = 0x2000;
pub const SIZE_EXTERNAL_RAM: usize = 0x2000;
pub const SIZE_EXTERNAL_RAM_HALFBANK: usize = 0x1000;
pub const SIZE_WORKING_RAM: usize = 0x8000;
pub const SIZE_WORKING_RAM_BANK0: usize = 0x1000;
pub const SIZE_OAM: usize = 0xA0;
pub const SIZE_IO: usize = 0x80;
pub const SIZE_HRAM: usize = 0x7F;
pub const SIZE_MBC6_FLASH: usize = 0x100000;

pub const SAVEDATA_DIRT_NEW: i32 = 1;
pub const SAVEDATA_DIRT_SEEN: i32 = 2;
const CLEANUP_THRESHOLD: u32 = 15;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bus {
    Cpu,
    Main,
    Vram,
    Ram,
}

const OAM_BLOCK_DMG: [Bus; 8] = [Bus::Main, Bus::Main, Bus::Main, Bus::Main, Bus::Vram, Bus::Main, Bus::Main, Bus::Cpu];
const OAM_BLOCK_CGB: [Bus; 8] = [Bus::Main, Bus::Main, Bus::Main, Bus::Main, Bus::Vram, Bus::Main, Bus::Ram, Bus::Cpu];

/// Which buffer `cpu->memory.activeRegion` points into.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActiveBuf {
    Rom,
    Bios,
    Sram,
    /// `_yankBuffer` / `_blockedRegion`: a single 0xFF byte.
    Open,
}

/// Where a switchable ROM bank pointer points (MBC6 can map flash, stored in SRAM).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct BankPtr {
    pub off: usize,
    pub flash: bool,
}

/// `struct GBMemory`
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct GbMemory {
    #[serde(skip)]
    pub rom: Arc<Vec<u8>>,
    /// `romBase` when it points into the ROM.
    pub rom_base: usize,
    /// `romBase` points at a separately mapped BIOS image (`GBMapBIOS`).
    pub bios_mapped: bool,
    pub bios_rom: Vec<u8>,
    pub rom_bank: BankPtr,
    pub mbc_type: i32,
    pub mbc_state: MbcState,
    pub current_bank: i32,
    pub current_bank0: i32,
    pub current_bank1: i32,
    pub rom_bank1: BankPtr,
    pub current_sram_bank1: i32,
    pub sram_bank1: usize,

    pub cart_bus_decay: i32,
    pub cart_bus_pc: u16,
    pub cart_bus: u8,

    pub wram: Vec<u8>,
    pub wram_bank: usize,
    pub wram_current_bank: i32,

    pub mbc_read_bank0: bool,
    pub mbc_read_bank1: bool,
    pub mbc_read_high: bool,
    pub mbc_write_high: bool,
    /// `mbcRead != NULL`
    pub has_mbc_read: bool,
    pub sram_access: bool,
    pub direct_sram_access: bool,
    pub sram: Vec<u8>,
    pub sram_bank: usize,
    pub sram_current_bank: i32,

    #[serde(with = "serde_big_array::BigArray")]
    pub io: [u8; SIZE_IO],
    pub ime: bool,
    pub ie: u8,
    #[serde(with = "serde_big_array::BigArray")]
    pub hram: [u8; SIZE_HRAM],

    pub dma_source: u16,
    pub dma_dest: u16,
    pub dma_remaining: i32,
    pub hdma_source: u16,
    pub hdma_dest: u16,
    pub hdma_remaining: i32,
    pub is_hdma: bool,

    pub rom_size: usize,

    pub rtc_access: bool,
    pub active_rtc_reg: i32,
    pub rtc_latched: bool,
    pub rtc_regs: [u8; 5],
    pub rtc_last_latch: i64,
    pub last_rumble: i32,

    // cpu->memory active region bookkeeping
    pub cart_fast_path: bool,
    pub active_buf: ActiveBuf,
    pub active_off: usize,
    pub active_mask: u16,
    pub active_region_end: u16,
}

impl GbMemory {
    pub fn new() -> GbMemory {
        GbMemory {
            rom: Arc::new(Vec::new()),
            rom_base: 0,
            bios_mapped: false,
            bios_rom: Vec::new(),
            rom_bank: BankPtr { off: SIZE_CART_BANK0, flash: false },
            mbc_type: mbc::MBC_AUTODETECT,
            mbc_state: MbcState::default(),
            current_bank: 1,
            current_bank0: 0,
            current_bank1: 0,
            rom_bank1: BankPtr::default(),
            current_sram_bank1: 0,
            sram_bank1: 0,
            cart_bus_decay: 1,
            cart_bus_pc: 0,
            cart_bus: 0xFF,
            wram: vec![0; SIZE_WORKING_RAM],
            wram_bank: SIZE_WORKING_RAM_BANK0,
            wram_current_bank: 1,
            mbc_read_bank0: false,
            mbc_read_bank1: false,
            mbc_read_high: false,
            mbc_write_high: false,
            has_mbc_read: false,
            sram_access: false,
            direct_sram_access: true,
            sram: Vec::new(),
            sram_bank: 0,
            sram_current_bank: 0,
            io: [0; SIZE_IO],
            ime: false,
            ie: 0,
            hram: [0; SIZE_HRAM],
            dma_source: 0,
            dma_dest: 0,
            dma_remaining: 0,
            hdma_source: 0,
            hdma_dest: 0,
            hdma_remaining: 0,
            is_hdma: false,
            rom_size: 0,
            rtc_access: false,
            active_rtc_reg: 0,
            rtc_latched: false,
            rtc_regs: [0; 5],
            rtc_last_latch: 0,
            last_rumble: 0,
            cart_fast_path: false,
            active_buf: ActiveBuf::Open,
            active_off: 0,
            active_mask: 0,
            active_region_end: 0,
        }
    }

    #[inline(always)]
    pub fn rom_byte(&self, off: usize) -> u8 {
        self.rom.get(off).copied().unwrap_or(0xFF)
    }

    /// `memory->romBase[offset]`
    #[inline(always)]
    pub fn rom_base_byte(&self, offset: usize) -> u8 {
        if self.bios_mapped {
            self.bios_rom[offset & (SIZE_CART_BANK0 - 1)]
        } else {
            self.rom_byte(self.rom_base + offset)
        }
    }

    #[inline(always)]
    pub fn bank_byte(&self, ptr: BankPtr, offset: usize) -> u8 {
        if ptr.flash {
            self.sram.get(ptr.off + offset).copied().unwrap_or(0xFF)
        } else {
            self.rom_byte(ptr.off + offset)
        }
    }

    #[inline(always)]
    pub fn sram_bank_byte(&self, offset: usize) -> u8 {
        self.sram.get(self.sram_bank + offset).copied().unwrap_or(0xFF)
    }

    #[inline(always)]
    pub fn sram_bank_set(&mut self, offset: usize, value: u8) {
        if let Some(b) = self.sram.get_mut(self.sram_bank + offset) {
            *b = value;
        }
    }

    /// `GBMemorySwitchWramBank`
    pub fn switch_wram_bank(&mut self, bank: i32) {
        let mut bank = bank & 7;
        if bank == 0 {
            bank = 1;
        }
        self.wram_bank = SIZE_WORKING_RAM_BANK0 * bank as usize;
        self.wram_current_bank = bank;
    }
}

impl Default for GbMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl Gb {
    #[inline(always)]
    fn dma_blocks(&self, address: u16) -> bool {
        let block = if self.model < model::CGB { &OAM_BLOCK_DMG } else { &OAM_BLOCK_CGB };
        let dma_bus = block[(self.memory.dma_source >> 13) as usize];
        let access_bus = block[(address >> 13) as usize];
        dma_bus != Bus::Cpu && dma_bus == access_bus
    }

    /// `GBSetActiveRegion`
    pub fn set_active_region_impl(&mut self, address: u16) {
        let mem = &mut self.memory;
        match address >> 12 {
            0x0..=0x3 => {
                if mem.mbc_read_bank0 {
                    mem.cart_fast_path = false;
                } else {
                    mem.cart_fast_path = true;
                    if mem.bios_mapped {
                        mem.active_buf = ActiveBuf::Bios;
                        mem.active_off = 0;
                    } else {
                        mem.active_buf = ActiveBuf::Rom;
                        mem.active_off = mem.rom_base;
                    }
                    mem.active_region_end = BASE_CART_BANK1;
                    mem.active_mask = (SIZE_CART_BANK0 - 1) as u16;
                    if mem.rom_size < SIZE_CART_BANK0 {
                        if address as usize >= mem.rom_size {
                            mem.active_buf = ActiveBuf::Open;
                            mem.active_mask = 0;
                        } else {
                            mem.active_region_end = mem.rom_size as u16;
                        }
                    }
                }
            }
            0x4..=0x7 => {
                if mem.mbc_read_bank1 {
                    mem.cart_fast_path = false;
                } else {
                    mem.cart_fast_path = true;
                    let set_ptr = |mem: &mut GbMemory, p: BankPtr| {
                        mem.active_buf = if p.flash { ActiveBuf::Sram } else { ActiveBuf::Rom };
                        mem.active_off = p.off;
                    };
                    if mem.mbc_type != mbc::MBC6 && !(mem.mbc_type == mbc::UNL_NT_NEW && mem.mbc_state.nt_new.split_mode) {
                        let p = mem.rom_bank;
                        set_ptr(mem, p);
                        mem.active_region_end = BASE_VRAM;
                        mem.active_mask = (SIZE_CART_BANK0 - 1) as u16;
                    } else {
                        mem.active_mask = (SIZE_CART_HALFBANK - 1) as u16;
                        if address & 0x2000 != 0 {
                            let p = mem.rom_bank1;
                            set_ptr(mem, p);
                            mem.active_region_end = BASE_VRAM;
                        } else {
                            let p = mem.rom_bank;
                            set_ptr(mem, p);
                            mem.active_region_end = BASE_CART_BANK1 + 0x2000;
                        }
                    }
                    if mem.rom_size < SIZE_CART_BANK0 * 2 {
                        if address as usize >= mem.rom_size {
                            mem.active_buf = ActiveBuf::Open;
                            mem.active_mask = 0;
                        } else {
                            mem.active_region_end = mem.rom_size as u16;
                        }
                    }
                }
            }
            _ => {
                mem.cart_fast_path = false;
                mem.cart_bus = 0xFF;
            }
        }
        if self.memory.dma_remaining != 0
            && (self.dma_blocks(address) || (BASE_OAM..BASE_UNUSABLE).contains(&address))
        {
            self.memory.active_buf = ActiveBuf::Open;
            self.memory.active_mask = 0;
        }
    }

    /// `GBCartLoad8` / `GBLoad8` as selected by `cpu->memory.cpuLoad8`
    #[inline(always)]
    pub fn cpu_load8_impl(&mut self, address: u16) -> u8 {
        if !self.memory.cart_fast_path {
            return self.load8(address);
        }
        if address >= self.memory.active_region_end {
            self.set_active_region_impl(address);
            return self.cpu_load8_impl(address);
        }
        let mem = &mut self.memory;
        mem.cart_bus_pc = address;
        let offset = (address & mem.active_mask) as usize;
        let value = match mem.active_buf {
            ActiveBuf::Rom => mem.rom_byte(mem.active_off + offset),
            ActiveBuf::Bios => mem.bios_rom[offset],
            ActiveBuf::Sram => mem.sram.get(mem.active_off + offset).copied().unwrap_or(0xFF),
            ActiveBuf::Open => 0xFF,
        };
        mem.cart_bus = value;
        value
    }

    /// `GBLoad8`
    pub fn load8(&mut self, address: u16) -> u8 {
        if self.memory.dma_remaining != 0 {
            if self.dma_blocks(address) {
                return 0xFF;
            }
            if (BASE_OAM..BASE_IO).contains(&address) {
                return 0xFF;
            }
        }
        let pc = self.cpu.pc;
        match address >> 12 {
            0x0..=0x3 => {
                let mem = &mut self.memory;
                if address as usize >= mem.rom_size {
                    mem.cart_bus = 0xFF;
                } else if mem.mbc_read_bank0 {
                    let v = self.mbc_read(address);
                    self.memory.cart_bus = v;
                } else {
                    mem.cart_bus = mem.rom_base_byte(address as usize & (SIZE_CART_BANK0 - 1));
                }
                let mem = &mut self.memory;
                mem.cart_bus_pc = pc;
                mem.cart_bus
            }
            0x4..=0x7 => {
                let mem = &mut self.memory;
                if address >= 0x6000
                    && (mem.mbc_type == mbc::MBC6 || (mem.mbc_type == mbc::UNL_NT_NEW && mem.mbc_state.nt_new.split_mode))
                {
                    mem.cart_bus = mem.bank_byte(mem.rom_bank1, address as usize & (SIZE_CART_HALFBANK - 1));
                    mem.cart_bus_pc = pc;
                    return mem.cart_bus;
                }
                if address as usize >= mem.rom_size {
                    mem.cart_bus = 0xFF;
                } else if mem.mbc_read_bank1 {
                    let v = self.mbc_read(address);
                    self.memory.cart_bus = v;
                } else {
                    mem.cart_bus = mem.bank_byte(mem.rom_bank, address as usize & (SIZE_CART_BANK0 - 1));
                }
                let mem = &mut self.memory;
                mem.cart_bus_pc = pc;
                mem.cart_bus
            }
            0x8 | 0x9 => {
                if self.video.mode != 3 {
                    return self.video.vram[self.video.vram_bank + (address as usize & (SIZE_VRAM_BANK0 - 1))];
                }
                0xFF
            }
            0xA | 0xB => {
                let t = self.cpu.t_multiplier;
                let mem = &mut self.memory;
                if mem.rtc_access {
                    mem.cart_bus = mem.rtc_regs[mem.active_rtc_reg as usize];
                } else if mem.has_mbc_read {
                    let v = self.mbc_read(address);
                    self.memory.cart_bus = v;
                } else if mem.sram_access && !mem.sram.is_empty() {
                    mem.cart_bus = mem.sram_bank_byte(address as usize & (SIZE_EXTERNAL_RAM - 1));
                } else if mem.mbc_type == mbc::HUC3 {
                    mem.cart_bus = 0x01; // TODO: Is this supposed to be the current SRAM bank?
                } else if t * (pc.wrapping_sub(mem.cart_bus_pc) as i32) >= mem.cart_bus_decay {
                    mem.cart_bus = 0xFF;
                }
                let mem = &mut self.memory;
                mem.cart_bus_pc = pc;
                mem.cart_bus
            }
            0xC | 0xE => {
                if self.memory.mbc_read_high {
                    self.mbc_read(address);
                }
                self.memory.wram[address as usize & (SIZE_WORKING_RAM_BANK0 - 1)]
            }
            0xD => {
                if self.memory.mbc_read_high {
                    self.mbc_read(address);
                }
                self.memory.wram[self.memory.wram_bank + (address as usize & (SIZE_WORKING_RAM_BANK0 - 1))]
            }
            _ => {
                if address < BASE_OAM {
                    return self.memory.wram[self.memory.wram_bank + (address as usize & (SIZE_WORKING_RAM_BANK0 - 1))];
                }
                if address < BASE_UNUSABLE {
                    if self.video.mode < 2 {
                        return self.video.oam[address as usize & 0xFF];
                    }
                    return 0xFF;
                }
                if address < BASE_IO {
                    return 0xFF;
                }
                if address < BASE_HRAM {
                    return self.io_read((address as usize) & (SIZE_IO - 1));
                }
                if address < BASE_IE {
                    return self.memory.hram[address as usize & SIZE_HRAM];
                }
                self.io_read(io::REG_IE)
            }
        }
    }

    /// `GBStore8`
    pub fn store8(&mut self, address: u16, value: u8) {
        if self.memory.dma_remaining != 0 {
            if self.dma_blocks(address) {
                return;
            }
            if (BASE_OAM..BASE_UNUSABLE).contains(&address) {
                return;
            }
        }
        match address >> 12 {
            0x0..=0x7 => {
                self.mbc_write(address, value);
                let pc = self.cpu.pc;
                self.set_active_region_impl(pc);
            }
            0x8 | 0x9 => {
                if self.video.mode != 3 {
                    let offset = address as usize & (SIZE_VRAM_BANK0 - 1);
                    self.video.vram[self.video.vram_bank + offset] = value;
                }
            }
            0xA | 0xB => {
                let mem = &mut self.memory;
                if mem.rtc_access {
                    mem.rtc_regs[mem.active_rtc_reg as usize] = value;
                } else if mem.sram_access && !mem.sram.is_empty() && mem.direct_sram_access {
                    let offset = address as usize & (SIZE_EXTERNAL_RAM - 1);
                    if mem.sram_bank_byte(offset) != value {
                        mem.sram_bank_set(offset, value);
                        self.sram_dirty |= SAVEDATA_DIRT_NEW;
                    }
                } else {
                    self.mbc_write(address, value);
                }
            }
            0xC | 0xE => {
                if self.memory.mbc_write_high {
                    self.mbc_write(address, value);
                }
                self.memory.wram[address as usize & (SIZE_WORKING_RAM_BANK0 - 1)] = value;
            }
            0xD => {
                if self.memory.mbc_write_high {
                    self.mbc_write(address, value);
                }
                let bank = self.memory.wram_bank;
                self.memory.wram[bank + (address as usize & (SIZE_WORKING_RAM_BANK0 - 1))] = value;
            }
            _ => {
                if address < BASE_OAM {
                    let bank = self.memory.wram_bank;
                    self.memory.wram[bank + (address as usize & (SIZE_WORKING_RAM_BANK0 - 1))] = value;
                } else if address < BASE_UNUSABLE {
                    if self.video.mode < 2 {
                        self.video.oam[address as usize & 0xFF] = value;
                    }
                } else if address < BASE_IO {
                    // Attempt to write to unusable memory
                } else if address < BASE_HRAM {
                    self.io_write((address as usize) & (SIZE_IO - 1), value);
                } else if address < BASE_IE {
                    self.memory.hram[address as usize & SIZE_HRAM] = value;
                } else {
                    self.io_write(io::REG_IE, value);
                }
            }
        }
    }

    /// `GBView8`: side-effect-free read for debuggers and tools.
    pub fn view8(&self, address: u16) -> u8 {
        let mem = &self.memory;
        match address >> 12 {
            0x0..=0x3 => mem.rom_base_byte(address as usize & (SIZE_CART_BANK0 - 1)),
            0x4..=0x7 => mem.bank_byte(mem.rom_bank, address as usize & (SIZE_CART_BANK0 - 1)),
            0x8 | 0x9 => self.video.vram[self.video.vram_bank + (address as usize & (SIZE_VRAM_BANK0 - 1))],
            0xA | 0xB => {
                if mem.rtc_access {
                    mem.rtc_regs[mem.active_rtc_reg as usize]
                } else if mem.sram_access && !mem.sram.is_empty() {
                    mem.sram_bank_byte(address as usize & (SIZE_EXTERNAL_RAM - 1))
                } else {
                    0xFF
                }
            }
            0xC | 0xE => mem.wram[address as usize & (SIZE_WORKING_RAM_BANK0 - 1)],
            0xD => mem.wram[mem.wram_bank + (address as usize & (SIZE_WORKING_RAM_BANK0 - 1))],
            _ => {
                if address < BASE_OAM {
                    mem.wram[mem.wram_bank + (address as usize & (SIZE_WORKING_RAM_BANK0 - 1))]
                } else if address < BASE_UNUSABLE {
                    self.video.oam[address as usize & 0xFF]
                } else if address < BASE_IO {
                    0xFF
                } else if address < BASE_HRAM {
                    mem.io[address as usize & (SIZE_IO - 1)]
                } else if address < BASE_IE {
                    mem.hram[address as usize & SIZE_HRAM]
                } else {
                    mem.ie
                }
            }
        }
    }

    /// `GBPatch8` (only the regions `GBSkipBIOS` and cheats need)
    pub fn patch8(&mut self, address: u16, value: u8) {
        match address >> 12 {
            0x0..=0x7 => {
                let rom = Arc::make_mut(&mut self.memory.rom);
                let off = if address < 0x4000 {
                    self.memory.rom_base + address as usize
                } else {
                    self.memory.rom_bank.off + (address as usize & (SIZE_CART_BANK0 - 1))
                };
                if let Some(b) = rom.get_mut(off) {
                    *b = value;
                }
                if self.memory.bios_mapped && address < 0x4000 {
                    self.memory.bios_rom[address as usize] = value;
                }
            }
            0x8 | 0x9 => {
                let offset = self.video.vram_bank + (address as usize & (SIZE_VRAM_BANK0 - 1));
                self.video.vram[offset] = value;
            }
            0xA | 0xB => {
                let mem = &mut self.memory;
                if mem.rtc_access {
                    mem.rtc_regs[mem.active_rtc_reg as usize] = value;
                } else if mem.sram_access && !mem.sram.is_empty() && mem.mbc_type != mbc::MBC2 {
                    mem.sram_bank_set(address as usize & (SIZE_EXTERNAL_RAM - 1), value);
                } else {
                    self.mbc_write(address, value);
                }
                self.sram_dirty |= SAVEDATA_DIRT_NEW;
            }
            0xC | 0xE => self.memory.wram[address as usize & (SIZE_WORKING_RAM_BANK0 - 1)] = value,
            0xD => {
                let bank = self.memory.wram_bank;
                self.memory.wram[bank + (address as usize & (SIZE_WORKING_RAM_BANK0 - 1))] = value;
            }
            _ => {
                if address < BASE_OAM {
                    let bank = self.memory.wram_bank;
                    self.memory.wram[bank + (address as usize & (SIZE_WORKING_RAM_BANK0 - 1))] = value;
                } else if address < BASE_UNUSABLE {
                    self.video.oam[address as usize & 0xFF] = value;
                } else if (BASE_HRAM..BASE_IE).contains(&address) {
                    self.memory.hram[address as usize & SIZE_HRAM] = value;
                }
            }
        }
    }

    /// `GBMemoryReset`
    pub(crate) fn memory_reset(&mut self) {
        self.memory.wram = vec![0; SIZE_WORKING_RAM];
        if self.model >= model::CGB {
            let base = &mut self.memory.wram;
            let mut pattern: u32 = 0;
            // Banks 0, 1, 3, 6, and 7 are cleared with this pattern
            let mut i = 0usize;
            let put = |base: &mut Vec<u8>, word: usize, v: u32| {
                base[word * 4..word * 4 + 4].copy_from_slice(&v.to_le_bytes());
            };
            while i < SIZE_WORKING_RAM / 4 {
                if (i & 0x1FFF) == 0x800 {
                    // Skip bank 2
                    i += 0x3FC;
                    i += 4;
                    continue;
                }
                if (i & 0x1FFF) == 0x1000 {
                    // Skip banks 4 and 5
                    i += 0x7FC;
                    i += 4;
                    continue;
                }
                if (i & 0x1FF) == 0 {
                    pattern = !pattern;
                }
                if i + 3 < SIZE_WORKING_RAM / 4 {
                    put(base, i, pattern);
                    put(base, i + 1, pattern);
                    put(base, i + 2, !pattern);
                    put(base, i + 3, !pattern);
                }
                i += 4;
            }
        }
        self.memory.switch_wram_bank(1);
        self.memory.ime = false;
        self.memory.ie = 0;

        self.memory.dma_remaining = 0;
        self.memory.dma_source = 0;
        self.memory.dma_dest = 0;
        self.memory.hdma_remaining = 0;
        self.memory.hdma_source = 0;
        self.memory.hdma_dest = 0;
        self.memory.is_hdma = false;

        self.memory.hram = [0; SIZE_HRAM];

        self.mbc_reset();
    }

    /// `GBMemoryDMA`
    pub(crate) fn memory_dma(&mut self, base: u16) {
        let mut base = base;
        if base >= 0xE000 {
            base &= 0xDFFF;
        }
        self.deschedule(GbEvent::Dma);
        self.schedule(GbEvent::Dma, 8 * (2 - self.double_speed as i32));
        self.memory.dma_source = base;
        self.memory.dma_dest = 0;
        self.memory.dma_remaining = 0xA0;
    }

    /// `GBMemoryWriteHDMA5`
    pub(crate) fn memory_write_hdma5(&mut self, value: u8) -> u8 {
        let mem = &mut self.memory;
        mem.hdma_source = ((mem.io[io::REG_HDMA1] as u16) << 8) | mem.io[io::REG_HDMA2] as u16;
        mem.hdma_dest = ((mem.io[io::REG_HDMA3] as u16) << 8) | mem.io[io::REG_HDMA4] as u16;
        mem.hdma_source &= 0xFFF0;
        if (0x8000..0xA000).contains(&mem.hdma_source) {
            return value | 0x80;
        }
        mem.hdma_dest &= 0x1FF0;
        mem.hdma_dest |= 0x8000;
        let was_hdma = mem.is_hdma;
        mem.is_hdma = value & 0x80 != 0;
        if (!was_hdma && !mem.is_hdma) || self.video.mode == 0 {
            if mem.is_hdma {
                mem.hdma_remaining = 0x10;
            } else {
                mem.hdma_remaining = ((value as i32 & 0x7F) + 1) * 0x10;
            }
            self.cpu_blocked = true;
            self.schedule(GbEvent::Hdma, 0);
        }
        value & 0x7F
    }

    /// `_GBMemoryDMAService`
    pub(crate) fn memory_dma_service(&mut self, cycles_late: u32) {
        let dma_remaining = self.memory.dma_remaining;
        self.memory.dma_remaining = 0;
        let src = self.memory.dma_source;
        let b = self.load8(src);
        // TODO: Can DMA write OAM during modes 2-3?
        self.video.oam[self.memory.dma_dest as usize] = b;
        self.memory.dma_source = self.memory.dma_source.wrapping_add(1);
        self.memory.dma_dest = self.memory.dma_dest.wrapping_add(1);
        self.memory.dma_remaining = dma_remaining - 1;
        if self.memory.dma_remaining != 0 {
            self.schedule(GbEvent::Dma, 4 * (2 - self.double_speed as i32) - cycles_late as i32);
        }
    }

    /// `_GBMemoryHDMAService`
    pub(crate) fn memory_hdma_service(&mut self, cycles_late: u32) {
        self.cpu_blocked = true;
        let src = self.memory.hdma_source;
        let b = self.load8(src);
        let dst = self.memory.hdma_dest;
        self.store8(dst, b);
        let mem = &mut self.memory;
        mem.hdma_source = mem.hdma_source.wrapping_add(1);
        mem.hdma_dest = mem.hdma_dest.wrapping_add(1);
        mem.hdma_remaining -= 1;
        if mem.hdma_remaining != 0 {
            self.deschedule(GbEvent::Hdma);
            self.schedule(GbEvent::Hdma, 4 - cycles_late as i32);
        } else {
            self.cpu_blocked = false;
            let mem = &mut self.memory;
            mem.io[io::REG_HDMA1] = (mem.hdma_source >> 8) as u8;
            mem.io[io::REG_HDMA2] = mem.hdma_source as u8;
            mem.io[io::REG_HDMA3] = (mem.hdma_dest >> 8) as u8;
            mem.io[io::REG_HDMA4] = mem.hdma_dest as u8;
            if mem.is_hdma {
                mem.io[io::REG_HDMA5] = mem.io[io::REG_HDMA5].wrapping_sub(1);
                if mem.io[io::REG_HDMA5] == 0xFF {
                    mem.is_hdma = false;
                }
            } else {
                mem.io[io::REG_HDMA5] = 0xFF;
            }
        }
    }

    /// `mSavedataClean` as used by `GBSramClean`. Returns true when a write has settled.
    pub(crate) fn sram_clean(&mut self, frame_count: u32) -> bool {
        if self.sram_dirty & SAVEDATA_DIRT_NEW != 0 {
            self.sram_dirt_age = frame_count;
            self.sram_dirty &= !SAVEDATA_DIRT_NEW;
            if self.sram_dirty & SAVEDATA_DIRT_SEEN == 0 {
                self.sram_dirty |= SAVEDATA_DIRT_SEEN;
            }
        } else if self.sram_dirty & SAVEDATA_DIRT_SEEN != 0 && frame_count.wrapping_sub(self.sram_dirt_age) > CLEANUP_THRESHOLD {
            self.sram_dirty = 0;
            return true;
        }
        false
    }
}
