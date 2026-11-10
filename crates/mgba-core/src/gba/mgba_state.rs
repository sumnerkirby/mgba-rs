// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's GBA save-state format (src/gba/serialize.c and the
// serializers it calls: GBAMemorySerialize, GBAIOSerialize, GBADMASerialize,
// GBAHardwareSerialize, GBAVideoSerialize, GBAAudioSerialize,
// GBAudioPSGSerialize, GBASavedataSerialize), so states can be exchanged
// with mGBA. This port's own states (serialize.rs) are separate.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;

use super::gpio::HW_GPIO;
use super::regs::*;
use super::savedata::SavedataType;
use super::video::VideoPhase;
use super::Gba;
use crate::arm::{self, ExecutionMode, Psr, ARM_PC, WORD_SIZE_ARM, WORD_SIZE_THUMB};
use crate::state_container::{self, EXTDATA_SAVEDATA};
use crate::timing::EventId;

/// `GBASavestateMagic + GBASavestateVersion`
const VERSION_MAGIC: u32 = 0x0100_000B;
const SAVESTATE_MAGIC: u32 = 0x0100_0000;

/// `_isValidRegister` (src/gba/io.c)
const IS_VALID_REGISTER: [bool; 266] = [
    true, false, true, true, true, true, true, true,
    true, true, true, true, true, true, true, true,
    true, true, true, true, true, true, true, true,
    true, true, true, true, true, true, true, true,
    true, true, true, true, true, true, true, false,
    true, true, true, false, false, false, false, false,
    true, true, true, false, true, false, true, false,
    true, true, true, false, true, false, true, false,
    true, true, true, false, true, false, false, false,
    true, true, true, true, true, true, true, true,
    true, true, true, true, false, false, false, false,
    true, true, true, true, true, true, true, true,
    true, true, true, true, true, true, true, true,
    true, true, true, true, true, true, true, true,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    true, true, true, true, true, true, true, true,
    false, false, false, false, false, false, false, false,
    true, true, true, true, true, false, false, false,
    true, true, true, false, false, false, false, false,
    true, false, false, false, false, false, false, false,
    true, true, true, true, true, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    true, true, true, false, true, false, false, false,
    true, true,
];

/// `_isRSpecialRegister` (src/gba/io.c)
const IS_R_SPECIAL_REGISTER: [bool; 266] = [
    false, false, true, true, false, false, false, false,
    true, true, true, true, true, true, true, true,
    true, true, true, true, true, true, true, true,
    true, true, true, true, true, true, true, true,
    true, true, true, true, true, true, true, true,
    true, true, true, true, true, true, true, true,
    false, false, true, false, false, false, true, false,
    false, false, true, false, false, false, true, false,
    false, false, false, false, true, false, false, false,
    true, true, true, true, true, true, true, true,
    true, true, true, true, false, false, false, false,
    true, true, true, true, true, true, true, true,
    true, true, true, true, true, true, true, true,
    true, true, true, true, true, true, true, true,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    true, true, true, true, true, true, true, true,
    false, false, false, false, false, false, false, false,
    true, true, true, true, false, false, false, false,
    true, true, false, false, false, false, false, false,
    true, false, false, false, false, false, false, false,
    true, true, true, true, true, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    true, true,
];

/// `_isWSpecialRegister` (src/gba/io.c)
const IS_W_SPECIAL_REGISTER: [bool; 266] = [
    false, false, true, true, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, true, false, false, false, true, false,
    false, false, true, false, false, false, true, false,
    false, false, true, false, false, false, false, false,
    true, true, true, true, true, true, true, true,
    true, true, true, true, false, false, false, false,
    false, false, false, false, true, true, false, false,
    false, false, true, true, false, false, false, false,
    true, true, false, false, false, false, true, true,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    true, true, true, true, true, true, true, true,
    false, false, false, false, false, false, false, false,
    true, true, true, true, true, false, false, false,
    true, true, true, false, false, false, false, false,
    true, false, false, false, false, false, false, false,
    true, true, true, true, true, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false,
    true, true, false, false, true, false, false, false,
    true, true,
];

