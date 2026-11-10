// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's Game Boy I/O registers and serial port
// (src/gb/io.c, src/gb/sio.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::audio::STYLE_CGB;
use super::audio::STYLE_DMG;
use super::{model, Gb, GbEvent, IRQ_KEYPAD, IRQ_SIO};

pub const REG_JOYP: usize = 0x00;
pub const REG_SB: usize = 0x01;
pub const REG_SC: usize = 0x02;
pub const REG_DIV: usize = 0x04;
pub const REG_TIMA: usize = 0x05;
pub const REG_TMA: usize = 0x06;
pub const REG_TAC: usize = 0x07;
pub const REG_IF: usize = 0x0F;
pub const REG_NR10: usize = 0x10;
pub const REG_NR11: usize = 0x11;
pub const REG_NR12: usize = 0x12;
pub const REG_NR13: usize = 0x13;
pub const REG_NR14: usize = 0x14;
pub const REG_NR21: usize = 0x16;
pub const REG_NR22: usize = 0x17;
pub const REG_NR23: usize = 0x18;
pub const REG_NR24: usize = 0x19;
pub const REG_NR30: usize = 0x1A;
pub const REG_NR31: usize = 0x1B;
pub const REG_NR32: usize = 0x1C;
pub const REG_NR33: usize = 0x1D;
pub const REG_NR34: usize = 0x1E;
pub const REG_NR41: usize = 0x20;
pub const REG_NR42: usize = 0x21;
pub const REG_NR43: usize = 0x22;
pub const REG_NR44: usize = 0x23;
pub const REG_NR50: usize = 0x24;
pub const REG_NR51: usize = 0x25;
pub const REG_NR52: usize = 0x26;
pub const REG_WAVE_0: usize = 0x30;
pub const REG_WAVE_F: usize = 0x3F;
pub const REG_LCDC: usize = 0x40;
pub const REG_STAT: usize = 0x41;
pub const REG_SCY: usize = 0x42;
pub const REG_SCX: usize = 0x43;
pub const REG_LY: usize = 0x44;
pub const REG_LYC: usize = 0x45;
pub const REG_DMA: usize = 0x46;
pub const REG_BGP: usize = 0x47;
pub const REG_OBP0: usize = 0x48;
pub const REG_OBP1: usize = 0x49;
pub const REG_WY: usize = 0x4A;
pub const REG_WX: usize = 0x4B;
pub const REG_KEY0: usize = 0x4C;
pub const REG_KEY1: usize = 0x4D;
pub const REG_VBK: usize = 0x4F;
pub const REG_BANK: usize = 0x50;
pub const REG_HDMA1: usize = 0x51;
pub const REG_HDMA2: usize = 0x52;
pub const REG_HDMA3: usize = 0x53;
pub const REG_HDMA4: usize = 0x54;
pub const REG_HDMA5: usize = 0x55;
pub const REG_RP: usize = 0x56;
pub const REG_BCPS: usize = 0x68;
pub const REG_BCPD: usize = 0x69;
pub const REG_OCPS: usize = 0x6A;
pub const REG_OCPD: usize = 0x6B;
pub const REG_OPRI: usize = 0x6C;
pub const REG_SVBK: usize = 0x70;
pub const REG_PSWX: usize = 0x72;
pub const REG_PSWY: usize = 0x73;
pub const REG_PSW: usize = 0x74;
pub const REG_UNK75: usize = 0x75;
pub const REG_PCM12: usize = 0x76;
pub const REG_PCM34: usize = 0x77;
pub const REG_IE: usize = 0xFF;

