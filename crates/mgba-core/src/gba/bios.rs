// Copyright (c) 2013-2015 Jeffrey Pfau
// Rust port of mGBA's high-level BIOS emulation (src/gba/bios.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::memory::{
    BASE_IO, BASE_OFFSET, REGION_BIOS, REGION_EWRAM, REGION_IWRAM, REGION_VRAM, SIZE_BIOS, SIZE_EWRAM, SIZE_IWRAM,
};
use super::regs::*;
use super::sio::RCNT_INITIAL;
use super::{glog, Gba, LogLevel};
use crate::arm::{self, ExecutionMode, ARM_LR, ARM_PC, ARM_SP, LSM_DB, MODE_SYSTEM};

pub const GBA_BIOS_CHECKSUM: u32 = 0xBAAE187F;
pub const GBA_DS_BIOS_CHECKSUM: u32 = 0xBAAE1880;
pub const GBA_DEBUG_BIOS_CHECKSUM: u32 = 0x242CEF81;

pub const SWI_SOFT_RESET: u32 = 0x00;
pub const SWI_REGISTER_RAM_RESET: u32 = 0x01;
pub const SWI_HALT: u32 = 0x02;
pub const SWI_STOP: u32 = 0x03;
pub const SWI_INTR_WAIT: u32 = 0x04;
pub const SWI_VBLANK_INTR_WAIT: u32 = 0x05;
pub const SWI_DIV: u32 = 0x06;
pub const SWI_DIV_ARM: u32 = 0x07;
pub const SWI_SQRT: u32 = 0x08;
pub const SWI_ARCTAN: u32 = 0x09;
pub const SWI_ARCTAN2: u32 = 0x0A;
pub const SWI_CPU_SET: u32 = 0x0B;
pub const SWI_CPU_FAST_SET: u32 = 0x0C;
pub const SWI_GET_BIOS_CHECKSUM: u32 = 0x0D;
pub const SWI_BG_AFFINE_SET: u32 = 0x0E;
pub const SWI_OBJ_AFFINE_SET: u32 = 0x0F;
pub const SWI_BIT_UNPACK: u32 = 0x10;
pub const SWI_LZ77_UNCOMP_WRAM: u32 = 0x11;
pub const SWI_LZ77_UNCOMP_VRAM: u32 = 0x12;
pub const SWI_HUFFMAN_UNCOMP: u32 = 0x13;
pub const SWI_RL_UNCOMP_WRAM: u32 = 0x14;
pub const SWI_RL_UNCOMP_VRAM: u32 = 0x15;
pub const SWI_DIFF_8BIT_UNFILTER_WRAM: u32 = 0x16;
pub const SWI_DIFF_8BIT_UNFILTER_VRAM: u32 = 0x17;
pub const SWI_DIFF_16BIT_UNFILTER: u32 = 0x18;
pub const SWI_SOUND_BIAS: u32 = 0x19;
pub const SWI_MIDI_KEY_2_FREQ: u32 = 0x1F;
pub const SWI_SOUND_DRIVER_GET_JUMP_LIST: u32 = 0x2A;
pub const SWI_BUG_JUMP_TO_EWRAM: u32 = 0xD4;

/// `GBAChecksum`
pub fn checksum(memory: &[u8]) -> u32 {
    memory
        .chunks_exact(4)
        .fold(0u32, |sum, w| sum.wrapping_add(u32::from_le_bytes([w[0], w[1], w[2], w[3]])))
}

fn mul_wait(r: i32) -> u32 {
    let r = r as u32;
    if (r & 0xFFFFFF00) == 0xFFFFFF00 || (r & 0xFFFFFF00) == 0 {
        1
    } else if (r & 0xFFFF0000) == 0xFFFF0000 || (r & 0xFFFF0000) == 0 {
        2
    } else if (r & 0xFF000000) == 0xFF000000 || (r & 0xFF000000) == 0 {
        3
    } else {
        4
    }
}

fn arctan(i: i32, r1: Option<&mut i32>, r3: Option<&mut i32>, cycles: &mut u32) -> i16 {
    let mut current_cycles: u32 = 37;
    current_cycles += mul_wait(i.wrapping_mul(i));
    let a = -((i.wrapping_mul(i)) >> 14);
    current_cycles += mul_wait(0xA9i32.wrapping_mul(a));
    let mut b = ((0xA9i32.wrapping_mul(a)) >> 14) + 0x390;
    current_cycles += mul_wait(b.wrapping_mul(a));
    b = ((b.wrapping_mul(a)) >> 14) + 0x91C;
    current_cycles += mul_wait(b.wrapping_mul(a));
    b = ((b.wrapping_mul(a)) >> 14) + 0xFB6;
    current_cycles += mul_wait(b.wrapping_mul(a));
    b = ((b.wrapping_mul(a)) >> 14) + 0x16AA;
    current_cycles += mul_wait(b.wrapping_mul(a));
    b = ((b.wrapping_mul(a)) >> 14) + 0x2081;
    current_cycles += mul_wait(b.wrapping_mul(a));
    b = ((b.wrapping_mul(a)) >> 14) + 0x3651;
    current_cycles += mul_wait(b.wrapping_mul(a));
    b = ((b.wrapping_mul(a)) >> 14) + 0xA2F9;
    if let Some(r1) = r1 {
        *r1 = a;
    }
    if let Some(r3) = r3 {
        *r3 = b;
    }
    *cycles = current_cycles;
    ((i.wrapping_mul(b)) >> 16) as i16
}