/// Offsets in `struct GBASerializedState` (include/mgba/internal/gba/serialize.h).
#[allow(dead_code)]
mod off {
    pub const VERSION_MAGIC: usize = 0x0;
    pub const BIOS_CHECKSUM: usize = 0x4;
    pub const ROM_CRC32: usize = 0x8;
    pub const MASTER_CYCLES: usize = 0xC;
    pub const TITLE: usize = 0x10;
    pub const ID: usize = 0x1C;
    pub const CPU_GPRS: usize = 0x20;
    pub const CPU_CPSR: usize = 0x60;
    pub const CPU_SPSR: usize = 0x64;
    pub const CPU_CYCLES: usize = 0x68;
    pub const CPU_NEXT_EVENT: usize = 0x6C;
    pub const CPU_BANKED_REGISTERS: usize = 0x70;
    pub const CPU_BANKED_SPSRS: usize = 0x118;
    pub const AUDIO_PSG: usize = 0x130;
    pub const AUDIO_FIFO_A: usize = 0x18C;
    pub const AUDIO_FIFO_B: usize = 0x1AC;
    pub const AUDIO_INTERNAL_A: usize = 0x1CC;
    pub const AUDIO_INTERNAL_B: usize = 0x1D0;
    pub const AUDIO_NEXT_SAMPLE: usize = 0x1D4;
    pub const AUDIO_SAMPLE_A: usize = 0x1D8;
    pub const AUDIO_SAMPLE_B: usize = 0x1D9;
    pub const AUDIO_GBA_FLAGS: usize = 0x1DA;
    pub const AUDIO_FLAGS: usize = 0x1DC;
    pub const AUDIO_LAST_SAMPLE: usize = 0x1E0;
    pub const AUDIO_GBA_FLAGS2: usize = 0x1E4;
    pub const VIDEO_NEXT_EVENT: usize = 0x1F4;
    pub const VIDEO_FLAGS: usize = 0x1F8;
    pub const VIDEO_FRAME_COUNTER: usize = 0x1FC;
    pub const TIMERS: usize = 0x200;
    pub const TIMER_STRIDE: usize = 0x14;
    pub const DMA: usize = 0x250;
    pub const DMA_STRIDE: usize = 0x10;
    pub const HW_PIN_STATE: usize = 0x290;
    pub const HW_WRITE_LATCH: usize = 0x291;
    pub const HW_PIN_DIRECTION: usize = 0x292;
    pub const HW_FLAGS3: usize = 0x293;
    pub const HW_RTC_BYTES_REMAINING: usize = 0x294;
    pub const HW_RTC_BITS_READ: usize = 0x29C;
    pub const HW_RTC_BITS: usize = 0x2A0;
    pub const HW_RTC_COMMAND_ACTIVE: usize = 0x2A4;
    pub const HW_RTC_COMMAND: usize = 0x2A8;
    pub const HW_RTC_CONTROL: usize = 0x2AC;
    pub const HW_TIME: usize = 0x2B0;
    pub const HW_DEVICES: usize = 0x2B7;
    pub const HW_GYRO_SAMPLE: usize = 0x2B8;
    pub const HW_TILT_X: usize = 0x2BA;
    pub const HW_TILT_Y: usize = 0x2BC;
    pub const HW_FLAGS1: usize = 0x2BE;
    pub const HW_LIGHT_SAMPLE: usize = 0x2C0;
    pub const HW_FLAGS2: usize = 0x2C1;
    pub const HW_SIO_NEXT_EVENT: usize = 0x2C4;
    pub const DMA_TRANSFER_REGISTER: usize = 0x2C8;
    pub const DMA_BLOCK_PC: usize = 0x2CC;
    pub const SAVEDATA_TYPE: usize = 0x2E0;
    pub const SAVEDATA_COMMAND: usize = 0x2E1;
    pub const SAVEDATA_FLAGS: usize = 0x2E2;
    pub const SAVEDATA_READ_BITS_REMAINING: usize = 0x2E3;
    pub const SAVEDATA_SETTLING_DUST: usize = 0x2E4;
    pub const SAVEDATA_READ_ADDRESS: usize = 0x2E8;
    pub const SAVEDATA_WRITE_ADDRESS: usize = 0x2EC;
    pub const SAVEDATA_SETTLING_SECTOR: usize = 0x2F0;
    pub const BIOS_PREFETCH: usize = 0x2F4;
    pub const CPU_PREFETCH: usize = 0x2F8;
    pub const GLOBAL_CYCLES: usize = 0x310;
    pub const LAST_PREFETCHED_PC: usize = 0x318;
    pub const MISC_FLAGS: usize = 0x31C;
    pub const NEXT_IRQ: usize = 0x320;
    pub const BIOS_STALL: usize = 0x324;
    pub const SAMPLES_A: usize = 0x370;
    pub const SAMPLES_B: usize = 0x380;
    pub const CURRENT_SAMPLES: usize = 0x390;
    pub const BUS: usize = 0x3D0;
    pub const DMA_LATCH: usize = 0x3D4;
    pub const DMA_COUNT_LATCH: usize = 0x3E0;
    pub const IO: usize = 0x400;
    pub const PRAM: usize = 0x800;
    pub const OAM: usize = 0xC00;
    pub const VRAM: usize = 0x1000;
    pub const IWRAM: usize = 0x19000;
    pub const WRAM: usize = 0x21000;
    pub const SIZE: usize = 0x61000;
    pub const PSG_CH1_ENVELOPE: usize = 0x0;
    pub const PSG_CH1_NEXT_FRAME: usize = 0x4;
    pub const PSG_CH1_SWEEP: usize = 0xC;
    pub const PSG_CH1_LAST_UPDATE: usize = 0x10;
    pub const PSG_CH2_ENVELOPE: usize = 0x14;
    pub const PSG_CH2_LAST_UPDATE: usize = 0x20;
    pub const PSG_CH3_WAVEBANKS: usize = 0x24;
    pub const PSG_CH3_LENGTH: usize = 0x44;
    pub const PSG_CH3_NEXT_EVENT: usize = 0x48;
    pub const PSG_CH4_LFSR: usize = 0x4C;
    pub const PSG_CH4_ENVELOPE: usize = 0x50;
    pub const PSG_CH4_LAST_EVENT: usize = 0x54;
    pub const PSG_CH4_NEXT_EVENT: usize = 0x58;
}

