// Copyright (c) 2013-2015 Jeffrey Pfau
// Rust port of mGBA's serial I/O (src/gba/sio.c), without link-cable drivers:
// transfers complete as if nothing is connected.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::{glog, regs, Gba, LogLevel, ARM7TDMI_FREQUENCY, IRQ_SIO};
use crate::timing::EventId;

pub const RCNT_INITIAL: u16 = 0x8000;
pub const JOYSTAT_TRANS: u16 = 8;
pub const JOYSTAT_RECV: u16 = 2;

pub const SIO_NORMAL_8: i32 = 0;
pub const SIO_NORMAL_32: i32 = 1;
pub const SIO_MULTI: i32 = 2;
pub const SIO_UART: i32 = 3;
pub const SIO_GPIO: i32 = 8;
pub const SIO_JOYBUS: i32 = 12;

const CYCLES_PER_TRANSFER: [[i32; 4]; 4] = [
    [31976, 63427, 94884, 125829],
    [8378, 16241, 24104, 31457],
    [5750, 10998, 16241, 20972],
    [3140, 5755, 8376, 10486],
];

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub struct Sio {
    pub rcnt: u16,
    pub siocnt: u16,
    pub mode: i32,
}

impl Default for Sio {
    fn default() -> Self {
        Sio { rcnt: RCNT_INITIAL, siocnt: 0, mode: -1 }
    }
}

fn transfer_cycles(mode: i32, siocnt: u16) -> i32 {
    let internal = siocnt & 2 != 0;
    match mode {
        SIO_MULTI => CYCLES_PER_TRANSFER[(siocnt & 3) as usize][0],
        SIO_NORMAL_8 => (8 * ARM7TDMI_FREQUENCY / ((if internal { 2048 } else { 256 }) * 1024)) as i32,
        SIO_NORMAL_32 => (32 * ARM7TDMI_FREQUENCY / ((if internal { 2048 } else { 256 }) * 1024)) as i32,
        _ => 0,
    }
}

impl Gba {
    fn sio_switch_mode(&mut self) {
        let mode = (((self.sio.rcnt & 0xC000) | (self.sio.siocnt & 0x3000)) >> 12) as i32;
        let new_mode = if mode < 8 { mode & 0x3 } else { mode & 0xC };
        if new_mode != self.sio.mode {
            self.sio.mode = new_mode;
            if new_mode == SIO_MULTI {
                // RCNT.SI reflects the device id (0 when unconnected)
                self.sio.rcnt &= !4;
            }
        }
    }

    /// `GBASIOReset`
    pub(crate) fn sio_reset(&mut self) {
        self.sio.rcnt = RCNT_INITIAL;
        self.sio.siocnt = 0;
        self.sio.mode = -1;
        self.sio_switch_mode();
    }

    /// `GBASIOWriteRCNT`
    pub(crate) fn sio_write_rcnt(&mut self, value: u16) {
        self.sio.rcnt &= 0x1FF;
        self.sio.rcnt |= value & 0xC000;
        self.sio_switch_mode();
        if self.sio.mode == SIO_GPIO {
            self.sio.rcnt &= 0xC000;
            self.sio.rcnt |= value & 0x1FF;
        } else {
            self.sio.rcnt &= 0xC00F;
            self.sio.rcnt |= value & 0x1F0;
        }
    }

    fn sio_start_transfer(&mut self) {
        self.deschedule(EventId::Sio);
        let cycles = transfer_cycles(self.sio.mode, self.sio.siocnt);
        self.schedule(EventId::Sio, cycles);
    }