fn arctan2(x: i32, y: i32, r1: &mut i32, cycles: &mut u32) -> i16 {
    if y == 0 {
        *cycles = 11;
        return if x >= 0 { 0 } else { 0x8000u16 as i16 };
    }
    if x == 0 {
        *cycles = 11;
        return if y >= 0 { 0x4000 } else { 0xC000u16 as i16 };
    }
    let at = |n: i32, d: i32, r1: &mut i32, cycles: &mut u32| -> i32 { arctan(n.wrapping_shl(14) / d, Some(r1), None, cycles) as i32 };
    let r = if y >= 0 {
        if x >= 0 {
            if x >= y {
                return at(y, x, r1, cycles) as i16;
            }
        } else if -x >= y {
            return (at(y, x, r1, cycles) + 0x8000) as i16;
        }
        0x4000 - at(x, y, r1, cycles)
    } else {
        if x <= 0 {
            if -x > -y {
                return (at(y, x, r1, cycles) + 0x8000) as i16;
            }
        } else if x >= -y {
            return (at(y, x, r1, cycles) + 0x10000) as i16;
        }
        0xC000 - at(x, y, r1, cycles)
    };
    r as i16
}

fn sqrt(x: u32, cycles: &mut u32) -> u32 {
    if x == 0 {
        *cycles = 53;
        return 0;
    }
    let mut current_cycles: u32 = 15;
    let mut lower;
    let mut upper = x;
    let mut bound: u32 = 1;
    while bound < upper {
        upper >>= 1;
        bound <<= 1;
        current_cycles += 6;
    }
    loop {
        current_cycles += 6;
        upper = x;
        let mut accum: u32 = 0;
        lower = bound;
        loop {
            current_cycles += 5;
            let old_lower = lower;
            if lower <= upper >> 1 {
                lower <<= 1;
            }
            if old_lower >= upper >> 1 {
                break;
            }
        }
        loop {
            current_cycles += 8;
            accum <<= 1;
            if upper >= lower {
                accum += 1;
                upper -= lower;
            }
            if lower == bound {
                break;
            }
            lower >>= 1;
        }
        let old_bound = bound;
        bound = bound.wrapping_add(accum);
        bound >>= 1;
        if bound >= old_bound {
            bound = old_bound;
            break;
        }
    }
    *cycles = current_cycles;
    bound
}

impl Gba {
    #[inline]
    fn s16(&mut self, addr: u32, v: u16) {
        self.mem_store16(addr, v, None);
    }
    #[inline]
    fn s32(&mut self, addr: u32, v: u32) {
        self.mem_store32(addr, v, None);
    }

