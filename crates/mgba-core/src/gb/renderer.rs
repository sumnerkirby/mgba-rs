// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's Game Boy software renderer (src/gb/renderers/software.c).
//
// Super Game Boy palettes, attribute files, VRAM transfers and borders are
// ported. With SGB borders on, output is 256x224 with the Game Boy screen at
// (48, 40); otherwise it's 160x144. The debugger's layer highlighting is left out.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::io;
use super::model;
use super::sgb;
use super::video::{HORIZONTAL_PIXELS, MAX_LINE_OBJ, MAX_OBJ, VERTICAL_PIXELS};
use crate::gba::renderer::color_from_555;

const PAL_BG: u16 = 0;
const PAL_OBJ: u16 = 0x20;
const PAL_HIGHLIGHT: usize = 0x80;
const PAL_SGB_BORDER: usize = 0x40;
const OBJ_PRIORITY: u16 = 0x100;
const OBJ_PRIO_MASK: u16 = 0x0FF;

const BASE_MAP: usize = 0x1800;
const SIZE_MAP: usize = 0x0400;
const SIZE_VRAM_BANK0: usize = 0x2000;

pub const SGB_SIZE_CHAR_RAM: usize = 0x2000;
pub const SGB_SIZE_MAP_RAM: usize = 0x1000;
pub const SGB_SIZE_PAL_RAM: usize = 0x1000;
pub const SGB_SIZE_ATF_RAM: usize = 0x1000;

const LCDC_BG_ENABLE: u8 = 0x01;
const LCDC_OBJ_ENABLE: u8 = 0x02;
const LCDC_OBJ_SIZE: u8 = 0x04;
const LCDC_TILE_MAP: u8 = 0x08;
const LCDC_TILE_DATA: u8 = 0x10;
const LCDC_WINDOW: u8 = 0x20;
const LCDC_WINDOW_TILE_MAP: u8 = 0x40;
const LCDC_ENABLE: u8 = 0x80;

// GBObjAttributes
const ATTR_BANK: u8 = 0x08;
const ATTR_PALETTE: u8 = 0x10;
const ATTR_XFLIP: u8 = 0x20;
const ATTR_YFLIP: u8 = 0x40;
const ATTR_PRIORITY: u8 = 0x80;

/// The SGB state mGBA keeps on `struct GBVideoRenderer`.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct SgbState {
    pub char_ram: Vec<u8>,
    pub map_ram: Vec<u8>,
    pub pal_ram: Vec<u8>,
    pub render_mode: i32,
    pub attributes: Vec<u8>,
    pub attribute_files: Vec<u8>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default)]
pub struct Sprite {
    /// y, x, tile, attr
    pub obj: [u8; 4],
    pub index: i8,
}

/// `struct GBVideoSoftwareRenderer`
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct GbRenderer {
    pub output: Vec<u32>,
    #[serde(with = "serde_big_array::BigArray")]
    row: [u16; HORIZONTAL_PIXELS + 8],
    #[serde(with = "serde_big_array::BigArray")]
    palette: [u32; 192],
    #[serde(with = "serde_big_array::BigArray")]
    lookup: [u8; 192],

    scy: u8,
    scx: u8,
    wy: u8,
    wx: u8,
    current_wy: u8,
    current_wx: u8,
    last_y: i32,
    last_x: i32,
    has_window: bool,

    lcdc: u8,
    model: u8,

    obj: [Sprite; MAX_LINE_OBJ],
    obj_max: usize,

    sgb_transfer: i32,
    #[serde(with = "serde_big_array::BigArray")]
    sgb_packet: [u8; 128],
    sgb_command_header: u8,
    sgb_borders: bool,
    sgb_border_mask: [u32; 18],
    /// Output dimensions: 256x224 for SGB with borders, else 160x144.
    width: usize,
    height: usize,

    pub sgb: SgbState,

    /// Debug layer toggles (`disableBG` etc.).
    pub disable_bg: bool,
    pub disable_obj: bool,
    pub disable_win: bool,
}

impl Default for GbRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn set_attribute(attrs: &mut [u8], x: usize, y: usize, palette: u8) {
    let i = (x >> 2) + 5 * y;
    if i >= attrs.len() {
        return;
    }
    let mut p = attrs[i];
    p &= !(3 << (2 * (3 - (x & 3))));
    p |= palette << (2 * (3 - (x & 3)));
    attrs[i] = p;
}

