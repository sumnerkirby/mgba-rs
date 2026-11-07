// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's Game Boy video timing (src/gb/video.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::memory::{SIZE_OAM, SIZE_VRAM, SIZE_VRAM_BANK0};
use super::renderer::{SGB_SIZE_ATF_RAM, SGB_SIZE_CHAR_RAM, SGB_SIZE_MAP_RAM, SGB_SIZE_PAL_RAM};
use super::{io, model, sgb, Gb, GbEvent, IRQ_LCDSTAT, IRQ_VBLANK};
use crate::sm83::CORE_FETCH;

pub const HORIZONTAL_PIXELS: usize = 160;
pub const VERTICAL_PIXELS: usize = 144;
pub const VBLANK_PIXELS: i32 = 10;
pub const VERTICAL_TOTAL_PIXELS: i32 = 154;
pub const MODE_2_LENGTH: i32 = 80;
pub const MODE_3_LENGTH_BASE: i32 = 172;
pub const MODE_0_LENGTH_BASE: i32 = 204;
pub const HORIZONTAL_LENGTH: i32 = 456;
pub const TOTAL_LENGTH: i32 = 70224;
pub const MAX_OBJ: usize = 40;
pub const MAX_LINE_OBJ: usize = 10;

// GBRegisterSTAT bits
const STAT_LYC: u8 = 0x04;
const STAT_HBLANK_IRQ: u8 = 0x08;
const STAT_VBLANK_IRQ: u8 = 0x10;
const STAT_OAM_IRQ: u8 = 0x20;
const STAT_LYC_IRQ: u8 = 0x40;

const LCDC_ENABLE: u8 = 0x80;
const LCDC_OBJ_SIZE: u8 = 0x04;

#[inline]
fn stat_set_mode(stat: u8, mode: i32) -> u8 {
    (stat & !3) | (mode as u8 & 3)
}

#[inline]
fn stat_set_lyc(stat: u8, lyc: bool) -> u8 {
    if lyc {
        stat | STAT_LYC
    } else {
        stat & !STAT_LYC
    }
}

/// `_statIRQAsserted`
fn stat_irq_asserted(stat: u8) -> bool {
    // TODO: variable for the IRQ line value?
    if stat & STAT_LYC_IRQ != 0 && stat & STAT_LYC != 0 {
        return true;
    }
    match stat & 3 {
        0 => stat & STAT_HBLANK_IRQ != 0,
        1 => stat & STAT_VBLANK_IRQ != 0,
        2 => stat & STAT_OAM_IRQ != 0,
        _ => false,
    }
}

/// `struct GBVideo`
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct GbVideo {
    pub x: i32,
    pub ly: i32,
    pub stat: u8,
    pub mode: i32,
    /// Which `_endModeN` the mode event will call.
    pub mode_callback: u8,
    pub dot_clock: i32,
    pub vram: Vec<u8>,
    pub vram_bank: usize,
    pub vram_current_bank: i32,
    #[serde(with = "serde_big_array::BigArray")]
    pub oam: [u8; SIZE_OAM],
    pub obj_max: i32,
    pub bcp_index: i32,
    pub bcp_increment: bool,
    pub ocp_index: i32,
    pub ocp_increment: bool,
    pub sgb_command_header: u8,
    pub sgb_buffer_index: i32,
    #[serde(with = "serde_big_array::BigArray")]
    pub sgb_packet_buffer: [u8; 128],
    pub dmg_palette: [u16; 12],
    #[serde(with = "serde_big_array::BigArray")]
    pub palette: [u16; 64],
    pub sgb_borders: bool,
    pub frame_counter: u32,
    pub frameskip: i32,
    pub frameskip_counter: i32,
}

impl GbVideo {
    /// `GBVideoInit`
    pub fn new() -> GbVideo {
        let gray = [0x7FFF, 0x56B5, 0x294A, 0x0000];
        let mut dmg_palette = [0u16; 12];
        for i in 0..12 {
            dmg_palette[i] = gray[i & 3];
        }
        GbVideo {
            x: 0,
            ly: 0,
            stat: 0,
            mode: 0,
            mode_callback: 0,
            dot_clock: 0,
            vram: vec![0; SIZE_VRAM],
            vram_bank: 0,
            vram_current_bank: 0,
            oam: [0; SIZE_OAM],
            obj_max: 0,
            bcp_index: 0,
            bcp_increment: false,
            ocp_index: 0,
            ocp_increment: false,
            sgb_command_header: 0,
            sgb_buffer_index: 0,
            sgb_packet_buffer: [0; 128],
            dmg_palette,
            palette: [0; 64],
            // mGBA defaults to borders on; this port doesn't render them.
            sgb_borders: false,
            frame_counter: 0,
            frameskip: 0,
            frameskip_counter: 0,
        }
    }