    fn register_ram_reset(&mut self) {
        let registers = self.cpu.gprs[0];
        self.s16(BASE_IO | DISPCNT, 0x0080);
        if registers & 0x01 != 0 {
            self.memory.wram[..SIZE_EWRAM as usize].fill(0);
        }
        if registers & 0x02 != 0 {
            self.memory.iwram[..(SIZE_IWRAM - 0x200) as usize].fill(0);
        }
        if registers & 0x04 != 0 {
            self.video.palette.fill(0);
        }
        if registers & 0x08 != 0 {
            self.video.vram.fill(0);
        }
        if registers & 0x10 != 0 {
            self.video.oam.fill(0);
        }
        if registers & 0x20 != 0 {
            self.s16(BASE_IO | SIOCNT, 0x0000);
            self.s16(BASE_IO | RCNT, RCNT_INITIAL);
            self.s16(BASE_IO | SIOMLT_SEND, 0);
            self.s16(BASE_IO | JOYCNT, 0);
            self.s32(BASE_IO | JOY_RECV_LO, 0);
            self.s32(BASE_IO | JOY_TRANS_LO, 0);
        }
        if registers & 0x40 != 0 {
            for reg in [
                SOUND1CNT_LO,
                SOUND1CNT_HI,
                SOUND1CNT_X,
                SOUND2CNT_LO,
                SOUND2CNT_HI,
                SOUND3CNT_LO,
                SOUND3CNT_HI,
                SOUND3CNT_X,
                SOUND4CNT_LO,
                SOUND4CNT_HI,
                SOUNDCNT_LO,
                SOUNDCNT_HI,
                SOUNDCNT_X,
            ] {
                self.s16(BASE_IO | reg, 0);
            }
            self.s16(BASE_IO | SOUNDBIAS, 0x200);
            self.audio.psg.ch3.wavedata32 = [0; 8];
        }
        if registers & 0x80 != 0 {
            for reg in [
                DISPSTAT, VCOUNT, BG0CNT, BG1CNT, BG2CNT, BG3CNT, BG0HOFS, BG0VOFS, BG1HOFS, BG1VOFS, BG2HOFS, BG2VOFS,
                BG3HOFS, BG3VOFS,
            ] {
                self.s16(BASE_IO | reg, 0);
            }
            self.s16(BASE_IO | BG2PA, 0x100);
            self.s16(BASE_IO | BG2PB, 0);
            self.s16(BASE_IO | BG2PC, 0);
            self.s16(BASE_IO | BG2PD, 0x100);
            self.s32(BASE_IO | BG2X_LO, 0);
            self.s32(BASE_IO | BG2Y_LO, 0);
            self.s16(BASE_IO | BG3PA, 0x100);
            self.s16(BASE_IO | BG3PB, 0);
            self.s16(BASE_IO | BG3PC, 0);
            self.s16(BASE_IO | BG3PD, 0x100);
            self.s32(BASE_IO | BG3X_LO, 0);
            self.s32(BASE_IO | BG3Y_LO, 0);
            for reg in [
                WIN0H, WIN1H, WIN0V, WIN1V, WININ, WINOUT, MOSAIC, BLDCNT, BLDALPHA, BLDY, DMA0SAD_LO, DMA0SAD_HI,
                DMA0DAD_LO, DMA0DAD_HI, DMA0CNT_LO, DMA0CNT_HI, DMA1SAD_LO, DMA1SAD_HI, DMA1DAD_LO, DMA1DAD_HI,
                DMA1CNT_LO, DMA1CNT_HI, DMA2SAD_LO, DMA2SAD_HI, DMA2DAD_LO, DMA2DAD_HI, DMA2CNT_LO, DMA2CNT_HI,
                DMA3SAD_LO, DMA3SAD_HI, DMA3DAD_LO, DMA3DAD_HI, DMA3CNT_LO, DMA3CNT_HI, TM0CNT_LO, TM0CNT_HI,
                TM1CNT_LO, TM1CNT_HI, TM2CNT_LO, TM2CNT_HI, TM3CNT_LO, TM3CNT_HI, IE,
            ] {
                self.s16(BASE_IO | reg, 0);
            }
            self.s16(BASE_IO | IF, 0xFFFF);
            self.s16(BASE_IO | WAITCNT, 0);
            self.s16(BASE_IO | IME, 0);
        }
        if registers & 0x9C != 0 {
            self.renderer.reset();
            self.renderer.sync_palette(&self.video.palette);
            let v = self.memory.io[(DISPCNT >> 1) as usize];
            self.renderer.write_video_register(DISPCNT, v);
            let mut i = BG0CNT;
            while i < SOUND1CNT_LO {
                let v = self.memory.io[(i >> 1) as usize];
                self.renderer.write_video_register(i, v);
                i += 2;
            }
        }
    }

    fn bg_affine_set(&mut self) {
        let mut i = self.cpu.gprs[2] as i32;
        let mut offset = self.cpu.gprs[0];
        let mut destination = self.cpu.gprs[1];
        while i > 0 {
            i -= 1;
            let ox = self.mem_load32(offset, None) as i32 as f32 / 256.0;
            let oy = self.mem_load32(offset + 4, None) as i32 as f32 / 256.0;
            let cx = self.mem_load16(offset + 8, None) as i16 as f32;
            let cy = self.mem_load16(offset + 10, None) as i16 as f32;
            let sx = self.mem_load16(offset + 12, None) as i16 as f32 / 256.0;
            let sy = self.mem_load16(offset + 14, None) as i16 as f32 / 256.0;
            let theta = (self.mem_load16(offset + 16, None) >> 8) as f32 / 128.0 * std::f32::consts::PI;
            offset = offset.wrapping_add(20);
            // Rotation
            let (mut a, mut d) = (theta.cos(), theta.cos());
            let (mut b, mut c) = (theta.sin(), theta.sin());
            // Scale
            a *= sx;
            b *= -sx;
            c *= sy;
            d *= sy;
            // Translate
            let rx = ox - (a * cx + b * cy);
            let ry = oy - (c * cx + d * cy);
            self.s16(destination, (a * 256.0) as i32 as u16);
            self.s16(destination + 2, (b * 256.0) as i32 as u16);
            self.s16(destination + 4, (c * 256.0) as i32 as u16);
            self.s16(destination + 6, (d * 256.0) as i32 as u16);
            self.s32(destination + 8, (rx * 256.0) as i32 as u32);
            self.s32(destination + 12, (ry * 256.0) as i32 as u32);
            destination = destination.wrapping_add(16);
        }
    }

