// Copyright (c) 2013-2015 Jeffrey Pfau
// Rust port of mGBA's GBA I/O register handling (src/gba/io.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::memory::{REGION_BIOS, SIZE_IO};
use super::regs::*;
use super::sio::{JOYSTAT_RECV, JOYSTAT_TRANS, RCNT_INITIAL};
use super::{glog, Gba, LogLevel};

#[inline(always)]
fn r(x: u32) -> usize {
    (x >> 1) as usize
}

/// `GBAIOIsReadConstant`
pub fn is_read_constant(address: u32) -> bool {
    matches!(
        address,
        BG0CNT
            | BG1CNT
            | BG2CNT
            | BG3CNT
            | WININ
            | WINOUT
            | BLDCNT
            | BLDALPHA
            | SOUND1CNT_LO
            | SOUND1CNT_HI
            | SOUND1CNT_X
            | SOUND2CNT_LO
            | SOUND2CNT_HI
            | SOUND3CNT_LO
            | SOUND3CNT_HI
            | SOUND3CNT_X
            | SOUND4CNT_LO
            | SOUND4CNT_HI
            | SOUNDCNT_LO
            | SOUNDCNT_HI
            | TM0CNT_HI
            | TM1CNT_HI
            | TM2CNT_HI
            | TM3CNT_HI
            | KEYINPUT
            | KEYCNT
            | IE
    )
}

impl Gba {
    /// `GBAIOInit`
    pub(crate) fn io_init(&mut self) {
        let io = &mut self.memory.io;
        io[r(DISPCNT)] = 0x0080;
        io[r(RCNT)] = RCNT_INITIAL;
        io[r(KEYINPUT)] = 0x3FF;
        io[r(SOUNDBIAS)] = 0x200;
        io[r(BG2PA)] = 0x100;
        io[r(BG2PD)] = 0x100;
        io[r(BG3PA)] = 0x100;
        io[r(BG3PD)] = 0x100;
        io[r(INTERNAL_EXWAITCNT_LO)] = 0x20;
        io[r(INTERNAL_EXWAITCNT_HI)] = 0xD00;

        if !self.memory.full_bios {
            io[r(VCOUNT)] = 0x7E;
            io[r(POSTFLG)] = 1;
        }
    }

