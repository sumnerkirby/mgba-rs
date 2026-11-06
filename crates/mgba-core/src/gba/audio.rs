// Copyright (c) 2013-2015 Jeffrey Pfau
// Rust port of mGBA's GBA audio mixer and DMA sound FIFOs (src/gba/audio.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::VecDeque;

use super::dma::{DMA_FIXED, DMA_TIMING_CUSTOM};
use super::memory::BASE_IO;
use super::psg::{Psg, FRAME_CYCLES};
use super::{glog, regs, Gba, LogLevel, ARM7TDMI_FREQUENCY};
use crate::timing::EventId;

pub const FIFO_SIZE: usize = 8;
pub const MAX_SAMPLES: usize = 16;
pub const VOLUME_MAX: i32 = 0x100;
const SAMPLE_INTERVAL: i32 = (ARM7TDMI_FREQUENCY / 0x4000) as i32;
/// Cap on buffered output so a stalled frontend can't grow memory without bound.
const MAX_BUFFERED: usize = 0x10000;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct AudioFifo {
    pub fifo: [u32; FIFO_SIZE],
    pub fifo_write: usize,
    pub fifo_read: usize,
    pub internal_sample: u32,
    pub internal_remaining: i32,
    pub dma_source: i32,
    pub samples: [i8; MAX_SAMPLES],
}

/// `struct GBAAudio`
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Audio {
    pub psg: Psg,
    pub ch_a: AudioFifo,
    pub ch_b: AudioFifo,

    pub volume: u8,
    pub volume_ch_a: bool,
    pub volume_ch_b: bool,
    pub ch_a_right: bool,
    pub ch_a_left: bool,
    pub ch_a_timer: bool,
    pub ch_b_right: bool,
    pub ch_b_left: bool,
    pub ch_b_timer: bool,
    pub enable: bool,

    pub soundbias: u16,
    pub sample_interval: i32,
    pub last_sample: i32,
    pub sample_index: usize,
    pub current_samples: [[i16; 2]; MAX_SAMPLES],

    pub force_disable_ch_a: bool,
    pub force_disable_ch_b: bool,
    pub master_volume: i32,

    /// Output buffer (stereo frames), drained by the frontend.
    #[serde(skip)]
    pub buffer: VecDeque<[i16; 2]>,
}

impl Audio {
    pub fn new() -> Audio {
        Audio {
            psg: Psg::default(),
            ch_a: AudioFifo::default(),
            ch_b: AudioFifo::default(),
            volume: 0,
            volume_ch_a: false,
            volume_ch_b: false,
            ch_a_right: false,
            ch_a_left: false,
            ch_a_timer: false,
            ch_b_right: false,
            ch_b_left: false,
            ch_b_timer: false,
            enable: false,
            soundbias: 0x200,
            sample_interval: (ARM7TDMI_FREQUENCY / 0x8000) as i32,
            last_sample: 0,
            sample_index: 0,
            current_samples: [[0; 2]; MAX_SAMPLES],
            force_disable_ch_a: false,
            force_disable_ch_b: false,
            master_volume: VOLUME_MAX,
            buffer: VecDeque::with_capacity(8192),
        }
    }

    #[inline]
    fn resolution(&self) -> u32 {
        ((self.soundbias >> 14) & 3) as u32
    }

    #[inline]
    fn bias(&self) -> i32 {
        (self.soundbias & 0x3FF) as i32
    }

    /// `_applyBias`
    fn apply_bias(&self, sample: i32) -> i32 {
        let mut sample = sample + self.bias();
        if sample >= 0x400 {
            sample = 0x3FF;
        } else if sample < 0 {
            sample = 0;
        }
        ((sample - self.bias()) * self.master_volume * 3) >> 4
    }
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

impl Gba {
    /// `GBAAudioReset`
    pub(crate) fn audio_reset(&mut self) {
        self.audio.psg.reset();
        self.deschedule(EventId::AudioFrame);
        self.schedule(EventId::AudioFrame, 0);
        self.deschedule(EventId::AudioSample);
        self.schedule(EventId::AudioSample, 0);
        let a = &mut self.audio;
        a.ch_a = AudioFifo { dma_source: 1, ..Default::default() };
        a.ch_b = AudioFifo { dma_source: 2, ..Default::default() };
        a.soundbias = 0x200;
        a.volume = 0;
        a.volume_ch_a = false;
        a.volume_ch_b = false;
        a.last_sample = 0;
        a.sample_index = 0;
        a.ch_a_right = false;
        a.ch_a_left = false;
        a.ch_a_timer = false;
        a.ch_b_right = false;
        a.ch_b_left = false;
        a.ch_b_timer = false;
        a.enable = false;
        a.sample_interval = (ARM7TDMI_FREQUENCY / 0x8000) as i32;
        a.psg.sample_interval = a.sample_interval;
        a.buffer.clear();
    }