    fn obj_affine_set(&mut self) {
        let mut i = self.cpu.gprs[2] as i32;
        let mut offset = self.cpu.gprs[0];
        let mut destination = self.cpu.gprs[1];
        let diff = self.cpu.gprs[3];
        while i > 0 {
            i -= 1;
            let sx = self.mem_load16(offset, None) as i16 as f32 / 256.0;
            let sy = self.mem_load16(offset + 2, None) as i16 as f32 / 256.0;
            let theta = (self.mem_load16(offset + 4, None) >> 8) as f32 / 128.0 * std::f32::consts::PI;
            offset = offset.wrapping_add(8);
            let (mut a, mut d) = (theta.cos(), theta.cos());
            let (mut b, mut c) = (theta.sin(), theta.sin());
            a *= sx;
            b *= -sx;
            c *= sy;
            d *= sy;
            self.s16(destination, (a * 256.0) as i32 as u16);
            self.s16(destination.wrapping_add(diff), (b * 256.0) as i32 as u16);
            self.s16(destination.wrapping_add(diff * 2), (c * 256.0) as i32 as u16);
            self.s16(destination.wrapping_add(diff * 3), (d * 256.0) as i32 as u16);
            destination = destination.wrapping_add(diff * 4);
        }
    }

    fn midi_key_2_freq(&mut self) {
        let old_region = self.memory.active_region;
        self.memory.active_region = REGION_BIOS as i32;
        let key = self.mem_load32(self.cpu.gprs[0].wrapping_add(4), None);
        self.memory.active_region = old_region;
        let r1 = self.cpu.gprs[1] as i32 as f32;
        let r2 = self.cpu.gprs[2] as i32 as f32;
        self.cpu.gprs[0] = (key as f32 / ((180.0 - r1 - r2 / 256.0) / 12.0).exp2()) as u32;
    }

    fn div(&mut self, num: i32, denom: i32) {
        if denom == 0 {
            glog!(self, LogLevel::GameError, "Attempting to divide {} by zero!", num);
            // If abs(num) > 1, this should hang, but that would be painful to
            // emulate in HLE, and no game will get into a state under normal
            // operation where it hangs...
            self.cpu.gprs[0] = if num < 0 { -1i32 as u32 } else { 1 };
            self.cpu.gprs[1] = num as u32;
            self.cpu.gprs[3] = 1;
        } else if denom == -1 && num == i32::MIN {
            glog!(self, LogLevel::GameError, "Attempting to divide INT_MIN by -1!");
            self.cpu.gprs[0] = i32::MIN as u32;
            self.cpu.gprs[1] = 0;
            self.cpu.gprs[3] = i32::MIN as u32;
        } else {
            let quot = num / denom;
            let rem = num % denom;
            self.cpu.gprs[0] = quot as u32;
            self.cpu.gprs[1] = rem as u32;
            self.cpu.gprs[3] = quot.unsigned_abs();
        }
        let clz = |x: i32| if x == 0 { 32 } else { (x as u32).leading_zeros() as i32 };
        let mut loops = clz(denom) - clz(num);
        if loops < 1 {
            loops = 1;
        }
        self.bios_stall = (4 /* prologue */ + 13 * loops + 7/* epilogue */) as u32;
    }

    fn push(&mut self, regs: u32) {
        let sp = self.cpu.gprs[ARM_SP];
        self.cpu.gprs[ARM_SP] = self.mem_store_multiple(sp, regs, LSM_DB, None);
    }

    fn dest_ok(&mut self, what: &str) -> bool {
        match self.cpu.gprs[1] >> BASE_OFFSET {
            REGION_EWRAM | REGION_IWRAM | REGION_VRAM => {}
            _ => {
                glog!(self, LogLevel::GameError, "Bad {} destination", what);
            }
        }
        true
    }