impl GbRenderer {
    /// `GBVideoSoftwareRendererCreate`
    pub fn new() -> GbRenderer {
        let mut r = GbRenderer {
            output: vec![0; HORIZONTAL_PIXELS * VERTICAL_PIXELS],
            row: [0; HORIZONTAL_PIXELS + 8],
            palette: [0; 192],
            lookup: [0; 192],
            scy: 0,
            scx: 0,
            wy: 0,
            wx: 0,
            current_wy: 0,
            current_wx: 0,
            last_y: VERTICAL_PIXELS as i32,
            last_x: 0,
            has_window: false,
            lcdc: 0,
            model: 0,
            obj: [Sprite::default(); MAX_LINE_OBJ],
            obj_max: 0,
            sgb_transfer: 0,
            sgb_packet: [0; 128],
            sgb_command_header: 0,
            sgb_borders: false,
            sgb_border_mask: [0; 18],
            width: HORIZONTAL_PIXELS,
            height: VERTICAL_PIXELS,
            sgb: SgbState::default(),
            disable_bg: false,
            disable_obj: false,
            disable_win: false,
        };
        r.init(model::DMG, false);
        r
    }

    /// The most recent frame, [`GbRenderer::size`] pixels, 0x00BBGGRR.
    pub fn output(&self) -> &[u32] {
        &self.output[..self.width * self.height]
    }

    /// Output width and height.
    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn uses_borders(&self) -> bool {
        self.model & model::SGB != 0 && self.sgb_borders
    }

    fn update_size(&mut self) {
        let (w, h) = if self.uses_borders() { (256, 224) } else { (HORIZONTAL_PIXELS, VERTICAL_PIXELS) };
        if (w, h) != (self.width, self.height) || self.output.len() != w * h {
            self.width = w;
            self.height = h;
            self.output = vec![0; w * h];
        }
    }

    /// `_regenerateSGBBorder`
    fn regenerate_sgb_border(&mut self) {
        if self.sgb.map_ram.len() < SGB_SIZE_MAP_RAM || self.sgb.char_ram.len() < SGB_SIZE_CHAR_RAM {
            return;
        }
        for i in 0..0x40 {
            let color = self.sgb.map_ram[0x800 + i * 2] as u16 | ((self.sgb.map_ram[0x801 + i * 2] as u16) << 8);
            self.write_palette(i + PAL_SGB_BORDER, color);
        }
        let stride = self.width;
        let char_word = |r: &GbRenderer, w: usize| -> u32 {
            let o = w * 4;
            u32::from_le_bytes(r.sgb.char_ram[o..o + 4].try_into().unwrap())
        };
        for y in 0..224usize {
            let local_y = y & 7;
            if local_y == 0 && (40..184).contains(&y) {
                self.sgb_border_mask[(y - 40) >> 3] = 0;
            }
            let mut x = 0usize;
            while x < 256 {
                let mo = (x >> 2) + (y & !7) * 8;
                let map_data = self.sgb.map_ram[mo] as u16 | ((self.sgb.map_ram[mo + 1] as u16) << 8);
                let tile = (map_data & 0x3FF) as usize;
                if tile >= 0x100 {
                    x += 8;
                    continue;
                }
                if (48..208).contains(&x) && (40..184).contains(&y) {
                    if local_y == 0 {
                        let tile_base = tile * 8;
                        let mut bits = 0u32;
                        for k in 0..8 {
                            bits |= char_word(self, tile_base + k);
                        }
                        if bits != 0 {
                            self.sgb_border_mask[(y - 40) >> 3] |= 1 << ((x - 48) >> 3);
                        }
                    }
                    x += 8;
                    continue;
                }
                let y_flip = if map_data & 0x8000 != 0 { 7 } else { 0 };
                let tile_base = (tile * 16 + (local_y ^ y_flip)) * 2;
                let td = [
                    self.sgb.char_ram[tile_base],
                    self.sgb.char_ram[tile_base + 1],
                    self.sgb.char_ram[tile_base + 0x10],
                    self.sgb.char_ram[tile_base + 0x11],
                ];
                let base = y * stride + x;
                let palette_base = ((map_data >> 10) & 7) as usize * 0x10;
                let x_flip = if map_data & 0x4000 != 0 { 7 } else { 0 };
                for i in (0..8).rev() {
                    let sel = ((td[0] >> i) & 1) | (((td[1] >> i) & 1) << 1) | (((td[2] >> i) & 1) << 2) | (((td[3] >> i) & 1) << 3);
                    self.output[(base + 7 - i) ^ x_flip] = self.palette[palette_base | sel as usize];
                }
                x += 8;
            }
        }
    }

    /// `GBVideoSoftwareRendererEnableSGBBorder`
    pub fn enable_sgb_border(&mut self, enable: bool) {
        if self.model & model::SGB != 0 {
            if enable == self.sgb_borders {
                return;
            }
            self.sgb_borders = enable;
            self.update_size();
            if self.sgb_borders && self.sgb.render_mode == 0 {
                self.regenerate_sgb_border();
            }
        }
    }

    /// `GBVideoSoftwareRendererInit`
    pub fn init(&mut self, model: u8, sgb_borders: bool) {
        self.lcdc = 0;
        self.scy = 0;
        self.scx = 0;
        self.wy = 0;
        self.current_wy = 0;
        self.current_wx = 0;
        self.last_y = VERTICAL_PIXELS as i32;
        self.last_x = 0;
        self.has_window = false;
        self.wx = 0;
        self.model = model;
        self.sgb_transfer = 0;
        self.sgb_command_header = 0;
        self.sgb_borders = sgb_borders;
        for (i, l) in self.lookup.iter_mut().enumerate() {
            *l = i as u8;
        }
        self.palette = [0; 192];
        self.sgb_border_mask = [0; 18];
        self.update_size();
    }

