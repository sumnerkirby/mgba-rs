// Copyright (c) 2013-2015 Jeffrey Pfau
// Rust port of mGBA's serial I/O (src/gba/sio.c). With no driver attached,
// transfers complete as if nothing is connected; the link cable is in
// `sio_lockstep.rs`.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::{glog, regs, Gba, LogLevel, ARM7TDMI_FREQUENCY, IRQ_SIO};
use super::sio_lockstep::MAX_GBAS;
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

/// `GBASIOTransferCycles`
pub(crate) fn transfer_cycles(mode: i32, siocnt: u16, connected: i32) -> i32 {
    if !(0..MAX_GBAS as i32).contains(&connected) {
        return 0;
    }
    let internal = siocnt & 2 != 0;
    match mode {
        SIO_MULTI => CYCLES_PER_TRANSFER[(siocnt & 3) as usize][connected as usize],
        SIO_NORMAL_8 => (8 * ARM7TDMI_FREQUENCY / ((if internal { 2048 } else { 256 }) * 1024)) as i32,
        SIO_NORMAL_32 => (32 * ARM7TDMI_FREQUENCY / ((if internal { 2048 } else { 256 }) * 1024)) as i32,
        _ => 0,
    }
}

impl Gba {
    /// `_switchMode`
    fn sio_switch_mode(&mut self) {
        let mode = (((self.sio.rcnt & 0xC000) | (self.sio.siocnt & 0x3000)) >> 12) as i32;
        let new_mode = if mode < 8 { mode & 0x3 } else { mode & 0xC };
        if new_mode != self.sio.mode {
            self.sio.mode = new_mode;
            if self.sio_driver.is_some() {
                self.sio_driver_set_mode(new_mode);
            }

            if new_mode == SIO_MULTI {
                let id = if self.sio_driver.is_some() { self.sio_driver_device_id() } else { 0 };
                // RCNT.SI reflects whether this is a secondary
                self.sio.rcnt = (self.sio.rcnt & !4) | (((id != 0) as u16) << 2);
            }
        }
    }

    /// `GBASIOReset`
    pub(crate) fn sio_reset(&mut self) {
        if self.sio_driver.is_some() {
            self.sio_driver_reset();
        }
        self.sio.rcnt = RCNT_INITIAL;
        self.sio.siocnt = 0;
        self.sio.mode = -1;
        self.sio_switch_mode();
    }

    /// `GBASIOWriteRCNT` (the lockstep driver's `writeRCNT` returns the value unchanged)
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

    /// `_startTransfer`
    fn sio_start_transfer(&mut self) {
        if self.sio_driver.is_some() && !self.sio_driver_start() {
            // Transfer completion is handled internally to the driver
            return;
        }
        let connected = if self.sio_driver.is_some() { self.sio_driver_connected_devices() } else { 0 };
        self.deschedule(EventId::Sio);
        let cycles = transfer_cycles(self.sio.mode, self.sio.siocnt, connected);
        self.schedule(EventId::Sio, cycles);
    }