    /// `GBASwi16`
    pub(crate) fn bios_swi16(&mut self, immediate: u32) {
        glog!(
            self,
            LogLevel::Debug,
            "SWI: {:02X} r0: {:08X} r1: {:08X} r2: {:08X} r3: {:08X}",
            immediate,
            self.cpu.gprs[0],
            self.cpu.gprs[1],
            self.cpu.gprs[2],
            self.cpu.gprs[3]
        );

        match immediate {
            0xF0 => {
                // Used for internal stall counting
                self.cpu.gprs[11] = self.bios_stall;
                return;
            }
            0xFA => {
                self.agb_print_flush();
                return;
            }
            _ => {}
        }

        if self.memory.full_bios {
            arm::raise_swi(self);
            return;
        }

        let mut use_stall = false;
        match immediate {
            SWI_SOFT_RESET => arm::raise_swi(self),
            SWI_REGISTER_RAM_RESET => self.register_ram_reset(),
            SWI_HALT => {
                arm::raise_swi(self);
                return;
            }
            SWI_STOP => self.stop(),
            SWI_VBLANK_INTR_WAIT | SWI_INTR_WAIT => {
                arm::raise_swi(self);
                return;
            }
            SWI_DIV => {
                use_stall = true;
                let (n, d) = (self.cpu.gprs[0] as i32, self.cpu.gprs[1] as i32);
                self.div(n, d);
            }
            SWI_DIV_ARM => {
                use_stall = true;
                let (n, d) = (self.cpu.gprs[1] as i32, self.cpu.gprs[0] as i32);
                self.div(n, d);
            }
            SWI_SQRT => {
                use_stall = true;
                let mut stall = 0;
                self.cpu.gprs[0] = sqrt(self.cpu.gprs[0], &mut stall);
                self.bios_stall = stall;
            }
            SWI_ARCTAN => {
                use_stall = true;
                let mut r1 = self.cpu.gprs[1] as i32;
                let mut r3 = self.cpu.gprs[3] as i32;
                let mut stall = 0;
                let r = arctan(self.cpu.gprs[0] as i32, Some(&mut r1), Some(&mut r3), &mut stall);
                self.cpu.gprs[0] = r as i32 as u32;
                self.cpu.gprs[1] = r1 as u32;
                self.cpu.gprs[3] = r3 as u32;
                self.bios_stall = stall;
            }
            SWI_ARCTAN2 => {
                use_stall = true;
                let mut r1 = self.cpu.gprs[1] as i32;
                let mut stall = 0;
                let r = arctan2(self.cpu.gprs[0] as i32, self.cpu.gprs[1] as i32, &mut r1, &mut stall);
                self.cpu.gprs[0] = r as u16 as u32;
                self.cpu.gprs[1] = r1 as u32;
                self.cpu.gprs[3] = 0x170;
                self.bios_stall = stall;
            }
            SWI_CPU_SET | SWI_CPU_FAST_SET => {
                if self.cpu.gprs[0] >> BASE_OFFSET < REGION_EWRAM {
                    glog!(self, LogLevel::GameError, "Cannot CpuSet from BIOS");
                } else {
                    let align = if self.cpu.gprs[2] & (1 << 26) != 0 { 3 } else { 1 };
                    if self.cpu.gprs[0] & align != 0 {
                        glog!(self, LogLevel::GameError, "Misaligned CpuSet source");
                    }
                    if self.cpu.gprs[1] & align != 0 {
                        glog!(self, LogLevel::GameError, "Misaligned CpuSet destination");
                    }
                    arm::raise_swi(self);
                    return;
                }
            }
            SWI_GET_BIOS_CHECKSUM => {
                self.cpu.gprs[0] = GBA_BIOS_CHECKSUM;
                self.cpu.gprs[1] = 1;
                self.cpu.gprs[3] = SIZE_BIOS;
            }
            SWI_BG_AFFINE_SET => self.bg_affine_set(),
            SWI_OBJ_AFFINE_SET => self.obj_affine_set(),
            SWI_BIT_UNPACK => {
                if self.cpu.gprs[0] < super::memory::BASE_EWRAM {
                    glog!(self, LogLevel::GameError, "Bad BitUnPack source");
                } else if self.dest_ok("BitUnPack") {
                    self.un_bit_pack();
                }
            }
            SWI_LZ77_UNCOMP_WRAM | SWI_LZ77_UNCOMP_VRAM => {
                if self.cpu.gprs[0] & 0x0E000000 == 0 {
                    glog!(self, LogLevel::GameError, "Bad LZ77 source");
                } else if self.dest_ok("LZ77") {
                    use_stall = true;
                    self.un_lz77(if immediate == SWI_LZ77_UNCOMP_WRAM { 1 } else { 2 });
                }
            }
            SWI_HUFFMAN_UNCOMP => {
                if self.cpu.gprs[0] & 0x0E000000 == 0 {
                    glog!(self, LogLevel::GameError, "Bad Huffman source");
                } else if self.dest_ok("Huffman") {
                    self.un_huffman();
                }
            }
            SWI_RL_UNCOMP_WRAM | SWI_RL_UNCOMP_VRAM => {
                if self.cpu.gprs[0] & 0x0E000000 == 0 {
                    glog!(self, LogLevel::GameError, "Bad RL source");
                } else if self.dest_ok("RL") {
                    self.un_rl(if immediate == SWI_RL_UNCOMP_WRAM { 1 } else { 2 });
                }
            }
            SWI_DIFF_8BIT_UNFILTER_WRAM | SWI_DIFF_8BIT_UNFILTER_VRAM | SWI_DIFF_16BIT_UNFILTER => {
                if self.cpu.gprs[0] & 0x0E000000 == 0 {
                    glog!(self, LogLevel::GameError, "Bad UnFilter source");
                } else if self.dest_ok("UnFilter") {
                    let inwidth = if immediate == SWI_DIFF_16BIT_UNFILTER { 2 } else { 1 };
                    let outwidth = if immediate == SWI_DIFF_8BIT_UNFILTER_WRAM { 1 } else { 2 };
                    self.un_filter(inwidth, outwidth);
                }
            }
            SWI_SOUND_BIAS => {
                // SoundBias is mostly meaningless here
                glog!(self, LogLevel::Stub, "Stub software interrupt: SoundBias (19)");
            }
            SWI_MIDI_KEY_2_FREQ => self.midi_key_2_freq(),
            SWI_SOUND_DRIVER_GET_JUMP_LIST => {
                arm::raise_swi(self);
                return;
            }
            SWI_BUG_JUMP_TO_EWRAM => {
                // Simulate the GBA BIOS SWI entry
                arm::raise_swi(self);
                // push {r11, r12, lr}
                self.push((1 << 11) | (1 << 12) | (1 << ARM_LR));
                // Save the SPSR to stack
                self.cpu.gprs[11] = self.cpu.spsr.0;
                self.push(1 << 11);
                // Switch from SVC to SYSTEM mode, keeping the IRQ dis/enabled as before
                self.cpu.gprs[11] &= 0x80;
                self.cpu.gprs[11] |= MODE_SYSTEM;
                self.cpu.cpsr.0 = self.cpu.gprs[11];
                arm::read_cpsr(self);
                // Save SYSTEM mode `lr` to stack (and r2 for some reason)
                self.push((1 << 2) | (1 << ARM_LR));
                // Set LR to hle-bios.s:54 / swi_complete
                self.cpu.gprs[ARM_LR] = 0xA4;
                // The address in EWRAM this will jump to
                self.cpu.gprs[12] = 0x02002040;
                self.cpu.gprs[ARM_PC] = 0x02002040;
                arm::arm_write_pc(self);
                return;
            }
            _ => {
                glog!(self, LogLevel::Stub, "Stub software interrupt: {:02X}", immediate);
            }
        }
        if use_stall {
            if self.bios_stall >= 18 {
                self.bios_stall -= 18;
                self.cpu.cycles += (self.bios_stall & 3) as i32;
                self.bios_stall &= !3;
                arm::raise_swi(self);
            } else {
                self.cpu.cycles += self.bios_stall as i32;
                use_stall = false;
            }
        }
        if !use_stall {
            self.cpu.cycles += 45 + self.cpu.memory.active_nonseq_cycles16 /* 8 bit load for SWI # */;
            // Return cycles
            if self.cpu.execution_mode == ExecutionMode::Arm {
                self.cpu.cycles += self.cpu.memory.active_nonseq_cycles32 + self.cpu.memory.active_seq_cycles32;
            } else {
                self.cpu.cycles += self.cpu.memory.active_nonseq_cycles16 + self.cpu.memory.active_seq_cycles16;
            }
        }
        self.memory.bios_prefetch = 0xE3A02004;
    }

