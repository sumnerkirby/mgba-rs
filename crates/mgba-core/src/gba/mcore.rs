// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of the GBA's `struct mCore` hooks used by tools (src/gba/core.c:
// memory blocks, bus/raw access, registers, step) and its debugger platform
// glue (src/arm/debugger/debugger.c, src/arm/debugger/cli-debugger.c,
// src/gba/gba.c `GBASetBreakpoint`/`GBAClearBreakpoint`).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::memory::*;
use super::savedata::SavedataType;
use super::{regs, Gba, ARM7TDMI_FREQUENCY};
use crate::arm::decoder::{decode_arm, decode_thumb, decode_thumb_combine, disassemble, DisassemblyContext};
use crate::arm::{self, ExecutionMode, ARM_LR, ARM_PC, ARM_SP, MODE_SYSTEM, MODE_USER, WORD_SIZE_ARM, WORD_SIZE_THUMB};
use crate::cheats::CheatBus;
use crate::core::*;
use crate::debugger::{self, DebugCore, Platform as DebugPlatform, SoftwareBreakpoint};
use crate::timing::EventId;

const fn rw(id: u32, internal: &'static str, short: &'static str, long: &'static str, base: u32, size: u32) -> MemoryBlock {
    MemoryBlock::new(id as i64, internal, short, long, base, base + size, size, MEMORY_RW | MEMORY_MAPPED)
}

/// `_GBAMemoryBlocks` without the save memory, which depends on its type.
const BASE_BLOCKS: [MemoryBlock; 11] = [
    MemoryBlock::new(-1, "mem", "All", "All", 0, 0x1000_0000, 0x1000_0000, MEMORY_VIRTUAL),
    MemoryBlock::new(REGION_BIOS as i64, "bios", "BIOS", "BIOS (16kiB)", BASE_BIOS, SIZE_BIOS, SIZE_BIOS, MEMORY_READ | MEMORY_MAPPED),
    rw(REGION_EWRAM, "wram", "EWRAM", "Working RAM (256kiB)", BASE_EWRAM, SIZE_EWRAM),
    rw(REGION_IWRAM, "iwram", "IWRAM", "Internal Working RAM (32kiB)", BASE_IWRAM, SIZE_IWRAM),
    rw(REGION_IO, "io", "MMIO", "Memory-Mapped I/O", BASE_IO, SIZE_IO),
    rw(REGION_PALETTE_RAM, "palette", "Palette", "Palette RAM (1kiB)", BASE_PALETTE_RAM, SIZE_PALETTE_RAM),
    rw(REGION_VRAM, "vram", "VRAM", "Video RAM (96kiB)", BASE_VRAM, SIZE_VRAM),
    rw(REGION_OAM, "oam", "OAM", "OBJ Attribute Memory (1kiB)", BASE_OAM, SIZE_OAM),
    MemoryBlock::new(REGION_ROM0 as i64, "cart0", "ROM", "Game Pak (32MiB)", BASE_ROM0, BASE_ROM0 + SIZE_ROM0, SIZE_ROM0, MEMORY_READ | MEMORY_WORM | MEMORY_MAPPED),
    MemoryBlock::new(REGION_ROM1 as i64, "cart1", "ROM WS1", "Game Pak (Waitstate 1)", BASE_ROM1, BASE_ROM1 + SIZE_ROM0, SIZE_ROM0, MEMORY_READ | MEMORY_WORM | MEMORY_MAPPED),
    MemoryBlock::new(REGION_ROM2 as i64, "cart2", "ROM WS2", "Game Pak (Waitstate 2)", BASE_ROM2, BASE_ROM2 + SIZE_ROM0, SIZE_ROM0, MEMORY_READ | MEMORY_WORM | MEMORY_MAPPED),
];