    /// `_clearScreen`
    fn clear_screen(&mut self) {
        if self.model & model::SGB != 0 {
            return;
        }
        let c = self.palette[0];
        self.output.iter_mut().for_each(|p| *p = c);
    }

    /// `_parseAttrBlock`
    fn parse_attr_block(&mut self, start: usize) {
        let block: [u8; 6] = self.sgb_packet[start..start + 6].try_into().unwrap();
        let x0 = block[2] as usize;
        let x1 = block[4] as usize;
        let y0 = block[3] as usize;
        let y1 = block[5] as usize;
        let p_in = block[1] & 3;
        let p_perim = (block[1] >> 2) & 3;
        let p_out = (block[1] >> 4) & 3;
        let attrs = &mut self.sgb.attributes;
        for y in 0..VERTICAL_PIXELS / 8 {
            for x in 0..HORIZONTAL_PIXELS / 8 {
                if y > y0 && y < y1 && x > x0 && x < x1 {
                    if block[0] & 1 != 0 {
                        set_attribute(attrs, x, y, p_in);
                    }
                } else if y < y0 || y > y1 || x < x0 || x > x1 {
                    if block[0] & 4 != 0 {
                        set_attribute(attrs, x, y, p_out);
                    }
                } else if block[0] & 2 != 0 {
                    set_attribute(attrs, x, y, p_perim);
                } else if block[0] & 1 != 0 {
                    set_attribute(attrs, x, y, p_in);
                } else if block[0] & 4 != 0 {
                    set_attribute(attrs, x, y, p_out);
                }
            }
        }
    }

    /// `_parseAttrLine`
    fn parse_attr_line(&mut self, start: usize) {
        let byte = self.sgb_packet[start];
        let line = (byte & 0x1F) as usize;
        let pal = (byte >> 5) & 3;
        let attrs = &mut self.sgb.attributes;
        if byte & 0x80 != 0 {
            if line > VERTICAL_PIXELS / 8 {
                return;
            }
            for x in 0..HORIZONTAL_PIXELS / 8 {
                set_attribute(attrs, x, line, pal);
            }
        } else {
            if line > HORIZONTAL_PIXELS / 8 {
                return;
            }
            for y in 0..VERTICAL_PIXELS / 8 {
                set_attribute(attrs, line, y, pal);
            }
        }
    }

    fn in_window(&self) -> bool {
        self.lcdc & LCDC_WINDOW != 0 && (HORIZONTAL_PIXELS as i32 + 7) > self.wx as i32
    }

    /// `GBVideoSoftwareRendererUpdateWindow`
    fn update_window(&mut self, before: bool, after: bool, old_wy: u8) {
        if self.last_y >= VERTICAL_PIXELS as i32 || !(after || before) {
            return;
        }
        if !self.has_window && self.last_x == HORIZONTAL_PIXELS as i32 && self.last_y != old_wy as i32 {
            return;
        }
        if self.last_y >= old_wy as i32 {
            if !after {
                self.current_wy = self.current_wy.wrapping_sub(self.last_y as u8);
                self.has_window = true;
            } else if !before {
                if !self.has_window {
                    self.current_wy = (self.last_y - self.wy as i32) as u8;
                    if self.last_y >= self.wy as i32 && self.last_x > self.wx as i32 {
                        self.current_wy = self.current_wy.wrapping_add(1);
                    }
                } else {
                    self.current_wy = self.current_wy.wrapping_add(self.last_y as u8);
                }
            } else if self.wy != old_wy {
                self.current_wy = self.current_wy.wrapping_add(old_wy.wrapping_sub(self.wy));
                self.has_window = true;
            }
        }
    }

    /// `GBVideoSoftwareRendererWriteVideoRegister`
    pub fn write_video_register(&mut self, address: usize, value: u8) -> u8 {
        let was_window = self.in_window();
        let wy = self.wy;
        match address {
            io::REG_LCDC => {
                self.lcdc = value;
                let now = self.in_window();
                self.update_window(was_window, now, wy);
            }
            io::REG_SCY => self.scy = value,
            io::REG_SCX => self.scx = value,
            io::REG_WY => {
                self.wy = value;
                let now = self.in_window();
                self.update_window(was_window, now, wy);
            }
            io::REG_WX => {
                self.wx = value;
                let now = self.in_window();
                self.update_window(was_window, now, wy);
            }
            io::REG_BGP | io::REG_OBP0 | io::REG_OBP1 => {
                let base = match address {
                    io::REG_BGP => PAL_BG as usize,
                    io::REG_OBP0 => PAL_OBJ as usize,
                    _ => PAL_OBJ as usize + 4,
                };
                for i in 0..4 {
                    let c = (value >> (i * 2)) & 3;
                    self.lookup[base + i] = c;
                    self.lookup[PAL_HIGHLIGHT + base + i] = PAL_HIGHLIGHT as u8 + c;
                }
            }
            _ => {}
        }
        value
    }