    fn un_lz77(&mut self, width: u32) {
        let mut source = self.cpu.gprs[0];
        let mut dest = self.cpu.gprs[1];
        let mut cycles: i32 = 20;
        let mut remaining = ((self.mem_load32(source, Some(&mut cycles)) & 0xFFFFFF00) >> 8) as i32;
        // We assume the signature byte (0x10) is correct
        let mut blockheader: u32 = 0;
        source = source.wrapping_add(4);
        let mut blocks_remaining = 0;
        let mut halfword: u32 = 0;
        while remaining > 0 {
            cycles += 14;
            if blocks_remaining != 0 {
                cycles += 18;
                if blockheader & 0x80 != 0 {
                    // Compressed
                    let block = self.mem_load8(source.wrapping_add(1), Some(&mut cycles))
                        | (self.mem_load8(source, Some(&mut cycles)) << 8);
                    source = source.wrapping_add(2);
                    let mut disp = dest.wrapping_sub(block & 0x0FFF).wrapping_sub(1);
                    let mut bytes = (block >> 12) + 3;
                    while bytes > 0 {
                        bytes -= 1;
                        cycles += 10;
                        if remaining != 0 {
                            remaining -= 1;
                        } else {
                            glog!(self, LogLevel::GameError, "Improperly compressed LZ77 data at {:08X}.", self.cpu.gprs[0]);
                            if self.vba_bug_compat {
                                break;
                            }
                        }
                        if width == 2 {
                            let mut byte = self.mem_load16(disp & !1, Some(&mut cycles)) as i16 as i32;
                            if dest & 1 != 0 {
                                byte >>= (disp & 1) * 8;
                                halfword |= (byte as u32) << 8;
                                self.mem_store16(dest ^ 1, halfword as u16, Some(&mut cycles));
                            } else {
                                byte >>= (disp & 1) * 8;
                                halfword = (byte & 0xFF) as u32;
                            }
                            cycles += 4;
                        } else {
                            let byte = self.mem_load8(disp, Some(&mut cycles));
                            self.mem_store8(dest, byte as u8, Some(&mut cycles));
                        }
                        disp = disp.wrapping_add(1);
                        dest = dest.wrapping_add(1);
                    }
                } else {
                    // Uncompressed
                    let byte = self.mem_load8(source, Some(&mut cycles));
                    source = source.wrapping_add(1);
                    if width == 2 {
                        if dest & 1 != 0 {
                            halfword |= byte << 8;
                            self.mem_store16(dest ^ 1, halfword as u16, Some(&mut cycles));
                        } else {
                            halfword = byte;
                        }
                    } else {
                        self.mem_store8(dest, byte as u8, Some(&mut cycles));
                    }
                    dest = dest.wrapping_add(1);
                    remaining -= 1;
                }
                blockheader <<= 1;
                blocks_remaining -= 1;
            } else {
                blockheader = self.mem_load8(source, Some(&mut cycles));
                source = source.wrapping_add(1);
                blocks_remaining = 8;
            }
        }
        self.cpu.gprs[0] = source;
        self.cpu.gprs[1] = dest;
        self.cpu.gprs[3] = 0;
        self.bios_stall = cycles as u32;
    }

