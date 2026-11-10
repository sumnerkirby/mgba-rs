// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of the Game Boy's `struct mCore` hooks used by tools
// (src/gb/core.c: memory blocks, bus/raw access, registers, step) and its
// debugger glue (src/sm83/debugger/*.c, src/gb/debugger/debugger.c,
// src/gb/memory.c `GBCurrentSegment`).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::io::{IO_REGISTER_NAMES, REG_IF, REG_LCDC, REG_LY, REG_STAT};
use super::memory::*;
use super::{model, video, Gb, GbEvent};
use crate::cheats::CheatBus;
use crate::core::*;
use crate::debugger::{self, DebugCore, Platform as DebugPlatform};
use crate::sm83::decoder::{decode_byte, disassemble, InstructionInfo};
use crate::sm83::CORE_FETCH;

const fn rw(id: i64, internal: &'static str, short: &'static str, long: &'static str, start: u32, end: u32, size: u32) -> MemoryBlock {
    MemoryBlock::new(id, internal, short, long, start, end, size, MEMORY_RW | MEMORY_MAPPED)
}

/// `_GBRegisters`
const REGISTERS: &[RegisterInfo] = &[
    RegisterInfo { name: "b", aliases: &[], width: 1, mask: 0xFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "c", aliases: &[], width: 1, mask: 0xFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "d", aliases: &[], width: 1, mask: 0xFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "e", aliases: &[], width: 1, mask: 0xFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "h", aliases: &[], width: 1, mask: 0xFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "l", aliases: &[], width: 1, mask: 0xFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "a", aliases: &[], width: 1, mask: 0xFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "f", aliases: &[], width: 1, mask: 0xF0, ty: RegisterType::Gpr },
    RegisterInfo { name: "bc", aliases: &[], width: 2, mask: 0xFFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "de", aliases: &[], width: 2, mask: 0xFFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "hl", aliases: &[], width: 2, mask: 0xFFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "af", aliases: &[], width: 2, mask: 0xFFF0, ty: RegisterType::Gpr },
    RegisterInfo { name: "pc", aliases: &[], width: 2, mask: 0xFFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "sp", aliases: &[], width: 2, mask: 0xFFFF, ty: RegisterType::Gpr },
];

/// Regions as mGBA numbers them (`GB_REGION_*` = address >> 12).
const REGION_CART_BANK0: i64 = 0x0;
const REGION_VRAM: i64 = 0x8;
const REGION_EXTERNAL_RAM: i64 = 0xA;
const REGION_WORKING_RAM_BANK0: i64 = 0xC;

fn event_name(id: GbEvent) -> &'static str {
    match id {
        GbEvent::VideoMode => "GB Video Mode",
        GbEvent::VideoFrame => "GB Video Frame",
        GbEvent::AudioSample => "GB Audio Sample",
        GbEvent::TimerIrq => "GB Timer IRQ",
        GbEvent::Timer => "GB Timer",
        GbEvent::Sio => "GB SIO",
        GbEvent::Dma => "GB DMA",
        GbEvent::Hdma => "GB HDMA",
        GbEvent::Ei => "GB EI",
        GbEvent::SioLockstep => "GB SIO Lockstep",
    }
}

impl Gb {
    /// The SM83 watchpoint shim (`DebuggerShim_load8`/`store8`).
    #[inline]
    pub(crate) fn debug_watch(&mut self, address: u16, access: u32, value: u8) {
        if self.debugger.as_ref().is_some_and(|d| d.has_watchpoints())
            && debugger::check_watchpoints(self, address as u32, access, value as u32, 1)
        {
            self.interrupt();
        }
    }

    /// `GBCurrentSegment`
    pub fn current_segment_at(&self, address: u16) -> i32 {
        match address >> 12 {
            0x4..=0x7 => self.memory.current_bank,
            0x8 | 0x9 => self.video.vram_current_bank,
            0xA | 0xB => self.memory.sram_current_bank,
            0xD => self.memory.wram_current_bank,
            _ => 0,
        }
    }