    /// `GBVideoSoftwareRendererWriteSGBPacket`
    pub fn write_sgb_packet(&mut self, data: &[u8; 128]) {
        self.sgb_packet = *data;
        self.sgb_command_header = data[0];
        self.sgb_transfer = 0;
        let count = ((self.sgb_command_header & 7) as usize) << 4;
        match self.sgb_command_header >> 3 {
            sgb::PAL_SET | sgb::ATTR_SET => {
                if self.sgb_command_header >> 3 == sgb::PAL_SET {
                    self.sgb_packet[1] = data[9];
                    if data[9] & 0x80 == 0 {
                        return;
                    }
                }
                let set = (self.sgb_packet[1] & 0x3F) as usize;
                if set <= 0x2C && self.sgb.attribute_files.len() >= set * 90 + 90 && self.sgb.attributes.len() >= 90 {
                    let src = self.sgb.attribute_files[set * 90..set * 90 + 90].to_vec();
                    self.sgb.attributes[..90].copy_from_slice(&src);
                }
            }
            sgb::ATTR_BLK => {
                let mut sets = self.sgb_packet[1];
                let mut i = 2;
                while i < count && sets != 0 && i + 6 <= 128 {
                    self.parse_attr_block(i);
                    i += 6;
                    sets -= 1;
                }
            }
            sgb::ATTR_LIN => {
                let mut sets = self.sgb_packet[1];
                let mut i = 2;
                while i < count && sets != 0 {
                    self.parse_attr_line(i);
                    i += 1;
                    sets -= 1;
                }
            }
            sgb::ATTR_DIV => {
                let p_after = self.sgb_packet[1] & 3;
                let p_before = (self.sgb_packet[1] >> 2) & 3;
                let p_div = (self.sgb_packet[1] >> 4) & 3;
                let mut attr_x = self.sgb_packet[2] as usize;
                let attrs = &mut self.sgb.attributes;
                let (w, h) = (HORIZONTAL_PIXELS / 8, VERTICAL_PIXELS / 8);
                if self.sgb_packet[1] & 0x40 != 0 {
                    if attr_x > h {
                        attr_x = h;
                    }
                    let mut j = 0;
                    while j < attr_x {
                        for i in 0..w {
                            set_attribute(attrs, i, j, p_before);
                        }
                        j += 1;
                    }
                    if attr_x < h {
                        for i in 0..w {
                            set_attribute(attrs, i, attr_x, p_div);
                        }
                    }
                    while j < h {
                        for i in 0..w {
                            set_attribute(attrs, i, j, p_after);
                        }
                        j += 1;
                    }
                } else {
                    if attr_x > w {
                        attr_x = w;
                    }
                    let mut j = 0;
                    while j < attr_x {
                        for i in 0..w {
                            set_attribute(attrs, j, i, p_before);
                        }
                        j += 1;
                    }
                    if attr_x < w {
                        for i in 0..h {
                            set_attribute(attrs, attr_x, i, p_div);
                        }
                    }
                    while j < w {
                        for i in 0..h {
                            set_attribute(attrs, j, i, p_after);
                        }
                        j += 1;
                    }
                }
            }
            sgb::ATTR_CHR => {
                let (w, h) = (HORIZONTAL_PIXELS / 8, VERTICAL_PIXELS / 8);
                let mut attr_x = self.sgb_packet[1] as usize;
                let mut attr_y = self.sgb_packet[2] as usize;
                if attr_x >= w {
                    attr_x = 0;
                }
                if attr_y >= h {
                    attr_y = 0;
                }
                let mut sets = self.sgb_packet[3] as u32 | ((self.sgb_packet[4] as u32) << 8);
                let attr_direction = self.sgb_packet[5];
                let mut i = 6;
                while i < count && sets != 0 {
                    let mut j = 0;
                    while j < 4 && sets != 0 {
                        let p = self.sgb_packet[i] >> (6 - j * 2);
                        set_attribute(&mut self.sgb.attributes, attr_x, attr_y, p & 3);
                        if attr_direction != 0 {
                            attr_y += 1;
                            if attr_y >= h {
                                attr_y = 0;
                                attr_x += 1;
                            }
                            if attr_x >= w {
                                attr_x = 0;
                            }
                        } else {
                            attr_x += 1;
                            if attr_x >= w {
                                attr_x = 0;
                                attr_y += 1;
                            }
                            if attr_y >= h {
                                attr_y = 0;
                            }
                        }
                        j += 1;
                        sets -= 1;
                    }
                    i += 1;
                }
            }
            sgb::ATRC_EN | sgb::MASK_EN => {
                if self.sgb_borders && self.sgb.render_mode == 0 {
                    self.regenerate_sgb_border();
                }
            }
            _ => {}
        }
    }