    /// `GBVideoSwitchBank`
    pub fn switch_bank(&mut self, value: u8) {
        let value = value & 1;
        self.vram_bank = value as usize * SIZE_VRAM_BANK0;
        self.vram_current_bank = value as i32;
    }

    /// `GBVideoSetPalette` (`color` is 0xRRGGBB)
    pub fn set_palette(&mut self, index: usize, color: u32) {
        if index >= 12 {
            return;
        }
        let r = (color >> 3) & 0x1F;
        let g = (color >> 11) & 0x1F;
        let b = (color >> 19) & 0x1F;
        self.dmg_palette[index] = (r | (g << 5) | (b << 10)) as u16;
    }
}

impl Default for GbVideo {
    fn default() -> Self {
        Self::new()
    }
}

impl Gb {
    #[inline]
    fn raise_if(&mut self, irq: u8) {
        self.memory.io[io::REG_IF] |= 1 << irq;
    }

    fn renderer_write_palette(&mut self, index: usize, value: u16) {
        self.renderer.write_palette(index, value);
    }

    /// `GBVideoReset`
    pub(crate) fn video_reset(&mut self) {
        let v = &mut self.video;
        v.ly = 0;
        v.x = 0;
        v.mode = 0;
        v.stat = 0;
        v.frame_counter = 0;
        v.frameskip_counter = 0;
        v.switch_bank(0);
        v.vram.iter_mut().for_each(|b| *b = 0);
        v.oam = [0; SIZE_OAM];
        v.palette = [0; 64];

        // SGB border graphics aren't rendered, so the default border data isn't loaded.
        let sgb = &mut self.renderer.sgb;
        if self.model & model::SGB != 0 {
            sgb.char_ram = vec![0; SGB_SIZE_CHAR_RAM];
            sgb.map_ram = vec![0; SGB_SIZE_MAP_RAM];
            sgb.pal_ram = vec![0; SGB_SIZE_PAL_RAM];
            sgb.attribute_files = vec![0; SGB_SIZE_ATF_RAM];
            sgb.attributes = vec![0; 90 * 45];
            self.video.sgb_command_header = 0;
            self.video.sgb_buffer_index = 0;
        } else {
            sgb.char_ram = Vec::new();
            sgb.map_ram = Vec::new();
            sgb.pal_ram = Vec::new();
            sgb.attribute_files = Vec::new();
            sgb.attributes = Vec::new();
        }

        let v = &mut self.video;
        v.palette[0] = v.dmg_palette[0];
        v.palette[1] = v.dmg_palette[1];
        v.palette[2] = v.dmg_palette[2];
        v.palette[3] = v.dmg_palette[3];
        v.palette[8 * 4] = v.dmg_palette[4];
        v.palette[8 * 4 + 1] = v.dmg_palette[5];
        v.palette[8 * 4 + 2] = v.dmg_palette[6];
        v.palette[8 * 4 + 3] = v.dmg_palette[7];
        v.palette[9 * 4] = v.dmg_palette[8];
        v.palette[9 * 4 + 1] = v.dmg_palette[9];
        v.palette[9 * 4 + 2] = v.dmg_palette[10];
        v.palette[9 * 4 + 3] = v.dmg_palette[11];

        let (m, borders) = (self.model, self.video.sgb_borders);
        self.renderer.init(m, borders);

        for i in [0, 1, 2, 3, 32, 33, 34, 35, 36, 37, 38, 39] {
            let c = self.video.palette[i];
            self.renderer_write_palette(i, c);
        }

        self.schedule(GbEvent::VideoFrame, TOTAL_LENGTH << 1);
    }