    /// `GBAIOWrite`
    pub(crate) fn io_write(&mut self, mut address: u32, mut value: u16) {
        if address < SOUND1CNT_LO && (address > VCOUNT || address < DISPSTAT) {
            let v = self.renderer.write_video_register(address, value);
            self.memory.io[r(address)] = v;
            return;
        }

        if (SOUND1CNT_LO..=SOUNDCNT_LO).contains(&address) && !self.audio.enable {
            // Ignore writes to most audio registers if the hardware is off.
            return;
        }

        match address {
            // Video
            DISPSTAT => value = self.video_write_dispstat(value),
            VCOUNT => {
                glog!(self, LogLevel::GameError, "Write to read-only I/O register: {:03X}", address);
                return;
            }

            // Audio
            SOUND1CNT_LO => {
                self.audio_write_sound1cnt_lo(value);
                value &= 0x007F;
            }
            SOUND1CNT_HI => {
                self.audio_write_sound1cnt_hi(value);
                value &= 0xFFC0;
            }
            SOUND1CNT_X => {
                self.audio_write_sound1cnt_x(value);
                value &= 0x4000;
            }
            SOUND2CNT_LO => {
                self.audio_write_sound2cnt_lo(value);
                value &= 0xFFC0;
            }
            SOUND2CNT_HI => {
                self.audio_write_sound2cnt_hi(value);
                value &= 0x4000;
            }
            SOUND3CNT_LO => {
                self.audio_write_sound3cnt_lo(value);
                value &= 0x00E0;
            }
            SOUND3CNT_HI => {
                self.audio_write_sound3cnt_hi(value);
                value &= 0xE000;
            }
            SOUND3CNT_X => {
                self.audio_write_sound3cnt_x(value);
                value &= 0x4000;
            }
            SOUND4CNT_LO => {
                self.audio_write_sound4cnt_lo(value);
                value &= 0xFF00;
            }
            SOUND4CNT_HI => {
                self.audio_write_sound4cnt_hi(value);
                value &= 0x40FF;
            }
            SOUNDCNT_LO => {
                self.audio_write_soundcnt_lo(value);
                value &= 0xFF77;
            }
            SOUNDCNT_HI => {
                self.audio_write_soundcnt_hi(value);
                value &= 0x770F;
            }
            SOUNDCNT_X => {
                self.audio_write_soundcnt_x(value);
                value &= 0x0080;
                value |= self.audio.psg.nr52 as u16 & 0xF;
            }
            SOUNDBIAS => {
                value &= 0xC3FE;
                self.audio_write_soundbias(value);
            }

            WAVE_RAM0_LO | WAVE_RAM1_LO | WAVE_RAM2_LO | WAVE_RAM3_LO => {
                let v = ((self.memory.io[r(address) + 1] as u32) << 16) | value as u32;
                self.io_write32(address, v);
            }
            WAVE_RAM0_HI | WAVE_RAM1_HI | WAVE_RAM2_HI | WAVE_RAM3_HI => {
                let v = self.memory.io[r(address) - 1] as u32 | ((value as u32) << 16);
                self.io_write32(address - 2, v);
            }

            FIFO_A_LO | FIFO_B_LO => {
                let v = ((self.memory.io[r(address) + 1] as u32) << 16) | value as u32;
                self.io_write32(address, v);
                return;
            }
            FIFO_A_HI | FIFO_B_HI => {
                let v = self.memory.io[r(address) - 1] as u32 | ((value as u32) << 16);
                self.io_write32(address - 2, v);
                return;
            }

            // DMA
            DMA0SAD_LO | DMA0DAD_LO | DMA1SAD_LO | DMA1DAD_LO | DMA2SAD_LO | DMA2DAD_LO | DMA3SAD_LO | DMA3DAD_LO => {
                let v = ((self.memory.io[r(address) + 1] as u32) << 16) | value as u32;
                self.io_write32(address, v);
            }
            DMA0SAD_HI | DMA0DAD_HI | DMA1SAD_HI | DMA1DAD_HI | DMA2SAD_HI | DMA2DAD_HI | DMA3SAD_HI | DMA3DAD_HI => {
                let v = self.memory.io[r(address) - 1] as u32 | ((value as u32) << 16);
                self.io_write32(address - 2, v);
            }
            DMA0CNT_LO | DMA1CNT_LO | DMA2CNT_LO | DMA3CNT_LO => {
                // Handled inside of DMA routines
            }
            DMA0CNT_HI => value = self.dma_write_cnt_hi(0, value),
            DMA1CNT_HI => value = self.dma_write_cnt_hi(1, value),
            DMA2CNT_HI => value = self.dma_write_cnt_hi(2, value),
            DMA3CNT_HI => value = self.dma_write_cnt_hi(3, value),

            // Timers
            TM0CNT_LO | TM1CNT_LO | TM2CNT_LO | TM3CNT_LO => {
                self.timer_write_tmcnt_lo(((address - TM0CNT_LO) >> 2) as usize, value);
                return;
            }
            TM0CNT_HI | TM1CNT_HI | TM2CNT_HI | TM3CNT_HI => {
                value &= 0x00C7;
                self.timer_write_tmcnt_hi(((address - TM0CNT_HI) >> 2) as usize, value);
            }

            // SIO
            SIOCNT => {
                value &= 0x7FFF;
                self.sio_write_siocnt(value);
            }
            RCNT => {
                value &= 0xC1FF;
                self.sio_write_rcnt(value);
            }
            JOY_TRANS_LO | JOY_TRANS_HI => {
                self.memory.io[r(JOYSTAT)] |= JOYSTAT_TRANS;
                value = self.sio_write_register(address, value);
            }
            SIODATA32_LO | SIODATA32_HI | SIOMLT_SEND | JOYCNT | JOYSTAT | JOY_RECV_LO | JOY_RECV_HI => {
                value = self.sio_write_register(address, value);
            }

            // Interrupts and misc
            KEYCNT => {
                value &= 0xC3FF;
                if self.keys_last < 0x400 {
                    self.keys_last &= self.memory.io[r(address)] | !value;
                }
                self.memory.io[r(address)] = value;
                self.test_keypad_irq();
                return;
            }
            WAITCNT => {
                value &= 0x5FFF;
                self.adjust_waitstates(value);
            }
            IE => {
                self.memory.io[r(IE)] = value;
                self.test_irq(1);
                return;
            }
            IF => {
                let v = self.memory.io[r(IF)] & !value;
                self.memory.io[r(IF)] = v;
                self.test_irq(1);
                return;
            }
            IME => {
                self.memory.io[r(IME)] = value & 1;
                self.test_irq(1);
                return;
            }
            MAX => {
                // Some bad interrupt libraries will write to this
            }
            POSTFLG => {
                if self.memory.active_region == REGION_BIOS as i32 {
                    if self.memory.io[r(address)] != 0 {
                        if value & 0x8000 != 0 {
                            self.stop();
                        } else {
                            self.halt();
                        }
                    }
                    value &= !0x8000;
                } else {
                    glog!(self, LogLevel::GameError, "Write to BIOS-only I/O register: {:03X}", address);
                    return;
                }
            }
            EXWAITCNT_HI => {
                // This register sits outside of the normal I/O block, so we need to stash it somewhere unused
                address = INTERNAL_EXWAITCNT_HI;
                value &= 0xFF00;
                self.adjust_ewram_waitstates(value);
            }
            DEBUG_ENABLE => {
                self.debug = value == 0xC0DE;
                return;
            }
            _ => {
                if address == DEBUG_FLAGS && self.debug {
                    self.gba_debug(value);
                    return;
                }
                if address >= DEBUG_STRING && address - DEBUG_STRING < 0x100 {
                    let off = (address - DEBUG_STRING) as usize;
                    self.debug_string[off] = value as u8;
                    if off + 1 < 0x100 {
                        self.debug_string[off + 1] = (value >> 8) as u8;
                    }
                    return;
                }
                glog!(self, LogLevel::Stub, "Stub I/O register write: {:03X}", address);
                if address >= MAX {
                    glog!(self, LogLevel::GameError, "Write to unused I/O register: {:03X}", address);
                    return;
                }
            }
        }
        if address == SOUNDCNT_X {
            // In mGBA, NR52 aliases the low byte of this register.
            self.audio.psg.nr52 = value as u8;
        }
        self.memory.io[r(address)] = value;
    }

