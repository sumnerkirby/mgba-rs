// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's GBA memory map (src/gba/memory.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::dma::Dma;
use super::gpio::CartridgeHardware;
use super::savedata::{Savedata, SavedataType};
use super::video::{
    VSTALL_A2, VSTALL_A3, VSTALL_B, vstall_t4, vstall_t8,
};
use super::{glog, regs, Gba, LogLevel};
use crate::arm::{ExecutionMode, ARM_PC, LSM_B, LSM_D, WORD_SIZE_ARM, WORD_SIZE_THUMB};
use crate::timing::EventId;

pub const REGION_BIOS: u32 = 0x0;
pub const REGION_EWRAM: u32 = 0x2;
pub const REGION_IWRAM: u32 = 0x3;
pub const REGION_IO: u32 = 0x4;
pub const REGION_PALETTE_RAM: u32 = 0x5;
pub const REGION_VRAM: u32 = 0x6;
pub const REGION_OAM: u32 = 0x7;
pub const REGION_ROM0: u32 = 0x8;
pub const REGION_ROM0_EX: u32 = 0x9;
pub const REGION_ROM1: u32 = 0xA;
pub const REGION_ROM1_EX: u32 = 0xB;
pub const REGION_ROM2: u32 = 0xC;
pub const REGION_ROM2_EX: u32 = 0xD;
pub const REGION_SRAM: u32 = 0xE;
pub const REGION_SRAM_MIRROR: u32 = 0xF;

pub const BASE_EWRAM: u32 = 0x02000000;
pub const BASE_IO: u32 = 0x04000000;
pub const BASE_ROM0: u32 = 0x08000000;
pub const BASE_SRAM: u32 = 0x0E000000;

pub const SIZE_BIOS: u32 = 0x00004000;
pub const SIZE_EWRAM: u32 = 0x00040000;
pub const SIZE_IWRAM: u32 = 0x00008000;
pub const SIZE_IO: u32 = 0x00000400;
pub const SIZE_PALETTE_RAM: u32 = 0x00000400;
pub const SIZE_VRAM: u32 = 0x00018000;
pub const SIZE_OAM: u32 = 0x00000400;
pub const SIZE_ROM0: u32 = 0x02000000;
pub const SIZE_SRAM: u32 = 0x00008000;
pub const SIZE_SRAM512: u32 = 0x00010000;
pub const SIZE_FLASH512: u32 = 0x00010000;
pub const SIZE_FLASH1M: u32 = 0x00020000;
pub const SIZE_EEPROM: u32 = 0x00002000;
pub const SIZE_EEPROM512: u32 = 0x00000200;

pub const OFFSET_MASK: u32 = 0x00FFFFFF;
pub const BASE_OFFSET: u32 = 24;

pub const SAVEDATA_FLASH_BASE: u32 = 0x0E005555;

const DEADBEEF: u32 = 0xE710B710; // Illegal instruction on both ARM and Thumb

const BASE_WAITSTATES: [i32; 16] = [0, 0, 2, 0, 0, 0, 0, 0, 4, 4, 4, 4, 4, 4, 4, 0];
const BASE_WAITSTATES_32: [i32; 16] = [0, 0, 5, 0, 0, 1, 1, 0, 7, 7, 9, 9, 13, 13, 9, 0];
const BASE_WAITSTATES_SEQ: [i32; 16] = [0, 0, 2, 0, 0, 0, 0, 0, 2, 2, 4, 4, 8, 8, 4, 0];
const BASE_WAITSTATES_SEQ_32: [i32; 16] = [0, 0, 5, 0, 0, 1, 1, 0, 5, 5, 9, 9, 17, 17, 9, 0];
const ROM_WAITSTATES: [i32; 4] = [4, 3, 2, 8];
const ROM_WAITSTATES_SEQ: [i32; 6] = [2, 1, 4, 1, 8, 1];

static HLE_BIOS: &[u8; 0x4000] = include_bytes!("hle_bios.bin");

/// What the CPU's prefetcher reads from (mGBA's `cpu->memory.activeRegion` pointer).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FetchRegion {
    Bios,
    Ewram,
    Iwram,
    Palette,
    VramLo,
    VramHi,
    Oam,
    Rom,
    Bad,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Memory {
    #[serde(skip)]
    pub bios: Vec<u8>,
    #[serde(skip)]
    pub full_bios: bool,
    pub wram: Vec<u8>,
    pub iwram: Vec<u8>,
    #[serde(skip, default = "empty_rom")]
    pub rom: std::sync::Arc<[u8]>,
    pub rom_size: u32,
    pub rom_mask: u32,
    #[serde(with = "serde_big_array::BigArray")]
    pub io: [u16; 512],

    pub hw: CartridgeHardware,
    pub savedata: Savedata,

    #[serde(with = "serde_big_array::BigArray")]
    pub waitstates_seq32: [i32; 256],
    #[serde(with = "serde_big_array::BigArray")]
    pub waitstates_seq16: [i32; 256],
    #[serde(with = "serde_big_array::BigArray")]
    pub waitstates_nonseq32: [i32; 256],
    #[serde(with = "serde_big_array::BigArray")]
    pub waitstates_nonseq16: [i32; 256],
    pub active_region: i32,
    pub fetch: FetchRegion,
    pub prefetch: bool,
    pub last_prefetched_pc: u32,
    pub bios_prefetch: u32,

    pub dma: [Dma; 4],
    pub active_dma: i32,
}

fn empty_rom() -> std::sync::Arc<[u8]> {
    std::sync::Arc::from(Vec::new())
}