    /// `GBAAudioScheduleFifoDma`
    fn audio_schedule_fifo_dma(&mut self, number: usize) {
        let info = &mut self.memory.dma[number];
        info.set_dest_control(DMA_FIXED);
        info.set_width(1);
        info.dest_offset = 0;
        // The width was just forced to 32-bit, but sourceOffset was cached at CNT_HI
        // write time from the width the game programmed. Rescale it, keeping the
        // direction the source control selected.
        if info.source_offset > 0 {
            info.source_offset = 4;
        } else if info.source_offset < 0 {
            info.source_offset = -4;
        }
        match info.dest {
            d if d == BASE_IO | regs::FIFO_A_LO => self.audio.ch_a.dma_source = number as i32,
            d if d == BASE_IO | regs::FIFO_B_LO => self.audio.ch_b.dma_source = number as i32,
            d => {
                glog!(self, LogLevel::GameError, "Invalid FIFO destination: 0x{:08X}", d);
            }
        }
    }

    pub(crate) fn audio_write_sound1cnt_lo(&mut self, value: u16) {
        let now = self.current_time();
        self.audio_sample(now);
        self.audio.psg.write_nr10(now, value as u8);
    }

    pub(crate) fn audio_write_sound1cnt_hi(&mut self, value: u16) {
        let now = self.current_time();
        self.audio_sample(now);
        self.audio.psg.write_nr11(now, value as u8);
        self.audio.psg.write_nr12(now, (value >> 8) as u8);
    }

    pub(crate) fn audio_write_sound1cnt_x(&mut self, value: u16) {
        let now = self.current_time();
        self.audio_sample(now);
        self.audio.psg.write_nr13(now, value as u8);
        self.audio.psg.write_nr14(now, (value >> 8) as u8);
    }

    pub(crate) fn audio_write_sound2cnt_lo(&mut self, value: u16) {
        let now = self.current_time();
        self.audio_sample(now);
        self.audio.psg.write_nr21(now, value as u8);
        self.audio.psg.write_nr22(now, (value >> 8) as u8);
    }

    pub(crate) fn audio_write_sound2cnt_hi(&mut self, value: u16) {
        let now = self.current_time();
        self.audio_sample(now);
        self.audio.psg.write_nr23(now, value as u8);
        self.audio.psg.write_nr24(now, (value >> 8) as u8);
    }

    pub(crate) fn audio_write_sound3cnt_lo(&mut self, value: u16) {
        let now = self.current_time();
        self.audio_sample(now);
        self.audio.psg.ch3.size = value & 0x20 != 0;
        self.audio.psg.ch3.bank = value & 0x40 != 0;
        self.audio.psg.write_nr30(now, value as u8);
    }

    pub(crate) fn audio_write_sound3cnt_hi(&mut self, value: u16) {
        let now = self.current_time();
        self.audio_sample(now);
        self.audio.psg.write_nr31(now, value as u8);
        self.audio.psg.ch3.volume = ((value >> 13) & 7) as i32;
    }

    pub(crate) fn audio_write_sound3cnt_x(&mut self, value: u16) {
        let now = self.current_time();
        self.audio_sample(now);
        self.audio.psg.write_nr33(now, value as u8);
        self.audio.psg.write_nr34(now, (value >> 8) as u8);
    }

    pub(crate) fn audio_write_sound4cnt_lo(&mut self, value: u16) {
        let now = self.current_time();
        self.audio_sample(now);
        self.audio.psg.write_nr41(now, value as u8);
        self.audio.psg.write_nr42(now, (value >> 8) as u8);
    }

    pub(crate) fn audio_write_sound4cnt_hi(&mut self, value: u16) {
        let now = self.current_time();
        self.audio_sample(now);
        self.audio.psg.write_nr43(now, value as u8);
        self.audio.psg.write_nr44(now, (value >> 8) as u8);
    }

    pub(crate) fn audio_write_soundcnt_lo(&mut self, value: u16) {
        let now = self.current_time();
        self.audio_sample(now);
        self.audio.psg.write_nr50(now, value as u8);
        self.audio.psg.write_nr51(now, (value >> 8) as u8);
    }