/// `_GBARegisters`
const REGISTERS: &[RegisterInfo] = &[
    RegisterInfo { name: "r0", aliases: &[], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "r1", aliases: &[], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "r2", aliases: &[], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "r3", aliases: &[], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "r4", aliases: &[], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "r5", aliases: &[], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "r6", aliases: &[], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "r7", aliases: &[], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "r8", aliases: &[], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "r9", aliases: &[], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "r10", aliases: &[], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "r11", aliases: &[], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "r12", aliases: &["ip"], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "sp", aliases: &["r13"], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "lr", aliases: &["r14"], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "pc", aliases: &["r15"], width: 4, mask: 0xFFFF_FFFF, ty: RegisterType::Gpr },
    RegisterInfo { name: "cpsr", aliases: &[], width: 4, mask: 0xF000_00FF, ty: RegisterType::Flags },
    RegisterInfo { name: "spsr", aliases: &[], width: 4, mask: 0xF000_00FF, ty: RegisterType::Flags },
    RegisterInfo { name: "spsr_irq", aliases: &[], width: 4, mask: 0xF000_00FF, ty: RegisterType::Flags },
    RegisterInfo { name: "spsr_fiq", aliases: &[], width: 4, mask: 0xF000_00FF, ty: RegisterType::Flags },
    RegisterInfo { name: "spsr_svc", aliases: &[], width: 4, mask: 0xF000_00FF, ty: RegisterType::Flags },
    RegisterInfo { name: "spsr_abt", aliases: &[], width: 4, mask: 0xF000_00FF, ty: RegisterType::Flags },
    RegisterInfo { name: "spsr_und", aliases: &[], width: 4, mask: 0xF000_00FF, ty: RegisterType::Flags },
];

fn event_name(id: EventId) -> &'static str {
    match id {
        EventId::Irq => "GBA IRQ",
        EventId::Video => "GBA Video",
        EventId::AudioFrame => "GBA Audio Frame Sequencer",
        EventId::AudioSample => "GBA Audio Sample",
        EventId::Timer0 => "GBA Timer 0",
        EventId::Timer1 => "GBA Timer 1",
        EventId::Timer2 => "GBA Timer 2",
        EventId::Timer3 => "GBA Timer 3",
        EventId::Dma => "GBA DMA Service",
        EventId::SavedataDust => "GBA Savedata Settling",
        EventId::Sio => "GBA SIO Complete",
        EventId::SioLockstep => "GBA SIO Lockstep",
    }
}

impl Gba {
    /// Run the watchpoint shim for an access (`DebuggerShim_*`).
    #[inline]
    pub(crate) fn debug_watch(&mut self, address: u32, access: u32, value: u32, width: u32) {
        if self.debugger.as_ref().is_some_and(|d| d.has_watchpoints())
            && debugger::check_watchpoints(self, address, access, value, width)
        {
            // ARMDebuggerEnter: stop the run loop after this instruction.
            self.cpu.next_event = self.cpu.cycles;
        }
    }

    /// `CREATE_MULTIPLE_WATCHPOINT_SHIM`: every word an LDM/STM touches.
    pub(crate) fn debug_watch_multiple(&mut self, address: u32, mask: u32, direction: u32, access: u32) {
        let popcount = mask.count_ones();
        let mut offset: i32 = 4;
        let mut base = address as i32;
        if direction & arm::LSM_D != 0 {
            offset = -4;
            base -= ((popcount << 2) as i32) - 4;
        }
        if direction & arm::LSM_B != 0 {
            base += offset;
        }
        for i in 0..popcount {
            self.debug_watch((base as u32).wrapping_add(4 * i), access, 0, 4);
        }
    }

    /// `GBAPatch32`/`GBAPatch16`, returning the replaced value.
    fn patch_instruction(&mut self, address: u32, thumb: bool, value: u32) -> u32 {
        let width = if thumb { 2 } else { 4 };
        let old = self.raw_read(address, -1, width);
        self.raw_write(address, -1, width, value);
        old
    }