    /// `GBAIOWrite8`
    pub(crate) fn io_write8(&mut self, address: u32, value: u8) {
        if address >= DEBUG_STRING && address - DEBUG_STRING < 0x100 {
            self.debug_string[(address - DEBUG_STRING) as usize] = value;
            return;
        }
        if address > SIZE_IO {
            return;
        }
        let now = self.current_time();
        match address {
            a if a == SOUND1CNT_HI => {
                self.audio_sample(now);
                self.psg_write_nr11(value);
                self.memory.io[r(SOUND1CNT_HI)] &= 0xFF00;
                self.memory.io[r(SOUND1CNT_HI)] |= value as u16 & 0xC0;
            }
            a if a == SOUND1CNT_HI + 1 => {
                self.audio_sample(now);
                self.psg_write_nr12(value);
                self.memory.io[r(SOUND1CNT_HI)] &= 0x00C0;
                self.memory.io[r(SOUND1CNT_HI)] |= (value as u16) << 8;
            }
            a if a == SOUND1CNT_X => {
                self.audio_sample(now);
                self.psg_write_nr13(value);
            }
            a if a == SOUND1CNT_X + 1 => {
                self.audio_sample(now);
                self.psg_write_nr14(value);
                self.memory.io[r(SOUND1CNT_X)] = ((value as u16) & 0x40) << 8;
            }
            a if a == SOUND2CNT_LO => {
                self.audio_sample(now);
                self.psg_write_nr21(value);
                self.memory.io[r(SOUND2CNT_LO)] &= 0xFF00;
                self.memory.io[r(SOUND2CNT_LO)] |= value as u16 & 0xC0;
            }
            a if a == SOUND2CNT_LO + 1 => {
                self.audio_sample(now);
                self.psg_write_nr22(value);
                self.memory.io[r(SOUND2CNT_LO)] &= 0x00C0;
                self.memory.io[r(SOUND2CNT_LO)] |= (value as u16) << 8;
            }
            a if a == SOUND2CNT_HI => {
                self.audio_sample(now);
                self.psg_write_nr23(value);
            }
            a if a == SOUND2CNT_HI + 1 => {
                self.audio_sample(now);
                self.psg_write_nr24(value);
                self.memory.io[r(SOUND2CNT_HI)] = ((value as u16) & 0x40) << 8;
            }
            a if a == SOUND3CNT_HI => {
                self.audio_sample(now);
                self.psg_write_nr31(value);
            }
            a if a == SOUND3CNT_HI + 1 => {
                self.audio_sample(now);
                self.audio.psg.ch3.volume = ((value >> 5) & 7) as i32;
                self.memory.io[r(SOUND3CNT_HI)] = ((value as u16) & 0xE0) << 8;
            }
            a if a == SOUND3CNT_X => {
                self.audio_sample(now);
                self.psg_write_nr33(value);
            }
            a if a == SOUND3CNT_X + 1 => {
                self.audio_sample(now);
                self.psg_write_nr34(value);
                self.memory.io[r(SOUND3CNT_X)] = ((value as u16) & 0x40) << 8;
            }
            a if a == SOUND4CNT_LO => {
                self.audio_sample(now);
                self.psg_write_nr41(value);
            }
            a if a == SOUND4CNT_LO + 1 => {
                self.audio_sample(now);
                self.psg_write_nr42(value);
                self.memory.io[r(SOUND4CNT_LO)] = (value as u16) << 8;
            }
            a if a == SOUND4CNT_HI => {
                self.audio_sample(now);
                self.psg_write_nr43(value);
                self.memory.io[r(SOUND4CNT_HI)] &= 0x4000;
                self.memory.io[r(SOUND4CNT_HI)] |= value as u16;
            }
            a if a == SOUND4CNT_HI + 1 => {
                self.audio_sample(now);
                self.psg_write_nr44(value);
                self.memory.io[r(SOUND4CNT_HI)] &= 0x00FF;
                self.memory.io[r(SOUND4CNT_HI)] |= ((value as u16) & 0x40) << 8;
            }
            _ => {
                let shift = 8 * (address & 1);
                let mut value16 = (value as u16) << shift;
                value16 |= self.memory.io[((address & (SIZE_IO - 1)) >> 1) as usize] & !(0xFFu16 << shift);
                self.io_write(address & 0xFFFFFFFE, value16);
            }
        }
    }