    /// `GBVideoSkipBIOS`
    pub(crate) fn video_skip_bios(&mut self) {
        self.video.mode = 1;
        self.video.mode_callback = 1;

        let next;
        if self.model & model::CGB != 0 {
            for i in 0..0x40 {
                self.video.palette[i] = 0x7FFF;
                self.renderer_write_palette(i, 0x7FFF);
            }
            self.video.ly = VERTICAL_PIXELS as i32;
            self.memory.io[io::REG_LY] = self.video.ly as u8;
            self.video.stat = stat_set_lyc(self.video.stat, false);
            next = 20;
        } else {
            self.video.ly = VERTICAL_TOTAL_PIXELS;
            self.memory.io[io::REG_LY] = 0;
            next = 56;
        }
        self.video.stat = stat_set_mode(self.video.stat, self.video.mode);

        self.raise_if(IRQ_VBLANK);
        self.update_irqs();
        self.memory.io[io::REG_STAT] = self.video.stat;
        self.deschedule(GbEvent::VideoMode);
        self.schedule(GbEvent::VideoMode, next << 1);
    }

    /// Dispatch for the mode event (`video->modeEvent.callback`).
    pub(crate) fn video_mode_event(&mut self, cycles_late: u32) {
        match self.video.mode_callback {
            0 => self.end_mode0(cycles_late),
            1 => self.end_mode1(cycles_late),
            2 => self.end_mode2(cycles_late),
            _ => self.end_mode3(cycles_late),
        }
    }

    /// `_endMode0`
    fn end_mode0(&mut self, cycles_late: u32) {
        if self.video.frameskip_counter <= 0 {
            let y = self.video.ly;
            self.renderer.finish_scanline(y);
        }
        let lyc = self.memory.io[io::REG_LYC] as i32;
        let next;
        self.video.ly += 1;
        self.memory.io[io::REG_LY] = self.video.ly as u8;
        let mut old_stat = self.video.stat;
        if self.video.ly < VERTICAL_PIXELS as i32 {
            next = MODE_2_LENGTH;
            self.video.mode = 2;
            self.video.mode_callback = 2;
        } else {
            next = HORIZONTAL_LENGTH;
            self.video.mode = 1;
            self.video.mode_callback = 1;

            self.deschedule(GbEvent::VideoFrame);
            self.schedule(GbEvent::VideoFrame, -(cycles_late as i32));

            if !stat_irq_asserted(old_stat) && self.video.stat & STAT_OAM_IRQ != 0 {
                self.raise_if(IRQ_LCDSTAT);
            }
            self.raise_if(IRQ_VBLANK);
        }
        self.video.stat = stat_set_mode(self.video.stat, self.video.mode);
        if !stat_irq_asserted(old_stat) && stat_irq_asserted(self.video.stat) {
            self.raise_if(IRQ_LCDSTAT);
        }

        // LYC stat is delayed 1 T-cycle
        old_stat = self.video.stat;
        self.video.stat = stat_set_lyc(self.video.stat, lyc == self.video.ly);
        if !stat_irq_asserted(old_stat) && stat_irq_asserted(self.video.stat) {
            self.raise_if(IRQ_LCDSTAT);
        }

        self.update_irqs();
        self.memory.io[io::REG_STAT] = self.video.stat;
        self.schedule(GbEvent::VideoMode, (next << 1) - cycles_late as i32);
    }

    /// `_endMode1`
    fn end_mode1(&mut self, cycles_late: u32) {
        if self.memory.io[io::REG_LCDC] & LCDC_ENABLE == 0 {
            return;
        }
        let lyc = self.memory.io[io::REG_LYC];
        // TODO: One M-cycle delay
        self.video.ly += 1;
        let next;
        if self.video.ly == VERTICAL_TOTAL_PIXELS + 1 {
            self.video.ly = 0;
            self.memory.io[io::REG_LY] = 0;
            next = MODE_2_LENGTH;
            self.video.mode = 2;
            self.video.mode_callback = 2;
        } else if self.video.ly == VERTICAL_TOTAL_PIXELS {
            self.memory.io[io::REG_LY] = 0;
            next = HORIZONTAL_LENGTH - 8;
        } else if self.video.ly == VERTICAL_TOTAL_PIXELS - 1 {
            self.memory.io[io::REG_LY] = self.video.ly as u8;
            next = 8;
        } else {
            self.memory.io[io::REG_LY] = self.video.ly as u8;
            next = HORIZONTAL_LENGTH;
        }

        let old_stat = self.video.stat;
        self.video.stat = stat_set_mode(self.video.stat, self.video.mode);
        self.video.stat = stat_set_lyc(self.video.stat, lyc == self.memory.io[io::REG_LY]);
        if !stat_irq_asserted(old_stat) && stat_irq_asserted(self.video.stat) {
            self.raise_if(IRQ_LCDSTAT);
            self.update_irqs();
        }
        self.memory.io[io::REG_STAT] = self.video.stat;
        self.schedule(GbEvent::VideoMode, (next << 1) - cycles_late as i32);
    }

