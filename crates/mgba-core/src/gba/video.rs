// Copyright (c) 2013-2015 Jeffrey Pfau
// Rust port of mGBA's GBA video timing (src/gba/video.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::{regs, Gba, IRQ_HBLANK, IRQ_VBLANK, IRQ_VCOUNTER, VIDEO_VERTICAL_PIXELS};
use crate::timing::EventId;

pub const VSTALL_A2: u16 = 0x100;
pub const VSTALL_A3: u16 = 0x200;
pub const VSTALL_B: u16 = 0x400;
pub const fn vstall_t4(x: u32) -> u16 {
    (0x011 << x) as u16
}
pub const fn vstall_t8(x: u32) -> u16 {
    (0x010 << x) as u16
}

pub const VIDEO_HBLANK_PIXELS: i32 = 68;
pub const VIDEO_HDRAW_LENGTH: i32 = 1008;
pub const VIDEO_HBLANK_LENGTH: i32 = 224;
pub const VIDEO_HORIZONTAL_LENGTH: i32 = 1232;
pub const VIDEO_VBLANK_PIXELS: i32 = 68;
pub const VIDEO_VERTICAL_TOTAL_PIXELS: i32 = 228;
pub const VIDEO_TOTAL_LENGTH: i32 = 280896;

pub const OBJ_SIZES: [[i32; 2]; 16] = [
    [8, 8],
    [16, 16],
    [32, 32],
    [64, 64],
    [16, 8],
    [32, 8],
    [32, 16],
    [64, 32],
    [8, 16],
    [8, 32],
    [16, 32],
    [32, 64],
    [0, 0],
    [0, 0],
    [0, 0],
    [0, 0],
];

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum VideoPhase {
    StartHblank,
    StartHdraw,
}

/// `struct GBAVideo`
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Video {
    pub next_phase: VideoPhase,
    pub vcount: i32,
    pub stall_mask: u32,
    pub palette: Vec<u16>,
    pub vram: Vec<u16>,
    pub oam: Vec<u16>,
    pub frame_counter: u32,
    pub frameskip: i32,
    pub frameskip_counter: i32,
}

impl Video {
    pub fn new() -> Video {
        Video {
            next_phase: VideoPhase::StartHblank,
            vcount: 0,
            stall_mask: 0,
            palette: vec![0; 512],
            vram: vec![0; 0x18000 / 2],
            oam: vec![0; 512],
            frame_counter: 0,
            frameskip: 0,
            frameskip_counter: 0,
        }
    }
}

impl Default for Video {
    fn default() -> Self {
        Self::new()
    }
}

impl Gba {
    /// `GBAVideoReset`
    pub(crate) fn video_reset(&mut self) {
        let mut next_event = VIDEO_HDRAW_LENGTH;
        if self.memory.full_bios {
            self.video.vcount = 0;
        } else {
            // TODO: Verify exact scanline on hardware
            self.video.vcount = 0x7E;
            next_event = 120;
        }
        self.memory.io[(regs::VCOUNT >> 1) as usize] = self.video.vcount as u16;

        self.video.next_phase = VideoPhase::StartHblank;
        self.schedule(EventId::Video, next_event);

        self.video.frame_counter = 0;
        self.video.frameskip_counter = 0;
        self.video.stall_mask = 0;

        self.video.palette.fill(0);
        self.video.oam.fill(0);
        self.video.vram.fill(0);

        self.renderer.reset();
        self.renderer.sync_palette(&self.video.palette);
    }

    pub(crate) fn video_event(&mut self, cycles_late: u32) {
        match self.video.next_phase {
            VideoPhase::StartHblank => self.video_start_hblank(cycles_late),
            VideoPhase::StartHdraw => self.video_start_hdraw(cycles_late),
        }
    }

