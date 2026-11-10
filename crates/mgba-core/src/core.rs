// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of the parts of mGBA's `struct mCore` (include/mgba/core/core.h,
// include/mgba/core/interface.h) that tools use: memory blocks, bus and raw
// memory access, registers, stepping and timing. Each system implements it
// (src/gba/core.c, src/gb/core.c in mGBA).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::debugger::Platform as DebugPlatform;

pub const MEMORY_READ: u32 = 0x01;
pub const MEMORY_WRITE: u32 = 0x02;
pub const MEMORY_RW: u32 = 0x03;
pub const MEMORY_WORM: u32 = 0x04;
pub const MEMORY_MAPPED: u32 = 0x10;
pub const MEMORY_VIRTUAL: u32 = 0x20;

/// `struct mCoreMemoryBlock`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryBlock {
    pub id: i64,
    pub internal_name: &'static str,
    pub short_name: &'static str,
    pub long_name: &'static str,
    pub start: u32,
    pub end: u32,
    pub size: u32,
    pub flags: u32,
    pub max_segment: u16,
    pub segment_start: u32,
}

impl MemoryBlock {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        id: i64,
        internal_name: &'static str,
        short_name: &'static str,
        long_name: &'static str,
        start: u32,
        end: u32,
        size: u32,
        flags: u32,
    ) -> MemoryBlock {
        MemoryBlock { id, internal_name, short_name, long_name, start, end, size, flags, max_segment: 0, segment_start: 0 }
    }

    pub const fn segmented(mut self, max_segment: u16, segment_start: u32) -> MemoryBlock {
        self.max_segment = max_segment;
        self.segment_start = segment_start;
        self
    }
}

/// `enum mCoreRegisterType`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterType {
    Gpr,
    Flags,
}

/// `struct mCoreRegisterInfo`
#[derive(Clone, Copy, Debug)]
pub struct RegisterInfo {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub width: u32,
    pub mask: u32,
    pub ty: RegisterType,
}

/// `enum mPlatform`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum System {
    Gba,
    Gb,
}

/// `struct mCore` as a trait. Bus accesses go through the system's memory
/// map (with side effects, like the CPU's); raw accesses peek and poke the
/// backing memory (`GBAView8`/`GBAPatch8`), with `segment` picking a bank on
/// the Game Boy (negative means the mapped one).
pub trait MCore {
    fn system(&self) -> System;

    /// `listMemoryBlocks`
    fn list_memory_blocks(&self) -> Vec<MemoryBlock>;
    /// `getMemoryBlock`, copied out.
    fn memory_block(&self, id: i64) -> Option<Vec<u8>>;

    fn bus_read8(&mut self, address: u32) -> u32;
    fn bus_read16(&mut self, address: u32) -> u32;
    fn bus_read32(&mut self, address: u32) -> u32;
    fn bus_write8(&mut self, address: u32, value: u32);
    fn bus_write16(&mut self, address: u32, value: u32);
    fn bus_write32(&mut self, address: u32, value: u32);

    fn raw_read8(&mut self, address: u32, segment: i32) -> u32;
    fn raw_read16(&mut self, address: u32, segment: i32) -> u32;
    fn raw_read32(&mut self, address: u32, segment: i32) -> u32;
    fn raw_write8(&mut self, address: u32, segment: i32, value: u32);
    fn raw_write16(&mut self, address: u32, segment: i32, value: u32);
    fn raw_write32(&mut self, address: u32, segment: i32, value: u32);

    /// `listRegisters`
    fn list_registers(&self) -> &'static [RegisterInfo];
    /// `readRegister`
    fn read_register(&self, name: &str) -> Option<i32>;
    /// `writeRegister`
    fn write_register(&mut self, name: &str, value: i32) -> bool;
    /// `lookupIdentifier`: I/O register names, as `(value, segment)`.
    fn lookup_identifier(&self, name: &str) -> Option<(i32, i32)>;

    /// `step`: one instruction.
    fn step(&mut self);
    /// `runLoop`
    fn run_loop(&mut self);
    /// `runFrame`
    fn run_frame(&mut self);
    /// `reset`
    fn reset(&mut self);

    fn frame_counter(&self) -> u32;
    /// `frameCycles`
    fn frame_cycles(&self) -> i32;
    /// `frequency`
    fn frequency(&self) -> u32;
    /// `mTimingGlobalTime`
    fn global_time(&self) -> u64;
    /// Scheduled events and the cycles until each fires (the CLI's `events`).
    fn events(&self) -> Vec<(String, i32)>;

    /// Address of the instruction about to run.
    fn instruction_address(&self) -> u32;
    /// The bank mapped at `address` (`currentSegment`), or -1 where there are none.
    fn current_segment(&self, _address: u32) -> i32 {
        -1
    }

    /// `getGameTitle`
    fn game_title(&self) -> String;
    /// `getGameCode`
    fn game_code(&self) -> String;
    /// `romSize`
    fn rom_size(&self) -> usize;
    /// `checksum(mCHECKSUM_CRC32)`
    fn crc32(&self) -> u32;
    /// This port's save state (this port's own format).
    fn save_state_bytes(&self) -> Vec<u8>;
    fn load_state_bytes(&mut self, data: &[u8]) -> bool;

    /// The debugger state a core carries while one is attached.
    fn debug_platform(&mut self) -> &mut Option<Box<DebugPlatform>>;
}