    /// The debugger case of `GBABreakpoint`, with `ARMDebuggerEnter`'s
    /// software-breakpoint handling: put the original instruction back, run
    /// it, and plant the breakpoint again.
    pub(crate) fn debugger_breakpoint(&mut self, pc_address: u32) {
        let Some(mut platform) = self.debugger.take() else { return };
        let mut info = crate::debugger::EntryInfo::software_breakpoint(pc_address);
        if let Some(i) = platform.sw_breakpoints.iter().position(|b| b.d.address == pc_address) {
            let bp = platform.sw_breakpoints[i].clone();
            info.point_id = bp.d.id;
            self.patch_instruction(bp.d.address, bp.thumb, bp.opcode);
            arm::run_fake(self, bp.opcode);
            if bp.d.is_temporary {
                platform.sw_breakpoints.remove(i);
            } else {
                let bkpt = if bp.thumb { 0xBE00 } else { 0xE120_0070 };
                platform.sw_breakpoints[i].opcode = self.patch_instruction(bp.d.address, bp.thumb, bkpt);
            }
        }
        platform.enter(crate::debugger::EntryReason::Breakpoint, info);
        self.debugger = Some(platform);
        self.cpu.next_event = self.cpu.cycles;
        self.interrupt();
    }

    fn psr_mode_string(mode: u32) -> &'static str {
        match mode {
            MODE_USER => "usr",
            arm::MODE_FIQ => "fiq",
            arm::MODE_IRQ => "irq",
            arm::MODE_SUPERVISOR => "svc",
            arm::MODE_ABORT => "abt",
            arm::MODE_UNDEFINED => "und",
            MODE_SYSTEM => "sys",
            _ => "invalid",
        }
    }

    /// `_printPSR`
    fn format_psr(psr: arm::Psr) -> String {
        format!(
            "{:08X} [{}{}{}{}{}{}{}] ({})\n",
            psr.0,
            if psr.n() { 'N' } else { '-' },
            if psr.z() { 'Z' } else { '-' },
            if psr.c() { 'C' } else { '-' },
            if psr.v() { 'V' } else { '-' },
            if psr.i() { 'I' } else { '-' },
            if psr.f() { 'F' } else { '-' },
            if psr.t() { 'T' } else { '-' },
            Self::psr_mode_string(psr.priv_mode())
        )
    }

    /// `_printLine` (ARM CLI debugger): one disassembled instruction and its size.
    fn print_line(&mut self, address: u32, thumb: bool) -> (String, u32) {
        let address = address & !(WORD_SIZE_THUMB - 1);
        let symbols = self.debugger.as_ref().and_then(|d| d.symbols.clone());
        let mut ctx = DisCtx { gba: self, symbols };
        if !thumb {
            let instruction = ctx.gba.mem_load32(address & !(WORD_SIZE_ARM - 1), None);
            let info = decode_arm(instruction);
            let text = disassemble(&info, Some(&mut ctx), address.wrapping_add(WORD_SIZE_ARM * 2));
            (format!("{address:08X}:  {instruction:08X}\t{text}"), WORD_SIZE_ARM)
        } else {
            let instruction = ctx.gba.mem_load16(address, None) as u16;
            let instruction2 = ctx.gba.mem_load16(address.wrapping_add(WORD_SIZE_THUMB), None) as u16;
            let info = decode_thumb(instruction);
            let info2 = decode_thumb(instruction2);
            let pc = address.wrapping_add(WORD_SIZE_THUMB * 2);
            match decode_thumb_combine(&info, &info2) {
                Some(combined) => {
                    let text = disassemble(&combined, Some(&mut ctx), pc);
                    (format!("{address:08X}:  {instruction:04X} {instruction2:04X}\t{text}"), WORD_SIZE_THUMB * 2)
                }
                None => {
                    let text = disassemble(&info, Some(&mut ctx), pc);
                    (format!("{address:08X}:  {instruction:04X}     \t{text}"), WORD_SIZE_THUMB)
                }
            }
        }
    }
}

/// Memory and symbols for the disassembler.
struct DisCtx<'a> {
    gba: &'a mut Gba,
    symbols: Option<std::sync::Arc<crate::debugger::symbols::Symbols>>,
}

impl DisassemblyContext for DisCtx<'_> {
    fn load(&mut self, address: u32, width: u32) -> u32 {
        match width {
            1 => self.gba.mem_load8(address, None),
            2 => self.gba.mem_load16(address, None),
            _ => self.gba.mem_load32(address, None),
        }
    }

    fn symbol_at(&self, address: u32) -> Option<String> {
        self.symbols.as_ref()?.reverse_lookup(address as i32, -1).map(str::to_string)
    }
}