    /// `_startHdraw`
    fn video_start_hdraw(&mut self, cycles_late: u32) {
        self.video.next_phase = VideoPhase::StartHblank;
        self.schedule(EventId::Video, VIDEO_HDRAW_LENGTH - cycles_late as i32);

        self.video.vcount += 1;
        if self.video.vcount == VIDEO_VERTICAL_TOTAL_PIXELS {
            self.video.vcount = 0;
        }
        self.memory.io[(regs::VCOUNT >> 1) as usize] = self.video.vcount as u16;

        if self.video.vcount < VIDEO_VERTICAL_PIXELS as i32 {
            let dispcnt = self.memory.io[(regs::DISPCNT >> 1) as usize];
            self.video.stall_mask = self.calculate_stall_mask(dispcnt);
        }

        let mut dispstat = self.memory.io[(regs::DISPSTAT >> 1) as usize];
        dispstat &= !2; // InHblank
        if self.video.vcount == (dispstat >> 8) as i32 {
            dispstat |= 4;
            if dispstat & 0x20 != 0 {
                self.raise_irq(IRQ_VCOUNTER, cycles_late);
            }
        } else {
            dispstat &= !4;
        }
        self.memory.io[(regs::DISPSTAT >> 1) as usize] = dispstat;

        // Note: state may be recorded during callbacks, so ensure it is consistent!
        match self.video.vcount {
            0 => self.frame_started(),
            v if v == VIDEO_VERTICAL_PIXELS as i32 => {
                self.memory.io[(regs::DISPSTAT >> 1) as usize] = dispstat | 1;
                if self.video.frameskip_counter <= 0 {
                    self.renderer.finish_frame();
                }
                self.dma_run_vblank(-(cycles_late as i32));
                if dispstat & 0x8 != 0 {
                    self.raise_irq(IRQ_VBLANK, cycles_late);
                }
                self.frame_ended();
                self.video.frameskip_counter -= 1;
                if self.video.frameskip_counter < 0 {
                    self.video.frameskip_counter = self.video.frameskip;
                }
                self.video.frame_counter = self.video.frame_counter.wrapping_add(1);
                self.interrupt();
            }
            v if v == VIDEO_VERTICAL_TOTAL_PIXELS - 1 => {
                self.memory.io[(regs::DISPSTAT >> 1) as usize] = dispstat & !1;
            }
            _ => {}
        }
    }

    /// `_startHblank`
    fn video_start_hblank(&mut self, cycles_late: u32) {
        self.video.next_phase = VideoPhase::StartHdraw;
        self.schedule(EventId::Video, VIDEO_HBLANK_LENGTH - cycles_late as i32);

        // Begin Hblank
        let mut dispstat = self.memory.io[(regs::DISPSTAT >> 1) as usize];
        dispstat |= 2;
        let vcount = self.video.vcount;
        if vcount < VIDEO_VERTICAL_PIXELS as i32 && self.video.frameskip_counter <= 0 {
            self.renderer.draw_scanline(vcount as usize, &self.video.vram, &self.video.oam);
        }

        if vcount < VIDEO_VERTICAL_PIXELS as i32 {
            self.dma_run_hblank(-(cycles_late as i32));
        }
        if vcount >= 2 && vcount < VIDEO_VERTICAL_PIXELS as i32 + 2 {
            self.dma_run_display_start(-(cycles_late as i32));
        }
        if dispstat & 0x10 != 0 {
            // TODO: Where does this fudge factor come from?
            self.raise_irq(IRQ_HBLANK, (cycles_late as i32 - 6) as u32);
        }
        self.video.stall_mask = 0;
        self.memory.io[(regs::DISPSTAT >> 1) as usize] = dispstat;
    }

    /// `GBAVideoWriteDISPSTAT`
    pub(crate) fn video_write_dispstat(&mut self, value: u16) -> u16 {
        let mut dispstat = self.memory.io[(regs::DISPSTAT >> 1) as usize] & 0x7;
        dispstat |= value & 0xFFF8;
        if self.video.vcount == (dispstat >> 8) as i32 {
            // Edge trigger only
            if dispstat & 0x20 != 0 && dispstat & 4 == 0 {
                self.raise_irq(IRQ_VCOUNTER, 0);
            }
            dispstat |= 4;
        } else {
            dispstat &= !4;
        }
        dispstat
    }

    /// `_calculateStallMask`
    pub(crate) fn calculate_stall_mask(&self, dispcnt: u16) -> u32 {
        let mut mask: u32 = 0;
        if dispcnt & 0x80 != 0 {
            return 0;
        }
        let bgcnt = |i: u32| self.memory.io[((regs::BG0CNT + 2 * i) >> 1) as usize];
        let bg_enabled = |i: u32| dispcnt & (0x100 << i) != 0;
        let text = |i: u32| -> u32 {
            if bgcnt(i) & 0x80 != 0 {
                vstall_t8(i) as u32
            } else {
                vstall_t4(i) as u32
            }
        };
        match dispcnt & 7 {
            0 => {
                for i in 0..4 {
                    if bg_enabled(i) {
                        mask |= text(i);
                    }
                }
            }
            1 => {
                for i in 0..2 {
                    if bg_enabled(i) {
                        mask |= text(i);
                    }
                }
                if bg_enabled(2) {
                    mask |= VSTALL_A2 as u32;
                }
            }
            2 => {
                if bg_enabled(2) {
                    mask |= VSTALL_A2 as u32;
                }
                if bg_enabled(3) {
                    mask |= VSTALL_A3 as u32;
                }
            }
            3..=5 => {
                if bg_enabled(2) {
                    mask |= VSTALL_B as u32;
                }
            }
            _ => {}
        }
        mask
    }
}