    /// `GBVideoSoftwareRendererWritePalette`
    pub fn write_palette(&mut self, index: usize, value: u16) {
        let mut color = color_from_555(value);
        if self.model & model::SGB != 0 {
            if index >= PAL_SGB_BORDER && index & 0xF == 0 {
                color = self.palette[0];
            } else if self.model & model::CGB == 0 {
                if index < 0x10 && index != 0 && index & 3 == 0 {
                    color = self.palette[0];
                } else if index > PAL_HIGHLIGHT && index < PAL_HIGHLIGHT + PAL_OBJ as usize && index & 3 == 0 {
                    color = self.palette[PAL_HIGHLIGHT];
                }
            }
        }
        if self.model == model::AGB {
            let mut r = (value & 0x1F) as u32;
            let mut g = ((value >> 5) & 0x1F) as u32;
            let mut b = ((value >> 10) & 0x1F) as u32;
            r *= r;
            g *= g;
            b *= b;
            r >>= 2;
            r += r >> 4;
            g >>= 2;
            g += g >> 4;
            b >>= 2;
            b += b >> 4;
            color = r | (g << 8) | (b << 16);
        }
        self.palette[index] = color;
        if index < PAL_SGB_BORDER && (index < PAL_OBJ as usize || index & 3 != 0) {
            // No highlight: mColorMix5Bit(16, color, 0, _) == color
            self.palette[index + PAL_HIGHLIGHT] = color;
        }

        if self.model & model::SGB != 0 && index == 0 && self.lcdc & LCDC_ENABLE != 0 {
            if self.model & model::CGB == 0 {
                for i in [0x04, 0x08, 0x0C, 0x40, 0x50, 0x60, 0x70] {
                    self.write_palette(i, value);
                }
            }
            if self.sgb_borders && self.sgb.render_mode == 0 {
                self.regenerate_sgb_border();
            }
        }
    }

    /// `_cleanOAM`
    fn clean_oam(&mut self, oam: &[u8], y: i32) {
        // TODO: GBC differences
        let sprite_height = if self.lcdc & LCDC_OBJ_SIZE != 0 { 16 } else { 8 };
        let mut o = 0usize;
        let mut ids = [0i16; MAX_LINE_OBJ];
        let mut i = 0;
        while i < MAX_OBJ && o < MAX_LINE_OBJ {
            let oy = oam[i * 4] as i32;
            if !(y < oy - 16 || y >= oy - 16 + sprite_height) {
                ids[o] = ((oam[i * 4 + 1] as i16) << 7) | i as i16;
                o += 1;
            }
            i += 1;
        }
        self.obj_max = o;
        if self.model < model::CGB {
            // Terrible n^2 sort, but it's only 10 elements so it shouldn't be that bad
            let mut ids2 = [0i16; MAX_LINE_OBJ];
            let mut min: i32 = -1;
            for slot in ids2.iter_mut().take(o) {
                let mut min2: i32 = 0xFFFF;
                for &id in ids.iter().take(o) {
                    if id as i32 > min && (id as i32) < min2 {
                        min2 = id as i32;
                    }
                }
                min = min2;
                *slot = min as i16;
            }
            ids = ids2;
        }
        for i in 0..o {
            let id = (ids[i] & 0x7F) as usize;
            self.obj[i].obj.copy_from_slice(&oam[id * 4..id * 4 + 4]);
            self.obj[i].index = id as i8;
        }
    }