    fn un_huffman(&mut self) {
        let mut source = self.cpu.gprs[0] & 0xFFFFFFFC;
        let mut dest = self.cpu.gprs[1];
        let header = self.mem_load32(source, None);
        let mut remaining = (header >> 8) as i32;
        let mut bits = header & 0xF;
        if bits == 0 {
            glog!(self, LogLevel::GameError, "Invalid Huffman bits");
            bits = 8;
        }
        if 32 % bits != 0 || bits == 1 {
            glog!(self, LogLevel::Stub, "Unimplemented unaligned Huffman");
            return;
        }
        // We assume the signature byte (0x20) is correct
        let treesize = (self.mem_load8(source + 4, None) << 1) + 1;
        let mut block: u32 = 0;
        let tree_base = source + 5;
        source = source.wrapping_add(5 + treesize);
        let mut n_pointer = tree_base;
        let mut bits_seen = 0;
        let mut node = self.mem_load8(n_pointer, None);
        while remaining > 0 {
            let mut bitstream = self.mem_load32(source, None);
            source = source.wrapping_add(4);
            let mut bits_remaining = 32;
            while bits_remaining > 0 && remaining > 0 {
                let next = (n_pointer & !1).wrapping_add((node & 0x3F) * 2 + 2);
                let read_bits;
                if bitstream & 0x80000000 != 0 {
                    // Go right
                    if node & 0x40 != 0 {
                        read_bits = self.mem_load8(next + 1, None);
                    } else {
                        n_pointer = next + 1;
                        node = self.mem_load8(n_pointer, None);
                        bits_remaining -= 1;
                        bitstream <<= 1;
                        continue;
                    }
                } else {
                    // Go left
                    if node & 0x80 != 0 {
                        read_bits = self.mem_load8(next, None);
                    } else {
                        n_pointer = next;
                        node = self.mem_load8(n_pointer, None);
                        bits_remaining -= 1;
                        bitstream <<= 1;
                        continue;
                    }
                }

                block |= (read_bits & ((1 << bits) - 1)) << bits_seen;
                bits_seen += bits;
                n_pointer = tree_base;
                node = self.mem_load8(n_pointer, None);
                if bits_seen == 32 {
                    bits_seen = 0;
                    self.mem_store32(dest, block, None);
                    dest = dest.wrapping_add(4);
                    remaining -= 4;
                    block = 0;
                }
                bits_remaining -= 1;
                bitstream <<= 1;
            }
        }
        self.cpu.gprs[0] = source;
        self.cpu.gprs[1] = dest;
    }