    /// `GBASIOWriteSIOCNT`
    pub(crate) fn sio_write_siocnt(&mut self, mut value: u16) {
        if (value ^ self.sio.siocnt) & 0x3000 != 0 {
            self.sio.siocnt = value & 0x3000;
            self.sio_switch_mode();
        }
        match self.sio.mode {
            SIO_MULTI => {
                value &= 0xFF83;
                value |= 1 << 2; // Slave (no connection)
                value &= !(3 << 4); // Id 0
                value |= self.sio.siocnt & 0x00FC;
                self.sio.rcnt |= 1; // SC
                if value & 0x80 != 0 && self.sio.siocnt & 0x80 == 0 {
                    for reg in [regs::SIOMULTI0, regs::SIOMULTI1, regs::SIOMULTI2, regs::SIOMULTI3] {
                        self.memory.io[(reg >> 1) as usize] = 0xFFFF;
                    }
                    self.sio.rcnt &= !1;
                    self.sio_start_transfer();
                }
                value |= 1 << 3; // Ready
            }
            SIO_NORMAL_8 | SIO_NORMAL_32 => {
                if value & 1 != 0 {
                    self.sio.rcnt |= 1;
                }
                if value & 0x80 != 0 && self.sio.siocnt & 0x80 == 0 {
                    self.sio_start_transfer();
                }
                value |= 1 << 2; // Si
            }
            _ => {}
        }
        self.sio.siocnt = value;
    }

    /// `GBASIOWriteRegister`
    pub(crate) fn sio_write_register(&mut self, address: u32, mut value: u16) -> u16 {
        let joycnt = self.memory.io[(regs::JOYCNT >> 1) as usize];
        let handled = match self.sio.mode {
            SIO_JOYBUS => match address {
                regs::SIODATA8 | regs::JOY_TRANS_LO | regs::JOY_TRANS_HI => true,
                regs::JOYCNT => {
                    value = (value & 0x0040) | (joycnt & !(value & 0x7) & !0x0040);
                    true
                }
                regs::JOYSTAT => {
                    value = (value & 0x0030) | (self.memory.io[(regs::JOYSTAT >> 1) as usize] & !0x30);
                    true
                }
                _ => false,
            },
            SIO_NORMAL_8 => match address {
                regs::SIODATA8 => true,
                regs::JOYCNT => {
                    value = (value & 0x0040) | (joycnt & !(value & 0x7) & !0x0040);
                    true
                }
                _ => false,
            },
            SIO_NORMAL_32 => match address {
                regs::SIODATA32_LO | regs::SIODATA32_HI | regs::SIODATA8 => true,
                regs::JOYCNT => {
                    value = (value & 0x0040) | (joycnt & !(value & 0x7) & !0x0040);
                    true
                }
                _ => false,
            },
            SIO_MULTI => match address {
                regs::SIOMLT_SEND => true,
                regs::JOYCNT => {
                    value = (value & 0x0040) | (joycnt & !(value & 0x7) & !0x0040);
                    true
                }
                _ => false,
            },
            SIO_UART => match address {
                regs::SIODATA8 => true,
                regs::JOYCNT => {
                    value = (value & 0x0040) | (joycnt & !(value & 0x7) & !0x0040);
                    true
                }
                _ => false,
            },
            _ => false,
        };
        if !handled {
            value = self.memory.io[(address >> 1) as usize];
        }
        value
    }

    /// `_sioFinish` with no driver attached.
    pub(crate) fn sio_finish(&mut self, cycles_late: u32) {
        match self.sio.mode {
            SIO_MULTI => {
                for reg in [regs::SIOMULTI0, regs::SIOMULTI1, regs::SIOMULTI2, regs::SIOMULTI3] {
                    self.memory.io[(reg >> 1) as usize] = 0;
                }
                self.sio.siocnt &= !0x80;
                self.sio.siocnt &= !(3 << 4);
                self.sio.rcnt |= 1;
                if self.sio.siocnt & 0x4000 != 0 {
                    self.raise_irq(IRQ_SIO, cycles_late);
                }
            }
            SIO_NORMAL_8 => {
                self.sio.siocnt &= !0x80;
                self.memory.io[(regs::SIODATA8 >> 1) as usize] = 0;
                if self.sio.siocnt & 0x4000 != 0 {
                    self.raise_irq(IRQ_SIO, cycles_late);
                }
            }
            SIO_NORMAL_32 => {
                self.sio.siocnt &= !0x80;
                self.memory.io[(regs::SIODATA32_LO >> 1) as usize] = 0;
                self.memory.io[(regs::SIODATA32_HI >> 1) as usize] = 0;
                if self.sio.siocnt & 0x4000 != 0 {
                    self.raise_irq(IRQ_SIO, cycles_late);
                }
            }
            _ => {
                glog!(self, LogLevel::Stub, "No dummy finish implemented for SIO mode {}", self.sio.mode);
            }
        }
    }
}