    /// `GBVideoSoftwareRendererDrawRange`
    pub fn draw_range(&mut self, vram: &[u8], oam: &[u8], start_x: i32, end_x: i32, y: i32) {
        self.last_y = y;
        self.last_x = end_x;
        if start_x >= end_x {
            return;
        }
        let mut maps = BASE_MAP;
        if self.lcdc & LCDC_TILE_MAP != 0 {
            maps += SIZE_MAP;
        }
        if self.disable_bg {
            for x in start_x..end_x {
                self.row[x as usize] = 0;
            }
        }
        if self.lcdc & LCDC_BG_ENABLE != 0 || self.model >= model::CGB {
            let wy = self.wy as i32 + self.current_wy as i32;
            let wx = self.wx as i32 + self.current_wx as i32 - 7;
            if self.lcdc & LCDC_WINDOW != 0 && wy == y && wx <= end_x {
                self.has_window = true;
            }
            if self.lcdc & LCDC_WINDOW != 0 && self.has_window && wx <= end_x && !self.disable_win {
                if wx > 0 && !self.disable_bg {
                    let (sx, sy) = (self.scx as i32, self.scy as i32 + y);
                    self.draw_background(vram, maps, start_x, wx, sx, sy);
                }
                maps = BASE_MAP;
                if self.lcdc & LCDC_WINDOW_TILE_MAP != 0 {
                    maps += SIZE_MAP;
                }
                self.draw_background(vram, maps, wx, end_x, -wx, y - wy);
            } else if !self.disable_bg {
                let (sx, sy) = (self.scx as i32, self.scy as i32 + y);
                self.draw_background(vram, maps, start_x, end_x, sx, sy);
            }
        } else if !self.disable_bg {
            for x in start_x..end_x {
                self.row[x as usize] = 0;
            }
        }

        if start_x == 0 {
            self.clean_oam(oam, y);
        }
        if self.lcdc & LCDC_OBJ_ENABLE != 0 && !self.disable_obj {
            for i in 0..self.obj_max {
                let sprite = self.obj[i];
                self.draw_obj(vram, &sprite, start_x, end_x, y);
            }
        }

        let sgb_offset = if self.uses_borders() { self.width * 40 + 48 } else { 0 };
        let row_base = self.width * y as usize + sgb_offset;
        let sgb_attrs = (self.model & (model::SGB | model::CGB)) == model::SGB;
        let attr = |r: &GbRenderer, x: i32| -> usize {
            if !sgb_attrs {
                return 0;
            }
            let i = (x >> 5) as usize + 5 * (y >> 3) as usize;
            let mut p = r.sgb.attributes.get(i).copied().unwrap_or(0) as i32;
            p >>= 6 - ((x / 4) & 0x6);
            p &= 3;
            (p << 2) as usize
        };
        let mut x = start_x;
        match self.sgb.render_mode {
            0 => {
                let mut p = attr(self, start_x);
                while x < ((start_x + 7) & !7) && x < end_x {
                    self.output[row_base + x as usize] =
                        self.palette[p | self.lookup[(self.row[x as usize] & OBJ_PRIO_MASK) as usize] as usize];
                    x += 1;
                }
                while x + 7 < (end_x & !7) {
                    p = attr(self, x);
                    for k in 0..8 {
                        let xi = (x + k) as usize;
                        self.output[row_base + xi] = self.palette[p | self.lookup[(self.row[xi] & OBJ_PRIO_MASK) as usize] as usize];
                    }
                    x += 8;
                }
                p = attr(self, x);
                while x < end_x {
                    self.output[row_base + x as usize] =
                        self.palette[p | self.lookup[(self.row[x as usize] & OBJ_PRIO_MASK) as usize] as usize];
                    x += 1;
                }
                self.draw_border_mask(row_base, start_x, end_x, y);
            }
            1 => {}
            2 => {
                for x in start_x..end_x {
                    self.output[row_base + x as usize] = 0;
                }
            }
            _ => {
                let c = self.palette[0];
                for x in start_x..end_x {
                    self.output[row_base + x as usize] = c;
                }
            }
        }
    }

    /// The SGB border tiles drawn over the Game Boy screen (end of `drawRange`, mode 0).
    fn draw_border_mask(&mut self, row_base: usize, start_x: i32, end_x: i32, y: i32) {
        let Some(&border_mask) = self.sgb_border_mask.get((y >> 3) as usize) else { return };
        if border_mask == 0 || self.sgb.map_ram.len() < SGB_SIZE_MAP_RAM {
            return;
        }
        let local_y = (y & 7) as usize;
        let mut x = start_x;
        while x < end_x {
            if border_mask & (1 << (x >> 3)) == 0 {
                x += 8;
                continue;
            }
            let mo = (x as usize >> 2) + 12 + (y as usize & !7) * 8 + 320;
            let map_data = self.sgb.map_ram[mo] as u16 | ((self.sgb.map_ram[mo + 1] as u16) << 8);
            let tile = (map_data & 0x3FF) as usize;
            if tile >= 0x100 {
                x += 8;
                continue;
            }
            let y_flip = if map_data & 0x8000 != 0 { 7 } else { 0 };
            let tile_base = (tile * 16 + (local_y ^ y_flip)) * 2;
            let td = [
                self.sgb.char_ram[tile_base],
                self.sgb.char_ram[tile_base + 1],
                self.sgb.char_ram[tile_base + 0x10],
                self.sgb.char_ram[tile_base + 0x11],
            ];
            let palette_base = ((map_data >> 10) & 7) as usize * 0x10;
            let flip = if map_data & 0x4000 != 0 { 7 } else { 0 };
            for i in (0..8).rev() {
                let sel = ((td[0] >> i) & 1) | (((td[1] >> i) & 1) << 1) | (((td[2] >> i) & 1) << 2) | (((td[3] >> i) & 1) << 3);
                if sel != 0 {
                    let px = ((x as usize + 7 - i) ^ flip) + row_base;
                    if let Some(o) = self.output.get_mut(px) {
                        *o = self.palette[palette_base | sel as usize];
                    }
                }
            }
            x += 8;
        }
    }

