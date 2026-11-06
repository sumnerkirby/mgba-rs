// Copyright (c) 2013-2015 Jeffrey Pfau
// Rust port of mGBA's GBA DMA controller (src/gba/dma.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::memory::{BASE_EWRAM, BASE_ROM0, BASE_SRAM, BASE_OFFSET, REGION_ROM0, REGION_ROM2_EX};
use super::savedata::SavedataType;
use super::{glog, regs, Gba, LogLevel, IRQ_DMA0, VIDEO_VERTICAL_PIXELS};
use crate::arm::ARM_PC;
use crate::timing::EventId;

pub const DMA_INCREMENT: u16 = 0;
pub const DMA_DECREMENT: u16 = 1;
pub const DMA_FIXED: u16 = 2;
pub const DMA_INCREMENT_RELOAD: u16 = 3;

pub const DMA_TIMING_NOW: u16 = 0;
pub const DMA_TIMING_VBLANK: u16 = 1;
pub const DMA_TIMING_HBLANK: u16 = 2;
pub const DMA_TIMING_CUSTOM: u16 = 3;

const DMA_OFFSET: [i32; 4] = [1, -1, 0, 1];
const DMA_SRC_MASK: [u32; 4] = [0x07FFFFFE, 0x0FFFFFFE, 0x0FFFFFFE, 0x0FFFFFFE];
const DMA_DST_MASK: [u32; 4] = [0x07FFFFFE, 0x07FFFFFE, 0x07FFFFFE, 0x0FFFFFFE];

/// `struct GBADMA`
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct Dma {
    pub reg: u16,
    pub source: u32,
    pub dest: u32,
    pub count: i32,
    pub next_source: u32,
    pub next_dest: u32,
    pub next_count: i32,
    pub when: u32,
    pub cycles: i32,
    pub latch: u32,
    pub source_offset: i32,
    pub dest_offset: i32,
}

impl Dma {
    #[inline]
    pub fn dest_control(&self) -> u16 {
        (self.reg >> 5) & 3
    }
    #[inline]
    pub fn src_control(&self) -> u16 {
        (self.reg >> 7) & 3
    }
    #[inline]
    pub fn repeat(&self) -> bool {
        self.reg & (1 << 9) != 0
    }
    #[inline]
    pub fn width(&self) -> u16 {
        (self.reg >> 10) & 1
    }
    #[inline]
    pub fn timing(&self) -> u16 {
        (self.reg >> 12) & 3
    }
    #[inline]
    pub fn do_irq(&self) -> bool {
        self.reg & (1 << 14) != 0
    }
    #[inline]
    pub fn enabled(&self) -> bool {
        self.reg & (1 << 15) != 0
    }
    #[inline]
    pub fn set_dest_control(&mut self, v: u16) {
        self.reg = (self.reg & !(3 << 5)) | ((v & 3) << 5);
    }
    #[inline]
    pub fn set_width(&mut self, v: u16) {
        self.reg = (self.reg & !(1 << 10)) | ((v & 1) << 10);
    }
}

fn is_valid_sad(dma: usize, address: u32) -> bool {
    if dma == 0 && (BASE_ROM0..BASE_SRAM).contains(&address) {
        return false;
    }
    address >= BASE_EWRAM
}

fn is_valid_dad(dma: usize, address: u32) -> bool {
    dma == 3 || address < BASE_ROM0
}

impl Gba {
    /// `GBADMAReset`
    pub(crate) fn dma_reset(&mut self) {
        self.memory.dma = [Dma::default(); 4];
        for i in 0..4 {
            self.memory.dma[i].count = 0x4000;
            self.memory.dma[i].latch = 0;
        }
        self.memory.dma[3].count = 0x10000;
        self.memory.active_dma = -1;
    }

    /// `GBADMAWriteSAD`
    pub(crate) fn dma_write_sad(&mut self, dma: usize, address: u32) -> u32 {
        if !is_valid_sad(dma, address) {
            glog!(self, LogLevel::GameError, "Invalid DMA source address: 0x{:08X}", address);
        }
        self.memory.dma[dma].source = address & DMA_SRC_MASK[dma];
        self.memory.dma[dma].source
    }

