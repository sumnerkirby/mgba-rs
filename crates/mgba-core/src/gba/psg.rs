// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's Game Boy sound channels as used by the GBA
// (src/gb/audio.c, GB_AUDIO_GBA style only).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::Gba;

const SQUARE_CHANNEL_DUTY: [[i32; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 0],
];

const NOISE_MASK_TABLE: [u16; 0x40] = [
    0x3f, 0x3e, 0x3c, 0x3d, 0x39, 0x38, 0x3a, 0x3b, 0x33, 0x32, 0x30, 0x31, 0x35, 0x34, 0x36, 0x37, 0x27, 0x26, 0x24,
    0x25, 0x21, 0x20, 0x22, 0x23, 0x2b, 0x2a, 0x28, 0x29, 0x2d, 0x2c, 0x2e, 0x2f, 0x0f, 0x0e, 0x0c, 0x0d, 0x09, 0x08,
    0x0a, 0x0b, 0x03, 0x02, 0x00, 0x01, 0x05, 0x04, 0x06, 0x07, 0x17, 0x16, 0x14, 0x15, 0x11, 0x10, 0x12, 0x13, 0x1b,
    0x1a, 0x18, 0x19, 0x1d, 0x1c, 0x1e, 0x1f,
];

const NOISE_POPULATION_TABLE: [u16; 0x40] = [
    6, 5, 4, 5, 4, 3, 4, 5, 4, 3, 2, 3, 4, 3, 4, 5, 4, 3, 2, 3, 2, 1, 2, 3, 4, 3, 2, 3, 4, 3, 4, 5, 4, 3, 2, 3, 2, 1,
    2, 3, 2, 1, 0, 1, 2, 1, 2, 3, 4, 3, 2, 3, 2, 1, 2, 3, 4, 3, 2, 3, 4, 3, 4, 5,
];