    /// `_endMode2`
    fn end_mode2(&mut self, cycles_late: u32) {
        let ly = self.video.ly;
        self.clean_oam(ly);
        self.video.x = -((self.memory.io[io::REG_SCX] & 7) as i32);
        self.video.dot_clock = self.current_time().wrapping_sub(cycles_late as i32).wrapping_add(10 - (self.video.x << 1));
        let next = MODE_3_LENGTH_BASE + self.video.obj_max * 6 - self.video.x;
        self.video.mode = 3;
        self.video.mode_callback = 3;
        let old_stat = self.video.stat;
        self.video.stat = stat_set_mode(self.video.stat, self.video.mode);
        if !stat_irq_asserted(old_stat) && stat_irq_asserted(self.video.stat) {
            self.raise_if(IRQ_LCDSTAT);
            self.update_irqs();
        }
        self.memory.io[io::REG_STAT] = self.video.stat;
        self.schedule(GbEvent::VideoMode, (next << 1) - cycles_late as i32);
    }

    /// `_endMode3`
    fn end_mode3(&mut self, cycles_late: u32) {
        self.video_process_dots(cycles_late);
        if self.video.ly < VERTICAL_PIXELS as i32 && self.memory.is_hdma && self.memory.io[io::REG_HDMA5] != 0xFF {
            self.memory.hdma_remaining = 0x10;
            self.cpu_blocked = true;
            self.deschedule(GbEvent::Hdma);
            self.schedule(GbEvent::Hdma, 0);
        }
        self.video.mode = 0;
        self.video.mode_callback = 0;
        let old_stat = self.video.stat;
        self.video.stat = stat_set_mode(self.video.stat, self.video.mode);
        if !stat_irq_asserted(old_stat) && stat_irq_asserted(self.video.stat) {
            self.raise_if(IRQ_LCDSTAT);
            self.update_irqs();
        }
        self.memory.io[io::REG_STAT] = self.video.stat;
        // TODO: Cache SCX & 7 in case it changes
        let next = MODE_0_LENGTH_BASE - self.video.obj_max * 6 - (self.memory.io[io::REG_SCX] & 7) as i32;
        self.schedule(GbEvent::VideoMode, (next << 1) - cycles_late as i32);
    }

    /// `_updateFrameCount`
    pub(crate) fn video_frame_event(&mut self, _cycles_late: u32) {
        if self.cpu.execution_state != CORE_FETCH {
            let delay = (4 - ((self.cpu.execution_state as i32 + 1) & 3)) * (2 - self.double_speed as i32);
            self.schedule(GbEvent::VideoFrame, delay);
            return;
        }
        if self.memory.io[io::REG_LCDC] & LCDC_ENABLE == 0 {
            self.schedule(GbEvent::VideoFrame, TOTAL_LENGTH << 1);
        }

        self.video.frameskip_counter -= 1;
        if self.video.frameskip_counter < 0 {
            self.renderer.finish_frame();
            self.video.frameskip_counter = self.video.frameskip;
        }
        self.frame_ended();
        self.video.frame_counter = self.video.frame_counter.wrapping_add(1);
        self.interrupt();

        self.frame_started();
    }

    /// `_cleanOAM`
    fn clean_oam(&mut self, y: i32) {
        let sprite_height = if self.memory.io[io::REG_LCDC] & LCDC_OBJ_SIZE != 0 { 16 } else { 8 };
        let mut o = 0;
        let mut i = 0;
        while i < MAX_OBJ && o < MAX_LINE_OBJ {
            let oy = self.video.oam[i * 4] as i32;
            i += 1;
            if y < oy - 16 || y >= oy - 16 + sprite_height {
                continue;
            }
            o += 1;
        }
        self.video.obj_max = o as i32;
    }