    /// Decode the instruction at `address` with raw reads (`_printLine`).
    fn decode_at(&mut self, address: u16, segment: i32) -> (InstructionInfo, String, u16) {
        let mut info = InstructionInfo::default();
        let mut bytes = String::new();
        let mut address = address;
        let mut remaining = 1usize;
        while remaining > 0 {
            let instruction = self.raw_read(address as u32, segment, 1) as u8;
            bytes.push_str(&format!("{instruction:02X}"));
            address = address.wrapping_add(1);
            remaining = remaining - 1 + decode_byte(instruction, &mut info);
        }
        (info, bytes, address)
    }

    /// `_printLine` (SM83 CLI debugger)
    fn print_line(&mut self, address: u16, segment: i32) -> (String, u16) {
        let mut out = String::new();
        if segment >= 0 {
            out.push_str(&format!("{segment:02X}:"));
        }
        out.push_str(&format!("{address:04X}:  "));
        let (info, bytes, next) = self.decode_at(address, segment);
        out.push_str(&format!("{bytes}\t{}", disassemble(&info, next)));
        (out, next)
    }

    fn segments(&self) -> &'static [(&'static str, u16)] {
        if self.model >= model::CGB {
            &[("ROM", BASE_CART_BANK1), ("RAM", 0xA000), ("WRAM", 0xD000), ("VRAM", BASE_VRAM)]
        } else {
            &[("ROM", BASE_CART_BANK1), ("RAM", 0xA000)]
        }
    }
}

impl MCore for Gb {
    fn system(&self) -> System {
        System::Gb
    }

    /// `_GBListMemoryBlocks` with the sizes `_GBCoreReset` fills in.
    fn list_memory_blocks(&self) -> Vec<MemoryBlock> {
        let cgb = self.model >= model::CGB;
        let rom_banks = ((self.memory.rom_size / SIZE_CART_BANK0) as u16).saturating_sub(1);
        let sram_banks = ((self.sram_size / SIZE_EXTERNAL_RAM) as u16).saturating_sub(1);
        let mut blocks = vec![
            MemoryBlock::new(-1, "mem", "All", "All", 0, 0x10000, 0x10000, MEMORY_VIRTUAL),
            MemoryBlock::new(
                REGION_CART_BANK0,
                "cart0",
                "ROM Bank",
                "Game Pak (32kiB)",
                0,
                (SIZE_CART_BANK0 * 2) as u32,
                SIZE_CART_MAX as u32,
                MEMORY_READ | MEMORY_WORM | MEMORY_MAPPED,
            )
            .segmented(rom_banks, BASE_CART_BANK1 as u32),
        ];
        if cgb {
            blocks.push(rw(REGION_VRAM, "vram", "VRAM", "Video RAM (8kiB)", 0x8000, 0xA000, (SIZE_VRAM_BANK0 * 2) as u32).segmented(1, 0x8000));
        } else {
            blocks.push(rw(REGION_VRAM, "vram", "VRAM", "Video RAM (8kiB)", 0x8000, 0xA000, SIZE_VRAM_BANK0 as u32));
        }
        blocks.push(
            rw(REGION_EXTERNAL_RAM, "sram", "SRAM", "External RAM (8kiB)", 0xA000, 0xC000, (SIZE_EXTERNAL_RAM * 4) as u32)
                .segmented(sram_banks, 0xA000),
        );
        if cgb {
            blocks.push(
                rw(REGION_WORKING_RAM_BANK0, "wram", "WRAM", "Working RAM (8kiB)", 0xC000, 0xE000, (SIZE_WORKING_RAM_BANK0 * 8) as u32)
                    .segmented(7, 0xD000),
            );
        } else {
            blocks.push(rw(REGION_WORKING_RAM_BANK0, "wram", "WRAM", "Working RAM (8kiB)", 0xC000, 0xE000, (SIZE_WORKING_RAM_BANK0 * 2) as u32));
        }
        blocks.push(rw(BASE_OAM as i64, "oam", "OAM", "OBJ Attribute Memory", BASE_OAM as u32, BASE_OAM as u32 + SIZE_OAM as u32, SIZE_OAM as u32));
        blocks.push(rw(BASE_IO as i64, "io", "MMIO", "Memory-Mapped I/O", BASE_IO as u32, BASE_IO as u32 + SIZE_IO as u32, SIZE_IO as u32));
        blocks.push(rw(BASE_HRAM as i64, "hram", "HRAM", "High RAM", BASE_HRAM as u32, BASE_HRAM as u32 + SIZE_HRAM as u32, SIZE_HRAM as u32));
        blocks
    }