    /// `GBVideoSoftwareRendererFinishScanline`
    pub fn finish_scanline(&mut self, y: i32) {
        self.last_x = 0;
        self.current_wx = 0;

        if self.sgb_transfer == 1 {
            let offset = 2 * ((y as usize & 7) + (y as usize >> 3) * HORIZONTAL_PIXELS);
            if offset >= 0x1000 {
                return;
            }
            let pkt1 = self.sgb_packet[1];
            let row = self.row;
            let buffer: Option<(&mut Vec<u8>, usize)> = match self.sgb_command_header >> 3 {
                sgb::PAL_TRN => Some((&mut self.sgb.pal_ram, 0)),
                sgb::CHR_TRN => Some((&mut self.sgb.char_ram, SGB_SIZE_CHAR_RAM / 2 * (pkt1 & 1) as usize)),
                sgb::PCT_TRN => Some((&mut self.sgb.map_ram, 0)),
                sgb::ATTR_TRN => Some((&mut self.sgb.attribute_files, 0)),
                _ => None,
            };
            if let Some((buffer, base)) = buffer {
                let mut i = 0;
                while i < HORIZONTAL_PIXELS {
                    if offset + (i << 1) + 1 >= 0x1000 {
                        break;
                    }
                    let mut hi = 0u8;
                    let mut lo = 0u8;
                    for k in 0..8 {
                        let px = row[i + k];
                        hi |= (((px & 0x2) >> 1) as u8) << (7 - k);
                        lo |= ((px & 0x1) as u8) << (7 - k);
                    }
                    let o = base + offset + (i << 1);
                    if o + 1 < buffer.len() {
                        buffer[o] = lo;
                        buffer[o + 1] = hi;
                    }
                    i += 8;
                }
            }
        }
    }

    /// `GBVideoSoftwareRendererFinishFrame`
    pub fn finish_frame(&mut self) {
        if self.lcdc & LCDC_ENABLE == 0 {
            self.clear_screen();
        }
        if self.model & model::SGB != 0 {
            match self.sgb_command_header >> 3 {
                sgb::PAL_SET | sgb::ATTR_SET => {
                    if self.sgb_packet[1] & 0x40 != 0 {
                        self.sgb.render_mode = 0;
                        if self.sgb_borders {
                            self.regenerate_sgb_border();
                        }
                    }
                }
                sgb::PAL_TRN | sgb::CHR_TRN | sgb::PCT_TRN | sgb::ATRC_EN | sgb::MASK_EN | sgb::ATTR_TRN => {
                    if self.sgb_command_header >> 3 != sgb::ATTR_TRN && self.sgb_borders && self.sgb.render_mode == 0 {
                        // Make sure every buffer sees this if we're multibuffering
                        self.regenerate_sgb_border();
                    }
                    self.sgb_transfer += 1;
                    if self.sgb_transfer == 5 {
                        self.sgb_command_header = 0;
                    }
                }
                _ => {}
            }
        }
        self.last_y = VERTICAL_PIXELS as i32;
        self.last_x = 0;
        self.current_wy = 0;
        self.current_wx = 0;
        self.has_window = false;
    }

    /// `GBVideoSoftwareRendererDrawBackground`
    fn draw_background(&mut self, vram: &[u8], maps: usize, start_x: i32, end_x: i32, sx: i32, sy: i32) {
        let tile_data_signed = self.lcdc & LCDC_TILE_DATA == 0;
        let data = if tile_data_signed { 0x1000usize } else { 0 };
        let attr = maps + SIZE_VRAM_BANK0;
        let top_y = (((sy >> 3) & 0x1F) * 0x20) as usize;
        let bottom_y = sy & 7;
        let mut start_x = start_x.max(0);
        let cgb = self.model >= model::CGB;
        let bg_enable = self.lcdc & LCDC_BG_ENABLE != 0;
        let tile_at = |top_x: usize| -> i32 {
            let raw = vram[maps + top_x + top_y];
            if tile_data_signed {
                raw as i8 as i32
            } else {
                raw as i32
            }
        };
        let tile_addr = |local_data: usize, bg_tile: i32, local_y: i32| -> usize { (local_data as i32 + (bg_tile * 8 + local_y) * 2) as usize };
        if (start_x + sx) & 7 != 0 {
            let start_x2 = start_x + 8 - ((start_x + sx) & 7);
            for x in start_x..start_x2 {
                let mut local_data = data;
                let mut local_y = bottom_y;
                let top_x = (((x + sx) >> 3) & 0x1F) as usize;
                let mut bottom_x = 7 - ((x + sx) & 7);
                let bg_tile = tile_at(top_x);
                let mut p = PAL_BG;
                if cgb {
                    let attrs = vram[attr + top_x + top_y];
                    p |= (attrs & 7) as u16 * 4;
                    if attrs & ATTR_PRIORITY != 0 && bg_enable {
                        p |= OBJ_PRIORITY;
                    }
                    if attrs & ATTR_BANK != 0 {
                        local_data += SIZE_VRAM_BANK0;
                    }
                    if attrs & ATTR_YFLIP != 0 {
                        local_y = 7 - bottom_y;
                    }
                    if attrs & ATTR_XFLIP != 0 {
                        bottom_x = 7 - bottom_x;
                    }
                }
                let a = tile_addr(local_data, bg_tile, local_y);
                let lower = vram[a] >> bottom_x;
                let upper = vram[a + 1] >> bottom_x;
                if (0..(HORIZONTAL_PIXELS as i32 + 8)).contains(&x) {
                    self.row[x as usize] = p | (((upper & 1) << 1) | (lower & 1)) as u16;
                }
            }
            start_x = start_x2;
        }
        let mut x = start_x;
        while x < end_x {
            let mut local_data = data;
            let mut local_y = bottom_y;
            let top_x = (((x + sx) >> 3) & 0x1F) as usize;
            let bg_tile = tile_at(top_x);
            let mut p = PAL_BG;
            let mut xflip = false;
            if cgb {
                let attrs = vram[attr + top_x + top_y];
                p |= (attrs & 7) as u16 * 4;
                if attrs & ATTR_PRIORITY != 0 && bg_enable {
                    p |= OBJ_PRIORITY;
                }
                if attrs & ATTR_BANK != 0 {
                    local_data += SIZE_VRAM_BANK0;
                }
                if attrs & ATTR_YFLIP != 0 {
                    local_y = 7 - bottom_y;
                }
                xflip = attrs & ATTR_XFLIP != 0;
            }
            let a = tile_addr(local_data, bg_tile, local_y);
            let lower = vram[a];
            let upper = vram[a + 1];
            for k in 0..8 {
                let bit = if xflip { k } else { 7 - k };
                let xi = (x + k) as usize;
                if xi < self.row.len() {
                    self.row[xi] = p | ((((upper >> bit) & 1) << 1) | ((lower >> bit) & 1)) as u16;
                }
            }
            x += 8;
        }
    }