pub const DMG_SM83_FREQUENCY: i32 = 0x400000;
pub const FRAME_CYCLES: i32 = DMG_SM83_FREQUENCY >> 9;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct Envelope {
    pub length: i32,
    pub duty: i32,
    pub step_time: i32,
    pub initial_volume: i32,
    pub current_volume: i32,
    pub direction: bool,
    pub dead: i32,
    pub next_step: i32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct SquareControl {
    pub frequency: i32,
    pub length: i32,
    pub stop: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct Sweep {
    pub shift: i32,
    pub time: i32,
    pub step: i32,
    pub direction: bool,
    pub enable: bool,
    pub occurred: bool,
    pub real_frequency: i32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct SquareChannel {
    pub sweep: Sweep,
    pub envelope: Envelope,
    pub control: SquareControl,
    pub last_update: i32,
    pub index: u8,
    pub sample: i8,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct WaveChannel {
    pub size: bool,
    pub bank: bool,
    pub enable: bool,
    pub sample: i8,
    pub length: u32,
    pub volume: i32,
    pub rate: i32,
    pub stop: bool,
    pub window: i32,
    pub readable: bool,
    pub wavedata32: [u32; 8],
    pub next_update: i32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct NoiseChannel {
    pub envelope: Envelope,
    pub ratio: i32,
    pub frequency: i32,
    pub power: bool,
    pub stop: bool,
    pub length: i32,
    pub lfsr: u32,
    pub n_samples: i32,
    pub samples: i32,
    pub last_event: u32,
    pub sample: i8,
}

/// `struct GBAudio` (GBA PSG subset)
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Psg {
    pub timing_factor: i32,
    pub ch1: SquareChannel,
    pub ch2: SquareChannel,
    pub ch3: WaveChannel,
    pub ch4: NoiseChannel,

    pub volume_right: u8,
    pub volume_left: u8,
    pub ch1_right: bool,
    pub ch2_right: bool,
    pub ch3_right: bool,
    pub ch4_right: bool,
    pub ch1_left: bool,
    pub ch2_left: bool,
    pub ch3_left: bool,
    pub ch4_left: bool,

    pub playing_ch1: bool,
    pub playing_ch2: bool,
    pub playing_ch3: bool,
    pub playing_ch4: bool,
    /// Aliases the low byte of SOUNDCNT_X in mGBA.
    pub nr52: u8,

    pub frame: i32,
    pub skip_frame: bool,
    pub sample_interval: i32,
    pub enable: bool,
    pub force_disable_ch: [bool; 4],
}

impl Default for Psg {
    fn default() -> Self {
        Psg {
            timing_factor: 4,
            ch1: SquareChannel::default(),
            ch2: SquareChannel::default(),
            ch3: WaveChannel::default(),
            ch4: NoiseChannel::default(),
            volume_right: 0,
            volume_left: 0,
            ch1_right: false,
            ch2_right: false,
            ch3_right: false,
            ch4_right: false,
            ch1_left: false,
            ch2_left: false,
            ch3_left: false,
            ch4_left: false,
            playing_ch1: false,
            playing_ch2: false,
            playing_ch3: false,
            playing_ch4: false,
            nr52: 0,
            frame: 0,
            skip_frame: false,
            sample_interval: 0,
            enable: false,
            force_disable_ch: [false; 4],
        }
    }
}

fn reset_envelope(envelope: &mut Envelope) -> bool {
    envelope.current_volume = envelope.initial_volume;
    envelope.next_step = envelope.step_time;
    update_envelope_dead(envelope);
    envelope.initial_volume != 0 || envelope.direction
}

fn reset_sweep(sweep: &mut Sweep) {
    sweep.step = sweep.time;
    sweep.enable = (sweep.step != 8) || sweep.shift != 0;
    sweep.occurred = false;
}

fn write_sweep(sweep: &mut Sweep, value: u8) -> bool {
    sweep.shift = (value & 7) as i32;
    let old_direction = sweep.direction;
    sweep.direction = value & 8 != 0;
    let mut on = true;
    if sweep.occurred && old_direction && !sweep.direction {
        on = false;
    }
    sweep.occurred = false;
    sweep.time = ((value >> 4) & 7) as i32;
    if sweep.time == 0 {
        sweep.time = 8;
    }
    on
}

fn write_duty(envelope: &mut Envelope, value: u8) {
    envelope.length = (value & 0x3F) as i32;
    envelope.duty = ((value >> 6) & 3) as i32;
}

fn write_envelope(envelope: &mut Envelope, value: u8) -> bool {
    envelope.step_time = (value & 7) as i32;
    envelope.direction = value & 8 != 0;
    envelope.initial_volume = ((value >> 4) & 0xF) as i32;
    if envelope.step_time == 0 {
        envelope.current_volume &= 0xF;
    }
    update_envelope_dead(envelope);
    envelope.initial_volume != 0 || envelope.direction
}

fn update_square_sample(ch: &mut SquareChannel) {
    ch.sample = (SQUARE_CHANNEL_DUTY[ch.envelope.duty as usize][ch.index as usize] * ch.envelope.current_volume) as i8;
}

fn update_envelope(envelope: &mut Envelope) {
    if envelope.direction {
        envelope.current_volume += 1;
    } else {
        envelope.current_volume -= 1;
    }
    if envelope.current_volume >= 15 {
        envelope.current_volume = 15;
        envelope.dead = 1;
    } else if envelope.current_volume <= 0 {
        envelope.current_volume = 0;
        envelope.dead = 2;
    } else {
        envelope.next_step = envelope.step_time;
    }
}

fn update_envelope_dead(envelope: &mut Envelope) {
    if envelope.step_time == 0 {
        envelope.dead = if envelope.current_volume != 0 { 1 } else { 2 };
    } else if !envelope.direction && envelope.current_volume == 0 {
        envelope.dead = 2;
    } else if envelope.direction && envelope.current_volume == 0xF {
        envelope.dead = 1;
    } else if envelope.dead != 0 {
        // GB_AUDIO_GBA style reloads the step
        envelope.next_step = envelope.step_time;
        envelope.dead = 0;
    }
}

fn update_sweep(ch: &mut SquareChannel, initial: bool) -> bool {
    if initial || ch.sweep.time != 8 {
        let mut frequency = ch.sweep.real_frequency;
        if ch.sweep.direction {
            frequency -= frequency >> ch.sweep.shift;
            if !initial && frequency >= 0 {
                ch.control.frequency = frequency;
                ch.sweep.real_frequency = frequency;
            }
        } else {
            frequency += frequency >> ch.sweep.shift;
            if frequency < 2048 {
                if !initial && ch.sweep.shift != 0 {
                    ch.control.frequency = frequency;
                    ch.sweep.real_frequency = frequency;
                    if !update_sweep(ch, true) {
                        return false;
                    }
                }
            } else {
                return false;
            }
        }
        ch.sweep.occurred = true;
    }
    ch.sweep.step = ch.sweep.time;
    true
}

fn ch3_volume_shift(volume: i32) -> i32 {
    match volume {
        0 => 4,
        1 => 0,
        2 => 1,
        _ => 2,
    }
}

impl Psg {
    /// `GBAudioReset`
    pub fn reset(&mut self) {
        self.ch1 = SquareChannel::default();
        self.ch1.sweep.time = 8;
        self.ch1.envelope.dead = 2;
        self.ch2 = SquareChannel::default();
        self.ch2.envelope.dead = 2;
        self.ch3 = WaveChannel::default();
        self.ch4 = NoiseChannel::default();
        self.ch4.envelope.dead = 2;
        self.frame = 0;
        self.playing_ch1 = false;
        self.playing_ch2 = false;
        self.playing_ch3 = false;
        self.playing_ch4 = false;
    }

    pub fn write_nr10(&mut self, now: i32, value: u8) {
        self.run(now, 0x1);
        if !write_sweep(&mut self.ch1.sweep, value) {
            self.playing_ch1 = false;
            self.nr52 &= !0x01;
        }
    }

    pub fn write_nr11(&mut self, now: i32, value: u8) {
        self.run(now, 0x1);
        write_duty(&mut self.ch1.envelope, value);
        self.ch1.control.length = 64 - self.ch1.envelope.length;
    }

    pub fn write_nr12(&mut self, now: i32, value: u8) {
        self.run(now, 0x1);
        if !write_envelope(&mut self.ch1.envelope, value) {
            self.playing_ch1 = false;
            self.nr52 &= !0x01;
        }
    }

    pub fn write_nr13(&mut self, now: i32, value: u8) {
        self.run(now, 0x1);
        self.ch1.control.frequency &= 0x700;
        self.ch1.control.frequency |= value as i32;
    }

    pub fn write_nr14(&mut self, now: i32, value: u8) {
        self.run(now, 0x1);
        self.ch1.control.frequency &= 0xFF;
        self.ch1.control.frequency |= ((value as i32) << 8) & 0x700;
        let was_stop = self.ch1.control.stop;
        self.ch1.control.stop = value & 0x40 != 0;
        if !was_stop && self.ch1.control.stop && self.ch1.control.length != 0 && self.frame & 1 == 0 {
            self.ch1.control.length -= 1;
            if self.ch1.control.length == 0 {
                self.playing_ch1 = false;
            }
        }
        if value & 0x80 != 0 {
            self.playing_ch1 = reset_envelope(&mut self.ch1.envelope);
            self.ch1.sweep.real_frequency = self.ch1.control.frequency;
            reset_sweep(&mut self.ch1.sweep);
            if self.playing_ch1 && self.ch1.sweep.shift != 0 {
                self.playing_ch1 = update_sweep(&mut self.ch1, true);
            }
            if self.ch1.control.length == 0 {
                self.ch1.control.length = 64;
                if self.ch1.control.stop && self.frame & 1 == 0 {
                    self.ch1.control.length -= 1;
                }
            }
            update_square_sample(&mut self.ch1);
        }
        self.nr52 &= !0x01;
        self.nr52 |= self.playing_ch1 as u8;
    }

    pub fn write_nr21(&mut self, now: i32, value: u8) {
        self.run(now, 0x2);
        write_duty(&mut self.ch2.envelope, value);
        self.ch2.control.length = 64 - self.ch2.envelope.length;
    }

    pub fn write_nr22(&mut self, now: i32, value: u8) {
        self.run(now, 0x2);
        if !write_envelope(&mut self.ch2.envelope, value) {
            self.playing_ch2 = false;
            self.nr52 &= !0x02;
        }
    }

    pub fn write_nr23(&mut self, now: i32, value: u8) {
        self.run(now, 0x2);
        self.ch2.control.frequency &= 0x700;
        self.ch2.control.frequency |= value as i32;
    }

    pub fn write_nr24(&mut self, now: i32, value: u8) {
        self.run(now, 0x2);
        self.ch2.control.frequency &= 0xFF;
        self.ch2.control.frequency |= ((value as i32) << 8) & 0x700;
        let was_stop = self.ch2.control.stop;
        self.ch2.control.stop = value & 0x40 != 0;
        if !was_stop && self.ch2.control.stop && self.ch2.control.length != 0 && self.frame & 1 == 0 {
            self.ch2.control.length -= 1;
            if self.ch2.control.length == 0 {
                self.playing_ch2 = false;
            }
        }
        if value & 0x80 != 0 {
            self.playing_ch2 = reset_envelope(&mut self.ch2.envelope);
            if self.ch2.control.length == 0 {
                self.ch2.control.length = 64;
                if self.ch2.control.stop && self.frame & 1 == 0 {
                    self.ch2.control.length -= 1;
                }
            }
            update_square_sample(&mut self.ch2);
        }
        self.nr52 &= !0x02;
        self.nr52 |= (self.playing_ch2 as u8) << 1;
    }

    pub fn write_nr30(&mut self, now: i32, value: u8) {
        self.run(now, 0x4);
        self.ch3.enable = value & 0x80 != 0;
        if !self.ch3.enable {
            self.playing_ch3 = false;
            self.nr52 &= !0x04;
        }
    }

    pub fn write_nr31(&mut self, now: i32, value: u8) {
        self.run(now, 0x4);
        self.ch3.length = 256 - value as u32;
    }

    pub fn write_nr32(&mut self, now: i32, value: u8) {
        self.run(now, 0x4);
        self.ch3.volume = ((value >> 5) & 3) as i32;
        let byte = self.wavedata8(self.ch3.window >> 1);
        let mut sample = byte as i32;
        if self.ch3.window & 1 == 0 {
            sample >>= 4;
        }
        sample &= 0xF;
        self.ch3.sample = (sample >> ch3_volume_shift(self.ch3.volume)) as i8;
    }

    pub fn write_nr33(&mut self, now: i32, value: u8) {
        self.run(now, 0x4);
        self.ch3.rate &= 0x700;
        self.ch3.rate |= value as i32;
    }

    pub fn write_nr34(&mut self, now: i32, value: u8) {
        self.run(now, 0x4);
        self.ch3.rate &= 0xFF;
        self.ch3.rate |= ((value as i32) << 8) & 0x700;
        let was_stop = self.ch3.stop;
        self.ch3.stop = value & 0x40 != 0;
        if !was_stop && self.ch3.stop && self.ch3.length != 0 && self.frame & 1 == 0 {
            self.ch3.length -= 1;
            if self.ch3.length == 0 {
                self.playing_ch3 = false;
            }
        }
        if value & 0x80 != 0 {
            self.playing_ch3 = self.ch3.enable;
            if self.ch3.length == 0 {
                self.ch3.length = 256;
                if self.ch3.stop && self.frame & 1 == 0 {
                    self.ch3.length -= 1;
                }
            }
            self.ch3.window = 0;
        }
        if self.playing_ch3 {
            self.ch3.readable = true;
            // TODO: Where does this cycle delay come from?
            self.ch3.next_update = now.wrapping_add((6 + 2 * (2048 - self.ch3.rate)) * self.timing_factor);
        }
        self.nr52 &= !0x04;
        self.nr52 |= (self.playing_ch3 as u8) << 2;
    }

    pub fn write_nr41(&mut self, now: i32, value: u8) {
        self.run(now, 0x8);
        write_duty(&mut self.ch4.envelope, value);
        self.ch4.length = 64 - self.ch4.envelope.length;
    }

    pub fn write_nr42(&mut self, now: i32, value: u8) {
        self.run(now, 0x8);
        if !write_envelope(&mut self.ch4.envelope, value) {
            self.playing_ch4 = false;
            self.nr52 &= !0x08;
        }
    }

    pub fn write_nr43(&mut self, now: i32, value: u8) {
        self.run(now, 0x8);
        self.ch4.ratio = (value & 7) as i32;
        self.ch4.frequency = ((value >> 4) & 0xF) as i32;
        self.ch4.power = value & 8 != 0;
    }

    pub fn write_nr44(&mut self, now: i32, value: u8) {
        self.run(now, 0x8);
        let was_stop = self.ch4.stop;
        self.ch4.stop = value & 0x40 != 0;
        if !was_stop && self.ch4.stop && self.ch4.length != 0 && self.frame & 1 == 0 {
            self.ch4.length -= 1;
            if self.ch4.length == 0 {
                self.playing_ch4 = false;
            }
        }
        if value & 0x80 != 0 {
            self.playing_ch4 = reset_envelope(&mut self.ch4.envelope);
            self.ch4.lfsr = 0;
            if self.ch4.length == 0 {
                self.ch4.length = 64;
                if self.ch4.stop && self.frame & 1 == 0 {
                    self.ch4.length -= 1;
                }
            }
            if self.playing_ch4 {
                self.ch4.last_event = now as u32;
            }
        }
        self.nr52 &= !0x08;
        self.nr52 |= (self.playing_ch4 as u8) << 3;
    }

    pub fn write_nr50(&mut self, now: i32, value: u8) {
        self.run(now, 0xF);
        self.volume_right = value & 7;
        self.volume_left = (value >> 4) & 7;
    }

    pub fn write_nr51(&mut self, now: i32, value: u8) {
        self.run(now, 0xF);
        self.ch1_right = value & 0x01 != 0;
        self.ch2_right = value & 0x02 != 0;
        self.ch3_right = value & 0x04 != 0;
        self.ch4_right = value & 0x08 != 0;
        self.ch1_left = value & 0x10 != 0;
        self.ch2_left = value & 0x20 != 0;
        self.ch3_left = value & 0x40 != 0;
        self.ch4_left = value & 0x80 != 0;
    }

    pub fn write_nr52(&mut self, now: i32, value: u8) {
        let was_enable = self.enable;
        self.enable = value & 0x80 != 0;
        if !self.enable {
            self.playing_ch1 = false;
            self.playing_ch2 = false;
            self.playing_ch3 = false;
            self.playing_ch4 = false;
            self.write_nr10(now, 0);
            self.write_nr12(now, 0);
            self.write_nr13(now, 0);
            self.write_nr14(now, 0);
            self.write_nr22(now, 0);
            self.write_nr23(now, 0);
            self.write_nr24(now, 0);
            self.write_nr30(now, 0);
            self.write_nr32(now, 0);
            self.write_nr33(now, 0);
            self.write_nr34(now, 0);
            self.write_nr42(now, 0);
            self.write_nr43(now, 0);
            self.write_nr44(now, 0);
            self.write_nr50(now, 0);
            self.write_nr51(now, 0);
            self.write_nr11(now, 0);
            self.write_nr21(now, 0);
            self.write_nr31(now, 0);
            self.write_nr41(now, 0);
            self.nr52 &= !0x0F;
        } else if !was_enable {
            self.skip_frame = false;
            self.frame = 7;
        }
    }

    #[inline]
    pub fn wavedata8(&self, i: i32) -> u8 {
        let i = i as usize & 0xF;
        (self.ch3.wavedata32[i >> 2] >> ((i & 3) * 8)) as u8
    }

    /// `GBAudioRun`
    pub fn run(&mut self, timestamp: i32, channels: i32) {
        if !self.enable {
            return;
        }
        if (channels & 0x1) != 0
            && ((self.playing_ch1 && self.ch1.envelope.dead != 2)
                || timestamp.wrapping_sub(self.ch1.last_update) > 0x40000000
                || channels == 0x1)
        {
            let period = 4 * (2048 - self.ch1.control.frequency) * self.timing_factor;
            let mut diff = timestamp.wrapping_sub(self.ch1.last_update);
            if diff >= period {
                diff /= period;
                self.ch1.index = ((self.ch1.index as i32 + diff) & 7) as u8;
                self.ch1.last_update = self.ch1.last_update.wrapping_add(diff * period);
                update_square_sample(&mut self.ch1);
            }
        }
        if (channels & 0x2) != 0
            && ((self.playing_ch2 && self.ch2.envelope.dead != 2)
                || timestamp.wrapping_sub(self.ch2.last_update) > 0x40000000
                || channels == 0x2)
        {
            let period = 4 * (2048 - self.ch2.control.frequency) * self.timing_factor;
            let mut diff = timestamp.wrapping_sub(self.ch2.last_update);
            if diff >= period {
                diff /= period;
                self.ch2.index = ((self.ch2.index as i32 + diff) & 7) as u8;
                self.ch2.last_update = self.ch2.last_update.wrapping_add(diff * period);
                update_square_sample(&mut self.ch2);
            }
        }
        if self.playing_ch3 && (channels & 0x4) != 0 {
            let cycles = 2 * (2048 - self.ch3.rate) * self.timing_factor;
            let mut diff = timestamp.wrapping_sub(self.ch3.next_update);
            if diff >= 0 {
                diff = (diff / cycles) + 1;
                let volume = ch3_volume_shift(self.ch3.volume);
                let mut start = 7i32;
                let mut end = 0i32;
                let mut mask = 0x1F;
                if self.ch3.size {
                    mask = 0x3F;
                } else if self.ch3.bank {
                    end = 4;
                } else {
                    start = 3;
                }
                let w = &mut self.ch3.wavedata32;
                for _ in 0..(diff & mask) {
                    let mut bits_carry = w[end as usize] & 0x000000F0;
                    let mut i = start;
                    while i >= end {
                        let bits = w[i as usize] & 0x000000F0;
                        w[i as usize] = ((w[i as usize] & 0x0F0F0F0F) << 4) | ((w[i as usize] & 0xF0F0F000) >> 12);
                        w[i as usize] |= bits_carry << 20;
                        bits_carry = bits;
                        i -= 1;
                    }
                    self.ch3.sample = (bits_carry >> 4) as i8;
                }
                let mut sample = self.ch3.sample as i32;
                if self.ch3.volume > 3 {
                    sample += sample << 1;
                }
                self.ch3.sample = (sample >> volume) as i8;
                self.ch3.next_update = self.ch3.next_update.wrapping_add(diff * cycles);
                self.ch3.readable = true;
            }
        }
        if self.playing_ch4 && (channels & 0x8) != 0 {
            let mut cycles: i32 = if self.ch4.ratio != 0 { 2 * self.ch4.ratio } else { 1 };
            cycles <<= self.ch4.frequency;
            cycles *= 8 * self.timing_factor;

            let diff = timestamp.wrapping_sub(self.ch4.last_event as i32);
            if diff >= cycles {
                let mut last: i32 = 0;
                let mut samples = 0;
                let mut positive_samples = 0;
                let mut lsb: u32 = 0;
                let coeff: u32;
                if self.ch4.power {
                    coeff = 0x4040;
                } else {
                    let mut bits = 0usize;
                    // Batch 5 steps at a time when possible
                    while last + cycles * 5 <= diff {
                        bits = (self.ch4.lfsr & 0x3F) as usize;
                        self.ch4.lfsr >>= 5;
                        self.ch4.lfsr |= 0x4000 * NOISE_MASK_TABLE[bits] as u32 >> 4;
                        self.ch4.lfsr &= 0x7FFF;
                        samples += 5;
                        positive_samples += NOISE_POPULATION_TABLE[bits] as i32;
                        last += cycles * 5;
                    }
                    lsb = NOISE_MASK_TABLE[bits] as u32 & 1;
                    coeff = 0x4000;
                }
                while last + cycles <= diff {
                    lsb = (self.ch4.lfsr ^ (self.ch4.lfsr >> 1) ^ 1) & 1;
                    self.ch4.lfsr >>= 1;
                    if lsb != 0 {
                        self.ch4.lfsr |= coeff;
                    } else {
                        self.ch4.lfsr &= !coeff;
                    }
                    samples += 1;
                    positive_samples += lsb as i32;
                    last += cycles;
                }
                self.ch4.sample = (lsb as i32 * self.ch4.envelope.current_volume) as i8;
                self.ch4.n_samples += samples;
                self.ch4.samples += positive_samples * self.ch4.envelope.current_volume;
                self.ch4.last_event = self.ch4.last_event.wrapping_add(last as u32);
            }
        }
    }

    /// `GBAudioUpdateFrame`
    pub fn update_frame(&mut self, now: i32) {
        if !self.enable {
            return;
        }
        if self.skip_frame {
            self.skip_frame = false;
            return;
        }
        self.run(now, 0x7);

        let frame = (self.frame + 1) & 7;
        self.frame = frame;

        match frame {
            2 | 6 | 0 | 4 => {
                if frame == 2 || frame == 6 {
                    if self.ch1.sweep.enable {
                        self.ch1.sweep.step -= 1;
                        if self.ch1.sweep.step == 0 {
                            if !update_sweep(&mut self.ch1, false) {
                                self.playing_ch1 = false;
                            }
                            self.nr52 &= !0x01;
                            self.nr52 |= self.playing_ch1 as u8;
                        }
                    }
                }
                if self.ch1.control.length != 0 && self.ch1.control.stop {
                    self.ch1.control.length -= 1;
                    if self.ch1.control.length == 0 {
                        self.playing_ch1 = false;
                        self.nr52 &= !0x01;
                    }
                }
                if self.ch2.control.length != 0 && self.ch2.control.stop {
                    self.ch2.control.length -= 1;
                    if self.ch2.control.length == 0 {
                        self.playing_ch2 = false;
                        self.nr52 &= !0x02;
                    }
                }
                if self.ch3.length != 0 && self.ch3.stop {
                    self.ch3.length -= 1;
                    if self.ch3.length == 0 {
                        self.playing_ch3 = false;
                        self.nr52 &= !0x04;
                    }
                }
                if self.ch4.length != 0 && self.ch4.stop {
                    self.ch4.length -= 1;
                    if self.ch4.length == 0 {
                        self.playing_ch4 = false;
                        self.nr52 &= !0x08;
                    }
                }
            }
            7 => {
                if self.playing_ch1 && self.ch1.envelope.dead == 0 {
                    self.ch1.envelope.next_step -= 1;
                    if self.ch1.envelope.next_step == 0 {
                        update_envelope(&mut self.ch1.envelope);
                        update_square_sample(&mut self.ch1);
                    }
                }
                if self.playing_ch2 && self.ch2.envelope.dead == 0 {
                    self.ch2.envelope.next_step -= 1;
                    if self.ch2.envelope.next_step == 0 {
                        update_envelope(&mut self.ch2.envelope);
                        update_square_sample(&mut self.ch2);
                    }
                }
                if self.playing_ch4 && self.ch4.envelope.dead == 0 {
                    self.ch4.envelope.next_step -= 1;
                    if self.ch4.envelope.next_step == 0 {
                        let sample = self.ch4.sample as i32;
                        update_envelope(&mut self.ch4.envelope);
                        self.ch4.sample = ((sample > 0) as i32 * self.ch4.envelope.current_volume) as i8;
                        if self.ch4.n_samples != 0 {
                            self.ch4.samples -= sample;
                            self.ch4.samples += self.ch4.sample as i32;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// `GBAudioSamplePSG` (GBA style: no DC offset, noise not coalesced)
    pub fn sample(&self) -> (i16, i16) {
        let mut left: i32 = 0;
        let mut right: i32 = 0;

        if !self.force_disable_ch[0] {
            if self.ch1_left {
                left += self.ch1.sample as i32;
            }
            if self.ch1_right {
                right += self.ch1.sample as i32;
            }
        }
        if !self.force_disable_ch[1] {
            if self.ch2_left {
                left += self.ch2.sample as i32;
            }
            if self.ch2_right {
                right += self.ch2.sample as i32;
            }
        }
        if !self.force_disable_ch[2] {
            if self.ch3_left {
                left += self.ch3.sample as i32;
            }
            if self.ch3_right {
                right += self.ch3.sample as i32;
            }
        }

        left <<= 3;
        right <<= 3;

        if !self.force_disable_ch[3] {
            let sample = (self.ch4.sample as i32) << 3;
            if self.ch4_left {
                left += sample;
            }
            if self.ch4_right {
                right += sample;
            }
        }

        (
            (left * (1 + self.volume_left as i32)) as i16,
            (right * (1 + self.volume_right as i32)) as i16,
        )
    }
}

impl Gba {
    pub(crate) fn psg_write_nr11(&mut self, v: u8) {
        let now = self.current_time();
        self.audio.psg.write_nr11(now, v);
    }
    pub(crate) fn psg_write_nr12(&mut self, v: u8) {
        let now = self.current_time();
        self.audio.psg.write_nr12(now, v);
    }
    pub(crate) fn psg_write_nr13(&mut self, v: u8) {
        let now = self.current_time();
        self.audio.psg.write_nr13(now, v);
    }
    pub(crate) fn psg_write_nr14(&mut self, v: u8) {
        let now = self.current_time();
        self.audio.psg.write_nr14(now, v);
    }
    pub(crate) fn psg_write_nr21(&mut self, v: u8) {
        let now = self.current_time();
        self.audio.psg.write_nr21(now, v);
    }
    pub(crate) fn psg_write_nr22(&mut self, v: u8) {
        let now = self.current_time();
        self.audio.psg.write_nr22(now, v);
    }
    pub(crate) fn psg_write_nr23(&mut self, v: u8) {
        let now = self.current_time();
        self.audio.psg.write_nr23(now, v);
    }
    pub(crate) fn psg_write_nr24(&mut self, v: u8) {
        let now = self.current_time();
        self.audio.psg.write_nr24(now, v);
    }
    pub(crate) fn psg_write_nr31(&mut self, v: u8) {
        let now = self.current_time();
        self.audio.psg.write_nr31(now, v);
    }
    pub(crate) fn psg_write_nr33(&mut self, v: u8) {
        let now = self.current_time();
        self.audio.psg.write_nr33(now, v);
    }
    pub(crate) fn psg_write_nr34(&mut self, v: u8) {
        let now = self.current_time();
        self.audio.psg.write_nr34(now, v);
    }
    pub(crate) fn psg_write_nr41(&mut self, v: u8) {
        let now = self.current_time();
        self.audio.psg.write_nr41(now, v);
    }
    pub(crate) fn psg_write_nr42(&mut self, v: u8) {
        let now = self.current_time();
        self.audio.psg.write_nr42(now, v);
    }
    pub(crate) fn psg_write_nr43(&mut self, v: u8) {
        let now = self.current_time();
        self.audio.psg.write_nr43(now, v);
    }
    pub(crate) fn psg_write_nr44(&mut self, v: u8) {
        let now = self.current_time();
        self.audio.psg.write_nr44(now, v);
    }
}