    /// `GBASIOWriteSIOCNT`
    pub(crate) fn sio_write_siocnt(&mut self, mut value: u16) {
        if (value ^ self.sio.siocnt) & 0x3000 != 0 {
            self.sio.siocnt = value & 0x3000;
            self.sio_switch_mode();
        }
        let mut id = 0;
        let mut connected = 0;
        // The lockstep driver handles every mode and has a writeSIOCNT that
        // returns the value unchanged.
        let handled = self.sio_driver.is_some();
        if handled {
            id = self.sio_driver_device_id();
            connected = self.sio_driver_connected_devices();
        }

        match self.sio.mode {
            SIO_MULTI => {
                value &= 0xFF83;
                let slave = id != 0 || connected == 0;
                value = (value & !(1 << 2)) | ((slave as u16) << 2);
                value = (value & !(3 << 4)) | (((id & 3) as u16) << 4);
                value |= self.sio.siocnt & 0x00FC;

                // SC appears to float in multi mode when not doing a transfer. While
                // it does spike at the end of a transfer, it appears to die down after
                // around 20-30 microseconds. However, the docs on akkit.org
                // (http://www.akkit.org/info/gba_comms.html) say this is high until
                // a transfer starts and low while active. Further, the Mario Bros.
                // multiplayer expects SC to be high in multi mode. This needs better
                // investigation than I managed, apparently.
                self.sio.rcnt |= 1;

                if value & 0x80 != 0 && self.sio.siocnt & 0x80 == 0 {
                    if id == 0 {
                        for reg in [regs::SIOMULTI0, regs::SIOMULTI1, regs::SIOMULTI2, regs::SIOMULTI3] {
                            self.memory.io[(reg >> 1) as usize] = 0xFFFF;
                        }
                        self.sio.rcnt &= !1;
                        self.sio_start_transfer();
                    } else {
                        // TODO
                    }
                }
            }
            SIO_NORMAL_8 | SIO_NORMAL_32 => {
                // This line is pulled up by the clock owner while the clock is idle.
                // If there is no clock owner it's just hi-Z.
                if value & 1 != 0 {
                    self.sio.rcnt |= 1;
                }
                if value & 0x80 != 0 && self.sio.siocnt & 0x80 == 0 {
                    self.sio_start_transfer();
                }
            }
            _ => {
                // TODO
            }
        }
        if !handled {
            // Dummy drivers
            match self.sio.mode {
                SIO_NORMAL_8 | SIO_NORMAL_32 => value |= 1 << 2, // Si
                SIO_MULTI => value |= 1 << 3,                    // Ready
                _ => {}
            }
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

    /// `GBASIOMultiplayerFinishTransfer`
    fn sio_multiplayer_finish_transfer(&mut self, data: [u16; 4], cycles_late: u32) {
        let id = if self.sio_driver.is_some() { self.sio_driver_device_id() } else { 0 };
        for (reg, d) in [regs::SIOMULTI0, regs::SIOMULTI1, regs::SIOMULTI2, regs::SIOMULTI3].into_iter().zip(data) {
            self.memory.io[(reg >> 1) as usize] = d;
        }
        self.sio.siocnt &= !0x80;
        self.sio.siocnt = (self.sio.siocnt & !(3 << 4)) | (((id & 3) as u16) << 4);
        self.sio.rcnt |= 1;
        if self.sio.siocnt & 0x4000 != 0 {
            self.raise_irq(IRQ_SIO, cycles_late);
        }
    }

    /// `GBASIONormal8FinishTransfer`
    fn sio_normal8_finish_transfer(&mut self, data: u8, cycles_late: u32) {
        self.sio.siocnt &= !0x80;
        self.memory.io[(regs::SIODATA8 >> 1) as usize] = data as u16;
        if self.sio.siocnt & 0x4000 != 0 {
            self.raise_irq(IRQ_SIO, cycles_late);
        }
    }

    /// `GBASIONormal32FinishTransfer`
    fn sio_normal32_finish_transfer(&mut self, data: u32, cycles_late: u32) {
        self.sio.siocnt &= !0x80;
        self.memory.io[(regs::SIODATA32_LO >> 1) as usize] = data as u16;
        self.memory.io[(regs::SIODATA32_HI >> 1) as usize] = (data >> 16) as u16;
        if self.sio.siocnt & 0x4000 != 0 {
            self.raise_irq(IRQ_SIO, cycles_late);
        }
    }

    /// `_sioFinish`
    pub(crate) fn sio_finish(&mut self, cycles_late: u32) {
        let linked = self.sio_driver.is_some();
        match self.sio.mode {
            SIO_MULTI => {
                let mut data = [0u16; 4];
                if linked {
                    self.sio_driver_finish_multiplayer(&mut data);
                }
                self.sio_multiplayer_finish_transfer(data, cycles_late);
            }
            SIO_NORMAL_8 => {
                let data = if linked { self.sio_driver_finish_normal8() } else { 0 };
                self.sio_normal8_finish_transfer(data, cycles_late);
            }
            SIO_NORMAL_32 => {
                let data = if linked { self.sio_driver_finish_normal32() } else { 0 };
                self.sio_normal32_finish_transfer(data, cycles_late);
            }
            _ => {
                // TODO
                glog!(self, LogLevel::Stub, "No dummy finish implemented for SIO mode {}", self.sio.mode);
            }
        }
    }
}
