// Copyright (c) 2013-2015 Jeffrey Pfau
// Rust port of mGBA's cartridge GPIO and real-time clock (src/gba/cart/gpio.c).
// Gyro, rumble, light and tilt sensors are recognized but not driven.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::Gba;

pub const HW_NO_OVERRIDE: u32 = 0x8000;
pub const HW_NONE: u32 = 0;
pub const HW_RTC: u32 = 1;
pub const HW_RUMBLE: u32 = 2;
pub const HW_LIGHT_SENSOR: u32 = 4;
pub const HW_GYRO: u32 = 8;
pub const HW_TILT: u32 = 16;
pub const HW_EREADER: u32 = 128;
pub const HW_GPIO: u32 = HW_RTC | HW_RUMBLE | HW_LIGHT_SENSOR | HW_GYRO | HW_TILT;

pub const GPIO_REG_DATA: u32 = 0xC4;
pub const GPIO_REG_DIRECTION: u32 = 0xC6;
pub const GPIO_REG_CONTROL: u32 = 0xC8;

const RTC_RESET: u32 = 0;
const RTC_DATETIME: u32 = 2;
const RTC_FORCE_IRQ: u32 = 3;
const RTC_CONTROL: u32 = 4;
const RTC_TIME: u32 = 6;

const RTC_BYTES: [i32; 8] = [0, 0, 7, 0, 1, 0, 3, 0];

pub fn is_gpio_register(reg: u32) -> bool {
    reg == GPIO_REG_DATA || reg == GPIO_REG_DIRECTION || reg == GPIO_REG_CONTROL
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default)]
pub struct Rtc {
    pub bytes_remaining: i32,
    pub bits_read: i32,
    pub bits: i32,
    pub command_active: bool,
    pub sck_edge: bool,
    pub sio_output: bool,
    pub command: u32,
    pub control: u8,
    pub time: [u8; 7],
    pub last_latch: i64,
    /// Seconds subtracted from the host clock.
    pub offset: i64,
}

/// `struct GBACartridgeHardware`
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub struct CartridgeHardware {
    pub devices: u32,
    pub read_write: bool,
    pub write_latch: u8,
    pub pin_state: u8,
    pub direction: u8,
    pub rtc: Rtc,
    /// The three GPIO registers as they appear in ROM when readable.
    pub regs_visible: [u16; 3],
    /// Once the game touches GPIO, the registers shadow ROM bytes 0xC4-0xC9.
    pub gpio_written: bool,
}

impl Default for CartridgeHardware {
    fn default() -> Self {
        CartridgeHardware {
            devices: HW_NO_OVERRIDE,
            read_write: false,
            write_latch: 0,
            pin_state: 0,
            direction: 0,
            rtc: Rtc::default(),
            regs_visible: [0; 3],
            gpio_written: false,
        }
    }
}

impl CartridgeHardware {
    /// `GBAHardwareInit`
    pub fn init(&mut self) {
        self.clear();
        self.gpio_written = false;
        self.regs_visible = [0; 3];
    }

    /// `GBAHardwareReset`
    pub fn reset(&mut self) {
        self.read_write = false;
        self.write_latch = 0;
        self.pin_state = 0;
        self.direction = 0;
    }

    /// `GBAHardwareClear`
    pub fn clear(&mut self) {
        self.devices = HW_NO_OVERRIDE;
        self.read_write = false;
        self.write_latch = 0;
        self.pin_state = 0;
        self.direction = 0;
    }

    pub fn has_gpio(&self) -> bool {
        self.devices & HW_GPIO != 0
    }

    /// `GBAHardwareInitRTC`
    pub fn init_rtc(&mut self) {
        self.devices &= !HW_NO_OVERRIDE;
        self.devices |= HW_RTC;
        let offset = self.rtc.offset;
        self.rtc = Rtc { sck_edge: true, sio_output: true, control: 0x40, offset, ..Default::default() };
    }

    fn output_pins(&mut self, pins: u8) {
        self.pin_state &= self.direction;
        self.pin_state |= pins & !self.direction & 0xF;
        if self.read_write {
            self.regs_visible[0] = self.pin_state as u16;
        }
    }

    fn read_pins(&mut self) {
        if self.devices & HW_RTC != 0 {
            self.rtc_read_pins();
        }
    }

    fn rtc_read_pins(&mut self) {
        // P: 0 - SCK | 1 - SIO | 2 - CS | 3 - Unused
        self.output_pins(self.pin_state & 2);

        if self.pin_state & 4 == 0 {
            self.rtc.bits_read = 0;
            self.rtc.bytes_remaining = 0;
            self.rtc.command_active = false;
            self.rtc.command = 0;
            self.rtc.sck_edge = true;
            self.rtc.sio_output = true;
            self.output_pins(2);
            return;
        }

        let reading = self.rtc.command & 0x80 != 0;
        if !self.rtc.command_active {
            self.output_pins(2);
            if self.pin_state & 1 == 0 {
                self.rtc.bits &= !(1 << self.rtc.bits_read);
                self.rtc.bits |= (((self.pin_state & 2) >> 1) as i32) << self.rtc.bits_read;
            }
            if !self.rtc.sck_edge && (self.pin_state & 1) != 0 {
                self.rtc.bits_read += 1;
                if self.rtc.bits_read == 8 {
                    self.rtc_begin_command();
                }
            }
        } else if !reading {
            self.output_pins(2);
            if self.pin_state & 1 == 0 {
                self.rtc.bits &= !(1 << self.rtc.bits_read);
                self.rtc.bits |= (((self.pin_state & 2) >> 1) as i32) << self.rtc.bits_read;
            }
            if !self.rtc.sck_edge && (self.pin_state & 1) != 0 {
                if ((self.rtc.bits >> self.rtc.bits_read) & 1) ^ (((self.pin_state & 2) >> 1) as i32) != 0 {
                    self.rtc.bits &= !(1 << self.rtc.bits_read);
                }
                self.rtc.bits_read += 1;
                if self.rtc.bits_read == 8 {
                    self.rtc_process_byte();
                }
            }
        } else {
            if self.rtc.sck_edge && self.pin_state & 1 == 0 {
                self.rtc.sio_output = self.rtc_output() != 0;
                self.rtc.bits_read += 1;
                if self.rtc.bits_read == 8 {
                    self.rtc.bytes_remaining -= 1;
                    if self.rtc.bytes_remaining <= 0 {
                        self.rtc.bytes_remaining = RTC_BYTES[((self.rtc.command >> 4) & 7) as usize];
                    }
                    self.rtc.bits_read = 0;
                }
            }
            self.output_pins((self.rtc.sio_output as u8) << 1);
        }

        self.rtc.sck_edge = self.pin_state & 1 != 0;
    }