    /// `_GBGetMemoryBlock`
    fn memory_block(&self, id: i64) -> Option<Vec<u8>> {
        let cgb = self.model >= model::CGB;
        Some(match id {
            REGION_CART_BANK0 => self.memory.rom[..self.memory.rom_size.min(self.memory.rom.len())].to_vec(),
            // mGBA's size here is inverted (1 bank on CGB, 2 otherwise); kept.
            REGION_VRAM => self.video.vram[..(SIZE_VRAM_BANK0 * if cgb { 1 } else { 2 }).min(self.video.vram.len())].to_vec(),
            REGION_EXTERNAL_RAM => self.memory.sram[..self.sram_size.min(self.memory.sram.len())].to_vec(),
            REGION_WORKING_RAM_BANK0 => {
                self.memory.wram[..(SIZE_WORKING_RAM_BANK0 * if cgb { 8 } else { 2 }).min(self.memory.wram.len())].to_vec()
            }
            x if x == BASE_OAM as i64 => self.video.oam.to_vec(),
            x if x == BASE_HRAM as i64 => self.memory.hram.to_vec(),
            _ => return None,
        })
    }

    fn bus_read8(&mut self, address: u32) -> u32 {
        self.load8(address as u16) as u32
    }
    fn bus_read16(&mut self, address: u32) -> u32 {
        self.bus_read(address, 2)
    }
    fn bus_read32(&mut self, address: u32) -> u32 {
        self.bus_read(address, 4)
    }
    fn bus_write8(&mut self, address: u32, value: u32) {
        self.store8(address as u16, value as u8)
    }
    fn bus_write16(&mut self, address: u32, value: u32) {
        self.bus_write(address, 2, value)
    }
    fn bus_write32(&mut self, address: u32, value: u32) {
        self.bus_write(address, 4, value)
    }
    fn raw_read8(&mut self, address: u32, segment: i32) -> u32 {
        self.raw_read(address, segment, 1)
    }
    fn raw_read16(&mut self, address: u32, segment: i32) -> u32 {
        self.raw_read(address, segment, 2)
    }
    fn raw_read32(&mut self, address: u32, segment: i32) -> u32 {
        self.raw_read(address, segment, 4)
    }
    fn raw_write8(&mut self, address: u32, segment: i32, value: u32) {
        self.raw_write(address, segment, 1, value)
    }
    fn raw_write16(&mut self, address: u32, segment: i32, value: u32) {
        self.raw_write(address, segment, 2, value)
    }
    fn raw_write32(&mut self, address: u32, segment: i32, value: u32) {
        self.raw_write(address, segment, 4, value)
    }