impl MCore for Gba {
    fn system(&self) -> System {
        System::Gba
    }

    /// `_GBACoreListMemoryBlocks`
    fn list_memory_blocks(&self) -> Vec<MemoryBlock> {
        let mut blocks = BASE_BLOCKS.to_vec();
        let flash = MEMORY_RW | MEMORY_MAPPED;
        match self.memory.savedata.ty {
            SavedataType::Sram => blocks.push(rw(REGION_SRAM, "sram", "SRAM", "Static RAM (32kiB)", BASE_SRAM, 0x8000)),
            SavedataType::Sram512 => {
                blocks.push(rw(REGION_SRAM, "sram", "SRAM", "Static RAM (64kiB)", BASE_SRAM, 0x10000))
            }
            SavedataType::Flash512 => blocks.push(MemoryBlock::new(
                REGION_SRAM as i64,
                "sram",
                "Flash",
                "Flash Memory (64kiB)",
                BASE_SRAM,
                BASE_SRAM + 0x10000,
                0x10000,
                flash,
            )),
            SavedataType::Flash1M => blocks.push(
                MemoryBlock::new(
                    REGION_SRAM as i64,
                    "sram",
                    "Flash",
                    "Flash Memory (128kiB)",
                    BASE_SRAM,
                    BASE_SRAM + 0x10000,
                    0x20000,
                    flash,
                )
                .segmented(1, BASE_SRAM),
            ),
            SavedataType::Eeprom => blocks.push(MemoryBlock::new(
                REGION_SRAM_MIRROR as i64,
                "eeprom",
                "EEPROM",
                "EEPROM (8kiB)",
                0,
                0x2000,
                0x2000,
                MEMORY_RW,
            )),
            SavedataType::Eeprom512 => blocks.push(MemoryBlock::new(
                REGION_SRAM_MIRROR as i64,
                "eeprom",
                "EEPROM",
                "EEPROM (512B)",
                0,
                0x2000,
                0x200,
                MEMORY_RW,
            )),
            _ => {}
        }
        blocks
    }

    /// `_GBACoreGetMemoryBlock`
    fn memory_block(&self, id: i64) -> Option<Vec<u8>> {
        let halfwords = |v: &[u16]| v.iter().flat_map(|h| h.to_le_bytes()).collect::<Vec<u8>>();
        Some(match id as u32 {
            REGION_BIOS => self.memory.bios.clone(),
            REGION_EWRAM => self.memory.wram.clone(),
            REGION_IWRAM => self.memory.iwram.clone(),
            REGION_PALETTE_RAM => halfwords(&self.video.palette),
            REGION_VRAM => halfwords(&self.video.vram),
            REGION_OAM => halfwords(&self.video.oam),
            REGION_ROM0 | REGION_ROM1 | REGION_ROM2 => self.memory.rom.to_vec(),
            REGION_SRAM | REGION_SRAM_MIRROR => {
                let size = self.memory.savedata.size().min(self.memory.savedata.data.len());
                self.memory.savedata.data[..size].to_vec()
            }
            _ => return None,
        })
    }