    fn rtc_begin_command(&mut self) {
        let command = self.rtc.bits as u32;
        if command & 0xF == 0x06 {
            self.rtc.command = command;
            let cmd = (command >> 4) & 7;
            self.rtc.bytes_remaining = RTC_BYTES[cmd as usize];
            self.rtc.command_active = true;
            match cmd {
                RTC_RESET => self.rtc.control = 0,
                RTC_DATETIME | RTC_TIME => self.rtc_update_clock(),
                _ => {}
            }
        }
        self.rtc.bits = 0;
        self.rtc.bits_read = 0;
    }

    fn rtc_process_byte(&mut self) {
        if (self.rtc.command >> 4) & 7 == RTC_CONTROL {
            self.rtc.control = self.rtc.bits as u8;
        }
        self.rtc.bits = 0;
        self.rtc.bits_read = 0;
        self.rtc.bytes_remaining -= 1;
        if self.rtc.bytes_remaining <= 0 {
            self.rtc.bytes_remaining = RTC_BYTES[((self.rtc.command >> 4) & 7) as usize];
        }
    }

    fn rtc_output(&self) -> u32 {
        let output_byte: u8 = match (self.rtc.command >> 4) & 7 {
            RTC_CONTROL => self.rtc.control,
            RTC_DATETIME | RTC_TIME => {
                let idx = 7 - self.rtc.bytes_remaining;
                if (0..7).contains(&idx) {
                    self.rtc.time[idx as usize]
                } else {
                    0xFF
                }
            }
            RTC_FORCE_IRQ | RTC_RESET => 0xFF,
            _ => 0xFF,
        };
        ((output_byte >> self.rtc.bits_read) & 1) as u32
    }

    fn rtc_update_clock(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.rtc.last_latch = now;
        let t = now - self.rtc.offset;
        let (year, month, day, wday, hour, min, sec) = civil_from_unix(t);
        self.rtc.time[0] = bcd((year - 2000).rem_euclid(100) as u32);
        self.rtc.time[1] = bcd(month);
        self.rtc.time[2] = bcd(day);
        self.rtc.time[3] = bcd(wday);
        self.rtc.time[4] = if self.rtc.control & 0x40 != 0 { bcd(hour) } else { bcd(hour % 12) };
        self.rtc.time[5] = bcd(min);
        self.rtc.time[6] = bcd(sec);
    }
}

/// `_rtcBCD`
fn bcd(value: u32) -> u8 {
    let mut counter = value % 10;
    let mut v = value / 10;
    counter += (v % 10) << 4;
    v /= 10;
    let _ = v;
    counter as u8
}

/// Convert unix seconds to (year, month, day, weekday, hour, minute, second), UTC.
fn civil_from_unix(t: i64) -> (i64, u32, u32, u32, u32, u32, u32) {
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    let (hour, min, sec) = ((secs / 3600) as u32, ((secs % 3600) / 60) as u32, (secs % 60) as u32);
    let wday = ((days + 4).rem_euclid(7)) as u32; // 1970-01-01 was a Thursday
    // Howard Hinnant's days_from_civil inverse
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, wday, hour, min, sec)
}

impl Gba {
    /// `GBAHardwareGPIOWrite`
    pub(crate) fn gpio_write(&mut self, address: u32, value: u16) {
        let vba = self.vba_bug_compat;
        let hw = &mut self.memory.hw;
        match address {
            GPIO_REG_DATA => {
                hw.write_latch = (value & 0xF) as u8;
                if !vba {
                    hw.pin_state &= !hw.direction;
                    hw.pin_state |= hw.write_latch & hw.direction;
                } else {
                    hw.pin_state = hw.write_latch;
                }
                hw.read_pins();
            }
            GPIO_REG_DIRECTION => {
                hw.direction = (value & 0xF) as u8;
                if !vba {
                    hw.pin_state &= !hw.direction;
                    hw.pin_state |= hw.write_latch & hw.direction;
                    hw.read_pins();
                }
            }
            GPIO_REG_CONTROL => hw.read_write = value & 1 != 0,
            _ => {}
        }
        if hw.read_write {
            hw.regs_visible = [hw.pin_state as u16, hw.direction as u16, hw.read_write as u16];
        } else {
            hw.regs_visible = [0; 3];
        }
        hw.gpio_written = true;
    }
}