/// `_registerMask`
const fn register_mask() -> [u8; 0x100] {
    let mut m = [0u8; 0x100];
    m[REG_SC] = 0x7E; // TODO: GBC differences
    m[REG_IF] = 0xE0;
    m[REG_TAC] = 0xF8;
    m[REG_NR10] = 0x80;
    m[REG_NR11] = 0x3F;
    m[REG_NR12] = 0x00;
    m[REG_NR13] = 0xFF;
    m[REG_NR14] = 0xBF;
    m[REG_NR21] = 0x3F;
    m[REG_NR22] = 0x00;
    m[REG_NR23] = 0xFF;
    m[REG_NR24] = 0xBF;
    m[REG_NR30] = 0x7F;
    m[REG_NR31] = 0xFF;
    m[REG_NR32] = 0x9F;
    m[REG_NR33] = 0xFF;
    m[REG_NR34] = 0xBF;
    m[REG_NR41] = 0xFF;
    m[REG_NR42] = 0x00;
    m[REG_NR43] = 0x00;
    m[REG_NR44] = 0xBF;
    m[REG_NR50] = 0x00;
    m[REG_NR51] = 0x00;
    m[REG_NR52] = 0x70;
    m[REG_STAT] = 0x80;
    m[REG_KEY1] = 0x7E;
    m[REG_VBK] = 0xFE;
    m[REG_OCPS] = 0x40;
    m[REG_BCPS] = 0x40;
    m[REG_OPRI] = 0xFE;
    m[REG_SVBK] = 0xF8;
    m[REG_UNK75] = 0x8F;
    m[REG_IE] = 0xE0;
    m
}
const REGISTER_MASK: [u8; 0x100] = register_mask();

pub const SIO_CYCLES_PER_TRANSFER: [i32; 2] = [512, 16];

/// `struct GBSIO` (no link cable driver)
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub struct GbSio {
    pub next_event: i32,
    pub period: i32,
    pub remaining_bits: i32,
    pub pending_sb: u8,
}

impl Default for GbSio {
    fn default() -> Self {
        GbSio { next_event: i32::MAX, period: 0, remaining_bits: 0, pending_sb: 0xFF }
    }
}

impl Gb {
    /// `_writeSGBBits`
    fn write_sgb_bits(&mut self, bits: i32) {
        if bits == 0 {
            self.sgb_bit = -1;
            self.sgb_packet = [0; 16];
        }
        if bits == self.current_sgb_bits {
            return;
        }
        if bits & 2 != 0 {
            if self.sgb_increment {
                self.sgb_increment = false;
                self.sgb_current_controller = (self.sgb_current_controller + 1) & self.sgb_controllers;
            }
        } else if self.current_sgb_bits & 2 != 0 {
            self.sgb_increment = !self.sgb_increment;
        }
        self.current_sgb_bits = bits;
        if self.sgb_bit == 128 && bits == 2 {
            let packet = self.sgb_packet;
            self.video_write_sgb_packet(&packet);
            self.sgb_bit += 1;
        }
        if self.sgb_bit >= 128 {
            return;
        }
        match bits {
            1 => {
                if self.sgb_bit < 0 {
                    return;
                }
                self.sgb_packet[(self.sgb_bit >> 3) as usize] |= 1 << (self.sgb_bit & 7);
            }
            3 => self.sgb_bit += 1,
            _ => {}
        }
    }

    /// `GBIOReset`
    pub(crate) fn io_reset(&mut self) {
        self.memory.io = [0; 0x80];

        self.io_write(REG_TIMA, 0);
        self.io_write(REG_TMA, 0);
        self.io_write(REG_TAC, 0);
        self.io_write(REG_IF, 1);
        self.io_write(REG_LCDC, 0x00);
        self.io_write(REG_SCY, 0x00);
        self.io_write(REG_SCX, 0x00);
        self.io_write(REG_LYC, 0x00);
        self.memory.io[REG_DMA] = 0xFF;
        self.io_write(REG_BGP, 0xFC);
        if self.model < model::CGB {
            self.io_write(REG_OBP0, 0xFF);
            self.io_write(REG_OBP1, 0xFF);
        }
        self.io_write(REG_WY, 0x00);
        self.io_write(REG_WX, 0x00);
        self.memory.io[REG_BANK] = 0xFF;
        if self.model & model::CGB != 0 {
            self.io_write(REG_KEY0, 0);
            self.io_write(REG_JOYP, 0xFF);
            self.io_write(REG_VBK, 0);
            self.io_write(REG_BCPS, 0x80);
            self.io_write(REG_OCPS, 0);
            self.io_write(REG_SVBK, 1);
            self.io_write(REG_HDMA1, 0xFF);
            self.io_write(REG_HDMA2, 0xFF);
            self.io_write(REG_HDMA3, 0xFF);
            self.io_write(REG_HDMA4, 0xFF);
            self.memory.io[REG_HDMA5] = 0xFF;
        } else {
            for r in REG_KEY0..=REG_PCM34 {
                self.memory.io[r] = 0xFF;
            }
        }

        if self.model & model::SGB != 0 {
            self.io_write(REG_JOYP, 0xFF);
        }
        self.io_write(REG_IE, 0x00);
    }