impl Memory {
    pub fn new() -> Memory {
        let mut m = Memory {
            bios: HLE_BIOS.to_vec(),
            full_bios: false,
            wram: vec![0; SIZE_EWRAM as usize],
            iwram: vec![0; SIZE_IWRAM as usize],
            rom: std::sync::Arc::from(Vec::new()),
            rom_size: 0,
            rom_mask: 0,
            io: [0; 512],
            hw: CartridgeHardware::default(),
            savedata: Savedata::new(),
            waitstates_seq32: [0; 256],
            waitstates_seq16: [0; 256],
            waitstates_nonseq32: [0; 256],
            waitstates_nonseq16: [0; 256],
            active_region: -1,
            fetch: FetchRegion::Bad,
            prefetch: false,
            last_prefetched_pc: 0,
            bios_prefetch: 0,
            dma: [Dma::default(); 4],
            active_dma: -1,
        };
        for i in 0..16 {
            m.waitstates_nonseq16[i] = BASE_WAITSTATES[i];
            m.waitstates_seq16[i] = BASE_WAITSTATES_SEQ[i];
            m.waitstates_nonseq32[i] = BASE_WAITSTATES_32[i];
            m.waitstates_seq32[i] = BASE_WAITSTATES_SEQ_32[i];
        }
        m
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

#[inline(always)]
pub(crate) fn rd32(m: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([m[off], m[off + 1], m[off + 2], m[off + 3]])
}
#[inline(always)]
pub(crate) fn rd16(m: &[u8], off: usize) -> u32 {
    u16::from_le_bytes([m[off], m[off + 1]]) as u32
}
#[inline(always)]
pub(crate) fn wr32(m: &mut [u8], off: usize, v: u32) {
    m[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
#[inline(always)]
pub(crate) fn wr16(m: &mut [u8], off: usize, v: u16) {
    m[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
#[inline(always)]
fn hw32(m: &[u16], off: usize) -> u32 {
    (m[off >> 1] as u32) | ((m[(off >> 1) + 1] as u32) << 16)
}

const VSTALL_LUT: [u16; 32] = {
    let t4 = [vstall_t4(0), vstall_t4(1), vstall_t4(2), vstall_t4(3)];
    let t8 = [vstall_t8(0), vstall_t8(1), vstall_t8(2), vstall_t8(3)];
    let a = [VSTALL_A3, VSTALL_A3, VSTALL_A2, VSTALL_A2 | VSTALL_B];
    let mut lut = [0u16; 32];
    let mut i = 0;
    while i < 32 {
        let group = i / 4;
        let k = i % 4;
        let t = match group {
            0 | 1 | 5 => t4[k],
            3 | 7 => t8[k],
            _ => 0,
        };
        lut[i] = t | a[k];
        i += 1;
    }
    lut
};

impl Gba {
    /// `GBAMemoryReset`
    pub(crate) fn memory_reset(&mut self) {
        if self.has_rom {
            self.memory.wram.fill(0);
        }
        self.memory.iwram.fill(0);
        self.memory.io = [0; 512];
        self.adjust_waitstates(0);
        self.adjust_ewram_waitstates(0x0D00);
        // GBAMemoryClearAGBPrint
        self.memory.active_region = -1;
        self.memory.prefetch = false;
        self.memory.last_prefetched_pc = 0;
        self.memory.savedata.reset();
        self.memory.hw.reset();
        self.dma_reset();
    }

    #[inline(always)]
    fn dispcnt_mode(&self) -> u16 {
        self.memory.io[(regs::DISPCNT >> 1) as usize] & 7
    }

    #[inline(always)]
    fn vram_obj_boundary(&self) -> u32 {
        if self.dispcnt_mode() >= 3 {
            0x00014000
        } else {
            0x00010000
        }
    }

    /// LOAD_32 from the prefetch region.
    #[inline(always)]
    pub(crate) fn active_fetch32(&self, a: u32) -> u32 {
        let a = a as usize;
        match self.memory.fetch {
            FetchRegion::Rom => rd32(&self.memory.rom, a),
            FetchRegion::Iwram => rd32(&self.memory.iwram, a),
            FetchRegion::Ewram => rd32(&self.memory.wram, a),
            FetchRegion::Bios => rd32(&self.memory.bios, a),
            FetchRegion::Palette => hw32(&self.video.palette, a),
            FetchRegion::VramLo => hw32(&self.video.vram, a),
            FetchRegion::VramHi => hw32(&self.video.vram, a + 0x10000),
            FetchRegion::Oam => hw32(&self.video.oam, a),
            FetchRegion::Bad => DEADBEEF,
        }
    }

    #[inline(always)]
    pub(crate) fn active_fetch16(&self, a: u32) -> u32 {
        let a = a as usize;
        match self.memory.fetch {
            FetchRegion::Rom => rd16(&self.memory.rom, a),
            FetchRegion::Iwram => rd16(&self.memory.iwram, a),
            FetchRegion::Ewram => rd16(&self.memory.wram, a),
            FetchRegion::Bios => rd16(&self.memory.bios, a),
            FetchRegion::Palette => self.video.palette[a >> 1] as u32,
            FetchRegion::VramLo => self.video.vram[a >> 1] as u32,
            FetchRegion::VramHi => self.video.vram[(a + 0x10000) >> 1] as u32,
            FetchRegion::Oam => self.video.oam[a >> 1] as u32,
            FetchRegion::Bad => DEADBEEF & 0xFFFF,
        }
    }

    /// ROM reads with the cartridge GPIO registers overlaid (mGBA writes them into the ROM buffer).
    #[inline(always)]
    fn rom_read16(&self, off: usize) -> u32 {
        if (0xC4..0xCA).contains(&off) && self.memory.hw.gpio_written {
            return self.memory.hw.regs_visible[(off - 0xC4) >> 1] as u32;
        }
        rd16(&self.memory.rom, off)
    }

    #[inline(always)]
    fn rom_read32(&self, off: usize) -> u32 {
        if (0xC4..0xCA).contains(&off) && self.memory.hw.gpio_written {
            return self.rom_read16(off) | (self.rom_read16(off + 2) << 16);
        }
        rd32(&self.memory.rom, off)
    }

    #[inline(always)]
    fn rom_read8(&self, off: usize) -> u32 {
        (self.rom_read16(off & !1) >> ((off & 1) * 8)) & 0xFF
    }

    fn update_active_cycles(&mut self) {
        let r = self.memory.active_region;
        if r < 0 {
            return;
        }
        let r = r as usize;
        self.cpu.memory.active_seq_cycles32 = self.memory.waitstates_seq32[r];
        self.cpu.memory.active_seq_cycles16 = self.memory.waitstates_seq16[r];
        self.cpu.memory.active_nonseq_cycles32 = self.memory.waitstates_nonseq32[r];
        self.cpu.memory.active_nonseq_cycles16 = self.memory.waitstates_nonseq16[r];
    }

    /// `GBASetActiveRegion`
    pub(crate) fn set_active_region_impl(&mut self, address: u32) {
        let new_region = (address >> BASE_OFFSET) as i32;
        if self.idle_optimization as i32 >= 0 && self.memory.active_region != REGION_BIOS as i32 && address == self.idle_loop {
            if self.halt_pending {
                self.halt_pending = false;
                self.halt();
            } else {
                self.halt_pending = true;
            }
        }

        self.last_jump = address;
        self.memory.last_prefetched_pc = 0;
        if new_region == self.memory.active_region {
            if self.cpu.cpsr.t() {
                self.cpu.memory.active_mask |= WORD_SIZE_THUMB;
            } else {
                self.cpu.memory.active_mask &= !(WORD_SIZE_ARM - 1);
            }
            if (new_region as u32) < REGION_ROM0 || (address & (SIZE_ROM0 - 1)) < self.memory.rom_size {
                return;
            }
        }

        if self.memory.active_region == REGION_BIOS as i32 {
            self.memory.bios_prefetch = self.cpu.prefetch[1];
        }
        self.memory.active_region = new_region;
        match new_region as u32 {
            REGION_BIOS => {
                self.memory.fetch = FetchRegion::Bios;
                self.cpu.memory.active_mask = SIZE_BIOS - 1;
            }
            REGION_EWRAM => {
                self.memory.fetch = FetchRegion::Ewram;
                self.cpu.memory.active_mask = SIZE_EWRAM - 1;
            }
            REGION_IWRAM => {
                self.memory.fetch = FetchRegion::Iwram;
                self.cpu.memory.active_mask = SIZE_IWRAM - 1;
            }
            REGION_PALETTE_RAM => {
                self.memory.fetch = FetchRegion::Palette;
                self.cpu.memory.active_mask = SIZE_PALETTE_RAM - 1;
            }
            REGION_VRAM => {
                if address & 0x10000 != 0 {
                    self.memory.fetch = FetchRegion::VramHi;
                    self.cpu.memory.active_mask = 0x00007FFF;
                } else {
                    self.memory.fetch = FetchRegion::VramLo;
                    self.cpu.memory.active_mask = 0x0000FFFF;
                }
            }
            REGION_OAM => {
                self.memory.fetch = FetchRegion::Oam;
                self.cpu.memory.active_mask = SIZE_OAM - 1;
            }
            REGION_ROM0..=REGION_ROM2_EX if (address & (SIZE_ROM0 - 1)) < self.memory.rom_size => {
                self.memory.fetch = FetchRegion::Rom;
                self.cpu.memory.active_mask = self.memory.rom_mask;
            }
            _ => {
                self.memory.active_region = -1;
                self.memory.fetch = FetchRegion::Bad;
                self.cpu.memory.active_mask = 0;
                if self.yanked_rom_size == 0 {
                    self.crashed = true;
                }
                glog!(self, LogLevel::GameError, "Jumped to invalid address: {:08X}", address);
                return;
            }
        }
        self.update_active_cycles();
        let width = if self.cpu.cpsr.t() { WORD_SIZE_THUMB } else { WORD_SIZE_ARM };
        self.cpu.memory.active_mask &= !(width - 1);
    }

    /// `GBALoadBad`: open bus.
    pub(crate) fn load_bad(&self) -> u32 {
        let cpu = &self.cpu;
        let width = if cpu.execution_mode == ExecutionMode::Thumb { WORD_SIZE_THUMB } else { WORD_SIZE_ARM };
        if self.performing_dma != 0 || cpu.gprs[ARM_PC].wrapping_sub(self.dma_pc) == width {
            self.bus
        } else {
            let mut value = cpu.prefetch[1];
            if cpu.execution_mode == ExecutionMode::Thumb {
                match cpu.gprs[ARM_PC] >> BASE_OFFSET {
                    REGION_BIOS | REGION_OAM => {
                        value <<= 16;
                        value |= cpu.prefetch[0];
                    }
                    REGION_IWRAM => {
                        if cpu.gprs[ARM_PC] & 2 != 0 {
                            value <<= 16;
                            value |= cpu.prefetch[0];
                        } else {
                            value |= cpu.prefetch[0] << 16;
                        }
                    }
                    _ => value |= value << 16,
                }
            }
            value
        }
    }

    #[inline(always)]
    fn finish_wait(&mut self, address: u32, mut wait: i32, cc: Option<&mut i32>) {
        if let Some(cc) = cc {
            if address < BASE_ROM0 {
                wait = self.memory_stall(wait);
            }
            *cc += wait;
        }
    }

    /// The body of LOAD_* for 32-bit loads (shared with LDM). Returns (value, wait).
    #[inline(always)]
    fn load32_region(&mut self, address: u32, ws: &[i32; 256]) -> (u32, i32, bool) {
        let mut assign = false;
        let mut wait = 0;
        let value = match address >> BASE_OFFSET {
            REGION_BIOS => {
                if address < SIZE_BIOS {
                    if self.memory.active_region == REGION_BIOS as i32 {
                        rd32(&self.memory.bios, (address & !3) as usize)
                    } else {
                        glog!(self, LogLevel::GameError, "Bad BIOS Load32: 0x{:08X}", address);
                        self.memory.bios_prefetch
                    }
                } else {
                    glog!(self, LogLevel::GameError, "Bad memory Load32: 0x{:08X}", address);
                    self.load_bad()
                }
            }
            REGION_EWRAM => {
                wait += ws[REGION_EWRAM as usize];
                rd32(&self.memory.wram, (address & (SIZE_EWRAM - 4)) as usize)
            }
            REGION_IWRAM => rd32(&self.memory.iwram, (address & (SIZE_IWRAM - 4)) as usize),
            REGION_IO => {
                let lo = self.io_read(address & OFFSET_MASK & !3) as u32;
                let hi = self.io_read((address & OFFSET_MASK & !1) | 2) as u32;
                lo | (hi << 16)
            }
            REGION_PALETTE_RAM => {
                wait += ws[REGION_PALETTE_RAM as usize];
                hw32(&self.video.palette, (address & (SIZE_PALETTE_RAM - 4)) as usize)
            }
            REGION_VRAM => {
                let v = if (address & 0x0001FFFF) >= SIZE_VRAM {
                    if (address & (SIZE_VRAM | 0x00014000)) == SIZE_VRAM && self.dispcnt_mode() >= 3 {
                        glog!(self, LogLevel::GameError, "Bad VRAM Load32: 0x{:08X}", address);
                        0
                    } else {
                        hw32(&self.video.vram, (address & 0x00017FFC) as usize)
                    }
                } else {
                    hw32(&self.video.vram, (address & 0x0001FFFC) as usize)
                };
                wait += 1;
                if self.video.stall_mask != 0 && (address & 0x0001FFFF) < self.vram_obj_boundary() {
                    wait += self.memory_stall_vram(wait, 1);
                }
                v
            }
            REGION_OAM => hw32(&self.video.oam, (address & (SIZE_OAM - 4)) as usize),
            REGION_ROM0..=REGION_ROM2_EX => {
                wait += ws[(address >> BASE_OFFSET) as usize];
                if (address & (SIZE_ROM0 - 4)) < self.memory.rom_size {
                    self.rom_read32((address & (SIZE_ROM0 - 4)) as usize)
                } else {
                    glog!(self, LogLevel::GameError, "Out of bounds ROM Load32: 0x{:08X}", address);
                    let mut v = ((address & !3) >> 1) & 0xFFFF;
                    v |= (((address & !3) + 2) >> 1) << 16;
                    v
                }
            }
            REGION_SRAM | REGION_SRAM_MIRROR => {
                wait = self.memory.waitstates_nonseq16[(address >> BASE_OFFSET) as usize];
                assign = true;
                let mut v = self.mem_load8(address, None);
                v |= v << 8;
                v |= v << 16;
                v
            }
            _ => {
                glog!(self, LogLevel::GameError, "Bad memory Load32: 0x{:08X}", address);
                self.load_bad()
            }
        };
        (value, wait, assign)
    }

    /// `GBALoad32`
    pub(crate) fn mem_load32(&mut self, address: u32, cc: Option<&mut i32>) -> u32 {
        let ws = self.memory.waitstates_nonseq32;
        let (value, wait, _) = self.load32_region(address, &ws);
        self.finish_wait(address, wait + 2, cc);
        // Unaligned 32-bit loads are "rotated" so they make some semblance of sense
        value.rotate_right((address & 3) << 3)
    }

    /// `GBALoad16`
    pub(crate) fn mem_load16(&mut self, address: u32, cc: Option<&mut i32>) -> u32 {
        let mut wait = 0;
        let value: u32 = match address >> BASE_OFFSET {
            REGION_BIOS => {
                if address < SIZE_BIOS {
                    if self.memory.active_region == REGION_BIOS as i32 {
                        rd16(&self.memory.bios, (address & !1) as usize)
                    } else {
                        glog!(self, LogLevel::GameError, "Bad BIOS Load16: 0x{:08X}", address);
                        (self.memory.bios_prefetch >> ((address & 2) * 8)) & 0xFFFF
                    }
                } else {
                    glog!(self, LogLevel::GameError, "Bad memory Load16: 0x{:08X}", address);
                    (self.load_bad() >> ((address & 2) * 8)) & 0xFFFF
                }
            }
            REGION_EWRAM => {
                wait = self.memory.waitstates_nonseq16[REGION_EWRAM as usize];
                rd16(&self.memory.wram, (address & (SIZE_EWRAM - 2)) as usize)
            }
            REGION_IWRAM => rd16(&self.memory.iwram, (address & (SIZE_IWRAM - 2)) as usize),
            REGION_IO => self.io_read(address & (OFFSET_MASK - 1)) as u32,
            REGION_PALETTE_RAM => self.video.palette[((address & (SIZE_PALETTE_RAM - 2)) >> 1) as usize] as u32,
            REGION_VRAM => {
                let v;
                if (address & 0x0001FFFF) >= SIZE_VRAM {
                    if (address & (SIZE_VRAM | 0x00014000)) == SIZE_VRAM && self.dispcnt_mode() >= 3 {
                        glog!(self, LogLevel::GameError, "Bad VRAM Load16: 0x{:08X}", address);
                        v = 0;
                    } else {
                        v = self.video.vram[((address & 0x00017FFE) >> 1) as usize] as u32;
                        if self.video.stall_mask != 0 && (address & 0x0001FFFF) < self.vram_obj_boundary() {
                            wait += self.memory_stall_vram(wait, 0);
                        }
                    }
                } else {
                    v = self.video.vram[((address & 0x0001FFFE) >> 1) as usize] as u32;
                    if self.video.stall_mask != 0 && (address & 0x0001FFFF) < self.vram_obj_boundary() {
                        wait += self.memory_stall_vram(wait, 0);
                    }
                }
                v
            }
            REGION_OAM => self.video.oam[((address & (SIZE_OAM - 2)) >> 1) as usize] as u32,
            REGION_ROM0..=REGION_ROM2 => {
                wait = self.memory.waitstates_nonseq16[(address >> BASE_OFFSET) as usize];
                if (address & (SIZE_ROM0 - 2)) < self.memory.rom_size {
                    self.rom_read16((address & (SIZE_ROM0 - 2)) as usize)
                } else {
                    glog!(self, LogLevel::GameError, "Out of bounds ROM Load16: 0x{:08X}", address);
                    (address >> 1) & 0xFFFF
                }
            }
            REGION_ROM2_EX => {
                wait = self.memory.waitstates_nonseq16[(address >> BASE_OFFSET) as usize];
                if self.memory.savedata.is_eeprom() {
                    let busy = self.timing.is_scheduled(EventId::SavedataDust);
                    self.memory.savedata.read_eeprom(busy) as u32
                } else if (address & (SIZE_ROM0 - 2)) < self.memory.rom_size {
                    self.rom_read16((address & (SIZE_ROM0 - 2)) as usize)
                } else {
                    glog!(self, LogLevel::GameError, "Out of bounds ROM Load16: 0x{:08X}", address);
                    (address >> 1) & 0xFFFF
                }
            }
            REGION_SRAM | REGION_SRAM_MIRROR => {
                wait = self.memory.waitstates_nonseq16[(address >> BASE_OFFSET) as usize];
                let mut v = self.mem_load8(address, None);
                v |= v << 8;
                v
            }
            _ => {
                glog!(self, LogLevel::GameError, "Bad memory Load16: 0x{:08X}", address);
                (self.load_bad() >> ((address & 2) * 8)) & 0xFFFF
            }
        };
        self.finish_wait(address, wait + 2, cc);
        // Unaligned 16-bit loads are "unpredictable", but the GBA rotates them, so we have to, too.
        value.rotate_right((address & 1) << 3)
    }

    /// `GBALoad8`
    pub(crate) fn mem_load8(&mut self, address: u32, cc: Option<&mut i32>) -> u32 {
        let mut wait = 0;
        let value: u32 = match address >> BASE_OFFSET {
            REGION_BIOS => {
                if address < SIZE_BIOS {
                    if self.memory.active_region == REGION_BIOS as i32 {
                        self.memory.bios[address as usize] as u32
                    } else {
                        glog!(self, LogLevel::GameError, "Bad BIOS Load8: 0x{:08X}", address);
                        (self.memory.bios_prefetch >> ((address & 3) * 8)) & 0xFF
                    }
                } else {
                    glog!(self, LogLevel::GameError, "Bad memory Load8: 0x{:08X}", address);
                    (self.load_bad() >> ((address & 3) * 8)) & 0xFF
                }
            }
            REGION_EWRAM => {
                wait = self.memory.waitstates_nonseq16[REGION_EWRAM as usize];
                self.memory.wram[(address & (SIZE_EWRAM - 1)) as usize] as u32
            }
            REGION_IWRAM => self.memory.iwram[(address & (SIZE_IWRAM - 1)) as usize] as u32,
            REGION_IO => ((self.io_read(address & 0xFFFE) as u32) >> ((address & 1) << 3)) & 0xFF,
            REGION_PALETTE_RAM => {
                let a = address & (SIZE_PALETTE_RAM - 1);
                ((self.video.palette[(a >> 1) as usize] as u32) >> ((a & 1) * 8)) & 0xFF
            }
            REGION_VRAM => {
                let v;
                if (address & 0x0001FFFF) >= SIZE_VRAM {
                    if (address & (SIZE_VRAM | 0x00014000)) == SIZE_VRAM && self.dispcnt_mode() >= 3 {
                        glog!(self, LogLevel::GameError, "Bad VRAM Load8: 0x{:08X}", address);
                        v = 0;
                    } else {
                        let a = address & 0x00017FFF;
                        v = ((self.video.vram[(a >> 1) as usize] as u32) >> ((a & 1) * 8)) & 0xFF;
                        if self.video.stall_mask != 0 {
                            wait += self.memory_stall_vram(wait, 0);
                        }
                    }
                } else {
                    let a = address & 0x0001FFFF;
                    v = ((self.video.vram[(a >> 1) as usize] as u32) >> ((a & 1) * 8)) & 0xFF;
                    if self.video.stall_mask != 0 {
                        wait += self.memory_stall_vram(wait, 0);
                    }
                }
                v
            }
            REGION_OAM => {
                let a = address & (SIZE_OAM - 1);
                ((self.video.oam[(a >> 1) as usize] as u32) >> ((a & 1) * 8)) & 0xFF
            }
            REGION_ROM0..=REGION_ROM2_EX => {
                wait = self.memory.waitstates_nonseq16[(address >> BASE_OFFSET) as usize];
                if (address & (SIZE_ROM0 - 1)) < self.memory.rom_size {
                    self.rom_read8((address & (SIZE_ROM0 - 1)) as usize)
                } else {
                    glog!(self, LogLevel::GameError, "Out of bounds ROM Load8: 0x{:08X}", address);
                    ((address >> 1) >> ((address & 1) * 8)) & 0xFF
                }
            }
            REGION_SRAM | REGION_SRAM_MIRROR => {
                wait = self.memory.waitstates_nonseq16[(address >> BASE_OFFSET) as usize];
                if self.memory.savedata.ty == SavedataType::Autodetect {
                    glog!(self, LogLevel::Info, "Detected SRAM savegame");
                    self.memory.savedata.init_sram();
                }
                if self.performing_dma == 1 {
                    0
                } else {
                    let busy = self.timing.is_scheduled(EventId::SavedataDust);
                    let sd = &mut self.memory.savedata;
                    let v = match sd.ty {
                        SavedataType::Sram => sd.data[(address & (SIZE_SRAM - 1)) as usize] as u32,
                        SavedataType::Flash512 | SavedataType::Flash1M => sd.read_flash((address & 0xFFFF) as u16, busy) as u32,
                        SavedataType::Sram512 => sd.data[(address & (SIZE_SRAM512 - 1)) as usize] as u32,
                        _ => {
                            glog!(self, LogLevel::GameError, "Reading from non-existent SRAM: 0x{:08X}", address);
                            0xFF
                        }
                    };
                    v & 0xFF
                }
            }
            _ => {
                glog!(self, LogLevel::GameError, "Bad memory Load8: 0x{:08X}", address);
                (self.load_bad() >> ((address & 3) * 8)) & 0xFF
            }
        };
        self.finish_wait(address, wait + 2, cc);
        value
    }

    #[inline(always)]
    fn store_palette32(&mut self, address: u32, value: u32) {
        let a = (address & (SIZE_PALETTE_RAM - 4)) as usize;
        let old = hw32(&self.video.palette, a);
        if old != value {
            self.video.palette[a >> 1] = value as u16;
            self.video.palette[(a >> 1) + 1] = (value >> 16) as u16;
            self.renderer.write_palette((a + 2) as u32, (value >> 16) as u16);
            self.renderer.write_palette(a as u32, value as u16);
        }
    }

    #[inline(always)]
    fn store_vram32(&mut self, address: u32, value: u32, wait: &mut i32) {
        if (address & 0x0001FFFF) >= SIZE_VRAM {
            if (address & (SIZE_VRAM | 0x00014000)) == SIZE_VRAM && self.dispcnt_mode() >= 3 {
                glog!(self, LogLevel::GameError, "Bad VRAM Store32: 0x{:08X}", address);
            } else {
                self.vram_write32((address & 0x00017FFC) as usize, value);
            }
        } else {
            self.vram_write32((address & 0x0001FFFC) as usize, value);
        }
        *wait += 1;
        if self.video.stall_mask != 0 && (address & 0x0001FFFF) < self.vram_obj_boundary() {
            *wait += self.memory_stall_vram(*wait, 1);
        }
    }

    #[inline(always)]
    fn vram_write32(&mut self, a: usize, value: u32) {
        let old = hw32(&self.video.vram, a);
        if old != value {
            self.video.vram[a >> 1] = value as u16;
            self.video.vram[(a >> 1) + 1] = (value >> 16) as u16;
            self.renderer.write_vram((a + 2) as u32);
            self.renderer.write_vram(a as u32);
        }
    }

    #[inline(always)]
    fn store_oam32(&mut self, address: u32, value: u32) {
        let a = (address & (SIZE_OAM - 4)) as usize;
        let old = hw32(&self.video.oam, a);
        if old != value {
            self.video.oam[a >> 1] = value as u16;
            self.video.oam[(a >> 1) + 1] = (value >> 16) as u16;
            self.renderer.write_oam((a >> 1) as u32);
            self.renderer.write_oam(((a >> 1) + 1) as u32);
        }
    }

    /// Body of GBAStore32 for one region (shared with STM).
    #[inline(always)]
    fn store32_region(&mut self, address: u32, value: u32, ws: &[i32; 256], wait: &mut i32, cc: Option<&mut i32>) {
        match address >> BASE_OFFSET {
            REGION_EWRAM => {
                wr32(&mut self.memory.wram, (address & (SIZE_EWRAM - 4)) as usize, value);
                *wait += ws[REGION_EWRAM as usize];
            }
            REGION_IWRAM => wr32(&mut self.memory.iwram, (address & (SIZE_IWRAM - 4)) as usize, value),
            REGION_IO => self.io_write32(address & (OFFSET_MASK - 3), value),
            REGION_PALETTE_RAM => {
                self.store_palette32(address, value);
                *wait += ws[REGION_PALETTE_RAM as usize];
            }
            REGION_VRAM => self.store_vram32(address, value, wait),
            REGION_OAM => self.store_oam32(address, value),
            REGION_ROM0..=REGION_ROM2_EX => {
                *wait += ws[(address >> BASE_OFFSET) as usize];
                glog!(self, LogLevel::Stub, "Unimplemented memory Store32: 0x{:08X}", address);
            }
            REGION_SRAM | REGION_SRAM_MIRROR => {
                self.mem_store8(address, (value >> (8 * (address & 3))) as u8, cc);
            }
            _ => {
                glog!(self, LogLevel::GameError, "Bad memory Store32: 0x{:08X}", address);
            }
        }
    }

    /// `GBAStore32`
    pub(crate) fn mem_store32(&mut self, address: u32, value: u32, mut cc: Option<&mut i32>) {
        let mut wait = 0;
        let ws = self.memory.waitstates_nonseq32;
        self.store32_region(address, value, &ws, &mut wait, cc.as_deref_mut());
        self.finish_wait(address, wait + 1, cc);
    }

    /// `GBAStore16`
    pub(crate) fn mem_store16(&mut self, address: u32, value: u16, mut cc: Option<&mut i32>) {
        let mut wait = 0;
        match address >> BASE_OFFSET {
            REGION_EWRAM => {
                wr16(&mut self.memory.wram, (address & (SIZE_EWRAM - 2)) as usize, value);
                wait = self.memory.waitstates_nonseq16[REGION_EWRAM as usize];
            }
            REGION_IWRAM => wr16(&mut self.memory.iwram, (address & (SIZE_IWRAM - 2)) as usize, value),
            REGION_IO => self.io_write(address & (OFFSET_MASK - 1), value),
            REGION_PALETTE_RAM => {
                let a = (address & (SIZE_PALETTE_RAM - 2)) as usize;
                if self.video.palette[a >> 1] != value {
                    self.video.palette[a >> 1] = value;
                    self.renderer.write_palette(a as u32, value);
                }
            }
            REGION_VRAM => {
                let a;
                if (address & 0x0001FFFF) >= SIZE_VRAM {
                    if (address & (SIZE_VRAM | 0x00014000)) == SIZE_VRAM && self.dispcnt_mode() >= 3 {
                        glog!(self, LogLevel::GameError, "Bad VRAM Store16: 0x{:08X}", address);
                        self.finish_wait(address, wait + 1, cc);
                        return;
                    }
                    a = address & 0x00017FFE;
                } else {
                    a = address & 0x0001FFFE;
                }
                if self.video.vram[(a >> 1) as usize] != value {
                    self.video.vram[(a >> 1) as usize] = value;
                    self.renderer.write_vram(a);
                }
                if self.video.stall_mask != 0 && (address & 0x0001FFFF) < self.vram_obj_boundary() {
                    wait += self.memory_stall_vram(wait, 0);
                }
            }
            REGION_OAM => {
                let a = (address & (SIZE_OAM - 2)) as usize;
                if self.video.oam[a >> 1] != value {
                    self.video.oam[a >> 1] = value;
                    self.renderer.write_oam((a >> 1) as u32);
                }
            }
            REGION_ROM0 | REGION_ROM0_EX => {
                if address >> BASE_OFFSET == REGION_ROM0 && super::gpio::is_gpio_register(address & 0xFFFFFE) {
                    if !self.memory.hw.has_gpio() {
                        glog!(self, LogLevel::Warn, "Write to GPIO address {:08X} on cartridge without GPIO", address);
                    } else {
                        self.gpio_write(address & 0xFFFFFE, value);
                    }
                } else {
                    glog!(self, LogLevel::GameError, "Bad cartridge Store16: 0x{:08X}", address);
                }
            }
            REGION_ROM2_EX => {
                if self.memory.savedata.ty == SavedataType::Autodetect {
                    glog!(self, LogLevel::Info, "Detected EEPROM savegame");
                    self.memory.savedata.init_eeprom();
                }
                if self.memory.savedata.is_eeprom() {
                    if self.memory.savedata.write_eeprom(value, 1) {
                        self.deschedule(EventId::SavedataDust);
                        self.schedule(EventId::SavedataDust, savedata_settle_cycles());
                    }
                } else {
                    glog!(self, LogLevel::GameError, "Bad memory Store16: 0x{:08X}", address);
                }
            }
            REGION_SRAM | REGION_SRAM_MIRROR => {
                let mut v = value;
                if address & 1 != 0 {
                    glog!(self, LogLevel::GameError, "Unaligned SRAM Store16: 0x{:08X}", address);
                    v >>= 8;
                }
                self.mem_store8(address, v as u8, cc.as_deref_mut());
            }
            _ => {
                glog!(self, LogLevel::GameError, "Bad memory Store16: 0x{:08X}", address);
            }
        }
        self.finish_wait(address, wait + 1, cc);
    }

    /// `GBAStore8`
    pub(crate) fn mem_store8(&mut self, address: u32, value: u8, mut cc: Option<&mut i32>) {
        let mut wait = 0;
        match address >> BASE_OFFSET {
            REGION_EWRAM => {
                self.memory.wram[(address & (SIZE_EWRAM - 1)) as usize] = value;
                wait = self.memory.waitstates_nonseq16[REGION_EWRAM as usize];
            }
            REGION_IWRAM => self.memory.iwram[(address & (SIZE_IWRAM - 1)) as usize] = value,
            REGION_IO => self.io_write8(address & OFFSET_MASK, value),
            REGION_PALETTE_RAM => {
                self.mem_store16(address & !1, (value as u16) | ((value as u16) << 8), cc.as_deref_mut());
            }
            REGION_VRAM => {
                if (address & 0x0001FFFF) >= self.vram_obj_boundary() {
                    glog!(self, LogLevel::GameError, "Cannot Store8 to OBJ: 0x{:08X}", address);
                } else {
                    let idx = ((address & 0x1FFFE) >> 1) as usize;
                    let v = (value as u16) | ((value as u16) << 8);
                    if idx < self.video.vram.len() && self.video.vram[idx] != v {
                        self.video.vram[idx] = v;
                        self.renderer.write_vram(address & 0x0001FFFE);
                    }
                    if self.video.stall_mask != 0 {
                        wait += self.memory_stall_vram(wait, 0);
                    }
                }
            }
            REGION_OAM => {
                glog!(self, LogLevel::GameError, "Cannot Store8 to OAM: 0x{:08X}", address);
            }
            REGION_ROM0 => {
                glog!(self, LogLevel::Stub, "Unimplemented memory Store8: 0x{:08X}", address);
            }
            REGION_SRAM | REGION_SRAM_MIRROR => {
                if self.memory.savedata.ty == SavedataType::Autodetect {
                    if address == SAVEDATA_FLASH_BASE {
                        glog!(self, LogLevel::Info, "Detected Flash savegame");
                        self.memory.savedata.init_flash();
                    } else {
                        glog!(self, LogLevel::Info, "Detected SRAM savegame");
                        self.memory.savedata.init_sram();
                    }
                }
                match self.memory.savedata.ty {
                    SavedataType::Flash512 | SavedataType::Flash1M => {
                        if let Some(cycles) = self.memory.savedata.write_flash((address & 0xFFFF) as u16, value) {
                            self.deschedule(EventId::SavedataDust);
                            self.schedule(EventId::SavedataDust, cycles);
                        }
                    }
                    SavedataType::Sram => {
                        self.memory.savedata.data[(address & (SIZE_SRAM - 1)) as usize] = value;
                        self.memory.savedata.dirty |= savedata::DIRT_NEW;
                    }
                    SavedataType::Sram512 => {
                        self.memory.savedata.data[(address & (SIZE_SRAM512 - 1)) as usize] = value;
                        self.memory.savedata.dirty |= savedata::DIRT_NEW;
                    }
                    _ => {
                        glog!(self, LogLevel::GameError, "Writing to non-existent SRAM: 0x{:08X}", address);
                    }
                }
                wait = self.memory.waitstates_nonseq16[REGION_SRAM as usize];
            }
            _ => {
                glog!(self, LogLevel::GameError, "Bad memory Store8: 0x{:08X}", address);
            }
        }
        self.finish_wait(address, wait + 1, cc);
    }

    /// `GBAView32`-style side-effect-free read for debuggers/frontends.
    pub fn view8(&mut self, address: u32) -> u8 {
        match address >> BASE_OFFSET {
            REGION_BIOS => {
                if address < SIZE_BIOS {
                    self.memory.bios[address as usize]
                } else {
                    0
                }
            }
            REGION_IO => {
                let a = address & OFFSET_MASK & !1;
                let v = if a < regs::MAX || a == regs::POSTFLG { self.memory.io[(a >> 1) as usize] } else { 0 };
                (v >> ((address & 1) * 8)) as u8
            }
            REGION_EWRAM | REGION_IWRAM | REGION_PALETTE_RAM | REGION_VRAM | REGION_OAM | REGION_ROM0..=REGION_ROM2_EX => {
                let saved = (self.log_enabled, self.memory.active_region);
                self.log_enabled = false;
                let v = self.mem_load8(address, None) as u8;
                self.log_enabled = saved.0;
                v
            }
            _ => 0,
        }
    }

    /// `GBALoadMultiple`
    pub(crate) fn mem_load_multiple(&mut self, mut address: u32, mask: u32, direction: u32, cc: Option<&mut i32>) -> u32 {
        let ws = self.memory.waitstates_seq32;
        let mut offset: u32 = 4;
        let mut popcount = 0;
        if direction & LSM_D != 0 {
            offset = (-4i32) as u32;
            popcount = mask.count_ones();
            address = address.wrapping_sub((popcount << 2).wrapping_sub(4));
        }
        if direction & LSM_B != 0 {
            address = address.wrapping_add(offset);
        }

        let address_misalign = address & 0x3;
        let region = address >> BASE_OFFSET;
        if region < REGION_SRAM {
            address &= 0xFFFFFFFC;
        }
        let ridx = (region & 0xFF) as usize;
        let mut wait = self.memory.waitstates_seq32[ridx] - self.memory.waitstates_nonseq32[ridx];

        if mask == 0 {
            let (value, w, assign) = self.load32_region(address, &ws);
            wait = if assign { w } else { wait + w };
            self.cpu.gprs[ARM_PC] = value;
            wait += 16;
            address = address.wrapping_add(64);
        }
        for i in 0..16 {
            if mask & (1 << i) != 0 {
                let (value, w, assign) = self.load32_region(address, &ws);
                wait = if assign { w } else { wait + w };
                self.cpu.gprs[i] = value;
                wait += 1;
                address = address.wrapping_add(4);
            }
        }

        if let Some(cc) = cc {
            wait += 1;
            if address < BASE_ROM0 {
                wait = self.memory_stall(wait);
            }
            *cc += wait;
        }

        if direction & LSM_B != 0 {
            address = address.wrapping_sub(offset);
        }
        if direction & LSM_D != 0 {
            address = address.wrapping_sub((popcount << 2).wrapping_add(4));
        }
        address | address_misalign
    }

    /// `GBAStoreMultiple`
    pub(crate) fn mem_store_multiple(&mut self, mut address: u32, mask: u32, direction: u32, mut cc: Option<&mut i32>) -> u32 {
        let ws = self.memory.waitstates_seq32;
        let mut offset: u32 = 4;
        let mut popcount = 0;
        if direction & LSM_D != 0 {
            offset = (-4i32) as u32;
            popcount = mask.count_ones();
            address = address.wrapping_sub((popcount << 2).wrapping_sub(4));
        }
        if direction & LSM_B != 0 {
            address = address.wrapping_add(offset);
        }

        let address_misalign = address & 0x3;
        let region = address >> BASE_OFFSET;
        if region < REGION_SRAM {
            address &= 0xFFFFFFFC;
        }
        let ridx = (region & 0xFF) as usize;
        let mut wait = self.memory.waitstates_seq32[ridx] - self.memory.waitstates_nonseq32[ridx];

        if mask == 0 {
            let width = if self.cpu.execution_mode == ExecutionMode::Arm { WORD_SIZE_ARM } else { WORD_SIZE_THUMB };
            let value = self.cpu.gprs[ARM_PC].wrapping_add(width);
            self.store32_region(address, value, &ws, &mut wait, cc.as_deref_mut());
            wait += 16;
            address = address.wrapping_add(64);
        }
        for i in 0..16 {
            if mask & (1 << i) != 0 {
                let mut value = self.cpu.gprs[i];
                if i == ARM_PC {
                    value = value.wrapping_add(WORD_SIZE_ARM);
                }
                self.store32_region(address, value, &ws, &mut wait, cc.as_deref_mut());
                wait += 1;
                address = address.wrapping_add(4);
            }
        }

        if let Some(cc) = cc {
            if address < BASE_ROM0 {
                wait = self.memory_stall(wait);
            }
            *cc += wait;
        }

        if direction & LSM_B != 0 {
            address = address.wrapping_sub(offset);
        }
        if direction & LSM_D != 0 {
            address = address.wrapping_sub((popcount << 2).wrapping_add(4));
        }
        address | address_misalign
    }

    /// `GBAAdjustWaitstates`
    pub(crate) fn adjust_waitstates(&mut self, parameters: u16) {
        let m = &mut self.memory;
        let sram = (parameters & 0x0003) as usize;
        let ws0 = ((parameters & 0x000C) >> 2) as usize;
        let ws0seq = ((parameters & 0x0010) >> 4) as usize;
        let ws1 = ((parameters & 0x0060) >> 5) as usize;
        let ws1seq = ((parameters & 0x0080) >> 7) as usize;
        let ws2 = ((parameters & 0x0300) >> 8) as usize;
        let ws2seq = ((parameters & 0x0400) >> 10) as usize;
        let prefetch = parameters & 0x4000 != 0;

        let s = REGION_SRAM as usize;
        let sm = REGION_SRAM_MIRROR as usize;
        m.waitstates_nonseq16[s] = ROM_WAITSTATES[sram];
        m.waitstates_nonseq16[sm] = ROM_WAITSTATES[sram];
        m.waitstates_seq16[s] = ROM_WAITSTATES[sram];
        m.waitstates_seq16[sm] = ROM_WAITSTATES[sram];
        m.waitstates_nonseq32[s] = 2 * ROM_WAITSTATES[sram] + 1;
        m.waitstates_nonseq32[sm] = 2 * ROM_WAITSTATES[sram] + 1;
        m.waitstates_seq32[s] = 2 * ROM_WAITSTATES[sram] + 1;
        m.waitstates_seq32[sm] = 2 * ROM_WAITSTATES[sram] + 1;

        let set = |arr: &mut [i32; 256], r: u32, v: i32| {
            arr[r as usize] = v;
            arr[r as usize + 1] = v;
        };
        set(&mut m.waitstates_nonseq16, REGION_ROM0, ROM_WAITSTATES[ws0]);
        set(&mut m.waitstates_nonseq16, REGION_ROM1, ROM_WAITSTATES[ws1]);
        set(&mut m.waitstates_nonseq16, REGION_ROM2, ROM_WAITSTATES[ws2]);

        set(&mut m.waitstates_seq16, REGION_ROM0, ROM_WAITSTATES_SEQ[ws0seq]);
        set(&mut m.waitstates_seq16, REGION_ROM1, ROM_WAITSTATES_SEQ[ws1seq + 2]);
        set(&mut m.waitstates_seq16, REGION_ROM2, ROM_WAITSTATES_SEQ[ws2seq + 4]);

        for r in [REGION_ROM0, REGION_ROM1, REGION_ROM2] {
            let n = m.waitstates_nonseq16[r as usize] + 1 + m.waitstates_seq16[r as usize];
            set(&mut m.waitstates_nonseq32, r, n);
            let sq = 2 * m.waitstates_seq16[r as usize] + 1;
            set(&mut m.waitstates_seq32, r, sq);
        }

        m.prefetch = prefetch;
        self.update_active_cycles_raw();

        if self.performing_dma != 0 {
            self.dma_recalculate_cycles();
        }
    }

    fn update_active_cycles_raw(&mut self) {
        // mGBA indexes the waitstate tables with activeRegion even when it is -1,
        // which reads garbage; we leave the cached cycles unchanged instead.
        self.update_active_cycles();
    }

    /// `GBAAdjustEWRAMWaitstates`
    pub(crate) fn adjust_ewram_waitstates(&mut self, parameters: u16) {
        let wait = 15 - ((parameters >> 8) & 0xF) as i32;
        if wait != 0 {
            let r = REGION_EWRAM as usize;
            self.memory.waitstates_nonseq16[r] = wait;
            self.memory.waitstates_seq16[r] = wait;
            self.memory.waitstates_nonseq32[r] = 2 * wait + 1;
            self.memory.waitstates_seq32[r] = 2 * wait + 1;
            self.update_active_cycles_raw();
        } else {
            glog!(self, LogLevel::GameError, "Cannot set EWRAM to 0 waitstates");
        }
    }

    /// `GBAMemoryStall`
    pub(crate) fn memory_stall(&mut self, mut wait: i32) -> i32 {
        if self.memory.active_region < REGION_ROM0 as i32 || !self.memory.prefetch {
            // The wait is the stall
            return wait;
        }

        let mut previous_loads = 0;

        // Don't prefetch too much if we're overlapping with a previous prefetch
        let dist = self.memory.last_prefetched_pc.wrapping_sub(self.cpu.gprs[ARM_PC]);
        let mut max_loads = 8;
        if dist < 16 {
            previous_loads = (dist >> 1) as i32;
            max_loads -= previous_loads;
        }

        // Figure out how many sequential loads we can jam in
        let s = self.cpu.memory.active_seq_cycles16;
        let mut stall = s + 1;
        let mut loads = 1;

        while stall < wait && loads < max_loads {
            stall += s;
            loads += 1;
        }
        self.memory.last_prefetched_pc = self.cpu.gprs[ARM_PC]
            .wrapping_add(WORD_SIZE_THUMB.wrapping_mul((loads + previous_loads - 1) as u32));

        if stall > wait {
            // The wait cannot take less time than the prefetch stalls
            wait = stall;
        }

        // This instruction used to have an N, convert it to an S.
        wait -= self.cpu.memory.active_nonseq_cycles16 - s;

        // The next |loads|S waitstates disappear entirely, so long as they're all in a row
        wait -= stall;

        wait
    }

    /// `GBAMemoryStallVRAM`
    pub(crate) fn memory_stall_vram(&mut self, wait: i32, mut extra: i32) -> i32 {
        let until = self.until(EventId::Video);
        let period = ((-until) & 0x1F) as usize;
        let mut stall = until;
        for i in 0..16 {
            if VSTALL_LUT[(period + i) & 0x1F] as u32 & self.video.stall_mask == 0 {
                if extra == 0 {
                    stall = i as i32;
                    break;
                }
                extra -= 1;
            }
        }
        stall -= wait;
        if stall < 0 {
            0
        } else {
            stall
        }
    }
}

fn savedata_settle_cycles() -> i32 {
    super::savedata::EEPROM_SETTLE_CYCLES
}

use super::savedata;

impl crate::cheats::CheatBus for Gba {
    fn bus_read(&mut self, address: u32, width: i32) -> u32 {
        match width {
            1 => self.mem_load8(address, None),
            2 => self.mem_load16(address, None),
            _ => self.mem_load32(address, None),
        }
    }
    fn bus_write(&mut self, address: u32, width: i32, value: u32) {
        match width {
            1 => self.mem_store8(address, value as u8, None),
            2 => self.mem_store16(address, value as u16, None),
            _ => self.mem_store32(address, value, None),
        }
    }
    fn raw_read(&mut self, address: u32, _segment: i32, width: i32) -> u32 {
        (0..width.max(1) as u32).fold(0, |acc, i| acc | (self.view8(address.wrapping_add(i)) as u32) << (8 * i))
    }
    /// `GBAPatch8/16/32`: ROM writes change the loaded image; other regions
    /// are written like a normal store.
    fn raw_write(&mut self, address: u32, _segment: i32, width: i32, value: u32) {
        if (REGION_ROM0..=REGION_ROM2_EX).contains(&(address >> BASE_OFFSET)) {
            if std::sync::Arc::get_mut(&mut self.memory.rom).is_none() {
                self.memory.rom = std::sync::Arc::from(self.memory.rom.to_vec());
            }
            let rom = std::sync::Arc::get_mut(&mut self.memory.rom).unwrap();
            for i in 0..width.max(1) as u32 {
                let off = (address.wrapping_add(i) & (SIZE_ROM0 - 1)) as usize;
                if let Some(b) = rom.get_mut(off) {
                    *b = (value >> (8 * i)) as u8;
                }
            }
        } else {
            crate::cheats::CheatBus::bus_write(self, address, width, value);
        }
    }
    fn max_segment(&self, _address: u32) -> Option<i32> {
        Some(0)
    }
}
