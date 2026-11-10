//! Game controller input.
//!
//! On Linux this reads evdev devices (`/dev/input/event*`) directly, so it
//! needs no system libraries. Controllers are picked up when plugged in.
//! The default mapping follows mGBA's SDL bindings: the south face button is
//! A, east is B, the shoulders are L/R, and the d-pad, hat and left stick
//! steer. On other platforms no controller is read.
//!
//! This Source Code Form is subject to the terms of the Mozilla Public
//! License, v. 2.0. If a copy of the MPL was not distributed with this
//! file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::app::Button;

/// Linux input event codes for gamepad buttons (`linux/input-event-codes.h`).
pub mod code {
    pub const BTN_SOUTH: u16 = 0x130;
    pub const BTN_EAST: u16 = 0x131;
    pub const BTN_NORTH: u16 = 0x133;
    pub const BTN_WEST: u16 = 0x134;
    pub const BTN_TL: u16 = 0x136;
    pub const BTN_TR: u16 = 0x137;
    pub const BTN_TL2: u16 = 0x138;
    pub const BTN_TR2: u16 = 0x139;
    pub const BTN_SELECT: u16 = 0x13A;
    pub const BTN_START: u16 = 0x13B;
    pub const BTN_MODE: u16 = 0x13C;
    pub const BTN_THUMBL: u16 = 0x13D;
    pub const BTN_THUMBR: u16 = 0x13E;
    pub const BTN_DPAD_UP: u16 = 0x220;
    pub const BTN_DPAD_DOWN: u16 = 0x221;
    pub const BTN_DPAD_LEFT: u16 = 0x222;
    pub const BTN_DPAD_RIGHT: u16 = 0x223;
}

/// mGBA's default controller bindings.
pub fn default_mapping() -> Vec<(Button, u16)> {
    use code::*;
    vec![
        (Button::A, BTN_SOUTH),
        (Button::B, BTN_EAST),
        (Button::L, BTN_TL),
        (Button::R, BTN_TR),
        (Button::Start, BTN_START),
        (Button::Select, BTN_SELECT),
        (Button::Up, BTN_DPAD_UP),
        (Button::Down, BTN_DPAD_DOWN),
        (Button::Left, BTN_DPAD_LEFT),
        (Button::Right, BTN_DPAD_RIGHT),
    ]
}

pub fn button_name(c: u16) -> String {
    use code::*;
    match c {
        BTN_SOUTH => "South (A/✕)".into(),
        BTN_EAST => "East (B/○)".into(),
        BTN_NORTH => "North (Y/△)".into(),
        BTN_WEST => "West (X/□)".into(),
        BTN_TL => "LB / L1".into(),
        BTN_TR => "RB / R1".into(),
        BTN_TL2 => "LT / L2".into(),
        BTN_TR2 => "RT / R2".into(),
        BTN_SELECT => "Select / Back".into(),
        BTN_START => "Start".into(),
        BTN_MODE => "Guide".into(),
        BTN_THUMBL => "Left stick click".into(),
        BTN_THUMBR => "Right stick click".into(),
        BTN_DPAD_UP => "D-pad up".into(),
        BTN_DPAD_DOWN => "D-pad down".into(),
        BTN_DPAD_LEFT => "D-pad left".into(),
        BTN_DPAD_RIGHT => "D-pad right".into(),
        other => format!("Button {other:#x}"),
    }
}

/// Directions read from the hat or left stick, as pseudo button codes so they
/// can share the binding table.
const DIRS: [u16; 4] = [code::BTN_DPAD_UP, code::BTN_DPAD_DOWN, code::BTN_DPAD_LEFT, code::BTN_DPAD_RIGHT];

#[derive(Default)]
pub struct Gamepads {
    #[cfg(target_os = "linux")]
    inner: linux::Pads,
}

impl Gamepads {
    pub fn new() -> Gamepads {
        Gamepads::default()
    }