    pub(crate) fn audio_write_soundcnt_hi(&mut self, value: u16) {
        let a = &mut self.audio;
        a.volume = (value & 3) as u8;
        a.volume_ch_a = value & 0x0004 != 0;
        a.volume_ch_b = value & 0x0008 != 0;
        a.ch_a_right = value & 0x0100 != 0;
        a.ch_a_left = value & 0x0200 != 0;
        a.ch_a_timer = value & 0x0400 != 0;
        a.ch_b_right = value & 0x1000 != 0;
        a.ch_b_left = value & 0x2000 != 0;
        a.ch_b_timer = value & 0x4000 != 0;
        if value & 0x0800 != 0 {
            a.ch_a.fifo_write = 0;
            a.ch_a.fifo_read = 0;
        }
        if value & 0x8000 != 0 {
            a.ch_b.fifo_write = 0;
            a.ch_b.fifo_read = 0;
        }
    }

    pub(crate) fn audio_write_soundcnt_x(&mut self, value: u16) {
        let now = self.current_time();
        self.audio_sample(now);
        self.audio.enable = value & 0x80 != 0;
        self.audio.psg.write_nr52(now, value as u8);
        if !self.audio.enable {
            let mut i = regs::SOUND1CNT_LO;
            while i < regs::SOUNDCNT_HI {
                self.memory.io[(i >> 1) as usize] = 0;
                i += 2;
            }
            self.audio.psg.ch3.size = false;
            self.audio.psg.ch3.bank = false;
            self.audio.psg.ch3.volume = 0;
            self.audio.volume = 0;
            self.audio.volume_ch_a = false;
            self.audio.volume_ch_b = false;
            self.memory.io[(regs::SOUNDCNT_HI >> 1) as usize] &= 0xFF00;
        }
    }

    pub(crate) fn audio_write_soundbias(&mut self, value: u16) {
        let mut timestamp = self.current_time();
        self.audio_sample(timestamp);
        self.audio.soundbias = value;
        let old_interval = self.audio.sample_interval;
        let res = self.audio.resolution();
        self.audio.sample_interval = 0x200 >> res;
        if old_interval != self.audio.sample_interval {
            timestamp = timestamp.wrapping_sub(self.audio.last_sample);
            self.audio.sample_index = (timestamp >> (9 - res)) as usize;
            if self.audio.sample_index >= MAX_SAMPLES {
                self.audio.sample_index = 0;
            }
        }
    }

    pub(crate) fn audio_write_wave_ram(&mut self, address: usize, value: u32) {
        let mut bank = !self.audio.psg.ch3.bank as usize;
        // When the audio hardware is turned off, it acts like bank 0 has been
        // selected in SOUND3CNT_L, so any read comes from bank 1.
        if !self.audio.enable {
            bank = 1;
        }
        let now = self.current_time();
        self.audio.psg.run(now, 0x4);
        self.audio.psg.ch3.wavedata32[address | (bank * 4)] = value;
    }

    pub(crate) fn audio_read_wave_ram(&mut self, address: usize) -> u32 {
        let mut bank = !self.audio.psg.ch3.bank as usize;
        if !self.audio.enable {
            bank = 1;
        }
        let now = self.current_time();
        self.audio.psg.run(now, 0x4);
        self.audio.psg.ch3.wavedata32[address | (bank * 4)]
    }

    pub(crate) fn audio_write_fifo(&mut self, address: u32, value: u32) -> u32 {
        let channel = match address {
            regs::FIFO_A_LO => &mut self.audio.ch_a,
            regs::FIFO_B_LO => &mut self.audio.ch_b,
            _ => return value,
        };
        channel.fifo[channel.fifo_write] = value;
        channel.fifo_write += 1;
        if channel.fifo_write == FIFO_SIZE {
            channel.fifo_write = 0;
        }
        channel.fifo[channel.fifo_write]
    }