    fn bus_read8(&mut self, address: u32) -> u32 {
        self.mem_load8(address, None)
    }
    fn bus_read16(&mut self, address: u32) -> u32 {
        self.mem_load16(address, None)
    }
    fn bus_read32(&mut self, address: u32) -> u32 {
        self.mem_load32(address, None)
    }
    fn bus_write8(&mut self, address: u32, value: u32) {
        self.mem_store8(address, value as u8, None)
    }
    fn bus_write16(&mut self, address: u32, value: u32) {
        self.mem_store16(address, value as u16, None)
    }
    fn bus_write32(&mut self, address: u32, value: u32) {
        self.mem_store32(address, value, None)
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

    /// `_GBACoreReadRegister`
    fn read_register(&self, name: &str) -> Option<i32> {
        let lower = name.to_ascii_lowercase();
        let cpu = &self.cpu;
        match lower.as_str() {
            "cpsr" => return Some(cpu.cpsr.0 as i32),
            "ip" => return Some(cpu.gprs[12] as i32),
            "sp" => return Some(cpu.gprs[ARM_SP] as i32),
            "lr" => return Some(cpu.gprs[ARM_LR] as i32),
            "pc" => return Some(cpu.gprs[ARM_PC] as i32),
            _ => {}
        }
        let n = lower.strip_prefix('r')?;
        let reg: usize = n.parse().ok()?;
        (reg <= 15).then(|| cpu.gprs[reg] as i32)
    }

    /// `_GBACoreWriteRegister`
    fn write_register(&mut self, name: &str, value: i32) -> bool {
        let lower = name.to_ascii_lowercase();
        let reg = match lower.as_str() {
            "cpsr" => {
                self.cpu.cpsr = arm::Psr(value as u32);
                let mode = self.cpu.cpsr.priv_mode();
                self.cpu.set_privilege_mode(mode);
                self.cpu.execution_mode =
                    if self.cpu.cpsr.t() { ExecutionMode::Thumb } else { ExecutionMode::Arm };
                return true;
            }
            "ip" => 12,
            "sp" => ARM_SP,
            "lr" => ARM_LR,
            "pc" => ARM_PC,
            _ => match lower.strip_prefix('r').and_then(|n| n.parse::<usize>().ok()) {
                Some(r) if r <= 15 => r,
                _ => return false,
            },
        };
        self.cpu.gprs[reg] = value as u32;
        if reg == ARM_PC {
            arm::write_pc(self);
        }
        true
    }

    /// `_GBACoreLookupIdentifier`
    fn lookup_identifier(&self, name: &str) -> Option<(i32, i32)> {
        regs::IO_REGISTER_NAMES
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, off)| ((BASE_IO | off) as i32, -1))
    }

    /// `_GBACoreStep`
    fn step(&mut self) {
        arm::step(self);
    }

    fn run_loop(&mut self) {
        arm::run_loop(self);
    }

    fn run_frame(&mut self) {
        Gba::run_frame(self);
    }

    fn reset(&mut self) {
        Gba::reset(self);
    }

    fn frame_counter(&self) -> u32 {
        self.video.frame_counter
    }

    fn frame_cycles(&self) -> i32 {
        super::video::VIDEO_TOTAL_LENGTH
    }

    fn frequency(&self) -> u32 {
        ARM7TDMI_FREQUENCY
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

    /// `_ARMPCAddress`
    fn instruction_address(&self) -> u32 {
        let width = if self.cpu.cpsr.t() { WORD_SIZE_THUMB } else { WORD_SIZE_ARM };
        self.cpu.gprs[ARM_PC].wrapping_sub(width)
    }

    fn debug_platform(&mut self) -> &mut Option<Box<DebugPlatform>> {
        &mut self.debugger
    }
}