    /// `GBIOWrite`
    pub fn io_write(&mut self, address: usize, value: u8) {
        let mut value = value;
        let enable = self.audio.enable;
        let style = self.audio.style;
        let now = self.current_time();
        macro_rules! audio_reg {
            ($f:ident) => {{
                if enable {
                    self.with_audio(|a| a.$f(now, value));
                } else {
                    value = 0;
                }
            }};
        }
        match address {
            REG_SB => self.sio_write_sb(value),
            REG_SC => self.sio_write_sc(value),
            REG_DIV => {
                self.timer_div_reset();
                return;
            }
            REG_NR10 => audio_reg!(write_nr10),
            REG_NR11 => {
                if enable {
                    self.with_audio(|a| a.write_nr11(now, value));
                } else {
                    if style == STYLE_DMG {
                        self.with_audio(|a| a.write_nr11(now, value & REGISTER_MASK[REG_NR11]));
                    }
                    value = 0;
                }
            }
            REG_NR12 => audio_reg!(write_nr12),
            REG_NR13 => audio_reg!(write_nr13),
            REG_NR14 => audio_reg!(write_nr14),
            REG_NR21 => {
                if enable {
                    self.with_audio(|a| a.write_nr21(now, value));
                } else {
                    if style == STYLE_DMG {
                        self.with_audio(|a| a.write_nr21(now, value & REGISTER_MASK[REG_NR21]));
                    }
                    value = 0;
                }
            }
            REG_NR22 => audio_reg!(write_nr22),
            REG_NR23 => audio_reg!(write_nr23),
            REG_NR24 => audio_reg!(write_nr24),
            REG_NR30 => audio_reg!(write_nr30),
            REG_NR31 => {
                if enable || style == STYLE_DMG {
                    self.with_audio(|a| a.write_nr31(now, value));
                } else {
                    value = 0;
                }
            }
            REG_NR32 => audio_reg!(write_nr32),
            REG_NR33 => audio_reg!(write_nr33),
            REG_NR34 => audio_reg!(write_nr34),
            REG_NR41 => {
                if enable || style == STYLE_DMG {
                    self.with_audio(|a| a.write_nr41(now, value));
                } else {
                    value = 0;
                }
            }
            REG_NR42 => audio_reg!(write_nr42),
            REG_NR43 => audio_reg!(write_nr43),
            REG_NR44 => audio_reg!(write_nr44),
            REG_NR50 => audio_reg!(write_nr50),
            REG_NR51 => audio_reg!(write_nr51),
            REG_NR52 => {
                self.audio_write_nr52(value);
                value &= 0x80;
                value |= self.memory.io[REG_NR52] & 0x0F;
            }
            REG_WAVE_0..=REG_WAVE_F => {
                self.with_audio(|a| a.run(now, 0x4));
                if !self.audio.playing_ch3 {
                    self.audio.ch3.wavedata8[address - REG_WAVE_0] = value;
                } else if self.audio.ch3.readable || self.audio.style == STYLE_CGB {
                    let w = (self.audio.ch3.window >> 1) as usize;
                    self.audio.ch3.wavedata8[w] = value;
                }
            }
            REG_JOYP => {
                self.memory.io[REG_JOYP] = value | 0x0F;
                self.read_keys();
                if self.model & model::SGB != 0 {
                    self.write_sgb_bits(((value >> 4) & 3) as i32);
                }
                return;
            }
            REG_TIMA => {
                let ds = self.double_speed as i32;
                if value != 0 && self.until(GbEvent::TimerIrq) > 2 - ds {
                    self.deschedule(GbEvent::TimerIrq);
                }
                if self.until(GbEvent::TimerIrq) == ds - 2 {
                    return;
                }
            }
            REG_TMA => {
                if self.until(GbEvent::TimerIrq) == self.double_speed as i32 - 2 {
                    self.memory.io[REG_TIMA] = value;
                }
            }
            REG_TAC => value = self.timer_update_tac(value),
            REG_IF => {
                self.memory.io[REG_IF] = value | 0xE0;
                self.update_irqs();
                return;
            }
            REG_LCDC => {
                // TODO: handle GBC differences
                self.video_process_dots(0);
                value = self.renderer.write_video_register(address, value);
                self.video_write_lcdc(value);
            }
            REG_LYC => self.video_write_lyc(value),
            REG_DMA => self.memory_dma((value as u16) << 8),
            REG_SCY | REG_SCX | REG_WY | REG_WX => {
                self.video_process_dots(0);
                value = self.renderer.write_video_register(address, value);
            }
            REG_BGP | REG_OBP0 | REG_OBP1 => {
                self.video_process_dots(0);
                self.video_write_palette(address, value);
            }
            REG_STAT => {
                self.video_write_stat(value);
                value = self.video.stat;
            }
            REG_BANK => {
                if self.memory.io[REG_BANK] == 0xFF {
                    self.unmap_bios();
                    if self.model >= model::CGB && self.memory.io[REG_KEY0] < 0x80 {
                        self.model = model::DMG;
                        self.video_disable_cgb();
                    }
                }
            }
            REG_IE => {
                self.memory.ie = value;
                self.update_irqs();
                return;
            }
            _ => {
                if self.model >= model::CGB {
                    match address {
                        REG_KEY0 => {}
                        REG_KEY1 => {
                            value &= 0x1;
                            value |= self.memory.io[address] & 0x80;
                        }
                        REG_VBK => self.video.switch_bank(value),
                        REG_HDMA1 | REG_HDMA2 | REG_HDMA3 | REG_HDMA4 => {}
                        REG_HDMA5 => value = self.memory_write_hdma5(value),
                        REG_BCPS => {
                            self.video.bcp_index = (value & 0x3F) as i32;
                            self.video.bcp_increment = value & 0x80 != 0;
                            let i = self.video.bcp_index;
                            self.memory.io[REG_BCPD] = (self.video.palette[(i >> 1) as usize] >> (8 * (i & 1))) as u8;
                        }
                        REG_BCPD => {
                            self.video_write_palette(address, value);
                            return;
                        }
                        REG_OCPS => {
                            self.video.ocp_index = (value & 0x3F) as i32;
                            self.video.ocp_increment = value & 0x80 != 0;
                            let i = self.video.ocp_index;
                            self.memory.io[REG_OCPD] = (self.video.palette[8 * 4 + (i >> 1) as usize] >> (8 * (i & 1))) as u8;
                        }
                        REG_OCPD => {
                            self.video_write_palette(address, value);
                            return;
                        }
                        REG_SVBK => {
                            self.memory.switch_wram_bank(value as i32);
                            value &= 7;
                        }
                        _ => return,
                    }
                } else {
                    return;
                }
            }
        }
        if address < 0x80 {
            self.memory.io[address] = value;
        }
    }