    /// `GBAIOWrite32`
    pub(crate) fn io_write32(&mut self, address: u32, mut value: u32) {
        match address {
            // Wave RAM can be written and read even if the audio hardware is disabled.
            WAVE_RAM0_LO => self.audio_write_wave_ram(0, value),
            WAVE_RAM1_LO => self.audio_write_wave_ram(1, value),
            WAVE_RAM2_LO => self.audio_write_wave_ram(2, value),
            WAVE_RAM3_LO => self.audio_write_wave_ram(3, value),
            FIFO_A_LO | FIFO_B_LO => value = self.audio_write_fifo(address, value),
            DMA0SAD_LO => value = self.dma_write_sad(0, value),
            DMA0DAD_LO => value = self.dma_write_dad(0, value),
            DMA1SAD_LO => value = self.dma_write_sad(1, value),
            DMA1DAD_LO => value = self.dma_write_dad(1, value),
            DMA2SAD_LO => value = self.dma_write_sad(2, value),
            DMA2DAD_LO => value = self.dma_write_dad(2, value),
            DMA3SAD_LO => value = self.dma_write_sad(3, value),
            DMA3DAD_LO => value = self.dma_write_dad(3, value),
            _ => {
                if address >= DEBUG_STRING && address - DEBUG_STRING < 0x100 {
                    let off = (address - DEBUG_STRING) as usize;
                    for (i, b) in value.to_le_bytes().iter().enumerate() {
                        if off + i < 0x100 {
                            self.debug_string[off + i] = *b;
                        }
                    }
                    return;
                }
                self.io_write(address, value as u16);
                self.io_write(address | 2, (value >> 16) as u16);
                return;
            }
        }
        self.memory.io[r(address)] = value as u16;
        self.memory.io[r(address) + 1] = (value >> 16) as u16;
    }