impl DebugCore for Gba {
    /// `ARMDebuggerTrace`
    fn trace(&mut self) -> String {
        let regs = &self.cpu.gprs;
        let mut out = String::new();
        for (i, r) in regs.iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(&format!("{r:08X}"));
        }
        out.push_str(&format!(" cpsr: {:08X}", self.cpu.cpsr.0));
        let pc = self.cpu.gprs[ARM_PC];
        let symbols = self.debugger.as_ref().and_then(|d| d.symbols.clone());
        let thumb = self.cpu.execution_mode == ExecutionMode::Thumb;
        let (p0, p1) = (self.cpu.prefetch[0], self.cpu.prefetch[1]);
        let mut ctx = DisCtx { gba: self, symbols };
        let disassembly = if !thumb {
            let info = decode_arm(p0);
            format!("{p0:08X}: {}", disassemble(&info, Some(&mut ctx), pc))
        } else {
            let info = decode_thumb(p0 as u16);
            let info2 = decode_thumb(p1 as u16);
            match decode_thumb_combine(&info, &info2) {
                Some(c) => format!("{:04X}{:04X}: {}", p0 as u16, p1 as u16, disassemble(&c, Some(&mut ctx), pc)),
                None => format!("    {:04X}: {}", p0 as u16, disassemble(&info, Some(&mut ctx), pc)),
            }
        };
        format!("{out} | {disassembly}")
    }

    /// `_printStatus` (ARM CLI debugger)
    fn print_status(&mut self) -> String {
        let mut out = String::new();
        let g = self.cpu.gprs;
        for r in (0..16).step_by(4) {
            let pad = |n: usize| if n < 10 { " " } else { "" };
            out.push_str(&format!(
                "{}r{}: {:08X}  {}r{}: {:08X}  {}r{}: {:08X}  {}r{}: {:08X}\n",
                pad(r),
                r,
                g[r],
                pad(r + 1),
                r + 1,
                g[r + 1],
                pad(r + 2),
                r + 2,
                g[r + 2],
                pad(r + 3),
                r + 3,
                g[r + 3]
            ));
        }
        out.push_str("cpsr: ");
        out.push_str(&Self::format_psr(self.cpu.cpsr));
        let mode = self.cpu.cpsr.priv_mode();
        if mode != MODE_USER && mode != MODE_SYSTEM {
            out.push_str("spsr: ");
            out.push_str(&Self::format_psr(self.cpu.spsr));
        }
        out.push_str(&format!("Cycle: {}\n", self.global_time()));
        let thumb = self.cpu.cpsr.t();
        let address = self.instruction_address();
        let (line, _) = self.print_line(address, thumb);
        out.push_str(&line);
        out.push('\n');
        out
    }

    /// `_disassembleMode`
    fn disassemble(&mut self, address: Option<u32>, _segment: i32, count: u32, mode: Option<bool>) -> Vec<String> {
        let thumb = mode.unwrap_or(self.cpu.execution_mode == ExecutionMode::Thumb);
        let width = if thumb { WORD_SIZE_THUMB } else { WORD_SIZE_ARM };
        let mut address = address.unwrap_or(self.cpu.gprs[ARM_PC].wrapping_sub(width));
        let mut lines = Vec::new();
        for _ in 0..count {
            let (line, size) = self.print_line(address, thumb);
            lines.push(line);
            address = address.wrapping_add(size);
        }
        lines
    }

    /// `ARMDebuggerSetSoftwareBreakpoint` with `GBASetBreakpoint`.
    fn set_software_breakpoint(&mut self, address: u32, thumb: bool) -> isize {
        if self.debugger.is_none() {
            return -1;
        }
        let address = address & !1;
        let bkpt = if thumb { 0xBE00 | (debugger::DEBUGGER_BKPT & 0xFF) } else { 0xE120_0070 };
        let opcode = self.patch_instruction(address, thumb, bkpt);
        let platform = self.debugger.as_mut().unwrap();
        let id = platform.next_id;
        platform.next_id += 1;
        let mut d = crate::debugger::Breakpoint::new(address, -1);
        d.id = id;
        d.ty = crate::debugger::BreakpointType::Software;
        platform.sw_breakpoints.push(SoftwareBreakpoint { d, opcode, thumb });
        id
    }

    /// `ARMDebuggerClearBreakpoint`
    fn clear_breakpoint(&mut self, id: isize) -> bool {
        let Some(platform) = self.debugger.as_mut() else { return false };
        if let Some(i) = platform.breakpoints.iter().position(|b| b.id == id) {
            platform.breakpoints.remove(i);
            let bps = std::mem::take(&mut platform.breakpoints);
            // Rebuild the bloom filter by re-adding.
            for bp in bps {
                let id = bp.id;
                let next = platform.next_id;
                platform.set_breakpoint(bp, true);
                platform.breakpoints.last_mut().unwrap().id = id;
                platform.next_id = next;
            }
            return true;
        }
        if let Some(i) = platform.sw_breakpoints.iter().position(|b| b.d.id == id) {
            let bp = platform.sw_breakpoints.remove(i);
            self.patch_instruction(bp.d.address, bp.thumb, bp.opcode);
            return true;
        }
        if let Some(i) = platform.watchpoints.iter().position(|w| w.id == id) {
            platform.watchpoints.remove(i);
            return true;
        }
        false
    }

    fn platform_name(&self) -> &'static str {
        "ARM"
    }

    fn system_name(&self) -> &'static str {
        "Game Boy Advance"
    }

    fn as_mcore(&mut self) -> &mut dyn MCore {
        self
    }
}