    /// Read pending events. Returns the set of pressed button codes and the
    /// last newly pressed button (for rebinding).
    pub fn poll(&mut self) -> (Vec<u16>, Option<u16>) {
        #[cfg(target_os = "linux")]
        {
            self.inner.poll()
        }
        #[cfg(not(target_os = "linux"))]
        {
            (Vec::new(), None)
        }
    }

    /// Pressed button codes of each controller separately, as of the last
    /// [`Gamepads::poll`] (for giving a second player their own controller).
    pub fn pressed_per_pad(&self) -> Vec<Vec<u16>> {
        #[cfg(target_os = "linux")]
        {
            self.inner.per_pad.clone()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Vec::new()
        }
    }

    /// Names of connected controllers.
    pub fn connected(&self) -> Vec<String> {
        #[cfg(target_os = "linux")]
        {
            self.inner.names()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Vec::new()
        }
    }

    /// GBA key bits for the pressed buttons under `mapping`.
    pub fn keys(pressed: &[u16], mapping: &[(Button, u16)]) -> u16 {
        mapping.iter().filter(|(_, c)| pressed.contains(c)).fold(0, |acc, (b, _)| acc | b.bit())
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{code, DIRS};
    use std::collections::HashSet;
    use std::fs::OpenOptions;
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    const EV_KEY: u16 = 0x01;
    const EV_ABS: u16 = 0x03;
    const ABS_X: u16 = 0x00;
    const ABS_Y: u16 = 0x01;
    const ABS_HAT0X: u16 = 0x10;
    const ABS_HAT0Y: u16 = 0x11;
    const KEY_MAX: usize = 0x2FF;

    const fn ioc_read(nr: u32, size: u32) -> libc::c_ulong {
        ((2u32 << 30) | (size << 16) | ((b'E' as u32) << 8) | nr) as libc::c_ulong
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct AbsInfo {
        value: i32,
        minimum: i32,
        maximum: i32,
        fuzz: i32,
        flat: i32,
        resolution: i32,
    }

    struct Axis {
        min: i32,
        max: i32,
        value: i32,
    }

    impl Axis {
        /// -1, 0 or 1 with a 50% dead zone (hats report -1..1 directly).
        fn direction(&self) -> i32 {
            let mid = (self.min + self.max) / 2;
            let half = (self.max - self.min) / 2;
            if half <= 1 {
                return self.value.signum();
            }
            let threshold = half / 2;
            if self.value < mid - threshold {
                -1
            } else if self.value > mid + threshold {
                1
            } else {
                0
            }
        }
    }

    struct Pad {
        path: PathBuf,
        file: std::fs::File,
        name: String,
        buttons: HashSet<u16>,
        axes: [Option<Axis>; 4], // X, Y, HAT0X, HAT0Y
    }

    #[derive(Default)]
    pub struct Pads {
        pads: Vec<Pad>,
        last_scan: Option<Instant>,
        pub per_pad: Vec<Vec<u16>>,
    }

    fn test_bit(bits: &[u8], bit: usize) -> bool {
        bits.get(bit / 8).map(|b| b & (1 << (bit % 8)) != 0).unwrap_or(false)
    }

    fn open_pad(path: PathBuf) -> Option<Pad> {
        let file = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(&path).ok()?;
        let fd = file.as_raw_fd();
        let mut keys = [0u8; KEY_MAX / 8 + 1];
        // EVIOCGBIT(EV_KEY, len)
        let r = unsafe { libc::ioctl(fd, ioc_read(0x20 + EV_KEY as u32, keys.len() as u32), keys.as_mut_ptr()) };
        if r < 0 {
            return None;
        }
        // Gamepads report BTN_GAMEPAD (BTN_SOUTH); plain joysticks BTN_JOYSTICK (0x120).
        if !test_bit(&keys, code::BTN_SOUTH as usize) && !test_bit(&keys, 0x120) {
            return None;
        }
        let mut name = [0u8; 128];
        let r = unsafe { libc::ioctl(fd, ioc_read(0x06, name.len() as u32), name.as_mut_ptr()) };
        let name = if r > 0 {
            let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
            String::from_utf8_lossy(&name[..end]).to_string()
        } else {
            path.display().to_string()
        };
        let mut axes: [Option<Axis>; 4] = [None, None, None, None];
        for (i, abs) in [ABS_X, ABS_Y, ABS_HAT0X, ABS_HAT0Y].into_iter().enumerate() {
            let mut info = AbsInfo::default();
            // EVIOCGABS(abs)
            let r = unsafe {
                libc::ioctl(fd, ioc_read(0x40 + abs as u32, std::mem::size_of::<AbsInfo>() as u32), &mut info as *mut AbsInfo)
            };
            if r >= 0 && info.maximum > info.minimum {
                axes[i] = Some(Axis { min: info.minimum, max: info.maximum, value: info.value });
            }
        }
        Some(Pad { path, file, name, buttons: HashSet::new(), axes })
    }

    impl Pads {
        fn rescan(&mut self) {
            let Ok(dir) = std::fs::read_dir("/dev/input") else { return };
            for entry in dir.flatten() {
                let path = entry.path();
                let is_event = path.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with("event")).unwrap_or(false);
                if !is_event || self.pads.iter().any(|p| p.path == path) {
                    continue;
                }
                if let Some(pad) = open_pad(path) {
                    self.pads.push(pad);
                }
            }
        }

        pub fn names(&self) -> Vec<String> {
            self.pads.iter().map(|p| p.name.clone()).collect()
        }

        pub fn poll(&mut self) -> (Vec<u16>, Option<u16>) {
            if self.last_scan.map(|t| t.elapsed() > Duration::from_secs(2)).unwrap_or(true) {
                self.last_scan = Some(Instant::now());
                self.rescan();
            }
            let mut newly = None;
            let mut gone = vec![];
            for (i, pad) in self.pads.iter_mut().enumerate() {
                let mut buf = [0u8; 24 * 64];
                loop {
                    match pad.file.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            for ev in buf[..n].chunks_exact(24) {
                                let ty = u16::from_ne_bytes([ev[16], ev[17]]);
                                let c = u16::from_ne_bytes([ev[18], ev[19]]);
                                let value = i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]);
                                match ty {
                                    EV_KEY => {
                                        if value != 0 {
                                            if pad.buttons.insert(c) {
                                                newly = Some(c);
                                            }
                                        } else {
                                            pad.buttons.remove(&c);
                                        }
                                    }
                                    EV_ABS => {
                                        let slot = match c {
                                            ABS_X => 0,
                                            ABS_Y => 1,
                                            ABS_HAT0X => 2,
                                            ABS_HAT0Y => 3,
                                            _ => continue,
                                        };
                                        if let Some(axis) = &mut pad.axes[slot] {
                                            axis.value = value;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => {
                            gone.push(i);
                            break;
                        }
                    }
                }
            }
            for i in gone.into_iter().rev() {
                self.pads.remove(i);
            }
            let mut pressed: Vec<u16> = vec![];
            self.per_pad.clear();
            for pad in &self.pads {
                let start = pressed.len();
                pressed.extend(pad.buttons.iter().copied());
                let dir = |a: &Option<Axis>| a.as_ref().map(|a| a.direction()).unwrap_or(0);
                let x = dir(&pad.axes[2]) + dir(&pad.axes[0]);
                let y = dir(&pad.axes[3]) + dir(&pad.axes[1]);
                if y < 0 {
                    pressed.push(DIRS[0]);
                }
                if y > 0 {
                    pressed.push(DIRS[1]);
                }
                if x < 0 {
                    pressed.push(DIRS[2]);
                }
                if x > 0 {
                    pressed.push(DIRS[3]);
                }
                self.per_pad.push(pressed[start..].to_vec());
            }
            (pressed, newly)
        }
    }
}