    /// `GBADMAWriteDAD`
    pub(crate) fn dma_write_dad(&mut self, dma: usize, address: u32) -> u32 {
        if !is_valid_dad(dma, address) {
            glog!(self, LogLevel::GameError, "Invalid DMA destination address: 0x{:08X}", address);
        }
        self.memory.dma[dma].dest = address & DMA_DST_MASK[dma];
        self.memory.dma[dma].dest
    }

    /// `GBADMAReload`
    fn dma_reload(&mut self, dma: usize) {
        let cnt = self.memory.io[((regs::DMA0CNT_LO + dma as u32 * (regs::DMA1CNT_LO - regs::DMA0CNT_LO)) >> 1) as usize] as i32;
        let d = &mut self.memory.dma[dma];
        d.count = cnt;
        if dma == 3 {
            if d.count == 0 {
                d.count = 0x10000;
            }
        } else {
            d.count &= 0x3FFF;
            if d.count == 0 {
                d.count = 0x4000;
            }
        }
    }

    /// `GBADMAWriteCNT_HI`
    pub(crate) fn dma_write_cnt_hi(&mut self, dma: usize, mut control: u16) -> u16 {
        let was_enabled = self.memory.dma[dma].enabled();
        if dma < 3 {
            control &= 0xF7E0;
        } else {
            control &= 0xFFE0;
        }
        let d = &mut self.memory.dma[dma];
        d.reg = control;

        let width = 2u32 << d.width();
        if (BASE_ROM0..BASE_SRAM).contains(&d.source) {
            d.source_offset = width as i32;
        } else {
            d.source_offset = DMA_OFFSET[d.src_control() as usize] * width as i32;
        }
        d.dest_offset = DMA_OFFSET[d.dest_control() as usize] * width as i32;

        if d.reg & (1 << 11) != 0 {
            glog!(self, LogLevel::Stub, "DRQ not implemented");
        }

        if !was_enabled && self.memory.dma[dma].enabled() {
            {
                let d = &mut self.memory.dma[dma];
                d.next_source = d.source;
                d.next_dest = d.dest;
            }
            self.dma_reload(dma);
            let d = self.memory.dma[dma];
            if d.next_source & (width - 1) != 0 {
                glog!(self, LogLevel::GameError, "Misaligned DMA source address: 0x{:08X}", d.next_source);
            }
            if d.next_dest & (width - 1) != 0 {
                glog!(self, LogLevel::GameError, "Misaligned DMA destination address: 0x{:08X}", d.next_dest);
            }
            glog!(self, LogLevel::Info, "Starting DMA {} 0x{:08X} -> 0x{:08X} ({:04X}:{:04X})", dma, d.next_source, d.next_dest, d.reg, d.count & 0xFFFF);
            let d = &mut self.memory.dma[dma];
            d.next_source &= !(width - 1);
            d.next_dest &= !(width - 1);

            if d.timing() == DMA_TIMING_NOW {
                d.next_count = d.count;
                let now = self.current_time();
                self.memory.dma[dma].when = (now as u32).wrapping_add(3); // DMAs take 3 cycles to start
                self.dma_update();
            }
        }
        // If the DMA has already occurred, this value might have changed since the function started
        self.memory.dma[dma].reg
    }

    fn dma_run_timing(&mut self, timing: u16, cycles: i32) {
        let mut found = false;
        let now = self.current_time();
        for i in 0..4 {
            let d = &mut self.memory.dma[i];
            if d.enabled() && d.timing() == timing {
                d.when = (now as u32).wrapping_add(3).wrapping_add(cycles as u32);
                if d.next_count == 0 {
                    d.next_count = d.count;
                }
                found = true;
            }
        }
        if found {
            self.dma_update();
        }
    }

    /// `GBADMARunHblank`
    pub(crate) fn dma_run_hblank(&mut self, cycles: i32) {
        self.dma_run_timing(DMA_TIMING_HBLANK, cycles);
    }

    /// `GBADMARunVblank`
    pub(crate) fn dma_run_vblank(&mut self, cycles: i32) {
        self.dma_run_timing(DMA_TIMING_VBLANK, cycles);
    }

