// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's less common Game Boy mappers: HuC-3 (src/gb/mbc/huc-3.c),
// TAMA5 (tama5.c), the Pocket Camera (pocket-cam.c) and the unlicensed
// mappers (unlicensed.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::mbc::*;
use super::memory::*;
use super::Gb;

// ----- State -----

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub struct HuC3State {
    pub index: u8,
    pub value: u8,
    pub mode: u8,
    #[serde(with = "serde_big_array::BigArray")]
    pub registers: [u8; 256],
}

impl Default for HuC3State {
    fn default() -> Self {
        HuC3State { index: 0, value: 0, mode: 0, registers: [0; 256] }
    }
}

pub const TAMA6_RTC_MAX: usize = 0x10;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct Tama5State {
    pub reg: u8,
    pub disabled: bool,
    pub registers: [u8; 8],
    pub rtc_timer_page: [u8; TAMA6_RTC_MAX],
    pub rtc_alarm_page: [u8; TAMA6_RTC_MAX],
    pub rtc_free_page0: [u8; TAMA6_RTC_MAX],
    pub rtc_free_page1: [u8; TAMA6_RTC_MAX],
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub struct PocketCamState {
    pub registers_active: bool,
    #[serde(with = "serde_big_array::BigArray")]
    pub registers: [u8; 0x36],
}

impl Default for PocketCamState {
    fn default() -> Self {
        PocketCamState { registers_active: false, registers: [0; 0x36] }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct PkjdState {
    pub reg: [u8; 2],
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct NtOldState {
    pub swapped: bool,
    pub base_bank: u8,
    pub bank_count: u8,
    pub rumble: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct BbdState {
    pub data_swap_mode: i32,
    pub bank_swap_mode: i32,
}

pub const SACHEN_LOCKED_DMG: i32 = 0;
pub const SACHEN_LOCKED_CGB: i32 = 1;
pub const SACHEN_UNLOCKED: i32 = 2;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct SachenState {
    pub locked: i32,
    pub transition: i32,
    pub mask: u8,
    pub unmasked_bank: u8,
    pub base_bank: u8,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct SintaxState {
    pub mode: u8,
    pub xor_values: [u8; 4],
    pub bank_no: u8,
    pub rom_bank_xor: u8,
}

pub const CAM_WIDTH: usize = 128;
pub const CAM_HEIGHT: usize = 112;

// HuC-3 registers, modes and commands
const HUC3_RTC_MINUTES_LO: usize = 0x10;
const HUC3_RTC_MINUTES_MI: usize = 0x11;
const HUC3_RTC_MINUTES_HI: usize = 0x12;
const HUC3_RTC_DAYS_LO: usize = 0x13;
const HUC3_RTC_DAYS_MI: usize = 0x14;
const HUC3_RTC_DAYS_HI: usize = 0x15;
const HUC3_MODE_SRAM_RO: u8 = 0x0;
const HUC3_MODE_SRAM_RW: u8 = 0xA;
const HUC3_MODE_IN: u8 = 0xB;
const HUC3_MODE_OUT: u8 = 0xC;
const HUC3_MODE_COMMIT: u8 = 0xD;
const HUC3_CMD_LATCH: u8 = 0x0;
const HUC3_CMD_SET_RTC: u8 = 0x1;

// TAMA5 registers
const TAMA5_BANK_LO: u8 = 0x0;
const TAMA5_BANK_HI: u8 = 0x1;
const TAMA5_WRITE_LO: usize = 0x4;
const TAMA5_WRITE_HI: usize = 0x5;
const TAMA5_ADDR_HI: usize = 0x6;
const TAMA5_ADDR_LO: usize = 0x7;
const TAMA5_MAX: u8 = 0x8;
const TAMA5_ACTIVE: u8 = 0xA;
const TAMA5_READ_LO: u8 = 0xC;
const TAMA5_READ_HI: u8 = 0xD;

// TAMA6 RTC registers
const RTC_PA0_SECOND_1: usize = 0x0;
const RTC_PA0_SECOND_10: usize = 0x1;
const RTC_PA0_MINUTE_1: usize = 0x2;
const RTC_PA0_MINUTE_10: usize = 0x3;
const RTC_PA0_HOUR_1: usize = 0x4;
const RTC_PA0_HOUR_10: usize = 0x5;
const RTC_PA0_WEEK: usize = 0x6;
const RTC_PA0_DAY_1: usize = 0x7;
const RTC_PA0_DAY_10: usize = 0x8;
const RTC_PA0_MONTH_1: usize = 0x9;
const RTC_PA0_MONTH_10: usize = 0xA;
const RTC_PA0_YEAR_1: usize = 0xB;
const RTC_PA0_YEAR_10: usize = 0xC;
const RTC_PA1_24_HOUR: usize = 0xA;
const RTC_PA1_LEAP_YEAR: usize = 0xB;
pub const RTC_PAGE: usize = 0xD;

const TAMA6_DISABLE_TIMER: u8 = 0x0;
const TAMA6_ENABLE_TIMER: u8 = 0x1;
const TAMA6_MINUTE_WRITE: u8 = 0x4;
const TAMA6_HOUR_WRITE: u8 = 0x5;
const TAMA6_MINUTE_READ: u8 = 0x6;
const TAMA6_HOUR_READ: u8 = 0x7;
const TAMA6_DISABLE_ALARM: u8 = 0x10;
const TAMA6_ENABLE_ALARM: u8 = 0x11;

const TAMA6_RTC_MASK: [u8; 32] = [
    0xF, 0x7, 0xF, 0x7, 0xF, 0x3, 0x7, 0xF, 0x3, 0xF, 0x1, 0xF, 0xF, 0x0, 0x0, 0x0, //
    0x0, 0x0, 0xF, 0x7, 0xF, 0x3, 0x7, 0xF, 0x3, 0x0, 0x1, 0x3, 0x0, 0x0, 0x0, 0x0,
];

const DAYS_TO_MONTH: [i32; 13] = [
    0,
    0,
    31,
    31 + 28,
    31 + 28 + 31,
    31 + 28 + 31 + 30,
    31 + 28 + 31 + 30 + 31,
    31 + 28 + 31 + 30 + 31 + 30,
    31 + 28 + 31 + 30 + 31 + 30 + 31,
    31 + 28 + 31 + 30 + 31 + 30 + 31 + 31,
    31 + 28 + 31 + 30 + 31 + 30 + 31 + 31 + 30,
    31 + 28 + 31 + 30 + 31 + 30 + 31 + 31 + 30 + 31,
    31 + 28 + 31 + 30 + 31 + 30 + 31 + 31 + 30 + 31 + 30,
];

fn tama6_dmy_to_day_of_year(day: i32, month: i32, year: i32) -> i32 {
    if !(1..=12).contains(&month) {
        return -1;
    }
    let mut day = day + DAYS_TO_MONTH[month as usize];
    if month > 2 && (year & 3) == 0 {
        day += 1;
    }
    day
}

fn tama6_day_of_year_to_month(day: i32, year: i32) -> i32 {
    let mut day = day;
    for month in 1..12 {
        if day <= DAYS_TO_MONTH[month as usize + 1] {
            return month;
        }
        if month == 2 && (year & 3) == 0 {
            if day == 60 {
                return 2;
            }
            day -= 1;
        }
    }
    12
}

fn tama6_day_of_year_to_day_of_month(day: i32, year: i32) -> i32 {
    let mut day = day;
    for month in 1..12 {
        if day <= DAYS_TO_MONTH[month as usize + 1] {
            return day - DAYS_TO_MONTH[month as usize];
        }
        if month == 2 && (year & 3) == 0 {
            if day == 60 {
                return 29;
            }
            day -= 1;
        }
    }
    day - DAYS_TO_MONTH[12]
}

/// `_latchHuC3Rtc`
fn latch_huc3_rtc(now: i64, regs: &mut [u8; 256], last_latch: &mut i64) {
    let mut t = now - *last_latch;
    t /= 60;
    if t == 0 {
        return;
    }
    *last_latch += t * 60;

    let mut minutes: i64 = (regs[HUC3_RTC_MINUTES_HI] as i64) << 8;
    minutes |= (regs[HUC3_RTC_MINUTES_MI] as i64) << 4;
    minutes |= regs[HUC3_RTC_MINUTES_LO] as i64;
    minutes += t % 1440;
    t /= 1440;
    if minutes >= 1440 {
        minutes -= 1440;
        t += 1;
    } else if minutes < 0 {
        minutes += 1440;
        t -= 1;
    }
    regs[HUC3_RTC_MINUTES_LO] = (minutes & 0xF) as u8;
    regs[HUC3_RTC_MINUTES_MI] = ((minutes >> 4) & 0xF) as u8;
    regs[HUC3_RTC_MINUTES_HI] = ((minutes >> 8) & 0xF) as u8;

    let mut days: i64 = regs[HUC3_RTC_DAYS_LO] as i64;
    days |= (regs[HUC3_RTC_DAYS_MI] as i64) << 4;
    days |= (regs[HUC3_RTC_DAYS_HI] as i64) << 8;
    days += t;
    regs[HUC3_RTC_DAYS_LO] = (days & 0xF) as u8;
    regs[HUC3_RTC_DAYS_MI] = ((days >> 4) & 0xF) as u8;
    regs[HUC3_RTC_DAYS_HI] = ((days >> 8) & 0xF) as u8;
}

/// `_latchTAMA6Rtc`
fn latch_tama6_rtc(now: i64, tama5: &mut Tama5State, last_latch: &mut i64) {
    let mut t = now;
    let current_latch = t;
    t -= *last_latch;
    *last_latch = current_latch;
    if t == 0 || tama5.disabled {
        return;
    }

    let is24hour = tama5.rtc_alarm_page[RTC_PA1_24_HOUR] != 0;
    let r = &mut tama5.rtc_timer_page;
    let mut diff: i64 = r[RTC_PA0_SECOND_1] as i64 + r[RTC_PA0_SECOND_10] as i64 * 10 + t % 60;
    if diff < 0 {
        diff += 60;
        t -= 60;
    }
    r[RTC_PA0_SECOND_1] = (diff % 10) as u8;
    r[RTC_PA0_SECOND_10] = ((diff % 60) / 10) as u8;
    t /= 60;
    t += diff / 60;

    diff = r[RTC_PA0_MINUTE_1] as i64 + r[RTC_PA0_MINUTE_10] as i64 * 10 + t % 60;
    if diff < 0 {
        diff += 60;
        t -= 60;
    }
    r[RTC_PA0_MINUTE_1] = (diff % 10) as u8;
    r[RTC_PA0_MINUTE_10] = ((diff % 60) / 10) as u8;
    t /= 60;
    t += diff / 60;

    diff = r[RTC_PA0_HOUR_1] as i64;
    if is24hour {
        diff += r[RTC_PA0_HOUR_10] as i64 * 10;
    } else {
        let hour10 = r[RTC_PA0_HOUR_10] as i64;
        diff += (hour10 & 1) * 10;
        diff += (hour10 & 2) * 12;
    }
    diff += t % 24;
    if diff < 0 {
        diff += 24;
        t -= 24;
    }
    if is24hour {
        r[RTC_PA0_HOUR_1] = ((diff % 24) % 10) as u8;
        r[RTC_PA0_HOUR_10] = ((diff % 24) / 10) as u8;
    } else {
        r[RTC_PA0_HOUR_1] = ((diff % 12) % 10) as u8;
        r[RTC_PA0_HOUR_10] = ((diff % 12) / 10 + (diff / 12) * 2) as u8;
    }
    t /= 24;
    t += diff / 24;

    let day = r[RTC_PA0_DAY_1] as i32 + r[RTC_PA0_DAY_10] as i32 * 10;
    let month = r[RTC_PA0_MONTH_1] as i32 + r[RTC_PA0_MONTH_10] as i32 * 10;
    let mut year = r[RTC_PA0_YEAR_1] as i32 + r[RTC_PA0_YEAR_10] as i32 * 10;
    let mut leap_year = tama5.rtc_alarm_page[RTC_PA1_LEAP_YEAR] as i32;
    let mut day_of_week = r[RTC_PA0_WEEK] as i64;
    let day_in_year = tama6_dmy_to_day_of_year(day, month, leap_year);
    let mut diff = day_in_year as i64 + t;
    while diff <= 0 {
        // Previous year
        diff += if leap_year & 3 != 0 { 365 } else { 366 };
        year -= 1;
        leap_year -= 1;
    }
    while diff > if leap_year & 3 != 0 { 365 } else { 366 } {
        // Future year
        diff -= if year % 4 != 0 { 365 } else { 366 };
        year += 1;
        leap_year += 1;
    }
    day_of_week = (day_of_week + diff) % 7;
    year %= 100;
    leap_year &= 3;

    let day = tama6_day_of_year_to_day_of_month(diff as i32, leap_year);
    let month = tama6_day_of_year_to_month(diff as i32, leap_year);

    r[RTC_PA0_WEEK] = day_of_week as u8;
    r[RTC_PA0_DAY_1] = (day % 10) as u8;
    r[RTC_PA0_DAY_10] = (day / 10) as u8;
    r[RTC_PA0_MONTH_1] = (month % 10) as u8;
    r[RTC_PA0_MONTH_10] = (month / 10) as u8;
    r[RTC_PA0_YEAR_1] = (year % 10) as u8;
    r[RTC_PA0_YEAR_10] = (year / 10) as u8;
    tama5.rtc_alarm_page[RTC_PA1_LEAP_YEAR] = leap_year as u8;
}

/// `_reorderBits`
fn reorder_bits(input: u8, reorder: &[u8; 8]) -> u8 {
    let mut out = 0u8;
    for (i, &old) in reorder.iter().enumerate() {
        out = out.wrapping_add(((input >> old) & 1) << i);
    }
    out
}

const NT_OLD1_REORDER: [u8; 8] = [0, 2, 1, 4, 3, 5, 6, 7];
const NT_OLD2_REORDER: [u8; 8] = [1, 2, 0, 3, 4, 5, 6, 7];

const BBD_DATA_REORDERING: [[u8; 8]; 8] = [
    [0, 1, 2, 3, 4, 5, 6, 7], // 00 - Normal
    [0, 1, 2, 3, 4, 5, 6, 7], // 01 - NOT KNOWN YET
    [0, 1, 2, 3, 4, 5, 6, 7], // 02 - NOT KNOWN YET
    [0, 1, 2, 3, 4, 5, 6, 7], // 03 - NOT KNOWN YET
    [0, 5, 1, 3, 4, 2, 6, 7], // 04 - Garou
    [0, 4, 2, 3, 1, 5, 6, 7], // 05 - Harry
    [0, 1, 2, 3, 4, 5, 6, 7], // 06 - NOT KNOWN YET
    [0, 1, 5, 3, 4, 2, 6, 7], // 07 - Digimon
];

const BBD_BANK_REORDERING: [[u8; 8]; 8] = [
    [0, 1, 2, 3, 4, 5, 6, 7], // 00 - Normal
    [0, 1, 2, 3, 4, 5, 6, 7], // 01 - NOT KNOWN YET
    [0, 1, 2, 3, 4, 5, 6, 7], // 02 - NOT KNOWN YET
    [3, 4, 2, 0, 1, 5, 6, 7], // 03 - 0,1 unconfirmed. Digimon/Garou
    [0, 1, 2, 3, 4, 5, 6, 7], // 04 - NOT KNOWN YET
    [1, 2, 3, 4, 0, 5, 6, 7], // 05 - 0,1 unconfirmed. Harry
    [0, 1, 2, 3, 4, 5, 6, 7], // 06 - NOT KNOWN YET
    [0, 1, 2, 3, 4, 5, 6, 7], // 07 - NOT KNOWN YET
];

const HITEK_DATA_REORDERING: [[u8; 8]; 8] = [
    [0, 1, 2, 3, 4, 5, 6, 7],
    [0, 6, 5, 3, 4, 1, 2, 7],
    [0, 5, 6, 3, 4, 2, 1, 7],
    [0, 6, 2, 3, 4, 5, 1, 7],
    [0, 6, 1, 3, 4, 5, 2, 7],
    [0, 1, 6, 3, 4, 5, 2, 7],
    [0, 2, 6, 3, 4, 1, 5, 7],
    [0, 6, 2, 3, 4, 1, 5, 7],
];

const HITEK_BANK_REORDERING: [[u8; 8]; 8] = [
    [0, 1, 2, 3, 4, 5, 6, 7],
    [3, 2, 1, 0, 4, 5, 6, 7],
    [2, 1, 0, 3, 4, 5, 6, 7],
    [1, 0, 3, 2, 4, 5, 6, 7],
    [0, 3, 2, 1, 4, 5, 6, 7],
    [2, 3, 0, 1, 4, 5, 6, 7],
    [3, 0, 1, 2, 4, 5, 6, 7],
    [2, 0, 3, 1, 4, 5, 6, 7],
];

const GGB81_DATA_REORDERING: [[u8; 8]; 8] = [
    [0, 1, 2, 3, 4, 5, 6, 7],
    [0, 2, 1, 3, 4, 6, 5, 7],
    [0, 6, 5, 3, 4, 2, 1, 7],
    [0, 5, 1, 3, 4, 2, 6, 7],
    [0, 5, 2, 3, 4, 1, 6, 7],
    [0, 2, 6, 3, 4, 5, 1, 7],
    [0, 1, 6, 3, 4, 2, 5, 7],
    [0, 2, 5, 3, 4, 6, 1, 7],
];

const SINTAX_REORDERING: [[u8; 8]; 16] = [
    [2, 1, 4, 3, 6, 5, 0, 7],
    [3, 2, 5, 4, 7, 6, 1, 0],
    [0, 1, 2, 3, 4, 5, 6, 7], // unknown
    [0, 1, 2, 3, 4, 5, 6, 7], // unknown
    [0, 1, 2, 3, 4, 5, 6, 7], // unknown
    [4, 5, 2, 3, 0, 1, 6, 7],
    [0, 1, 2, 3, 4, 5, 6, 7], // unknown
    [6, 7, 4, 5, 1, 3, 0, 2],
    [0, 1, 2, 3, 4, 5, 6, 7], // unknown
    [7, 6, 1, 0, 3, 2, 5, 4],
    [0, 1, 2, 3, 4, 5, 6, 7], // unknown
    [5, 4, 7, 6, 1, 0, 3, 2],
    [0, 1, 2, 3, 4, 5, 6, 7], // unknown
    [2, 3, 4, 5, 6, 7, 0, 1],
    [0, 1, 2, 3, 4, 5, 6, 7], // unknown
    [0, 1, 2, 3, 4, 5, 6, 7],
];

/// `_unscrambleSachen`
fn unscramble_sachen(address: u16) -> u16 {
    let mut u = address & 0xFFAC;
    u |= (address & 0x40) >> 6;
    u |= (address & 0x10) >> 3;
    u |= (address & 0x02) << 3;
    u |= (address & 0x01) << 6;
    u
}

impl Gb {
    // ----- HuC-3 -----

    /// `_huc3Commit`
    fn huc3_commit(&mut self) {
        let now = self.rtc_unix_time();
        let mem = &mut self.memory;
        let state = &mut mem.mbc_state.huc3;
        match state.value & 0x70 {
            0x10 => {
                if (state.index & 0xF8) == 0x10 {
                    latch_huc3_rtc(now, &mut state.registers, &mut mem.rtc_last_latch);
                }
                state.value &= 0xF0;
                state.value |= state.registers[state.index as usize] & 0xF;
                if state.value & 0x10 != 0 {
                    state.index = state.index.wrapping_add(1);
                }
            }
            0x30 => {
                state.registers[state.index as usize] = state.value & 0xF;
                if state.value & 0x10 != 0 {
                    state.index = state.index.wrapping_add(1);
                }
            }
            0x40 => {
                state.index &= 0xF0;
                state.index |= state.value & 0xF;
            }
            0x50 => {
                state.index &= 0x0F;
                state.index |= (state.value & 0xF) << 4;
            }
            0x60 => {
                match state.value & 0xF {
                    HUC3_CMD_LATCH => {
                        latch_huc3_rtc(now, &mut state.registers, &mut mem.rtc_last_latch);
                        state.registers.copy_within(HUC3_RTC_MINUTES_LO..HUC3_RTC_MINUTES_LO + 6, 0);
                    }
                    HUC3_CMD_SET_RTC => {
                        state.registers.copy_within(0..6, HUC3_RTC_MINUTES_LO);
                    }
                    // Read-only mode and the speaker tone aren't emulated (mGBA only logs them).
                    _ => {}
                }
                state.value = 0xE1;
            }
            _ => {}
        }
    }

    /// `_GBHuC3`
    pub(crate) fn huc3_write(&mut self, address: u16, value: u8) {
        let bank = (value & 0x7F) as i32;
        match address >> 13 {
            0x0 => {
                if value == 0xA {
                    self.memory.sram_access = true;
                    let b = self.memory.sram_current_bank;
                    self.mbc_switch_sram_bank(b);
                } else {
                    self.memory.sram_access = false;
                }
                self.memory.mbc_state.huc3.mode = value;
            }
            0x1 => self.mbc_switch_bank(bank),
            0x2 => self.mbc_switch_sram_bank(bank),
            0x5 => match self.memory.mbc_state.huc3.mode {
                HUC3_MODE_IN => self.memory.mbc_state.huc3.value = 0x80 | value,
                HUC3_MODE_COMMIT => self.huc3_commit(),
                _ => {}
            },
            _ => {}
        }
    }

    /// `_GBHuC3Read`
    pub(crate) fn huc3_read(&self, address: u16) -> u8 {
        let state = &self.memory.mbc_state.huc3;
        match state.mode {
            HUC3_MODE_SRAM_RO | HUC3_MODE_SRAM_RW => self.memory.sram_bank_byte(address as usize & (SIZE_EXTERNAL_RAM - 1)),
            HUC3_MODE_IN | HUC3_MODE_OUT => 0x80 | state.value,
            _ => 0xFF,
        }
    }

    // ----- TAMA5 -----

    /// `_GBTAMA5`
    pub(crate) fn tama5_write(&mut self, address: u16, value: u8) {
        if address >> 13 != 0x5 {
            return;
        }
        if address & 1 != 0 {
            self.memory.mbc_state.tama5.reg = value;
            return;
        }
        let value = value & 0xF;
        let reg = self.memory.mbc_state.tama5.reg;
        if reg >= TAMA5_MAX {
            return;
        }
        let tama5 = &mut self.memory.mbc_state.tama5;
        tama5.registers[reg as usize] = value;
        let mut addr = ((tama5.registers[TAMA5_ADDR_HI] << 4) & 0x10) | tama5.registers[TAMA5_ADDR_LO];
        let mut out = (tama5.registers[TAMA5_WRITE_HI] << 4) | tama5.registers[TAMA5_WRITE_LO];
        match reg {
            TAMA5_BANK_LO | TAMA5_BANK_HI => {
                let bank = tama5.registers[TAMA5_BANK_LO as usize] as i32 | ((tama5.registers[TAMA5_BANK_HI as usize] as i32) << 4);
                self.mbc_switch_bank(bank);
            }
            0x7 => {
                // TAMA5_ADDR_LO
                match tama5.registers[TAMA5_ADDR_HI] >> 1 {
                    0x0 => {
                        // RAM write
                        if let Some(b) = self.memory.sram.get_mut(addr as usize) {
                            *b = out;
                        }
                        self.sram_dirty |= SAVEDATA_DIRT_NEW;
                    }
                    0x1 => {} // RAM read
                    0x2 => {
                        let pages = |t: &mut Tama5State, f: &dyn Fn(&mut u8)| {
                            f(&mut t.rtc_timer_page[RTC_PAGE]);
                            f(&mut t.rtc_alarm_page[RTC_PAGE]);
                            f(&mut t.rtc_free_page0[RTC_PAGE]);
                            f(&mut t.rtc_free_page1[RTC_PAGE]);
                        };
                        match addr {
                            TAMA6_DISABLE_TIMER => {
                                tama5.disabled = true;
                                pages(tama5, &|p| *p &= 0x7);
                            }
                            TAMA6_ENABLE_TIMER => {
                                tama5.disabled = false;
                                tama5.rtc_timer_page[RTC_PA0_SECOND_1] = 0;
                                tama5.rtc_timer_page[RTC_PA0_SECOND_10] = 0;
                                pages(tama5, &|p| *p |= 0x8);
                            }
                            TAMA6_MINUTE_WRITE => {
                                tama5.rtc_timer_page[RTC_PA0_MINUTE_1] = out & 0xF;
                                tama5.rtc_timer_page[RTC_PA0_MINUTE_10] = out >> 4;
                            }
                            TAMA6_HOUR_WRITE => {
                                tama5.rtc_timer_page[RTC_PA0_HOUR_1] = out & 0xF;
                                tama5.rtc_timer_page[RTC_PA0_HOUR_10] = out >> 4;
                            }
                            TAMA6_DISABLE_ALARM => pages(tama5, &|p| *p &= 0xB),
                            TAMA6_ENABLE_ALARM => pages(tama5, &|p| *p |= 0x4),
                            _ => {}
                        }
                    }
                    0x4 => {
                        // RTC access
                        addr = tama5.registers[TAMA5_WRITE_LO];
                        if addr as usize >= RTC_PAGE {
                            return;
                        }
                        out = tama5.registers[TAMA5_WRITE_HI];
                        let a = addr as usize;
                        match tama5.registers[TAMA5_ADDR_LO] {
                            0 => tama5.rtc_timer_page[a] = out & TAMA6_RTC_MASK[a],
                            2 => tama5.rtc_alarm_page[a] = out & TAMA6_RTC_MASK[a | 0x10],
                            4 => tama5.rtc_free_page0[a] = out,
                            6 => tama5.rtc_free_page1[a] = out,
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// `_GBTAMA5Read`
    pub(crate) fn tama5_read(&mut self, address: u16) -> u8 {
        if address & 1 != 0 {
            return 0xFF;
        }
        let now = self.rtc_unix_time();
        let mem = &mut self.memory;
        let tama5 = &mut mem.mbc_state.tama5;
        let mut value: u8 = 0xF0;
        let mut addr = ((tama5.registers[TAMA5_ADDR_HI] << 4) & 0x10) | tama5.registers[TAMA5_ADDR_LO];
        match tama5.reg {
            TAMA5_ACTIVE => 0xF1,
            TAMA5_READ_LO | TAMA5_READ_HI => {
                match tama5.registers[TAMA5_ADDR_HI] >> 1 {
                    0x1 => value = mem.sram.get(addr as usize).copied().unwrap_or(0xFF),
                    0x2 => {
                        latch_tama6_rtc(now, tama5, &mut mem.rtc_last_latch);
                        value = match addr {
                            TAMA6_MINUTE_READ => (tama5.rtc_timer_page[RTC_PA0_MINUTE_10] << 4) | tama5.rtc_timer_page[RTC_PA0_MINUTE_1],
                            TAMA6_HOUR_READ => (tama5.rtc_timer_page[RTC_PA0_HOUR_10] << 4) | tama5.rtc_timer_page[RTC_PA0_HOUR_1],
                            _ => addr,
                        };
                    }
                    0x4 => {
                        if tama5.reg != TAMA5_READ_HI {
                            latch_tama6_rtc(now, tama5, &mut mem.rtc_last_latch);
                            addr = tama5.registers[TAMA5_WRITE_LO];
                            if addr as usize > RTC_PAGE {
                                value = 0;
                            } else if matches!(tama5.registers[TAMA5_ADDR_LO], 1 | 3 | 5 | 7) {
                                value = tama5.rtc_timer_page[addr as usize];
                            }
                        }
                    }
                    _ => {}
                }
                if tama5.reg == TAMA5_READ_HI {
                    value >>= 4;
                }
                value | 0xF0
            }
            _ => 0xF1,
        }
    }

    // ----- Pocket Camera -----

    /// `_GBPocketCam`
    pub(crate) fn pocket_cam_write(&mut self, address: u16, value: u8) {
        let bank = (value & 0x3F) as i32;
        match address >> 13 {
            0x0 => match value {
                0 => self.memory.sram_access = false,
                0xA => {
                    self.memory.sram_access = true;
                    let b = self.memory.sram_current_bank;
                    self.mbc_switch_sram_bank(b);
                }
                _ => {}
            },
            0x1 => self.mbc_switch_bank(bank),
            0x2 => {
                if value < 0x10 {
                    self.mbc_switch_sram_bank(value as i32);
                    self.memory.mbc_state.pocket_cam.registers_active = false;
                    self.memory.direct_sram_access = true;
                } else {
                    self.memory.mbc_state.pocket_cam.registers_active = true;
                    self.memory.direct_sram_access = false;
                }
            }
            0x5 => {
                if !self.memory.mbc_state.pocket_cam.registers_active {
                    return;
                }
                let address = (address & 0x7F) as usize;
                let mut value = value;
                if address == 0 && value & 1 != 0 {
                    value &= 6; // TODO: Timing
                    self.sram_dirty |= SAVEDATA_DIRT_NEW;
                    self.pocket_cam_capture();
                }
                if address < 0x36 {
                    self.memory.mbc_state.pocket_cam.registers[address] = value;
                }
            }
            _ => {}
        }
    }

    /// `_GBPocketCamRead`
    pub(crate) fn pocket_cam_read(&self, address: u16) -> u8 {
        let cam = &self.memory.mbc_state.pocket_cam;
        if cam.registers_active {
            if address & 0x7F == 0 {
                return cam.registers[0];
            }
            return 0;
        }
        self.memory.sram_bank_byte(address as usize & (SIZE_EXTERNAL_RAM - 1))
    }

    /// `_GBPocketCamCapture`, from `camera_image` (128x112 8-bit grayscale)
    fn pocket_cam_capture(&mut self) {
        let Some(image) = self.camera_image.clone() else { return };
        if image.len() < CAM_WIDTH * CAM_HEIGHT || self.memory.sram.len() < 0x100 + CAM_WIDTH * CAM_HEIGHT / 4 {
            return;
        }
        let sram = &mut self.memory.sram;
        sram[0x100..0x100 + CAM_HEIGHT * CAM_WIDTH / 4].iter_mut().for_each(|b| *b = 0);
        let regs = self.memory.mbc_state.pocket_cam.registers;
        let exposure = ((regs[2] as u32) << 8) | regs[3] as u32;
        for y in 0..CAM_HEIGHT {
            for x in 0..CAM_WIDTH {
                let mut gray = image[y * CAM_WIDTH + x] as u32;
                gray = (gray + 1) * exposure / 0x100;
                // TODO: Additional processing
                let matrix_entry = 3 * ((x & 3) + 4 * (y & 3));
                gray = if gray < regs[matrix_entry + 6] as u32 {
                    0x101
                } else if gray < regs[matrix_entry + 7] as u32 {
                    0x100
                } else if gray < regs[matrix_entry + 8] as u32 {
                    0x001
                } else {
                    0
                };
                let coord = (((x >> 3) & 0xF) * 8 + (y & 0x7)) * 2 + (y & !0x7) * 0x20 + 0x100;
                let mut existing = u16::from_le_bytes([sram[coord], sram[coord + 1]]);
                existing |= (gray << (7 - (x & 7))) as u16;
                sram[coord..coord + 2].copy_from_slice(&existing.to_le_bytes());
            }
        }
    }

    // ----- Unlicensed -----

    /// `_GBWisdomTree`
    pub(crate) fn wisdom_tree_write(&mut self, address: u16, _value: u8) {
        let bank = (address & 0x3F) as i32;
        if address >> 14 == 0 {
            self.mbc_switch_bank0(bank * 2);
            self.mbc_switch_bank(bank * 2 + 1);
        }
    }

    /// `_GBPKJD`
    pub(crate) fn pkjd_write(&mut self, address: u16, value: u8) {
        let mem = &mut self.memory;
        match address >> 13 {
            0x2 => {
                if value < 8 {
                    mem.direct_sram_access = true;
                    mem.active_rtc_reg = 0;
                } else if (0xD..=0xF).contains(&value) {
                    mem.direct_sram_access = false;
                    mem.rtc_access = false;
                    mem.active_rtc_reg = value as i32 - 8;
                }
            }
            0x5 => {
                if !mem.sram_access {
                    return;
                }
                let reg = &mut mem.mbc_state.pkjd.reg;
                match mem.active_rtc_reg {
                    0 => mem.sram_bank_set(address as usize & (SIZE_EXTERNAL_RAM - 1), value),
                    5 | 6 => reg[(mem.active_rtc_reg - 5) as usize] = value,
                    7 => match value {
                        0x11 => reg[0] = reg[0].wrapping_sub(1),
                        0x12 => reg[1] = reg[1].wrapping_sub(1),
                        0x41 => reg[0] = reg[0].wrapping_add(reg[1]),
                        0x42 => reg[1] = reg[1].wrapping_add(reg[0]),
                        0x51 => reg[0] = reg[0].wrapping_add(1),
                        0x52 => reg[1] = reg[1].wrapping_sub(1),
                        _ => {}
                    },
                    _ => {}
                }
                return;
            }
            _ => {}
        }
        self.mbc3_write(address, value);
    }

    /// `_GBPKJDRead`
    pub(crate) fn pkjd_read(&self, address: u16) -> u8 {
        let mem = &self.memory;
        if !mem.sram_access {
            return 0xFF;
        }
        match mem.active_rtc_reg {
            0 => mem.sram_bank_byte(address as usize & (SIZE_EXTERNAL_RAM - 1)),
            5 | 6 => mem.mbc_state.pkjd.reg[(mem.active_rtc_reg - 5) as usize],
            _ => 0,
        }
    }

    /// `_ntOldMulticart`
    fn nt_old_multicart(&mut self, address: u16, value: u8, reorder: &[u8; 8]) {
        match address & 3 {
            1 => {
                let base = (value & 0x3F) * 2;
                self.memory.mbc_state.nt_old.base_bank = base;
                if base != 0 {
                    self.mbc_switch_bank0(base as i32);
                    self.mbc_switch_bank(base as i32 + 1);
                }
            }
            2 => {
                if (value & 0xF0) == 0xE0 {
                    self.sram_size = 0x2000;
                    self.resize_sram(0x2000);
                }
                self.memory.mbc_state.nt_old.bank_count = match value & 0xF {
                    0x00 => 32,
                    0x08 => 16,
                    0x0C => 8,
                    0x0E => 4,
                    0x0F => 2,
                    _ => 32,
                };
            }
            3 => {
                let swapped = value & 0x10 != 0;
                self.memory.mbc_state.nt_old.swapped = swapped;
                let mut bank = self.memory.current_bank;
                if swapped {
                    bank = reorder_bits(bank as u8, reorder) as i32;
                }
                self.mbc_switch_bank(bank);
            }
            _ => {}
        }
    }

    /// `_GBNTOld1` / `_GBNTOld2`
    pub(crate) fn nt_old_write(&mut self, address: u16, value: u8, two: bool) {
        let reorder = if two { &NT_OLD2_REORDER } else { &NT_OLD1_REORDER };
        match address >> 12 {
            0x0 | 0x1 => self.mbc3_write(address, value),
            0x2 | 0x3 => {
                let mut bank = value as i32;
                if !two {
                    bank &= 0x1F;
                }
                if bank == 0 {
                    bank = 1;
                }
                let st = self.memory.mbc_state.nt_old;
                if st.swapped {
                    bank = reorder_bits(bank as u8, reorder) as i32;
                }
                if st.bank_count != 0 {
                    bank &= st.bank_count as i32 - 1;
                }
                self.mbc_switch_bank(bank + st.base_bank as i32);
            }
            0x4 | 0x5 => {
                if address >> 12 == 0x5 {
                    self.nt_old_multicart(address, value, reorder);
                    if !two {
                        return;
                    }
                } else if !two {
                    return;
                }
                if address == 0x5001 {
                    self.memory.mbc_state.nt_old.rumble = value & 0x80 != 0;
                }
                let st = self.memory.mbc_state.nt_old;
                if st.rumble {
                    self.rumble = if st.swapped { value & 0x08 != 0 } else { value & 0x02 != 0 };
                }
            }
            _ => {}
        }
    }

    /// `_GBNTNew`
    pub(crate) fn nt_new_write(&mut self, address: u16, value: u8) {
        if address >> 8 == 0x14 {
            self.memory.mbc_state.nt_new.split_mode = true;
            return;
        }
        if self.memory.mbc_state.nt_new.split_mode {
            let bank = (value as i32).max(2);
            match address >> 10 {
                8 => {
                    self.mbc_switch_half_bank(0, bank);
                    return;
                }
                9 => {
                    self.mbc_switch_half_bank(1, bank);
                    return;
                }
                _ => {}
            }
        }
        self.mbc5_write(address, value);
    }

    /// `_GBBBD`
    pub(crate) fn bbd_write(&mut self, address: u16, value: u8) {
        let mut value = value;
        let st = &mut self.memory.mbc_state.bbd;
        match address & 0xF0FF {
            0x2000 => value = reorder_bits(value, &BBD_BANK_REORDERING[st.bank_swap_mode as usize]),
            0x2001 => st.data_swap_mode = (value & 0x07) as i32,
            0x2080 => st.bank_swap_mode = (value & 0x07) as i32,
            _ => {}
        }
        self.mbc5_write(address, value);
    }

    /// `_GBHitek`
    pub(crate) fn hitek_write(&mut self, address: u16, value: u8) {
        let mut value = value;
        let st = &mut self.memory.mbc_state.bbd;
        match address & 0xF0FF {
            0x2000 => value = reorder_bits(value, &HITEK_BANK_REORDERING[st.bank_swap_mode as usize]),
            0x2001 => st.data_swap_mode = (value & 0x07) as i32,
            0x2080 => st.bank_swap_mode = (value & 0x07) as i32,
            // See hhugboy src/memory/mbc/MbcUnlHitek.cpp for commentary on this return
            0x300 => return,
            _ => {}
        }
        self.mbc5_write(address, value);
    }

    /// `_GBGGB81`
    pub(crate) fn ggb81_write(&mut self, address: u16, value: u8) {
        if address & 0xF0FF == 0x2001 {
            self.memory.mbc_state.bbd.data_swap_mode = (value & 0x07) as i32;
        }
        self.mbc5_write(address, value);
    }

    /// `_GBBBDRead`, `_GBHitekRead`, `_GBGGB81Read`
    pub(crate) fn reordered_read(&self, address: u16, table: u8) -> u8 {
        let mem = &self.memory;
        let byte = mem.bank_byte(mem.rom_bank, address as usize & (SIZE_CART_BANK0 - 1));
        if address >> 14 != 1 {
            return byte;
        }
        let mode = mem.mbc_state.bbd.data_swap_mode as usize & 7;
        let reorder = match table {
            0 => &BBD_DATA_REORDERING[mode],
            1 => &HITEK_DATA_REORDERING[mode],
            _ => &GGB81_DATA_REORDERING[mode],
        };
        reorder_bits(byte, reorder)
    }

    /// `_GBLiCheng`
    pub(crate) fn li_cheng_write(&mut self, address: u16, value: u8) {
        if address > 0x2100 && address < 0x3000 {
            return;
        }
        self.mbc5_write(address, value);
    }

    /// `_GBSachen`
    pub(crate) fn sachen_write(&mut self, address: u16, value: u8) {
        let mut bank = value;
        match address >> 13 {
            0 => {
                let st = &mut self.memory.mbc_state.sachen;
                if (st.unmasked_bank & 0x30) == 0x30 {
                    st.base_bank = bank;
                    let b = (st.base_bank & st.mask) as i32;
                    self.mbc_switch_bank0(b);
                }
            }
            1 => {
                if bank == 0 {
                    bank = 1;
                }
                let st = &mut self.memory.mbc_state.sachen;
                st.unmasked_bank = bank;
                let bank = (bank & !st.mask) | (st.base_bank & st.mask);
                self.mbc_switch_bank(bank as i32);
            }
            2 => {
                let st = &mut self.memory.mbc_state.sachen;
                if (st.unmasked_bank & 0x30) == 0x30 {
                    st.mask = value;
                    let bank = (st.unmasked_bank & !st.mask) | (st.base_bank & st.mask);
                    let bank0 = (st.base_bank & st.mask) as i32;
                    self.mbc_switch_bank(bank as i32);
                    self.mbc_switch_bank0(bank0);
                }
            }
            6 => {
                let st = &mut self.memory.mbc_state.sachen;
                if self.memory.mbc_type == UNL_SACHEN_MMC2 && st.locked == SACHEN_LOCKED_DMG {
                    st.locked = SACHEN_LOCKED_CGB;
                    st.transition = 0;
                }
            }
            _ => {}
        }
    }

    fn sachen_rom_read(&self, address: u16) -> u8 {
        let mem = &self.memory;
        if address < 0x4000 {
            mem.rom_base_byte(address as usize)
        } else if address < 0x8000 {
            mem.bank_byte(mem.rom_bank, address as usize & (SIZE_CART_BANK0 - 1))
        } else {
            0xFF
        }
    }

    /// `_GBSachenMMC1Read`
    pub(crate) fn sachen_mmc1_read(&mut self, address: u16) -> u8 {
        let mut address = address;
        let st = &mut self.memory.mbc_state.sachen;
        if st.locked != SACHEN_UNLOCKED && (address & 0xFF00) == 0x100 {
            st.transition += 1;
            if st.transition == 0x31 {
                st.locked = SACHEN_UNLOCKED;
            } else {
                address |= 0x80;
            }
        }
        if (address & 0xFF00) == 0x0100 {
            address = unscramble_sachen(address);
        }
        self.sachen_rom_read(address)
    }

    /// `_GBSachenMMC2Read`
    pub(crate) fn sachen_mmc2_read(&mut self, address: u16) -> u8 {
        let mut address = address;
        let st = &mut self.memory.mbc_state.sachen;
        if address >= 0xC000 && st.locked == SACHEN_LOCKED_DMG {
            st.transition = 0;
            st.locked = SACHEN_LOCKED_CGB;
        }
        if st.locked != SACHEN_UNLOCKED && (address & 0x8700) == 0x0100 {
            st.transition += 1;
            if st.transition == 0x31 {
                st.locked += 1;
                st.transition = 0;
            }
        }
        if (address & 0xFF00) == 0x0100 {
            if st.locked == SACHEN_LOCKED_CGB {
                address |= 0x80;
            }
            address = unscramble_sachen(address);
        }
        self.sachen_rom_read(address)
    }

    /// `_GBSintax`
    pub(crate) fn sintax_write(&mut self, address: u16, value: u8) {
        let mut value = value;
        if (0x2000..0x3000).contains(&address) {
            let st = &mut self.memory.mbc_state.sintax;
            st.bank_no = value;
            value = reorder_bits(value, &SINTAX_REORDERING[st.mode as usize & 0xF]);
            st.rom_bank_xor = st.xor_values[(st.bank_no & 0x3) as usize];
        }
        if (address & 0xF0F0) == 0x5010 {
            // contrary to previous belief it IS possible to change the mode after setting it initially
            let st = &mut self.memory.mbc_state.sintax;
            st.mode = value & 0xF;
            let bank_no = st.bank_no;
            self.sintax_write(0x2000, bank_no); // fake a bank switch to select the correct bank
            return;
        }
        if (0x7000..0x8000).contains(&address) {
            let st = &mut self.memory.mbc_state.sintax;
            let xor_no = (address & 0x00F0) >> 4;
            if (2..=5).contains(&xor_no) {
                st.xor_values[xor_no as usize - 2] = value;
            }
            // xor is applied immediately to the current bank
            st.rom_bank_xor = st.xor_values[(st.bank_no & 0x3) as usize];
        }
        self.mbc5_write(address, value);
    }

    /// `_GBSintaxRead`
    pub(crate) fn sintax_read(&self, address: u16) -> u8 {
        let mem = &self.memory;
        match address >> 13 {
            0x2 | 0x3 => mem.bank_byte(mem.rom_bank, address as usize & (SIZE_CART_BANK0 - 1)) ^ mem.mbc_state.sintax.rom_bank_xor,
            0x5 => {
                if mem.sram_access && !mem.sram.is_empty() {
                    mem.sram_bank_byte(address as usize & (SIZE_EXTERNAL_RAM - 1))
                } else {
                    0xFF
                }
            }
            _ => 0xFF,
        }
    }

    // ----- Save file suffixes (struct GBMBCHuC3SaveBuffer / GBMBCTAMA5SaveBuffer) -----

    pub(crate) fn huc3_save_suffix(&self) -> Vec<u8> {
        let regs = &self.memory.mbc_state.huc3.registers;
        let mut out: Vec<u8> = (0..0x80).map(|i| (regs[i * 2] & 0xF) | (regs[i * 2 + 1] << 4)).collect();
        out.extend_from_slice(&(self.memory.rtc_last_latch as u64).to_le_bytes());
        out
    }

    pub(crate) fn huc3_load_suffix(&mut self, data: &[u8]) {
        if data.len() < 0x88 {
            return;
        }
        let regs = &mut self.memory.mbc_state.huc3.registers;
        for i in 0..0x80 {
            regs[i * 2] = data[i] & 0xF;
            regs[i * 2 + 1] = data[i] >> 4;
        }
        self.memory.rtc_last_latch = u64::from_le_bytes(data[0x80..0x88].try_into().unwrap()) as i64;
    }

    pub(crate) fn tama5_save_suffix(&self) -> Vec<u8> {
        let t = &self.memory.mbc_state.tama5;
        let mut out = Vec::with_capacity(40);
        for page in [&t.rtc_timer_page, &t.rtc_alarm_page, &t.rtc_free_page0, &t.rtc_free_page1] {
            out.extend((0..8).map(|i| (page[i * 2] & 0xF) | (page[i * 2 + 1] << 4)));
        }
        out.extend_from_slice(&(self.memory.rtc_last_latch as u64).to_le_bytes());
        out
    }

    pub(crate) fn tama5_load_suffix(&mut self, data: &[u8]) {
        let t = &mut self.memory.mbc_state.tama5;
        if data.len() < 40 {
            t.disabled = false;
            return;
        }
        for (p, page) in [&mut t.rtc_timer_page, &mut t.rtc_alarm_page, &mut t.rtc_free_page0, &mut t.rtc_free_page1]
            .into_iter()
            .enumerate()
        {
            for i in 0..8 {
                page[i * 2] = data[p * 8 + i] & 0xF;
                page[i * 2 + 1] = data[p * 8 + i] >> 4;
            }
        }
        self.memory.rtc_last_latch = u64::from_le_bytes(data[32..40].try_into().unwrap()) as i64;
        t.disabled = t.rtc_timer_page[RTC_PAGE] & 0x8 == 0;
        t.rtc_timer_page[RTC_PAGE] &= 0xC;
        t.rtc_alarm_page[RTC_PAGE] &= 0xC;
        t.rtc_alarm_page[RTC_PAGE] |= 1;
        t.rtc_free_page0[RTC_PAGE] &= 0xC;
        t.rtc_free_page0[RTC_PAGE] |= 2;
        t.rtc_free_page1[RTC_PAGE] &= 0xC;
        t.rtc_free_page1[RTC_PAGE] |= 3;
    }
}

/// `_isWisdomTree`
pub(crate) fn is_wisdom_tree(mem: &[u8], size: usize) -> bool {
    if mem.len() < 0x300 || size > mem.len() {
        return false;
    }
    if mem[0x134..0x14C].iter().any(|&b| b != 0) || mem[0xF0..0x100].iter().any(|&b| b != 0) {
        return false;
    }
    if mem[0x14D] != 0xE7 {
        return false;
    }
    let mut i = 0x300;
    while i + 11 < size {
        if &mem[i..i + 6] == b"WISDOM" && &mem[i + 7..i + 11] == b"TREE" {
            return true;
        }
        i += 1;
    }
    false
}
