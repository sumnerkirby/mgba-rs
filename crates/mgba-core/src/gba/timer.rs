// Copyright (c) 2013-2015 Jeffrey Pfau
// Rust port of mGBA's GBA timers (src/gba/timer.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::{regs, Gba, IRQ_TIMER0};
use crate::timing::EventId;

/// `struct GBATimer`
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct Timer {
    pub reload: u16,
    pub last_event: i32,
    /// GBATimerFlags: PrescaleBits(0-3), CountUp(4), DoIrq(5), Enable(6)
    pub flags: u32,
}

impl Timer {
    #[inline]
    pub fn prescale_bits(&self) -> u32 {
        self.flags & 0xF
    }
    #[inline]
    pub fn count_up(&self) -> bool {
        self.flags & 0x10 != 0
    }
    #[inline]
    pub fn do_irq(&self) -> bool {
        self.flags & 0x20 != 0
    }
    #[inline]
    pub fn enabled(&self) -> bool {
        self.flags & 0x40 != 0
    }
}

pub(crate) fn timer_event(id: usize) -> EventId {
    match id {
        0 => EventId::Timer0,
        1 => EventId::Timer1,
        2 => EventId::Timer2,
        _ => EventId::Timer3,
    }
}

#[inline]
fn tmcnt_lo(timer: usize) -> usize {
    ((regs::TM0CNT_LO + ((timer as u32) << 2)) >> 1) as usize
}

impl Gba {
    /// `GBATimerInit`
    pub(crate) fn timer_init(&mut self) {
        self.timers = [Timer::default(); 4];
    }

    /// `GBATimerUpdate`
    pub(crate) fn timer_update(&mut self, timer_id: usize, cycles_late: u32) {
        let timer = self.timers[timer_id];
        if timer.count_up() {
            self.memory.io[tmcnt_lo(timer_id)] = timer.reload;
        } else {
            self.timer_update_register(timer_id, cycles_late as i32);
        }

        if timer.do_irq() {
            self.raise_irq(IRQ_TIMER0 + timer_id as u32, cycles_late);
        }

        if self.audio.enable && timer_id < 2 {
            if (self.audio.ch_a_left || self.audio.ch_a_right) && self.audio.ch_a_timer as usize == timer_id {
                self.audio_sample_fifo(0, cycles_late as i32);
            }
            if (self.audio.ch_b_left || self.audio.ch_b_right) && self.audio.ch_b_timer as usize == timer_id {
                self.audio_sample_fifo(1, cycles_late as i32);
            }
        }

        if timer_id < 3 {
            let next = self.timers[timer_id + 1];
            if next.count_up() && next.enabled() {
                let idx = tmcnt_lo(timer_id + 1);
                self.memory.io[idx] = self.memory.io[idx].wrapping_add(1);
                if self.memory.io[idx] == 0 && next.enabled() {
                    self.timer_update(timer_id + 1, cycles_late);
                }
            }
        }
    }

    /// `GBATimerUpdateRegister`
    pub(crate) fn timer_update_register(&mut self, timer: usize, cycles_late: i32) {
        let t = self.timers[timer];
        if !t.enabled() || t.count_up() {
            return;
        }

        // Align timer
        let prescale_bits = t.prescale_bits();
        let mut current_time = self.current_time().wrapping_sub(cycles_late);
        let tick_mask = (1i32 << prescale_bits) - 1;
        current_time &= !tick_mask;

        // Update register
        let mut tick_increment = current_time.wrapping_sub(t.last_event);
        self.timers[timer].last_event = current_time;
        tick_increment >>= prescale_bits;
        tick_increment = tick_increment.wrapping_add(self.memory.io[tmcnt_lo(timer)] as i32);
        while tick_increment >= 0x10000 {
            tick_increment -= 0x10000 - t.reload as i32;
        }
        self.memory.io[tmcnt_lo(timer)] = tick_increment as u16;

        // Schedule next update
        tick_increment = (0x10000 - tick_increment) << prescale_bits;
        current_time = current_time.wrapping_add(tick_increment);
        current_time &= !tick_mask;
        let ev = timer_event(timer);
        self.deschedule(ev);
        self.schedule_absolute(ev, current_time);
    }

    /// `GBATimerWriteTMCNT_LO`
    pub(crate) fn timer_write_tmcnt_lo(&mut self, timer: usize, reload: u16) {
        self.timers[timer].reload = reload;
    }

    /// `GBATimerWriteTMCNT_HI`
    pub(crate) fn timer_write_tmcnt_hi(&mut self, timer: usize, control: u16) {
        self.timer_update_register(timer, 0);

        const PRESCALE_TABLE: [u32; 4] = [0, 6, 8, 10];
        let prescale_bits = PRESCALE_TABLE[(control & 0x0003) as usize];

        let old = self.timers[timer];
        let mut flags = (old.flags & !0xF) | prescale_bits;
        let set = |f: &mut u32, bit: u32, on: bool| {
            if on {
                *f |= bit
            } else {
                *f &= !bit
            }
        };
        set(&mut flags, 0x10, timer > 0 && (control & 0x0004) != 0);
        set(&mut flags, 0x20, control & 0x0040 != 0);
        set(&mut flags, 0x40, control & 0x0080 != 0);
        self.timers[timer].flags = flags;
        let cur = self.timers[timer];

        let mut reschedule = false;
        if old.enabled() != cur.enabled() {
            reschedule = true;
            if cur.enabled() {
                self.memory.io[tmcnt_lo(timer)] = cur.reload;
            }
        } else if old.count_up() != cur.count_up() || cur.prescale_bits() != old.prescale_bits() {
            reschedule = true;
        }

        if reschedule {
            self.deschedule(timer_event(timer));
            if cur.enabled() && !cur.count_up() {
                let tick_mask = (1i32 << prescale_bits) - 1;
                self.timers[timer].last_event = self.current_time() & !tick_mask;
                self.timer_update_register(timer, 0);
            }
        }
    }
}