    /// `GBVideoProcessDots`
    pub(crate) fn video_process_dots(&mut self, cycles_late: u32) {
        if self.video.mode != 3 {
            return;
        }
        let mut old_x = self.video.x;
        self.video.x = (self.current_time().wrapping_sub(cycles_late as i32).wrapping_sub(self.video.dot_clock)) >> 1;
        if self.video.x > HORIZONTAL_PIXELS as i32 {
            self.video.x = HORIZONTAL_PIXELS as i32;
        } else if self.video.x < 0 {
            return;
        }
        if old_x < 0 {
            old_x = 0;
        }
        if self.video.frameskip_counter <= 0 {
            let (x, ly) = (self.video.x, self.video.ly);
            self.renderer.draw_range(&self.video.vram, &self.video.oam, old_x, x, ly);
        }
    }

    /// `GBVideoWriteLCDC`
    pub(crate) fn video_write_lcdc(&mut self, value: u8) {
        let old = self.memory.io[io::REG_LCDC];
        if old & LCDC_ENABLE == 0 && value & LCDC_ENABLE != 0 {
            self.video.mode = 2;
            self.video.mode_callback = 2;
            let next = MODE_2_LENGTH - 5; // TODO: Why is this fudge factor needed? Might be related to T-cycles for load/store differing
            self.deschedule(GbEvent::VideoMode);
            self.schedule(GbEvent::VideoMode, next << 1);

            self.video.ly = 0;
            self.memory.io[io::REG_LY] = 0;
            let old_stat = self.video.stat;
            self.video.stat = stat_set_mode(self.video.stat, 0);
            self.video.stat = stat_set_lyc(self.video.stat, self.video.ly == self.memory.io[io::REG_LYC] as i32);
            if !stat_irq_asserted(old_stat) && stat_irq_asserted(self.video.stat) {
                self.raise_if(IRQ_LCDSTAT);
                self.update_irqs();
            }
            self.memory.io[io::REG_STAT] = self.video.stat;
            let c = self.video.palette[0];
            self.renderer_write_palette(0, c);

            self.deschedule(GbEvent::VideoFrame);
        }
        if old & LCDC_ENABLE != 0 && value & LCDC_ENABLE == 0 {
            // TODO: Fix serialization; this gets internal and visible modes out of sync
            self.video.mode = 0;
            self.video.stat = stat_set_mode(self.video.stat, 0);
            self.memory.io[io::REG_STAT] = self.video.stat;
            self.video.ly = 0;
            self.memory.io[io::REG_LY] = 0;
            let c = self.video.dmg_palette[0];
            self.renderer_write_palette(0, c);

            self.deschedule(GbEvent::VideoMode);
            self.deschedule(GbEvent::VideoFrame);
            self.schedule(GbEvent::VideoFrame, TOTAL_LENGTH << 1);
        }
        self.memory.io[io::REG_STAT] = self.video.stat;
    }

    /// `GBVideoWriteSTAT`
    pub(crate) fn video_write_stat(&mut self, value: u8) {
        let old_stat = self.video.stat;
        self.video.stat = (self.video.stat & 0x7) | (value & 0x78);
        if self.memory.io[io::REG_LCDC] & LCDC_ENABLE == 0 || self.model >= model::CGB {
            return;
        }
        // Writing to STAT on a DMG selects all STAT IRQ types for one cycle.
        if !stat_irq_asserted(old_stat) && (self.video.mode < 2 || self.video.stat & STAT_LYC != 0) {
            self.raise_if(IRQ_LCDSTAT);
            self.update_irqs();
        }
    }

    /// `GBVideoWriteLYC`
    pub(crate) fn video_write_lyc(&mut self, value: u8) {
        let old_stat = self.video.stat;
        if self.memory.io[io::REG_LCDC] & LCDC_ENABLE != 0 {
            self.video.stat = stat_set_lyc(self.video.stat, value as i32 == self.video.ly);
            if !stat_irq_asserted(old_stat) && stat_irq_asserted(self.video.stat) {
                self.raise_if(IRQ_LCDSTAT);
                self.update_irqs();
            }
        }
        self.memory.io[io::REG_STAT] = self.video.stat;
    }