    /// `GBAIORead`
    pub(crate) fn io_read(&mut self, mut address: u32) -> u16 {
        if !is_read_constant(address) {
            // Most IO reads need to disable idle removal
            self.halt_pending = false;
        }

        match address {
            // Reading this takes two cycles (1N+1I), so let's remove them preemptively
            TM0CNT_LO => self.timer_update_register(0, 2),
            TM1CNT_LO => self.timer_update_register(1, 2),
            TM2CNT_LO => self.timer_update_register(2, 2),
            TM3CNT_LO => self.timer_update_register(3, 2),

            KEYINPUT => {
                let mut input = self.keys_active;
                if !self.allow_opposing_directions {
                    let rl = input & 0x030;
                    let ud = input & 0x0C0;
                    input &= 0x30F;
                    if rl != 0x030 {
                        input |= rl;
                    }
                    if ud != 0x0C0 {
                        input |= ud;
                    }
                }
                self.memory.io[r(address)] = 0x3FF ^ input;
            }
            SIOCNT => return self.sio.siocnt,
            RCNT => return self.sio.rcnt,

            BG0HOFS | BG0VOFS | BG1HOFS | BG1VOFS | BG2HOFS | BG2VOFS | BG3HOFS | BG3VOFS | BG2PA | BG2PB | BG2PC | BG2PD
            | BG2X_LO | BG2X_HI | BG2Y_LO | BG2Y_HI | BG3PA | BG3PB | BG3PC | BG3PD | BG3X_LO | BG3X_HI | BG3Y_LO
            | BG3Y_HI | WIN0H | WIN1H | WIN0V | WIN1V | MOSAIC | BLDY | FIFO_A_LO | FIFO_A_HI | FIFO_B_LO | FIFO_B_HI
            | DMA0SAD_LO | DMA0SAD_HI | DMA0DAD_LO | DMA0DAD_HI | DMA1SAD_LO | DMA1SAD_HI | DMA1DAD_LO | DMA1DAD_HI
            | DMA2SAD_LO | DMA2SAD_HI | DMA2DAD_LO | DMA2DAD_HI | DMA3SAD_LO | DMA3SAD_HI | DMA3DAD_LO | DMA3DAD_HI => {
                // Write-only register
                glog!(self, LogLevel::GameError, "Read from write-only I/O register: {:03X}", address);
                return self.load_bad() as u16;
            }

            DMA0CNT_LO | DMA1CNT_LO | DMA2CNT_LO | DMA3CNT_LO | MAX => {
                // Many, many things read from the DMA register
                return 0;
            }

            JOY_RECV_LO | JOY_RECV_HI => {
                self.memory.io[r(JOYSTAT)] &= !JOYSTAT_RECV;
            }

            WAVE_RAM0_LO => return self.audio_read_wave_ram(0) as u16,
            WAVE_RAM0_HI => return (self.audio_read_wave_ram(0) >> 16) as u16,
            WAVE_RAM1_LO => return self.audio_read_wave_ram(1) as u16,
            WAVE_RAM1_HI => return (self.audio_read_wave_ram(1) >> 16) as u16,
            WAVE_RAM2_LO => return self.audio_read_wave_ram(2) as u16,
            WAVE_RAM2_HI => return (self.audio_read_wave_ram(2) >> 16) as u16,
            WAVE_RAM3_LO => return self.audio_read_wave_ram(3) as u16,
            WAVE_RAM3_HI => return (self.audio_read_wave_ram(3) >> 16) as u16,

            SOUND1CNT_LO | SOUND1CNT_HI | SOUND1CNT_X | SOUND2CNT_LO | SOUND2CNT_HI | SOUND3CNT_LO | SOUND3CNT_HI
            | SOUND3CNT_X | SOUND4CNT_LO | SOUND4CNT_HI | SOUNDCNT_LO => {
                if self.audio.psg.nr52 & 0x80 == 0 {
                    return 0;
                }
            }
            SOUNDCNT_X => {
                return (self.memory.io[r(SOUNDCNT_X)] & 0xFF00) | self.audio.psg.nr52 as u16;
            }
            DISPCNT | STEREOCNT | DISPSTAT | VCOUNT | BG0CNT | BG1CNT | BG2CNT | BG3CNT | WININ | WINOUT | BLDCNT
            | BLDALPHA | SOUNDCNT_HI | SOUNDBIAS | DMA0CNT_HI | DMA1CNT_HI | DMA2CNT_HI | DMA3CNT_HI | TM0CNT_HI
            | TM1CNT_HI | TM2CNT_HI | TM3CNT_HI | KEYCNT | SIOMULTI0 | SIOMULTI1 | SIOMULTI2 | SIOMULTI3 | SIOMLT_SEND
            | JOYCNT | JOY_TRANS_LO | JOY_TRANS_HI | JOYSTAT | IE | IF | WAITCNT | IME | POSTFLG => {
                // Handled transparently by registers
            }
            0x066 | 0x06A | 0x06E | 0x076 | 0x07A | 0x07E | 0x086 | 0x08A | 0x136 | 0x142 | 0x15A | 0x206 | 0x302 => {
                glog!(self, LogLevel::GameError, "Read from unused I/O register: {:03X}", address);
                return 0;
            }
            // These registers sit outside of the normal I/O block, so we need to stash them somewhere unused
            EXWAITCNT_LO | EXWAITCNT_HI => {
                address = address - EXWAITCNT_LO + INTERNAL_EXWAITCNT_LO;
            }
            _ => {
                if address == DEBUG_ENABLE && self.debug {
                    return 0x1DEA;
                }
                glog!(self, LogLevel::GameError, "Read from unused I/O register: {:03X}", address);
                return self.load_bad() as u16;
            }
        }
        self.memory.io[r(address)]
    }
}