    /// `GBADMARunDisplayStart`
    pub(crate) fn dma_run_display_start(&mut self, cycles: i32) {
        let now = self.current_time();
        let d = &mut self.memory.dma[3];
        if d.enabled() && d.timing() == DMA_TIMING_CUSTOM {
            d.when = (now as u32).wrapping_add(3).wrapping_add(cycles as u32);
            if d.next_count == 0 {
                d.next_count = d.count;
            }
            self.dma_update();
        }
    }

    /// `_dmaEvent`
    pub(crate) fn dma_event(&mut self, cycles_late: u32) {
        let active = self.memory.active_dma;
        if active < 0 {
            return;
        }
        let n = active as usize;
        if self.memory.dma[n].next_count == self.memory.dma[n].count {
            self.memory.dma[n].when = self.current_time() as u32;
        }
        if self.memory.dma[n].next_count & 0xFFFFF != 0 {
            self.dma_service(n);
        } else {
            let vcount = self.video.vcount;
            let d = &mut self.memory.dma[n];
            d.next_count = 0;
            let mut no_repeat = !d.repeat();
            no_repeat |= d.timing() == DMA_TIMING_NOW;
            no_repeat |= n == 3 && d.timing() == DMA_TIMING_CUSTOM && vcount == VIDEO_VERTICAL_PIXELS as i32 + 1;
            if no_repeat {
                d.reg &= !0x8000;
                // Clear the enable bit in memory
                self.memory.io[((regs::DMA0CNT_HI + n as u32 * (regs::DMA1CNT_HI - regs::DMA0CNT_HI)) >> 1) as usize] &= 0x7FE0;
            } else {
                self.dma_reload(n);
            }
            let d = &mut self.memory.dma[n];
            if d.dest_control() == DMA_INCREMENT_RELOAD {
                d.next_dest = d.dest;
            }
            if d.do_irq() {
                self.raise_irq(IRQ_DMA0 + n as u32, cycles_late);
            }
            self.dma_update();
        }
    }

    /// `GBADMAUpdate`
    pub fn dma_update(&mut self) {
        let current_time = self.current_time() as u32;
        let mut least_time = i32::MAX;
        self.memory.active_dma = -1;
        for i in 0..4 {
            let d = &self.memory.dma[i];
            if d.enabled() && d.next_count != 0 {
                let time = d.when.wrapping_sub(current_time) as i32;
                if self.memory.active_dma == -1 || time < least_time {
                    least_time = time;
                    self.memory.active_dma = i as i32;
                }
            }
        }

        if self.memory.active_dma >= 0 {
            self.dma_pc = self.cpu.gprs[ARM_PC];
            self.deschedule(EventId::Dma);
            let when = self.memory.dma[self.memory.active_dma as usize].when.wrapping_sub(current_time) as i32;
            self.schedule(EventId::Dma, when);
        } else {
            self.cpu_blocked = false;
        }
    }