    fn list_registers(&self) -> &'static [RegisterInfo] {
        REGISTERS
    }

    /// `_GBCoreReadRegister`
    fn read_register(&self, name: &str) -> Option<i32> {
        let c = &self.cpu;
        let pair = |hi: u8, lo: u8| ((hi as i32) << 8) | lo as i32;
        Some(match name.to_ascii_lowercase().as_str() {
            "b" => c.b as i32,
            "c" => c.c as i32,
            "d" => c.d as i32,
            "e" => c.e as i32,
            "h" => c.h as i32,
            "l" => c.l as i32,
            "a" => c.a as i32,
            "f" => c.f as i32,
            "bc" => pair(c.b, c.c),
            "de" => pair(c.d, c.e),
            "hl" => pair(c.h, c.l),
            "af" => pair(c.a, c.f),
            "pc" => c.pc as i32,
            "sp" => c.sp as i32,
            _ => return None,
        })
    }

    /// `_GBCoreWriteRegister`
    fn write_register(&mut self, name: &str, value: i32) -> bool {
        let v8 = value as u8;
        let (hi, lo) = ((value >> 8) as u8, value as u8);
        let c = &mut self.cpu;
        match name.to_ascii_lowercase().as_str() {
            "b" => c.b = v8,
            "c" => c.c = v8,
            "d" => c.d = v8,
            "e" => c.e = v8,
            "h" => c.h = v8,
            "l" => c.l = v8,
            "a" => c.a = v8,
            "f" => c.f = v8 & 0xF0,
            "bc" => (c.b, c.c) = (hi, lo),
            "de" => (c.d, c.e) = (hi, lo),
            "hl" => (c.h, c.l) = (hi, lo),
            "af" => (c.a, c.f) = (hi, lo & 0xF0),
            "pc" => {
                c.pc = value as u16;
                let pc = c.pc;
                self.set_active_region_impl(pc);
            }
            "sp" => c.sp = value as u16,
            _ => return false,
        }
        true
    }

    /// `_GBCoreLookupIdentifier`
    fn lookup_identifier(&self, name: &str) -> Option<(i32, i32)> {
        IO_REGISTER_NAMES
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, off)| ((BASE_IO as usize | off) as i32, -1))
    }

    /// `_GBCoreStep`
    fn step(&mut self) {
        loop {
            crate::sm83::tick(self);
            if self.cpu.execution_state == CORE_FETCH {
                break;
            }
        }
    }

    fn run_loop(&mut self) {
        crate::sm83::run(self);
    }

    fn run_frame(&mut self) {
        Gb::run_frame(self);
    }

    fn reset(&mut self) {
        Gb::reset(self);
    }

    fn frame_counter(&self) -> u32 {
        self.video.frame_counter
    }

    fn frame_cycles(&self) -> i32 {
        video::TOTAL_LENGTH
    }

    fn frequency(&self) -> u32 {
        Gb::frequency(self)
    }

    fn global_time(&self) -> u64 {
        self.timing.global_cycles.wrapping_add(self.cpu.cycles as i64 as u64)
    }

    fn events(&self) -> Vec<(String, i32)> {
        let list = if self.timing.root.is_empty() { &self.timing.reroot } else { &self.timing.root };
        list.iter()
            .map(|e| {
                let until = e.when.wrapping_sub(self.timing.master_cycles).wrapping_sub(self.cpu.cycles as u32) as i32;
                (event_name(e.id).to_string(), until)
            })
            .collect()
    }

    fn instruction_address(&self) -> u32 {
        self.cpu.pc as u32
    }

    fn current_segment(&self, address: u32) -> i32 {
        self.current_segment_at(address as u16)
    }

    fn debug_platform(&mut self) -> &mut Option<Box<DebugPlatform>> {
        &mut self.debugger
    }
}

impl DebugCore for Gb {
    /// `SM83DebuggerTrace`
    fn trace(&mut self) -> String {
        let pc = self.cpu.pc;
        let (info, bytes, next) = self.decode_at(pc, -1);
        let c = &self.cpu;
        format!(
            "A: {:02X} F: {:02X} B: {:02X} C: {:02X} D: {:02X} E: {:02X} H: {:02X} L: {:02X} SP: {:04X} PC: {:02X}:{:04X} | {}: {}",
            c.a,
            c.f,
            c.b,
            c.c,
            c.d,
            c.e,
            c.h,
            c.l,
            c.sp,
            self.current_segment_at(pc),
            pc,
            bytes,
            disassemble(&info, next)
        )
    }