    /// `GBVideoSoftwareRendererDrawObj`
    fn draw_obj(&mut self, vram: &[u8], sprite: &Sprite, start_x: i32, end_x: i32, y: i32) {
        let [oy, ox, tile, attr] = sprite.obj;
        let obj_x = ox as i32;
        let ix = obj_x - 8;
        if end_x < ix || start_x >= ix + 8 {
            return;
        }
        let mut end_x = end_x;
        let mut start_x = start_x;
        if obj_x < end_x {
            end_x = obj_x;
        }
        if obj_x - 8 > start_x {
            start_x = obj_x - 8;
        }
        if start_x < 0 {
            start_x = 0;
        }
        let mut data = 0usize;
        let mut tile_offset = 0i32;
        let bottom_y;
        let obj_y = oy as i32;
        let tall = self.lcdc & LCDC_OBJ_SIZE != 0;
        if attr & ATTR_YFLIP != 0 {
            bottom_y = 7 - ((y - obj_y - 16) & 7);
            if tall && y - obj_y < -8 {
                tile_offset += 1;
            }
        } else {
            bottom_y = (y - obj_y - 16) & 7;
            if tall && y - obj_y >= -8 {
                tile_offset += 1;
            }
        }
        if tall && tile & 1 != 0 {
            tile_offset -= 1;
        }
        let mut mask: u16 = if attr & ATTR_PRIORITY != 0 { 0x63 } else { 0x60 };
        let mut mask2: u16 = if attr & ATTR_PRIORITY != 0 { 0 } else { OBJ_PRIORITY | 3 };
        let mut p = PAL_OBJ;
        if self.model >= model::CGB {
            p |= (attr & 7) as u16 * 4;
            if attr & ATTR_BANK != 0 {
                data += SIZE_VRAM_BANK0;
            }
            if self.lcdc & LCDC_BG_ENABLE == 0 {
                mask = 0x60;
                mask2 = OBJ_PRIORITY | 3;
            }
        } else {
            p |= ((((attr & ATTR_PALETTE) >> 4) + 8) as u16) * 4;
        }
        let obj_tile = tile as i32 + tile_offset;
        let a = (data as i32 + (obj_tile * 8 + bottom_y) * 2) as usize;
        let lower = vram[a & 0x3FFF];
        let upper = vram[(a + 1) & 0x3FFF];
        let xflip = attr & ATTR_XFLIP != 0;
        let put = |row: &mut [u16], x: usize, bit: i32| {
            let current = row[x];
            if ((upper | lower) >> bit) & 1 != 0 && current & mask == 0 && (current & mask2) <= OBJ_PRIORITY {
                row[x] = p | ((((upper >> bit) & 1) << 1) | ((lower >> bit) & 1)) as u16;
            }
        };
        let mut x = start_x;
        if (x - obj_x) & 7 != 0 {
            while x < end_x {
                let bottom_x = if xflip { (x - obj_x) & 7 } else { 7 - ((x - obj_x) & 7) };
                put(&mut self.row, x as usize, bottom_x);
                x += 1;
            }
        } else if xflip {
            for k in 0..8 {
                put(&mut self.row, (x + k) as usize, k);
            }
        } else {
            for k in (0..8).rev() {
                put(&mut self.row, (x + 7 - k) as usize, k);
            }
        }
    }
}