    /// `GBADMAService`
    fn dma_service(&mut self, number: usize) {
        let mut info = self.memory.dma[number];
        let width = 2u32 << info.width();
        let source = info.next_source;
        let dest = info.next_dest;
        let source_region = (source >> BASE_OFFSET) as usize & 0xFF;
        let dest_region = (dest >> BASE_OFFSET) as usize & 0xFF;
        let mut cycles: i32 = 2;

        self.cpu_blocked = true;
        self.performing_dma = 1 | ((number as u32) << 1);

        if info.count == info.next_count {
            if width == 4 {
                cycles += self.memory.waitstates_nonseq32[source_region] + self.memory.waitstates_nonseq32[dest_region];
                info.cycles = self.memory.waitstates_seq32[source_region] + self.memory.waitstates_seq32[dest_region];
            } else {
                if source >= BASE_EWRAM {
                    info.latch = self.mem_load32(source, None);
                }
                cycles += self.memory.waitstates_nonseq16[source_region] + self.memory.waitstates_nonseq16[dest_region];
                info.cycles = self.memory.waitstates_seq16[source_region] + self.memory.waitstates_seq16[dest_region];
            }
        } else {
            cycles += info.cycles;
        }
        info.when = info.when.wrapping_add(cycles as u32);

        if width == 4 {
            if source >= BASE_EWRAM {
                info.latch = self.mem_load32(source, None);
            }
            self.memory.dma[number] = info;
            self.mem_store32(dest, info.latch, None);
            info = self.memory.dma[number];
            self.bus = info.latch;
        } else {
            if source_region as u32 == REGION_ROM2_EX && self.memory.savedata.is_eeprom() {
                let busy = self.timing.is_scheduled(EventId::SavedataDust);
                info.latch = self.memory.savedata.read_eeprom(busy) as u32;
                info.latch |= info.latch << 16;
            } else if source >= BASE_EWRAM {
                info.latch = self.mem_load16(source, None);
                info.latch |= info.latch << 16;
            }
            if dest_region as u32 == REGION_ROM2_EX {
                if self.memory.savedata.ty == SavedataType::Autodetect {
                    glog!(self, LogLevel::Info, "Detected EEPROM savegame");
                    self.memory.savedata.init_eeprom();
                }
                if self.memory.savedata.is_eeprom() && self.memory.savedata.write_eeprom(info.latch as u16, info.next_count as u32) {
                    self.deschedule(EventId::SavedataDust);
                    self.schedule(EventId::SavedataDust, super::savedata::EEPROM_SETTLE_CYCLES);
                }
                self.memory.dma[number] = info;
            } else {
                self.memory.dma[number] = info;
                self.mem_store16(dest, (info.latch >> (8 * (dest & 2))) as u16, None);
                info = self.memory.dma[number];
            }
            self.bus = (info.latch & 0xFFFF) | (info.latch << 16);
        }

        info.next_source = info.next_source.wrapping_add(info.source_offset as u32);
        info.next_dest = info.next_dest.wrapping_add(info.dest_offset as u32);
        if source_region as u32 != info.next_source >> BASE_OFFSET || dest_region as u32 != info.next_dest >> BASE_OFFSET {
            // Crossed region boundary
            if (BASE_ROM0..BASE_SRAM).contains(&info.next_source) {
                info.source_offset = width as i32;
            } else {
                info.source_offset = DMA_OFFSET[info.src_control() as usize] * width as i32;
            }
            // Recalculate cached cycles
            let ns = (info.next_source >> BASE_OFFSET) as usize & 0xFF;
            let nd = (info.next_dest >> BASE_OFFSET) as usize & 0xFF;
            if width == 4 {
                info.cycles = self.memory.waitstates_seq32[ns] + self.memory.waitstates_seq32[nd];
            } else {
                info.cycles = self.memory.waitstates_seq16[ns] + self.memory.waitstates_seq16[nd];
            }
        }
        info.next_count -= 1;

        self.performing_dma = 0;

        for i in 0..4 {
            if i == number {
                continue;
            }
            let d = &mut self.memory.dma[i];
            if d.enabled() && d.next_count != 0 {
                let time = d.when.wrapping_sub(info.when) as i32;
                if time < 0 {
                    d.when = info.when;
                }
            }
        }

        if info.next_count == 0 {
            info.next_count |= 0x80000000u32 as i32;
            if (source_region as u32) < REGION_ROM0 || (dest_region as u32) < REGION_ROM0 {
                info.when = info.when.wrapping_add(2);
            }
        }
        self.memory.dma[number] = info;
        self.dma_update();
    }

    /// `GBADMARecalculateCycles`
    pub(crate) fn dma_recalculate_cycles(&mut self) {
        for i in 0..4 {
            let d = self.memory.dma[i];
            if !d.enabled() {
                continue;
            }
            let s = (d.next_source >> BASE_OFFSET) as usize & 0xFF;
            let t = (d.next_dest >> BASE_OFFSET) as usize & 0xFF;
            self.memory.dma[i].cycles = if d.width() != 0 {
                self.memory.waitstates_seq32[s] + self.memory.waitstates_seq32[t]
            } else {
                self.memory.waitstates_seq16[s] + self.memory.waitstates_seq16[t]
            };
        }
    }
}