    /// `_readKeys`
    pub(crate) fn read_keys(&mut self) -> u8 {
        let mut keys = self.keys as u8;
        if self.sgb_current_controller != 0 {
            keys = 0;
        }
        let joyp = self.memory.io[REG_JOYP];
        match joyp & 0x30 {
            0x30 => keys = self.sgb_current_controller as u8,
            0x20 => keys >>= 4,
            0x10 => {}
            _ => keys |= keys >> 4,
        }
        self.memory.io[REG_JOYP] = (0xCF | joyp) ^ (keys & 0xF);
        if joyp & !self.memory.io[REG_JOYP] & 0xF != 0 {
            self.memory.io[REG_IF] |= 1 << IRQ_KEYPAD;
            self.update_irqs();
        }
        self.memory.io[REG_JOYP]
    }

    /// `_readKeysFiltered`
    fn read_keys_filtered(&mut self) -> u8 {
        let mut keys = self.read_keys();
        if !self.allow_opposing_directions && (keys & 0x30) == 0x20 {
            let rl = keys & 0x03;
            let ud = keys & 0x0C;
            if rl == 0 {
                keys |= 0x03;
            }
            if ud == 0 {
                keys |= 0x0C;
            }
        }
        keys
    }

    /// `GBIORead`
    pub fn io_read(&mut self, address: usize) -> u8 {
        match address {
            REG_JOYP => return self.read_keys_filtered(),
            REG_IE => return self.memory.ie,
            REG_WAVE_0..=REG_WAVE_F => {
                if self.audio.playing_ch3 {
                    let now = self.current_time();
                    self.with_audio(|a| a.run(now, 0x4));
                    if self.audio.ch3.readable || self.audio.style == STYLE_CGB {
                        return self.audio.ch3.wavedata8[(self.audio.ch3.window >> 1) as usize];
                    } else {
                        return 0xFF;
                    }
                } else {
                    return self.audio.ch3.wavedata8[address - REG_WAVE_0];
                }
            }
            REG_PCM12 => {
                if self.model >= model::CGB && self.audio.enable {
                    let now = self.current_time();
                    self.with_audio(|a| a.run(now, 0x3));
                    return (self.audio.ch1.sample as u8) | ((self.audio.ch2.sample as u8) << 4);
                }
            }
            REG_PCM34 => {
                if self.model >= model::CGB && self.audio.enable {
                    let now = self.current_time();
                    self.with_audio(|a| a.run(now, 0xC));
                    return (self.audio.ch3.sample as u8) | ((self.audio.ch4.sample as u8) << 4);
                }
            }
            REG_SB | REG_SC | REG_IF | REG_NR10 | REG_NR11 | REG_NR12 | REG_NR14 | REG_NR21 | REG_NR22 | REG_NR24
            | REG_NR30 | REG_NR32 | REG_NR34 | REG_NR41 | REG_NR42 | REG_NR43 | REG_NR44 | REG_NR50 | REG_NR51
            | REG_NR52 | REG_DIV | REG_TIMA | REG_TMA | REG_TAC | REG_STAT | REG_LCDC | REG_SCY | REG_SCX | REG_LY
            | REG_LYC | REG_DMA | REG_BGP | REG_OBP0 | REG_OBP1 | REG_WY | REG_WX => {}
            REG_KEY1 | REG_VBK | REG_HDMA1 | REG_HDMA2 | REG_HDMA3 | REG_HDMA4 | REG_HDMA5 | REG_BCPS | REG_BCPD
            | REG_OCPS | REG_OCPD | REG_SVBK | REG_PSWX | REG_PSWY | REG_UNK75 => {}
            _ => return 0xFF,
        }
        self.memory.io[address] | REGISTER_MASK[address]
    }