fn get8(s: &[u8], o: usize) -> u8 {
    s[o]
}
fn get16(s: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([s[o], s[o + 1]])
}
fn get32(s: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(s[o..o + 4].try_into().unwrap())
}
fn get64(s: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(s[o..o + 8].try_into().unwrap())
}
fn put8(s: &mut [u8], o: usize, v: u8) {
    s[o] = v;
}
fn put16(s: &mut [u8], o: usize, v: u16) {
    s[o..o + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(s: &mut [u8], o: usize, v: u32) {
    s[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(s: &mut [u8], o: usize, v: u64) {
    s[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

/// `DMA_OFFSET`
const DMA_OFFSET: [i32; 4] = [1, -1, 0, 1];

fn savedata_type_to_mgba(ty: SavedataType) -> u8 {
    match ty {
        SavedataType::Autodetect => 0xFF,
        SavedataType::ForceNone => 0,
        SavedataType::Sram => 1,
        SavedataType::Flash512 => 2,
        SavedataType::Flash1M => 3,
        SavedataType::Eeprom => 4,
        SavedataType::Eeprom512 => 5,
        SavedataType::Sram512 => 6,
    }
}

fn savedata_type_from_mgba(v: u8) -> SavedataType {
    match v as i8 {
        0 => SavedataType::ForceNone,
        1 => SavedataType::Sram,
        2 => SavedataType::Flash512,
        3 => SavedataType::Flash1M,
        4 => SavedataType::Eeprom,
        5 => SavedataType::Eeprom512,
        6 => SavedataType::Sram512,
        _ => SavedataType::Autodetect,
    }
}

const TIMER_EVENTS: [EventId; 4] = [EventId::Timer0, EventId::Timer1, EventId::Timer2, EventId::Timer3];

impl Gba {
    /// `GBASerialize`: this machine as mGBA's `struct GBASerializedState`.
    pub fn serialize_mgba(&mut self) -> Vec<u8> {
        let mut st = vec![0u8; off::SIZE];
        let s = &mut st[..];
        put32(s, off::VERSION_MAGIC, VERSION_MAGIC);
        put32(s, off::BIOS_CHECKSUM, self.bios_checksum);
        put32(s, off::ROM_CRC32, self.rom_crc32);
        put32(s, off::MASTER_CYCLES, self.timing.master_cycles);
        put64(s, off::GLOBAL_CYCLES, self.timing.global_cycles);
        if self.memory.rom.len() >= 0xB0 {
            s[off::TITLE..off::TITLE + 12].copy_from_slice(&self.memory.rom[0xA0..0xAC]);
            s[off::ID..off::ID + 4].copy_from_slice(&self.memory.rom[0xAC..0xB0]);
        }
        for i in 0..16 {
            put32(s, off::CPU_GPRS + i * 4, self.cpu.gprs[i]);
        }
        put32(s, off::CPU_CPSR, self.cpu.cpsr.0);
        put32(s, off::CPU_SPSR, self.cpu.spsr.0);
        put32(s, off::CPU_CYCLES, self.cpu.cycles as u32);
        put32(s, off::CPU_NEXT_EVENT, self.cpu.next_event as u32);
        for i in 0..6 {
            for j in 0..7 {
                put32(s, off::CPU_BANKED_REGISTERS + (i * 7 + j) * 4, self.cpu.banked_registers[i][j]);
            }
            put32(s, off::CPU_BANKED_SPSRS + i * 4, self.cpu.banked_spsrs[i]);
        }
        put32(s, off::BIOS_PREFETCH, self.memory.bios_prefetch);
        put32(s, off::CPU_PREFETCH, self.cpu.prefetch[0]);
        put32(s, off::CPU_PREFETCH + 4, self.cpu.prefetch[1]);
        put32(s, off::LAST_PREFETCHED_PC, self.memory.last_prefetched_pc);
        let mut misc = self.cpu.halted as u32;
        misc |= (self.memory.io[(POSTFLG >> 1) as usize] as u32 & 1) << 1;
        if self.timing.is_scheduled(EventId::Irq) {
            misc |= 1 << 2;
            put32(s, off::NEXT_IRQ, self.until(EventId::Irq) as u32);
        }
        misc |= (self.cpu_blocked as u32) << 3;
        misc |= (self.keys_last as u32 & 0x7FF) << 4;
        put32(s, off::MISC_FLAGS, misc);
        put32(s, off::BIOS_STALL, self.bios_stall);

        // GBAMemorySerialize
        st[off::WRAM..off::WRAM + self.memory.wram.len()].copy_from_slice(&self.memory.wram);
        st[off::IWRAM..off::IWRAM + self.memory.iwram.len()].copy_from_slice(&self.memory.iwram);
        self.serialize_io(&mut st);
        self.serialize_video(&mut st);
        self.serialize_audio(&mut st);
        self.serialize_savedata(&mut st);
        st
    }

    /// `GBAIOSerialize` (with `GBADMASerialize` and `GBAHardwareSerialize`)
    fn serialize_io(&mut self, st: &mut [u8]) {
        let mut i = 0;
        while i < INTERNAL_MAX {
            let r = (i >> 1) as usize;
            if IS_R_SPECIAL_REGISTER[r] {
                put16(st, off::IO + i as usize, self.memory.io[r]);
            } else if IS_VALID_REGISTER[r] {
                let v = self.io_read(i);
                put16(st, off::IO + i as usize, v);
            }
            i += 2;
        }
        for i in 0..4 {
            let reg = DMA0CNT_LO + i as u32 * 12;
            put16(st, off::IO + reg as usize, self.memory.io[(reg >> 1) as usize]);
            let t = off::TIMERS + i * off::TIMER_STRIDE;
            put16(st, t, self.timers[i].reload);
            let now = self.current_time();
            put32(st, t + 4, self.timers[i].last_event.wrapping_sub(now) as u32);
            put32(st, t + 8, self.until(TIMER_EVENTS[i]) as u32);
            put32(st, t + 16, self.timers[i].flags);
        }
        put32(st, off::BUS, self.bus);
        // GBADMASerialize
        for i in 0..4 {
            let d = &self.memory.dma[i];
            let o = off::DMA + i * off::DMA_STRIDE;
            put32(st, o, d.next_source);
            put32(st, o + 4, d.next_dest);
            put32(st, o + 8, d.next_count as u32);
            put32(st, o + 12, d.when);
            put16(st, off::DMA_COUNT_LATCH + i * 2, d.count as u16);
        }
        put32(st, off::DMA_TRANSFER_REGISTER, self.memory.dma[0].latch);
        for i in 1..4 {
            put32(st, off::DMA_LATCH + (i - 1) * 4, self.memory.dma[i].latch);
        }
        put32(st, off::DMA_BLOCK_PC, self.dma_pc);
        // GBAHardwareSerialize
        let hw = &self.memory.hw;
        let mut flags1 = hw.read_write as u16;
        put8(st, off::HW_WRITE_LATCH, hw.write_latch);
        put8(st, off::HW_PIN_STATE, hw.pin_state);
        put8(st, off::HW_PIN_DIRECTION, hw.direction);
        put8(st, off::HW_DEVICES, hw.devices as u8);
        put8(st, off::HW_FLAGS3, hw.rtc.sio_output as u8);
        put32(st, off::HW_RTC_BYTES_REMAINING, hw.rtc.bytes_remaining as u32);
        put32(st, off::HW_RTC_BITS_READ, hw.rtc.bits_read as u32);
        put32(st, off::HW_RTC_BITS, hw.rtc.bits as u32);
        put32(st, off::HW_RTC_COMMAND_ACTIVE, hw.rtc.command_active as u32);
        flags1 |= (hw.rtc.sck_edge as u16) << 3;
        put32(st, off::HW_RTC_COMMAND, hw.rtc.command);
        put8(st, off::HW_RTC_CONTROL, hw.rtc.control);
        st[off::HW_TIME..off::HW_TIME + 7].copy_from_slice(&hw.rtc.time);
        // Gyro, tilt and light sensors aren't emulated: store the idle
        // readings `GBAHardwareInit` starts with.
        put16(st, off::HW_TILT_X, 0xFFF);
        put16(st, off::HW_TILT_Y, 0xFFF);
        put8(st, off::HW_LIGHT_SAMPLE, 0xFF);
        put16(st, off::HW_FLAGS1, flags1);
        let sio = self.until(EventId::Sio) as u32;
        put32(st, off::HW_SIO_NEXT_EVENT, sio);
    }

    /// `GBAVideoSerialize`
    fn serialize_video(&mut self, st: &mut [u8]) {
        for (i, v) in self.video.vram.iter().enumerate() {
            put16(st, off::VRAM + i * 2, *v);
        }
        for (i, v) in self.video.oam.iter().enumerate() {
            put16(st, off::OAM + i * 2, *v);
        }
        for (i, v) in self.video.palette.iter().enumerate() {
            put16(st, off::PRAM + i * 2, *v);
        }
        put32(st, off::VIDEO_NEXT_EVENT, self.until(EventId::Video) as u32);
        let mode = match self.video.next_phase {
            VideoPhase::StartHdraw => 1,
            VideoPhase::StartHblank => 2,
        };
        put32(st, off::VIDEO_FLAGS, mode);
        put32(st, off::VIDEO_FRAME_COUNTER, self.video.frame_counter);
    }

    /// `GBAAudioSerialize` with `GBAudioPSGSerialize`
    fn serialize_audio(&mut self, st: &mut [u8]) {
        let now = self.current_time();
        let frame_until = self.until(EventId::AudioFrame);
        let psg = &self.audio.psg;
        let p = off::AUDIO_PSG;
        let mut flags = 0u32;
        flags |= (psg.frame as u32 & 7) << 22;
        flags |= (psg.skip_frame as u32) << 28;
        put32(st, p + off::PSG_CH1_NEXT_FRAME, frame_until as u32);
        flags |= psg.ch1.envelope.current_volume as u32 & 0xF;
        flags |= (psg.ch1.envelope.dead as u32 & 3) << 4;
        flags |= (psg.ch1.sweep.enable as u32) << 25;
        flags |= (psg.ch1.sweep.occurred as u32) << 26;
        let envelope = |length: i32, next_step: i32, frequency: i32, duty: u8| {
            (length as u32 & 0x7F) | ((next_step as u32 & 7) << 7) | ((frequency as u32 & 0x7FF) << 10) | ((duty as u32 & 7) << 21)
        };
        put32(
            st,
            p + off::PSG_CH1_ENVELOPE,
            envelope(psg.ch1.control.length, psg.ch1.envelope.next_step, psg.ch1.sweep.real_frequency, psg.ch1.index),
        );
        put32(st, p + off::PSG_CH1_SWEEP, psg.ch1.sweep.time as u32 & 7);
        put32(st, p + off::PSG_CH1_LAST_UPDATE, psg.ch1.last_update.wrapping_sub(now) as u32);
        flags |= (psg.ch2.envelope.current_volume as u32 & 0xF) << 8;
        flags |= (psg.ch2.envelope.dead as u32 & 3) << 12;
        put32(st, p + off::PSG_CH2_ENVELOPE, envelope(psg.ch2.control.length, psg.ch2.envelope.next_step, 0, psg.ch2.index));
        put32(st, p + off::PSG_CH2_LAST_UPDATE, psg.ch2.last_update.wrapping_sub(now) as u32);
        flags |= (psg.ch3.readable as u32) << 27;
        for (i, w) in psg.ch3.wavedata32.iter().enumerate() {
            put32(st, p + off::PSG_CH3_WAVEBANKS + i * 4, *w);
        }
        put16(st, p + off::PSG_CH3_LENGTH, psg.ch3.length as u16);
        put32(st, p + off::PSG_CH3_NEXT_EVENT, psg.ch3.next_update.wrapping_sub(now) as u32);
        flags |= (psg.ch4.envelope.current_volume as u32 & 0xF) << 16;
        flags |= (psg.ch4.envelope.dead as u32 & 3) << 20;
        put32(st, p + off::PSG_CH4_LFSR, psg.ch4.lfsr);
        put32(st, p + off::PSG_CH4_ENVELOPE, envelope(psg.ch4.length, psg.ch4.envelope.next_step, 0, 0));
        put32(st, p + off::PSG_CH4_LAST_EVENT, psg.ch4.last_event);
        let mut cycles: i32 = if psg.ch4.ratio != 0 { 2 * psg.ch4.ratio } else { 1 };
        cycles = cycles.wrapping_shl(psg.ch4.frequency as u32);
        cycles = cycles.wrapping_mul(8 * psg.timing_factor);
        put32(st, p + off::PSG_CH4_NEXT_EVENT, psg.ch4.last_event.wrapping_add(cycles as u32));
        put32(st, off::AUDIO_FLAGS, flags);

        let a = &self.audio;
        put32(st, off::AUDIO_INTERNAL_A, a.ch_a.internal_sample);
        put32(st, off::AUDIO_INTERNAL_B, a.ch_b.internal_sample);
        for i in 0..16 {
            st[off::SAMPLES_A + i] = a.ch_a.samples[i] as u8;
            st[off::SAMPLES_B + i] = a.ch_b.samples[i] as u8;
        }
        for i in 0..16 {
            put16(st, off::CURRENT_SAMPLES + i * 4, a.current_samples[i][0] as u16);
            put16(st, off::CURRENT_SAMPLES + i * 4 + 2, a.current_samples[i][1] as u16);
        }
        put32(st, off::AUDIO_LAST_SAMPLE, a.last_sample as u32);
        let fifo_len = super::audio::FIFO_SIZE;
        let (mut read_a, mut read_b) = (a.ch_a.fifo_read, a.ch_b.fifo_read);
        for i in 0..fifo_len {
            put32(st, off::AUDIO_FIFO_A + i * 4, a.ch_a.fifo[read_a]);
            put32(st, off::AUDIO_FIFO_B + i * 4, a.ch_b.fifo[read_b]);
            read_a = (read_a + 1) % fifo_len;
            read_b = (read_b + 1) % fifo_len;
        }
        let size = |w: usize, r: usize| if w >= r { w - r } else { fifo_len - r + w };
        let size_a = size(a.ch_a.fifo_write, a.ch_a.fifo_read) as u16;
        let size_b = size(a.ch_b.fifo_write, a.ch_b.fifo_read) as u16;
        let mut gba_flags: u16 = a.ch_b.internal_remaining as u16 & 3;
        gba_flags |= (size_b & 7) << 2;
        gba_flags |= (a.ch_a.internal_remaining as u16 & 3) << 5;
        gba_flags |= (size_a & 7) << 7;
        put16(st, off::AUDIO_GBA_FLAGS, gba_flags);
        let mut flags2 = a.sample_index as u32 & 0xF;
        flags2 |= ((a.ch_a.dma_source + 1) as u32 & 3) << 4;
        flags2 |= ((a.ch_b.dma_source + 1) as u32 & 3) << 6;
        put32(st, off::AUDIO_GBA_FLAGS2, flags2);
        put32(st, off::AUDIO_NEXT_SAMPLE, self.until(EventId::AudioSample) as u32);
    }

    /// `GBASavedataSerialize`
    fn serialize_savedata(&mut self, st: &mut [u8]) {
        let sd = &self.memory.savedata;
        put8(st, off::SAVEDATA_TYPE, savedata_type_to_mgba(sd.ty));
        put8(st, off::SAVEDATA_COMMAND, sd.command as u8);
        let mut flags = (sd.flash_state as u8) & 3;
        if sd.current_bank == 0x10000 {
            flags |= 1 << 4;
        }
        if self.timing.is_scheduled(EventId::SavedataDust) {
            put32(st, off::SAVEDATA_SETTLING_DUST, self.until(EventId::SavedataDust) as u32);
            flags |= 1 << 5;
        }
        let sd = &self.memory.savedata;
        put8(st, off::SAVEDATA_FLAGS, flags);
        put8(st, off::SAVEDATA_READ_BITS_REMAINING, sd.read_bits_remaining as u8);
        put32(st, off::SAVEDATA_READ_ADDRESS, sd.read_address);
        put32(st, off::SAVEDATA_WRITE_ADDRESS, sd.write_address);
        put16(st, off::SAVEDATA_SETTLING_SECTOR, sd.settling as u16);
    }

    /// `GBADeserialize`. Rejects states for another game, like mGBA.
    pub fn deserialize_mgba(&mut self, st: &[u8]) -> Result<(), String> {
        if st.len() < off::SIZE {
            return Err("not an mGBA GBA save state (too short)".into());
        }
        let magic = get32(st, off::VERSION_MAGIC);
        if magic > VERSION_MAGIC || magic < SAVESTATE_MAGIC {
            return Err(format!("not an mGBA GBA save state (version {magic:08X})"));
        }
        if self.memory.rom.len() >= 0xB0
            && (st[off::ID..off::ID + 4] != self.memory.rom[0xAC..0xB0]
                || st[off::TITLE..off::TITLE + 12] != self.memory.rom[0xA0..0xAC])
        {
            return Err("save state is for a different game".into());
        }
        let cycles = get32(st, off::CPU_CYCLES) as i32;
        if !(0..super::ARM7TDMI_FREQUENCY as i32).contains(&cycles) {
            return Err("save state is corrupted (CPU cycles out of range)".into());
        }
        let bios_checksum = get32(st, off::BIOS_CHECKSUM);
        let pc = get32(st, off::CPU_GPRS + ARM_PC * 4);
        if bios_checksum != self.bios_checksum
            && (bios_checksum == super::bios::GBA_BIOS_CHECKSUM || self.bios_checksum == super::bios::GBA_BIOS_CHECKSUM)
            && pc < 0x4000
            && pc >= 0x20
        {
            return Err("save state was made inside a different BIOS".into());
        }

        self.timing.clear();
        self.timing.master_cycles = get32(st, off::MASTER_CYCLES);
        self.timing.global_cycles = get64(st, off::GLOBAL_CYCLES);
        for i in 0..16 {
            self.cpu.gprs[i] = get32(st, off::CPU_GPRS + i * 4);
        }
        self.cpu.cpsr = Psr(get32(st, off::CPU_CPSR));
        self.cpu.spsr = Psr(get32(st, off::CPU_SPSR));
        self.cpu.cycles = cycles;
        self.cpu.next_event = get32(st, off::CPU_NEXT_EVENT) as i32;
        for i in 0..6 {
            for j in 0..7 {
                self.cpu.banked_registers[i][j] = get32(st, off::CPU_BANKED_REGISTERS + (i * 7 + j) * 4);
            }
            self.cpu.banked_spsrs[i] = get32(st, off::CPU_BANKED_SPSRS + i * 4);
        }
        self.cpu.privilege_mode = self.cpu.cpsr.priv_mode();
        if self.cpu.gprs[ARM_PC] & 1 != 0 {
            self.cpu.gprs[ARM_PC] &= !1;
        }
        self.memory.active_region = -1;
        let pc = self.cpu.gprs[ARM_PC];
        self.set_active_region_impl(pc);
        let bios_prefetch = get32(st, off::BIOS_PREFETCH);
        if bios_prefetch != 0 {
            self.memory.bios_prefetch = bios_prefetch;
        }
        self.memory.last_prefetched_pc = get32(st, off::LAST_PREFETCHED_PC);
        let (p0, p1) = (get32(st, off::CPU_PREFETCH), get32(st, off::CPU_PREFETCH + 4));
        if self.cpu.cpsr.t() {
            self.cpu.execution_mode = ExecutionMode::Thumb;
            if p0 != 0 && p1 != 0 {
                self.cpu.prefetch = [p0 & 0xFFFF, p1 & 0xFFFF];
            } else {
                let mask = self.cpu.memory.active_mask;
                self.cpu.prefetch[0] = self.active_fetch16(pc.wrapping_sub(WORD_SIZE_THUMB) & mask);
                self.cpu.prefetch[1] = self.active_fetch16(pc & mask);
            }
        } else {
            self.cpu.execution_mode = ExecutionMode::Arm;
            if p0 != 0 && p1 != 0 {
                self.cpu.prefetch = [p0, p1];
            } else {
                let mask = self.cpu.memory.active_mask;
                self.cpu.prefetch[0] = self.active_fetch32(pc.wrapping_sub(WORD_SIZE_ARM) & mask);
                self.cpu.prefetch[1] = self.active_fetch32(pc & mask);
            }
        }
        let misc = get32(st, off::MISC_FLAGS);
        self.cpu.halted = misc & 1 != 0;
        self.memory.io[(POSTFLG >> 1) as usize] = ((misc >> 1) & 1) as u16;
        if misc & (1 << 2) != 0 {
            let when = get32(st, off::NEXT_IRQ) as i32;
            self.schedule(EventId::Irq, when);
        }
        self.cpu_blocked = misc & (1 << 3) != 0;
        self.keys_last = ((misc >> 4) & 0x7FF) as u16;
        self.bios_stall = get32(st, off::BIOS_STALL);

        self.deserialize_video(st, magic);
        // GBAMemoryDeserialize
        let wl = self.memory.wram.len();
        self.memory.wram.copy_from_slice(&st[off::WRAM..off::WRAM + wl]);
        let il = self.memory.iwram.len();
        self.memory.iwram.copy_from_slice(&st[off::IWRAM..off::IWRAM + il]);
        self.deserialize_io(st, magic);
        self.deserialize_audio(st, magic);
        self.deserialize_savedata(st);
        self.timing.interrupt();
        self.rebuild_renderer();
        if self.sio_driver.is_some() && !self.timing.is_scheduled(EventId::SioLockstep) {
            self.schedule(EventId::SioLockstep, 0);
        }
        Ok(())
    }

    /// `GBAVideoDeserialize`
    fn deserialize_video(&mut self, st: &[u8], magic: u32) {
        for i in 0..self.video.vram.len() {
            self.video.vram[i] = get16(st, off::VRAM + i * 2);
        }
        self.renderer.reset();
        for i in 0..self.video.oam.len() {
            let v = get16(st, off::OAM + i * 2);
            self.mem_store16(super::memory::BASE_OAM | (i as u32 * 2), v, None);
        }
        for i in 0..self.video.palette.len() {
            let v = get16(st, off::PRAM + i * 2);
            self.mem_store16(super::memory::BASE_PALETTE_RAM | (i as u32 * 2), v, None);
        }
        self.video.frame_counter = get32(st, off::VIDEO_FRAME_COUNTER);
        self.video.stall_mask = 0;
        let flags = get32(st, off::VIDEO_FLAGS);
        let dispstat = get16(st, off::IO + DISPSTAT as usize);
        self.video.next_phase = match flags & 3 {
            0 => {
                if dispstat & 2 != 0 {
                    VideoPhase::StartHdraw
                } else {
                    VideoPhase::StartHblank
                }
            }
            2 => {
                let dispcnt = get16(st, off::IO + DISPCNT as usize);
                self.video.stall_mask = self.calculate_stall_mask(dispcnt);
                VideoPhase::StartHblank
            }
            _ => VideoPhase::StartHdraw,
        };
        let when = if magic < 0x0100_0007 { get32(st, off::AUDIO_LAST_SAMPLE) } else { get32(st, off::VIDEO_NEXT_EVENT) };
        self.schedule(EventId::Video, when as i32);
        self.video.vcount = get16(st, off::IO + VCOUNT as usize) as i32;
    }

    /// `GBAIODeserialize` (with `GBADMADeserialize` and `GBAHardwareDeserialize`)
    fn deserialize_io(&mut self, st: &[u8], magic: u32) {
        let soundcnt_x = get16(st, off::IO + SOUNDCNT_X as usize);
        self.memory.io[(SOUNDCNT_X >> 1) as usize] = soundcnt_x;
        // NR52 aliases this register's low byte in mGBA.
        self.audio.psg.nr52 = soundcnt_x as u8;
        self.audio_write_soundcnt_x(soundcnt_x);
        let mut i = 0;
        while i < 0x20A {
            let r = (i >> 1) as usize;
            let v = get16(st, off::IO + i as usize);
            if IS_W_SPECIAL_REGISTER[r] {
                self.memory.io[r] = v;
                if i == SOUNDCNT_X {
                    self.audio.psg.nr52 = v as u8;
                }
            } else if IS_VALID_REGISTER[r] {
                self.io_write(i, v);
            }
            i += 2;
        }
        if magic >= 0x0100_0006 {
            let v = self.memory.io[(INTERNAL_EXWAITCNT_HI >> 1) as usize];
            self.io_write(EXWAITCNT_HI, v);
        }
        for i in 0..4 {
            let t = off::TIMERS + i * off::TIMER_STRIDE;
            self.timers[i].reload = get16(st, t);
            self.timers[i].flags = get32(st, t + 16);
            let now = self.current_time();
            self.timers[i].last_event = (get32(st, t + 4) as i32).wrapping_add(now);
            let when = get32(st, t + 8) as i32;
            let count_up = self.timers[i].flags & (1 << 4) != 0;
            let enable = self.timers[i].flags & (1 << 6) != 0;
            if (i < 1 || !count_up) && enable {
                self.schedule(TIMER_EVENTS[i], when);
            } else {
                let abs = (when as u32).wrapping_add(now as u32);
                self.timing.whens[TIMER_EVENTS[i].index()] = abs;
            }
        }
        self.sio.siocnt = self.memory.io[(SIOCNT >> 1) as usize];
        let rcnt = self.memory.io[(RCNT >> 1) as usize];
        self.sio_write_rcnt(rcnt);
        self.bus = get32(st, off::BUS);

        // GBADMADeserialize
        for i in 0..4 {
            let o = off::DMA + i * off::DMA_STRIDE;
            let reg = get16(st, off::IO + (DMA0CNT_HI as usize) + i * 12);
            let d = &mut self.memory.dma[i];
            d.reg = reg;
            d.next_source = get32(st, o);
            d.next_dest = get32(st, o + 4);
            d.next_count = get32(st, o + 8) as i32;
            d.when = get32(st, o + 12);
            d.count = if magic >= SAVESTATE_MAGIC + 0xB {
                get16(st, off::DMA_COUNT_LATCH + i * 2) as i32
            } else {
                self.memory.io[((DMA0CNT_LO as usize) + i * 12) >> 1] as i32
            };
            if i == 3 {
                if d.count == 0 {
                    d.count = 0x10000;
                }
            } else {
                d.count &= 0x3FFF;
                if d.count == 0 {
                    d.count = 0x4000;
                }
            }
            let width = 2 << ((reg >> 10) & 1);
            if d.source >= super::memory::BASE_ROM0 && d.source < super::memory::BASE_SRAM {
                d.source_offset = width;
            } else {
                d.source_offset = DMA_OFFSET[((reg >> 7) & 3) as usize] * width;
            }
            d.dest_offset = DMA_OFFSET[((reg >> 5) & 3) as usize] * width;
        }
        self.memory.dma[0].latch = get32(st, off::DMA_TRANSFER_REGISTER);
        for i in 1..4 {
            self.memory.dma[i].latch =
                if magic >= SAVESTATE_MAGIC + 0xA { get32(st, off::DMA_LATCH + (i - 1) * 4) } else { self.memory.dma[0].latch };
        }
        self.dma_pc = get32(st, off::DMA_BLOCK_PC);
        self.dma_recalculate_cycles();
        self.dma_update();

        // GBAHardwareDeserialize
        let flags1 = get16(st, off::HW_FLAGS1);
        let hw = &mut self.memory.hw;
        hw.read_write = flags1 & 1 != 0;
        hw.write_latch = get8(st, off::HW_WRITE_LATCH) & 0xF;
        hw.pin_state = get8(st, off::HW_PIN_STATE) & 0xF;
        hw.direction = get8(st, off::HW_PIN_DIRECTION) & 0xF;
        hw.devices = get8(st, off::HW_DEVICES) as u32;
        if hw.devices & HW_GPIO != 0 {
            hw.gpio_written = true;
            hw.regs_visible =
                if hw.read_write { [hw.pin_state as u16, hw.direction as u16, hw.read_write as u16] } else { [0; 3] };
        }
        hw.rtc.sio_output = get8(st, off::HW_FLAGS3) & 1 != 0;
        hw.rtc.bytes_remaining = get32(st, off::HW_RTC_BYTES_REMAINING) as i32;
        hw.rtc.bits_read = get32(st, off::HW_RTC_BITS_READ) as i32;
        hw.rtc.bits = get32(st, off::HW_RTC_BITS) as i32;
        hw.rtc.command_active = get32(st, off::HW_RTC_COMMAND_ACTIVE) != 0;
        hw.rtc.sck_edge = flags1 & (1 << 3) != 0;
        hw.rtc.command = get32(st, off::HW_RTC_COMMAND);
        hw.rtc.control = get8(st, off::HW_RTC_CONTROL);
        hw.rtc.time.copy_from_slice(&st[off::HW_TIME..off::HW_TIME + 7]);
        let when = get32(st, off::HW_SIO_NEXT_EVENT);
        if self.memory.io[(SIOCNT >> 1) as usize] & 0x0080 != 0 && when < 0x20000 {
            self.schedule(EventId::Sio, when as i32);
        }
    }

    /// `GBAAudioDeserialize` with `GBAudioPSGDeserialize`
    fn deserialize_audio(&mut self, st: &[u8], magic: u32) {
        let p = off::AUDIO_PSG;
        {
            let psg = &mut self.audio.psg;
            psg.playing_ch1 = psg.nr52 & 1 != 0;
            psg.playing_ch2 = psg.nr52 & 2 != 0;
            psg.playing_ch3 = psg.nr52 & 4 != 0;
            psg.playing_ch4 = psg.nr52 & 8 != 0;
            psg.enable = psg.nr52 & 0x80 != 0;
        }
        let when = get32(st, p + off::PSG_CH1_NEXT_FRAME) as i32;
        self.schedule(EventId::AudioFrame, when);
        let now = self.current_time();
        let flags = get32(st, off::AUDIO_FLAGS);
        let ch1_flags = get32(st, p + off::PSG_CH1_ENVELOPE);
        let sweep = get32(st, p + off::PSG_CH1_SWEEP);
        let psg = &mut self.audio.psg;
        psg.frame = ((flags >> 22) & 7) as i32;
        psg.skip_frame = flags & (1 << 28) != 0;
        psg.ch1.envelope.current_volume = (flags & 0xF) as i32;
        psg.ch1.envelope.dead = ((flags >> 4) & 3) as i32;
        psg.ch1.sweep.enable = flags & (1 << 25) != 0;
        psg.ch1.sweep.occurred = flags & (1 << 26) != 0;
        psg.ch1.sweep.time = (sweep & 7) as i32;
        if psg.ch1.sweep.time == 0 {
            psg.ch1.sweep.time = 8;
        }
        psg.ch1.control.length = (ch1_flags & 0x7F) as i32;
        psg.ch1.envelope.next_step = ((ch1_flags >> 7) & 7) as i32;
        psg.ch1.sweep.real_frequency = ((ch1_flags >> 10) & 0x7FF) as i32;
        psg.ch1.index = ((ch1_flags >> 21) & 7) as u8;
        psg.ch1.last_update = (get32(st, p + off::PSG_CH1_LAST_UPDATE) as i32).wrapping_add(now);
        let ch2_flags = get32(st, p + off::PSG_CH2_ENVELOPE);
        psg.ch2.envelope.current_volume = ((flags >> 8) & 0xF) as i32;
        psg.ch2.envelope.dead = ((flags >> 12) & 3) as i32;
        psg.ch2.control.length = (ch2_flags & 0x7F) as i32;
        psg.ch2.envelope.next_step = ((ch2_flags >> 7) & 7) as i32;
        psg.ch2.index = ((ch2_flags >> 21) & 7) as u8;
        psg.ch2.last_update = (get32(st, p + off::PSG_CH2_LAST_UPDATE) as i32).wrapping_add(now);
        psg.ch3.readable = flags & (1 << 27) != 0;
        for i in 0..8 {
            psg.ch3.wavedata32[i] = get32(st, p + off::PSG_CH3_WAVEBANKS + i * 4);
        }
        psg.ch3.length = get16(st, p + off::PSG_CH3_LENGTH) as i16 as i32 as u32;
        psg.ch3.next_update = (get32(st, p + off::PSG_CH3_NEXT_EVENT) as i32).wrapping_add(now);
        let ch4_flags = get32(st, p + off::PSG_CH4_ENVELOPE);
        psg.ch4.envelope.current_volume = ((flags >> 16) & 0xF) as i32;
        psg.ch4.envelope.dead = ((flags >> 20) & 3) as i32;
        psg.ch4.length = (ch4_flags & 0x7F) as i32;
        psg.ch4.envelope.next_step = ((ch4_flags >> 7) & 7) as i32;
        psg.ch4.lfsr = get32(st, p + off::PSG_CH4_LFSR);
        psg.ch4.last_event = get32(st, p + off::PSG_CH4_LAST_EVENT);
        let when = get32(st, p + off::PSG_CH4_NEXT_EVENT);
        if psg.ch4.envelope.dead < 2 && psg.playing_ch4 && psg.ch4.last_event == 0 {
            // Back-compat: fake this value
            let mut cycles: i32 = if psg.ch4.ratio != 0 { 2 * psg.ch4.ratio } else { 1 };
            cycles = cycles.wrapping_shl(psg.ch4.frequency as u32);
            cycles = cycles.wrapping_mul(8 * psg.timing_factor);
            psg.ch4.last_event =
                (now as u32).wrapping_add(when & (cycles as u32).wrapping_sub(1)).wrapping_sub(cycles as u32);
        }
        psg.ch4.n_samples = 0;
        psg.ch4.samples = 0;

        for reg in [SOUND1CNT_X, SOUND2CNT_HI, SOUND3CNT_X, SOUND4CNT_HI] {
            let v = get16(st, off::IO + reg as usize);
            self.io_write(reg, v & 0x7FFF);
        }
        let a = &mut self.audio;
        a.ch_a.internal_sample = get32(st, off::AUDIO_INTERNAL_A);
        a.ch_b.internal_sample = get32(st, off::AUDIO_INTERNAL_B);
        for i in 0..16 {
            a.ch_a.samples[i] = st[off::SAMPLES_A + i] as i8;
            a.ch_b.samples[i] = st[off::SAMPLES_B + i] as i8;
        }
        for i in 0..16 {
            a.current_samples[i][0] = get16(st, off::CURRENT_SAMPLES + i * 4) as i16;
            a.current_samples[i][1] = get16(st, off::CURRENT_SAMPLES + i * 4 + 2) as i16;
        }
        a.last_sample = get32(st, off::AUDIO_LAST_SAMPLE) as i32;
        for i in 0..super::audio::FIFO_SIZE {
            a.ch_a.fifo[i] = get32(st, off::AUDIO_FIFO_A + i * 4);
            a.ch_b.fifo[i] = get32(st, off::AUDIO_FIFO_B + i * 4);
        }
        a.ch_a.fifo_read = 0;
        a.ch_b.fifo_read = 0;
        let gba_flags = get16(st, off::AUDIO_GBA_FLAGS);
        a.ch_a.fifo_write = ((gba_flags >> 7) & 7) as usize;
        a.ch_b.fifo_write = ((gba_flags >> 2) & 7) as usize;
        a.ch_a.internal_remaining = ((gba_flags >> 5) & 3) as i32;
        a.ch_b.internal_remaining = (gba_flags & 3) as i32;
        let flags2 = get32(st, off::AUDIO_GBA_FLAGS2);
        a.sample_index = (flags2 & 0xF) as usize;
        if (flags2 >> 4) & 3 > 0 {
            a.ch_a.dma_source = ((flags2 >> 4) & 3) as i32 - 1;
        }
        if (flags2 >> 6) & 3 > 0 {
            a.ch_b.dma_source = ((flags2 >> 6) & 3) as i32 - 1;
        }
        let when = get32(st, off::AUDIO_NEXT_SAMPLE) as i32;
        if magic < 0x0100_0007 {
            a.last_sample = when.wrapping_sub(a.sample_interval);
        }
        self.schedule(EventId::AudioSample, when);
    }

    /// `GBASavedataDeserialize`
    fn deserialize_savedata(&mut self, st: &[u8]) {
        let ty = savedata_type_from_mgba(get8(st, off::SAVEDATA_TYPE));
        if self.memory.savedata.ty != ty {
            self.memory.savedata.force_type(ty);
        }
        let flags = get8(st, off::SAVEDATA_FLAGS);
        let sd = &mut self.memory.savedata;
        sd.command = get8(st, off::SAVEDATA_COMMAND) as u32;
        sd.flash_state = (flags & 3) as u32;
        sd.read_bits_remaining = get8(st, off::SAVEDATA_READ_BITS_REMAINING) as i8;
        sd.read_address = get32(st, off::SAVEDATA_READ_ADDRESS);
        sd.write_address = get32(st, off::SAVEDATA_WRITE_ADDRESS);
        sd.settling = get16(st, off::SAVEDATA_SETTLING_SECTOR) as u32;
        if sd.ty == SavedataType::Flash1M {
            sd.flash_switch_bank(((flags >> 4) & 1) as usize);
        }
        if flags & (1 << 5) != 0 {
            let when = get32(st, off::SAVEDATA_SETTLING_DUST) as i32;
            self.schedule(EventId::SavedataDust, when);
        }
    }

    /// Bring the renderer in line with video memory and registers
    /// (what `GBAVideoAssociateRenderer` does).
    pub(crate) fn rebuild_renderer(&mut self) {
        self.renderer.sync_palette(&self.video.palette);
        let dispcnt = self.memory.io[(DISPCNT >> 1) as usize];
        self.renderer.write_video_register(DISPCNT, dispcnt);
        let stereo = self.memory.io[(STEREOCNT >> 1) as usize];
        self.renderer.write_video_register(STEREOCNT, stereo);
        let mut address = BG0CNT;
        while address < 0x56 {
            if address != 0x4E {
                let v = self.memory.io[(address >> 1) as usize];
                self.renderer.write_video_register(address, v);
            }
            address += 2;
        }
        self.renderer.write_oam(0);
    }

    /// An mGBA save state file for this machine: the raw state followed by
    /// extdata with the save data (`mCoreSaveStateNamed` without a screenshot).
    pub fn save_state_mgba(&mut self) -> Vec<u8> {
        let state = self.serialize_mgba();
        let mut extdata = BTreeMap::new();
        if let Some(save) = self.savedata() {
            if !save.is_empty() {
                extdata.insert(EXTDATA_SAVEDATA, save.to_vec());
            }
        }
        state_container::build_raw(&state, &extdata)
    }

    /// Load an mGBA save state file (raw or PNG, `mCoreLoadStateNamed`).
    pub fn load_state_mgba(&mut self, file: &[u8]) -> Result<(), String> {
        let (state, extdata) =
            state_container::extract(file, off::SIZE).ok_or_else(|| "not an mGBA save state".to_string())?;
        self.deserialize_mgba(&state)?;
        if let Some(save) = extdata.get(&EXTDATA_SAVEDATA) {
            let n = save.len().min(self.memory.savedata.data.len());
            self.memory.savedata.data[..n].copy_from_slice(&save[..n]);
        }
        Ok(())
    }
}

#[allow(unused_imports)]
use arm as _;
