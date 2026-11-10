// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's Game Boy timer (src/gb/timer.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::{io, Gb, GbEvent, IRQ_TIMER};

pub const DMG_DIV_PERIOD: i32 = 16;

/// `struct GBTimer`
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default)]
pub struct GbTimer {
    pub internal_div: u32,
    pub next_div: i32,
    pub tima_period: u32,
}

impl Gb {
    /// `_GBTimerIRQ`
    pub(crate) fn timer_irq(&mut self, _cycles_late: u32) {
        self.memory.io[io::REG_TIMA] = self.memory.io[io::REG_TMA];
        self.memory.io[io::REG_IF] |= 1 << IRQ_TIMER;
        self.update_irqs();
    }

    /// `_GBTimerDivIncrement`
    fn timer_div_increment(&mut self, cycles_late: u32) {
        let t_multiplier = 2 - self.double_speed as i32;
        while self.timer.next_div >= DMG_DIV_PERIOD * t_multiplier {
            self.timer.next_div -= DMG_DIV_PERIOD * t_multiplier;

            // Make sure to trigger when the correct bit is a falling edge
            let period = self.timer.tima_period;
            if period > 0 && (self.timer.internal_div & (period - 1)) == period - 1 {
                self.memory.io[io::REG_TIMA] = self.memory.io[io::REG_TIMA].wrapping_add(1);
                if self.memory.io[io::REG_TIMA] == 0 {
                    let state = self.cpu.execution_state as i32;
                    let delay = 7 * t_multiplier - ((state * t_multiplier - cycles_late as i32) & (3 * t_multiplier));
                    self.schedule(GbEvent::TimerIrq, delay);
                }
            }
            let timing_factor = (0x200u32 << self.double_speed) - 1;
            if (self.timer.internal_div & timing_factor) == timing_factor {
                self.audio_update_frame();
            }
            self.timer.internal_div = self.timer.internal_div.wrapping_add(1);
            self.memory.io[io::REG_DIV] = (self.timer.internal_div >> 4) as u8;
        }
    }

    /// `_GBTimerUpdate`
    pub(crate) fn timer_update(&mut self, cycles_late: u32) {
        self.timer.next_div += cycles_late as i32;
        self.timer_div_increment(cycles_late);
        // Batch div increments
        let mut divs_to_go = 16 - (self.timer.internal_div & 15) as i32;
        let mut tima_to_go = i32::MAX;
        if self.timer.tima_period != 0 {
            tima_to_go = (self.timer.tima_period - (self.timer.internal_div & (self.timer.tima_period - 1))) as i32;
        }
        if tima_to_go < divs_to_go {
            divs_to_go = tima_to_go;
        }
        self.timer.next_div = DMG_DIV_PERIOD * divs_to_go * (2 - self.double_speed as i32);
        let next = self.timer.next_div - cycles_late as i32;
        self.schedule(GbEvent::Timer, next);
    }

    /// `GBTimerReset`
    pub(crate) fn timer_reset(&mut self) {
        self.timer.next_div = DMG_DIV_PERIOD * 2;
        self.timer.tima_period = 1024 >> 4;
    }

    /// `GBTimerDivReset`
    pub(crate) fn timer_div_reset(&mut self) {
        self.timer.next_div -= self.until(GbEvent::Timer);
        self.deschedule(GbEvent::Timer);
        self.timer_div_increment(0);
        let ds = self.double_speed as i32;
        let t_multiplier = 2 - ds;
        if (((self.timer.internal_div << 1) | ((self.timer.next_div >> (4 - ds)) & 1) as u32) & self.timer.tima_period) != 0 {
            self.memory.io[io::REG_TIMA] = self.memory.io[io::REG_TIMA].wrapping_add(1);
            if self.memory.io[io::REG_TIMA] == 0 {
                let delay = (7 - (self.cpu.execution_state as i32 & 3)) * t_multiplier;
                self.schedule(GbEvent::TimerIrq, delay);
            }
        }
        if self.timer.internal_div & (0x200 << self.double_speed) != 0 {
            self.audio_update_frame();
        }
        self.memory.io[io::REG_DIV] = 0;
        self.timer.internal_div = 0;
        self.timer.next_div = DMG_DIV_PERIOD * (2 - ds);
        let next = self.timer.next_div - ((self.cpu.execution_state as i32 + 1) & 3) * t_multiplier;
        self.schedule(GbEvent::Timer, next);
    }

    /// `GBTimerUpdateTAC`
    pub(crate) fn timer_update_tac(&mut self, tac: u8) -> u8 {
        if tac & 4 != 0 {
            self.timer.next_div -= self.until(GbEvent::Timer);
            self.deschedule(GbEvent::Timer);
            let ds = self.double_speed as i32;
            let late = ((self.cpu.execution_state as i32 + 2) & 3) * (2 - ds);
            self.timer_div_increment(late as u32);

            self.timer.tima_period = match tac & 3 {
                0 => 1024 >> 4,
                1 => 16 >> 4,
                2 => 64 >> 4,
                _ => 256 >> 4,
            };

            self.timer.next_div += DMG_DIV_PERIOD * (2 - ds);
            let next = self.timer.next_div;
            self.schedule(GbEvent::Timer, next);
        } else {
            self.timer.tima_period = 0;
        }
        tac
    }
}