    fn un_rl(&mut self, width: u32) {
        let mut source = self.cpu.gprs[0];
        let mut remaining = ((self.mem_load32(source & 0xFFFFFFFC, None) & 0xFFFFFF00) >> 8) as i32;
        let mut padding = (4 - remaining) & 0x3;
        // We assume the signature byte (0x30) is correct
        source = source.wrapping_add(4);
        let mut dest = self.cpu.gprs[1];
        let mut halfword: u32 = 0;
        while remaining > 0 {
            let mut blockheader = self.mem_load8(source, None) as i32;
            source = source.wrapping_add(1);
            if blockheader & 0x80 != 0 {
                // Compressed
                blockheader &= 0x7F;
                blockheader += 3;
                let block = self.mem_load8(source, None);
                source = source.wrapping_add(1);
                while blockheader > 0 && remaining > 0 {
                    blockheader -= 1;
                    remaining -= 1;
                    if width == 2 {
                        if dest & 1 != 0 {
                            halfword |= block << 8;
                            self.mem_store16(dest ^ 1, halfword as u16, None);
                        } else {
                            halfword = block;
                        }
                    } else {
                        self.mem_store8(dest, block as u8, None);
                    }
                    dest = dest.wrapping_add(1);
                }
            } else {
                // Uncompressed
                blockheader += 1;
                while blockheader > 0 && remaining > 0 {
                    blockheader -= 1;
                    remaining -= 1;
                    let byte = self.mem_load8(source, None);
                    source = source.wrapping_add(1);
                    if width == 2 {
                        if dest & 1 != 0 {
                            halfword |= byte << 8;
                            self.mem_store16(dest ^ 1, halfword as u16, None);
                        } else {
                            halfword = byte;
                        }
                    } else {
                        self.mem_store8(dest, byte as u8, None);
                    }
                    dest = dest.wrapping_add(1);
                }
            }
        }
        if width == 2 {
            if dest & 1 != 0 {
                padding -= 1;
                dest = dest.wrapping_add(1);
            }
            while padding > 0 {
                self.mem_store16(dest, 0, None);
                padding -= 2;
                dest = dest.wrapping_add(2);
            }
        } else {
            while padding > 0 {
                padding -= 1;
                self.mem_store8(dest, 0, None);
                dest = dest.wrapping_add(1);
            }
        }
        self.cpu.gprs[0] = source;
        self.cpu.gprs[1] = dest;
    }

    fn un_filter(&mut self, inwidth: u32, outwidth: u32) {
        let mut source = self.cpu.gprs[0] & 0xFFFFFFFC;
        let mut dest = self.cpu.gprs[1];
        let header = self.mem_load32(source, None);
        let mut remaining = (header >> 8) as i32;
        // We assume the signature nybble (0x8) is correct
        let mut halfword: u16 = 0;
        let mut old: u16 = 0;
        source = source.wrapping_add(4);
        while remaining > 0 {
            let mut new = if inwidth == 1 {
                self.mem_load8(source, None) as u16
            } else {
                self.mem_load16(source, None) as u16
            };
            new = new.wrapping_add(old);
            if outwidth > inwidth {
                halfword >>= 8;
                halfword |= new << 8;
                if source & 1 != 0 {
                    self.mem_store16(dest, halfword, None);
                    dest = dest.wrapping_add(outwidth);
                    remaining -= outwidth as i32;
                }
            } else if outwidth == 1 {
                self.mem_store8(dest, new as u8, None);
                dest = dest.wrapping_add(outwidth);
                remaining -= outwidth as i32;
            } else {
                self.mem_store16(dest, new, None);
                dest = dest.wrapping_add(outwidth);
                remaining -= outwidth as i32;
            }
            old = new;
            source = source.wrapping_add(inwidth);
        }
        self.cpu.gprs[0] = source;
        self.cpu.gprs[1] = dest;
    }

    fn un_bit_pack(&mut self) {
        let mut source = self.cpu.gprs[0];
        let mut dest = self.cpu.gprs[1];
        let info = self.cpu.gprs[2];
        let mut source_len = self.mem_load16(info, None) as i32;
        let source_width = self.mem_load8(info + 2, None);
        let dest_width = self.mem_load8(info + 3, None);
        if !matches!(source_width, 1 | 2 | 4 | 8) {
            glog!(self, LogLevel::GameError, "Bad BitUnPack source width: {}", source_width);
            return;
        }
        if !matches!(dest_width, 1 | 2 | 4 | 8 | 16 | 32) {
            glog!(self, LogLevel::GameError, "Bad BitUnPack destination width: {}", dest_width);
            return;
        }
        let bias = self.mem_load32(info + 4, None);
        let mut inb: u8 = 0;
        let mut out: u32 = 0;
        let mut bits_remaining: i32 = 0;
        let mut bits_eaten: u32 = 0;
        while source_len > 0 || bits_remaining != 0 {
            if bits_remaining == 0 {
                inb = self.mem_load8(source, None) as u8;
                bits_remaining = 8;
                source = source.wrapping_add(1);
                source_len -= 1;
            }
            let mut scaled = (inb as u32) & ((1u32 << source_width) - 1);
            inb = if source_width >= 8 { 0 } else { inb >> source_width };
            if scaled != 0 || bias & 0x80000000 != 0 {
                scaled = scaled.wrapping_add(bias & 0x7FFFFFFF);
            }
            bits_remaining -= source_width as i32;
            out |= scaled.wrapping_shl(bits_eaten);
            bits_eaten += dest_width;
            if bits_eaten == 32 {
                self.mem_store32(dest, out, None);
                bits_eaten = 0;
                out = 0;
                dest = dest.wrapping_add(4);
            }
        }
        self.cpu.gprs[0] = source;
        self.cpu.gprs[1] = dest;
    }
}

#[allow(dead_code)]
const _: () = {
    let _ = ARM_PC;
};