    /// `_printStatus` (SM83 CLI debugger, with the GB platform's extra lines)
    fn print_status(&mut self) -> String {
        let c = &self.cpu;
        let af = ((c.a as u16) << 8) | c.f as u16;
        let bc = ((c.b as u16) << 8) | c.c as u16;
        let de = ((c.d as u16) << 8) | c.e as u16;
        let hl = ((c.h as u16) << 8) | c.l as u16;
        let mut out = format!(
            "A: {:02X}  F: {:02X}  (AF: {af:04X})\nB: {:02X}  C: {:02X}  (BC: {bc:04X})\nD: {:02X}  E: {:02X}  (DE: {de:04X})\nH: {:02X}  L: {:02X}  (HL: {hl:04X})\nPC: {:04X}  SP: {:04X}\n",
            c.a, c.f, c.b, c.c, c.d, c.e, c.h, c.l, c.pc, c.sp
        );
        let f = c.f;
        out.push_str(&format!(
            "F: [{}{}{}{}]\n",
            if f & 0x80 != 0 { 'Z' } else { '-' },
            if f & 0x40 != 0 { 'N' } else { '-' },
            if f & 0x20 != 0 { 'H' } else { '-' },
            if f & 0x10 != 0 { 'C' } else { '-' }
        ));
        out.push_str(&format!("T-cycle: {}\n", self.global_time()));
        let segs: Vec<String> = self
            .segments()
            .iter()
            .map(|(name, start)| format!("{name}: {:02X}", self.current_segment_at(*start)))
            .collect();
        if !segs.is_empty() {
            out.push_str(&segs.join("  "));
            out.push('\n');
        }
        out.push_str(&format!(
            "IE: {:02X}  IF: {:02X}  IME: {}\n",
            self.memory.ie, self.memory.io[REG_IF], self.memory.ime as i32
        ));
        out.push_str(&format!(
            "LCDC: {:02X}  STAT: {:02X}  LY: {:02X}\n",
            self.memory.io[REG_LCDC],
            self.memory.io[REG_STAT] | 0x80,
            self.memory.io[REG_LY]
        ));
        out.push_str(&format!("Next video mode: {}\n", self.until(GbEvent::VideoMode) / 4));
        let pc = self.cpu.pc;
        let seg = self.current_segment_at(pc);
        let (line, _) = self.print_line(pc, seg);
        out.push_str(&line);
        out.push('\n');
        out
    }

    /// `_disassemble` (SM83 CLI debugger)
    fn disassemble(&mut self, address: Option<u32>, segment: i32, count: u32, _mode: Option<bool>) -> Vec<String> {
        let (mut address, segment) = match address {
            Some(a) => (a as u16, segment),
            None => (self.cpu.pc, -1),
        };
        let mut lines = Vec::new();
        for _ in 0..count {
            let (line, next) = self.print_line(address, segment);
            lines.push(line);
            address = next;
        }
        lines
    }

    /// `SM83DebuggerClearBreakpoint`
    fn clear_breakpoint(&mut self, id: isize) -> bool {
        let Some(platform) = self.debugger.as_mut() else { return false };
        if let Some(i) = platform.breakpoints.iter().position(|b| b.id == id) {
            platform.breakpoints.remove(i);
            return true;
        }
        if let Some(i) = platform.watchpoints.iter().position(|w| w.id == id) {
            platform.watchpoints.remove(i);
            return true;
        }
        false
    }

    fn platform_name(&self) -> &'static str {
        "SM83"
    }

    fn system_name(&self) -> &'static str {
        "Game Boy"
    }

    fn as_mcore(&mut self) -> &mut dyn MCore {
        self
    }
}