    /// `GBVideoWritePalette`
    pub(crate) fn video_write_palette(&mut self, address: usize, value: u8) {
        if self.model < model::SGB {
            let (base, dmg) = match address {
                io::REG_BGP => (0, 0),
                io::REG_OBP0 => (8 * 4, 4),
                io::REG_OBP1 => (9 * 4, 8),
                _ => return,
            };
            for i in 0..4 {
                let c = self.video.dmg_palette[((value >> (i * 2)) & 3) as usize + dmg];
                self.video.palette[base + i] = c;
                self.renderer_write_palette(base + i, c);
            }
        } else if self.model >= model::CGB {
            match address {
                io::REG_BCPD => {
                    let idx = self.video.bcp_index;
                    if self.video.mode != 3 {
                        let p = &mut self.video.palette[(idx >> 1) as usize];
                        if idx & 1 != 0 {
                            *p &= 0x00FF;
                            *p |= (value as u16) << 8;
                        } else {
                            *p &= 0xFF00;
                            *p |= value as u16;
                        }
                        let c = *p;
                        self.renderer_write_palette((idx >> 1) as usize, c);
                    }
                    if self.video.bcp_increment {
                        self.video.bcp_index = (self.video.bcp_index + 1) & 0x3F;
                        self.memory.io[io::REG_BCPS] &= 0x80;
                        self.memory.io[io::REG_BCPS] |= self.video.bcp_index as u8;
                    }
                    let idx = self.video.bcp_index;
                    self.memory.io[io::REG_BCPD] = (self.video.palette[(idx >> 1) as usize] >> (8 * (idx & 1))) as u8;
                }
                io::REG_OCPD => {
                    let idx = self.video.ocp_index;
                    if self.video.mode != 3 {
                        let p = &mut self.video.palette[8 * 4 + (idx >> 1) as usize];
                        if idx & 1 != 0 {
                            *p &= 0x00FF;
                            *p |= (value as u16) << 8;
                        } else {
                            *p &= 0xFF00;
                            *p |= value as u16;
                        }
                        let c = *p;
                        self.renderer_write_palette(8 * 4 + (idx >> 1) as usize, c);
                    }
                    if self.video.ocp_increment {
                        self.video.ocp_index = (self.video.ocp_index + 1) & 0x3F;
                        self.memory.io[io::REG_OCPS] &= 0x80;
                        self.memory.io[io::REG_OCPS] |= self.video.ocp_index as u8;
                    }
                    let idx = self.video.ocp_index;
                    self.memory.io[io::REG_OCPD] = (self.video.palette[8 * 4 + (idx >> 1) as usize] >> (8 * (idx & 1))) as u8;
                }
                _ => {}
            }
        } else {
            self.renderer.write_video_register(address, value);
        }
    }

    /// `GBVideoDisableCGB`
    pub(crate) fn video_disable_cgb(&mut self) {
        let v = &mut self.video;
        v.dmg_palette[0] = v.palette[0];
        v.dmg_palette[1] = v.palette[1];
        v.dmg_palette[2] = v.palette[2];
        v.dmg_palette[3] = v.palette[3];
        v.dmg_palette[4] = v.palette[8 * 4];
        v.dmg_palette[5] = v.palette[8 * 4 + 1];
        v.dmg_palette[6] = v.palette[8 * 4 + 2];
        v.dmg_palette[7] = v.palette[8 * 4 + 3];
        v.dmg_palette[8] = v.palette[9 * 4];
        v.dmg_palette[9] = v.palette[9 * 4 + 1];
        v.dmg_palette[10] = v.palette[9 * 4 + 2];
        v.dmg_palette[11] = v.palette[9 * 4 + 3];
        let (m, b) = (self.model, self.video.sgb_borders);
        self.renderer.init(m, b);
    }

    fn sgb_color(&self, i: usize) -> u16 {
        self.video.sgb_packet_buffer[i] as u16 | ((self.video.sgb_packet_buffer[i + 1] as u16) << 8)
    }

    fn set_palette_entries(&mut self, entries: &[(usize, u16)]) {
        for &(i, c) in entries {
            self.video.palette[i] = c;
        }
        for &(i, _) in entries {
            let c = self.video.palette[i];
            self.renderer_write_palette(i, c);
        }
    }