    /// `GBAAudioSampleFIFO`
    pub(crate) fn audio_sample_fifo(&mut self, fifo_id: usize, cycles: i32) {
        let channel = if fifo_id == 0 { self.audio.ch_a } else { self.audio.ch_b };
        let fifo_size = if channel.fifo_write >= channel.fifo_read {
            channel.fifo_write - channel.fifo_read
        } else {
            FIFO_SIZE - channel.fifo_read + channel.fifo_write
        };
        if FIFO_SIZE - fifo_size > 4 && channel.dma_source > 0 {
            let n = channel.dma_source as usize;
            if self.memory.dma[n].timing() == DMA_TIMING_CUSTOM {
                let now = self.current_time();
                self.memory.dma[n].when = now.wrapping_sub(cycles) as u32;
                self.memory.dma[n].next_count = 4;
                self.dma_recalculate_cycles();
                self.audio_schedule_fifo_dma(n);
                self.dma_update();
            }
        }
        let until_event = self.until(EventId::AudioSample);
        let res = self.audio.resolution();
        let channel = if fifo_id == 0 { &mut self.audio.ch_a } else { &mut self.audio.ch_b };
        if channel.internal_remaining == 0 && fifo_size != 0 {
            channel.internal_sample = channel.fifo[channel.fifo_read];
            channel.internal_remaining = 4;
            channel.fifo_read += 1;
            if channel.fifo_read == FIFO_SIZE {
                channel.fifo_read = 0;
            }
        }
        let mut until = until_event - 1;
        let bits = 2i32 << res;
        until += 1 << (9 - res);
        until >>= 9 - res;
        if bits < until {
            until = bits;
        }
        let start = (bits - until).max(0);
        for i in start..bits {
            channel.samples[i as usize] = channel.internal_sample as i8;
        }
        if channel.internal_remaining != 0 {
            channel.internal_sample >>= 8;
            channel.internal_remaining -= 1;
        }
    }

    /// `GBAAudioSample`
    pub(crate) fn audio_sample(&mut self, timestamp: i32) {
        let a = &mut self.audio;
        let mut timestamp = timestamp.wrapping_sub(a.last_sample);
        timestamp = timestamp.wrapping_sub(a.sample_index as i32 * a.sample_interval);

        let max_sample = 2usize << a.resolution();
        let mut sample = a.sample_index;
        while timestamp >= a.sample_interval && sample < max_sample {
            let psg_shift = 4 - a.volume as i32;
            a.psg.run((sample as i32 * a.sample_interval).wrapping_add(a.last_sample), 0xF);
            let (l, r) = a.psg.sample();
            let mut left = (l >> psg_shift.max(0)) as i32;
            let mut right = (r >> psg_shift.max(0)) as i32;

            if !a.force_disable_ch_a {
                let s = ((a.ch_a.samples[sample] as i32) << 2) >> (!a.volume_ch_a) as i32;
                if a.ch_a_left {
                    left += s;
                }
                if a.ch_a_right {
                    right += s;
                }
            }
            if !a.force_disable_ch_b {
                let s = ((a.ch_b.samples[sample] as i32) << 2) >> (!a.volume_ch_b) as i32;
                if a.ch_b_left {
                    left += s;
                }
                if a.ch_b_right {
                    right += s;
                }
            }

            let left = a.apply_bias(left as i16 as i32);
            let right = a.apply_bias(right as i16 as i32);
            a.current_samples[sample] = [left as i16, right as i16];
            sample += 1;
            timestamp -= a.sample_interval;
        }

        a.sample_index = sample;
        if sample == max_sample {
            a.last_sample = a.last_sample.wrapping_add(SAMPLE_INTERVAL);
            a.sample_index = 0;
        }
    }

    /// `_sample`
    pub(crate) fn audio_sample_event(&mut self, cycles_late: u32) {
        let now = self.current_time();
        self.audio_sample(now.wrapping_sub(cycles_late as i32));

        let samples = 2usize << self.audio.resolution();
        let a = &mut self.audio;
        let fill_a = a.ch_a.samples[samples - 1];
        a.ch_a.samples = [fill_a; MAX_SAMPLES];
        let fill_b = a.ch_b.samples[samples - 1];
        a.ch_b.samples = [fill_b; MAX_SAMPLES];

        if a.buffer.len() + samples > MAX_BUFFERED {
            let excess = a.buffer.len() + samples - MAX_BUFFERED;
            a.buffer.drain(..excess);
        }
        for i in 0..samples {
            let s = a.current_samples[i];
            a.buffer.push_back(s);
        }
        self.schedule(EventId::AudioSample, SAMPLE_INTERVAL - cycles_late as i32);
    }

    /// `_updateFrame` (PSG frame sequencer)
    pub(crate) fn audio_update_frame_event(&mut self, cycles_late: u32) {
        let now = self.current_time();
        self.audio_sample(now);
        let tf = self.audio.psg.timing_factor;
        self.schedule(EventId::AudioFrame, tf * FRAME_CYCLES - cycles_late as i32);
        self.audio.psg.update_frame(now);
    }
}