    /// `GBTestKeypadIRQ`
    pub fn test_keypad_irq(&mut self) {
        self.read_keys();
    }

    // ----- sio.c -----

    /// `GBSIOReset`
    pub(crate) fn sio_reset(&mut self) {
        self.sio.next_event = i32::MAX;
        self.sio.remaining_bits = 0;
    }

    /// `_GBSIOProcessEvents`
    pub(crate) fn sio_process_events(&mut self, _cycles_late: u32) {
        let mut do_irq = false;
        if self.sio.remaining_bits != 0 {
            do_irq = true;
            self.sio.remaining_bits -= 1;
            let bit = 128u8 >> self.sio.remaining_bits;
            self.memory.io[REG_SB] &= !bit;
            self.memory.io[REG_SB] |= self.sio.pending_sb & bit;
        }
        if self.sio.remaining_bits == 0 {
            self.memory.io[REG_SC] &= !0x80;
            if do_irq {
                self.memory.io[REG_IF] |= 1 << IRQ_SIO;
                self.update_irqs();
                self.sio.pending_sb = 0xFF;
            }
        } else {
            let period = self.sio.period * (2 - self.double_speed as i32);
            self.schedule(GbEvent::Sio, period);
        }
    }

    /// `GBSIOWriteSB` (no driver attached)
    fn sio_write_sb(&mut self, _sb: u8) {}

    /// `GBSIOWriteSC`
    fn sio_write_sc(&mut self, sc: u8) {
        self.sio.period = SIO_CYCLES_PER_TRANSFER[((sc >> 1) & 1) as usize]; // TODO Shift Clock
        if sc & 0x80 != 0 {
            if sc & 1 != 0 {
                self.deschedule(GbEvent::Sio);
                let period = self.sio.period * (2 - self.double_speed as i32);
                self.schedule(GbEvent::Sio, period);
                self.sio.remaining_bits = 8;
            }
        } else {
            self.deschedule(GbEvent::Sio);
        }
    }
}