    /// `GBVideoWriteSGBPacket`
    pub(crate) fn video_write_sgb_packet(&mut self, data: &[u8; 16]) {
        if self.video.sgb_command_header & 7 == 0 {
            self.video.sgb_buffer_index = 0;
            if (data[0] >> 3) > sgb::OBJ_TRN {
                self.video.sgb_command_header = 0;
                return;
            }
            self.video.sgb_command_header = data[0];
        }
        self.video.sgb_command_header = self.video.sgb_command_header.wrapping_sub(1);
        let idx = (self.video.sgb_buffer_index as usize & 7) << 4;
        self.video.sgb_packet_buffer[idx..idx + 16].copy_from_slice(data);
        self.video.sgb_buffer_index += 1;
        if self.video.sgb_command_header & 7 != 0 {
            return;
        }
        match self.video.sgb_command_header >> 3 {
            sgb::PAL01 => {
                let c0 = self.sgb_color(1);
                let e = [
                    (0, c0),
                    (1, self.sgb_color(3)),
                    (2, self.sgb_color(5)),
                    (3, self.sgb_color(7)),
                    (4, c0),
                    (5, self.sgb_color(9)),
                    (6, self.sgb_color(11)),
                    (7, self.sgb_color(13)),
                    (8, c0),
                    (12, c0),
                ];
                self.set_palette_entries(&e);
            }
            sgb::PAL23 => {
                let e = [
                    (9, self.sgb_color(3)),
                    (10, self.sgb_color(5)),
                    (11, self.sgb_color(7)),
                    (13, self.sgb_color(9)),
                    (14, self.sgb_color(11)),
                    (15, self.sgb_color(13)),
                ];
                self.set_palette_entries(&e);
            }
            sgb::PAL03 => {
                let c0 = self.sgb_color(1);
                let e = [
                    (0, c0),
                    (1, self.sgb_color(3)),
                    (2, self.sgb_color(5)),
                    (3, self.sgb_color(7)),
                    (4, c0),
                    (8, c0),
                    (12, c0),
                    (13, self.sgb_color(9)),
                    (14, self.sgb_color(11)),
                    (15, self.sgb_color(13)),
                ];
                self.set_palette_entries(&e);
            }
            sgb::PAL12 => {
                let e = [
                    (5, self.sgb_color(3)),
                    (6, self.sgb_color(5)),
                    (7, self.sgb_color(7)),
                    (9, self.sgb_color(9)),
                    (10, self.sgb_color(11)),
                    (11, self.sgb_color(13)),
                ];
                self.set_palette_entries(&e);
            }
            sgb::PAL_SET => {
                for i in 0..4 {
                    let entry = ((self.video.sgb_packet_buffer[2 + i * 2] as usize) << 8)
                        | self.video.sgb_packet_buffer[1 + i * 2] as usize;
                    if entry >= 0x200 {
                        continue;
                    }
                    for j in 0..4 {
                        let o = entry * 8 + j * 2;
                        let pal = &self.renderer.sgb.pal_ram;
                        let c = pal.get(o).copied().unwrap_or(0) as u16 | ((pal.get(o + 1).copied().unwrap_or(0) as u16) << 8);
                        self.video.palette[i * 4 + j] = c;
                        self.renderer_write_palette(i * 4 + j, c);
                    }
                }
            }
            sgb::ATTR_BLK | sgb::ATTR_DIV | sgb::ATTR_CHR | sgb::ATTR_LIN | sgb::PAL_TRN | sgb::ATRC_EN
            | sgb::CHR_TRN | sgb::PCT_TRN | sgb::ATTR_TRN | sgb::ATTR_SET => {}
            sgb::MLT_REQ => {
                if (self.video.sgb_packet_buffer[1] & 0x3) == 2 {
                    // XXX: This unmasked increment appears to be an SGB hardware bug
                    self.sgb_current_controller += 1;
                }
                self.sgb_controllers = (self.video.sgb_packet_buffer[1] & 0x3) as i32;
                self.sgb_current_controller &= self.sgb_controllers;
                return;
            }
            sgb::MASK_EN => {
                self.renderer.sgb.render_mode = (self.video.sgb_packet_buffer[1] & 0x3) as i32;
            }
            _ => return,
        }
        let buf = self.video.sgb_packet_buffer;
        self.renderer.write_sgb_packet(&buf);
    }
}
