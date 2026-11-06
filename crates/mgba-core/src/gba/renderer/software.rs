// Copyright (c) 2013-2015 Jeffrey Pfau
// Rust port of mGBA's software video renderer (src/gba/renderers/video-software.c,
// software-bg.c, software-mode0.c, software-obj.c, software-private.h, common.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::gba::regs::*;
use crate::gba::video::{OBJ_SIZES, VIDEO_VERTICAL_TOTAL_PIXELS};
use crate::gba::{VIDEO_HORIZONTAL_PIXELS as W, VIDEO_VERTICAL_PIXELS as H};

const OFFSET_PRIORITY: u32 = 30;
const OFFSET_INDEX: u32 = 28;

const FLAG_PRIORITY: u32 = 0xC0000000;
const FLAG_IS_BACKGROUND: u32 = 0x08000000;
const FLAG_UNWRITTEN: u32 = 0xFC000000;
const FLAG_REBLEND: u32 = 0x04000000;
const FLAG_TARGET_1: u32 = 0x02000000;
const FLAG_TARGET_2: u32 = 0x01000000;
const FLAG_OBJWIN: u32 = 0x01000000;
const FLAG_ORDER_MASK: u32 = 0xF8000000;

const ENABLED_MAX: i32 = 4;
const MAX_WINDOW: usize = 5;
const BASE_TILE: usize = 0x00010000;

const OBJ_HBLANK_FREE_LENGTH: i32 = 954;
const OBJ_LENGTH: i32 = 1210;

const BLEND_NONE: u32 = 0;
const BLEND_ALPHA: u32 = 1;
const BLEND_BRIGHTEN: u32 = 2;
const BLEND_DARKEN: u32 = 3;

const OBJ_MODE_SEMITRANSPARENT: u16 = 1;
const OBJ_MODE_OBJWIN: u16 = 2;

pub const COLOR_WHITE: u32 = 0x00FFFFFF;
const COLOR_RED: u32 = 0x000000FF;
const COLOR_GREEN: u32 = 0x0000FF00;
const COLOR_BLUE: u32 = 0x00FF0000;

/// Row buffers carry slack past the visible width, like the adjacent
/// arrays in mGBA's struct, so edge-case overdraw stays harmless.
const ROW_LEN: usize = W + 16;

#[inline(always)]
fn is_writable(pixel: u32) -> bool {
    pixel & 0xFE000000 != 0
}

/// `mColorFrom555` (32-bit, R in the low byte)
#[inline]
pub fn color_from_555(value: u16) -> u32 {
    let v = value as u32;
    let r = v & 0x1F;
    let g = (v >> 5) & 0x1F;
    let b = (v >> 10) & 0x1F;
    let mut color = (r << 3) | (g << 11) | (b << 19);
    color |= (color >> 5) & 0x070707;
    color
}

/// `mColorMix5Bit` (32-bit)
#[inline]
fn mix_5bit(weight_a: u32, color_a: u32, weight_b: u32, color_b: u32) -> u32 {
    let mut c: u32 = 0;
    let a = color_a & 0xFF;
    let b = color_b & 0xFF;
    c |= ((a * weight_a + b * weight_b) / 16) & 0x1FF;
    if c & 0x00000100 != 0 {
        c = 0x000000FF;
    }
    let a = color_a & 0xFF00;
    let b = color_b & 0xFF00;
    c |= ((a * weight_a + b * weight_b) / 16) & 0x1FF00;
    if c & 0x00010000 != 0 {
        c = (c & 0x000000FF) | 0x0000FF00;
    }
    let a = color_a & 0xFF0000;
    let b = color_b & 0xFF0000;
    c |= ((a as u64 * weight_a as u64 + b as u64 * weight_b as u64) / 16) as u32 & 0x1FF0000;
    if c & 0x01000000 != 0 {
        c = (c & 0x0000FFFF) | 0x00FF0000;
    }
    c
}

#[inline]
fn brighten(color: u32, y: u32) -> u32 {
    let mut c = 0;
    let a = color & 0xFF;
    c |= (a + ((0xFF - a) * y) / 16) & 0xFF;
    let a = color & 0xFF00;
    c |= (a + ((0xFF00 - a) * y) / 16) & 0xFF00;
    let a = color & 0xFF0000;
    c |= (a + ((0xFF0000 - a) * y) / 16) & 0xFF0000;
    c
}

#[inline]
fn darken(color: u32, y: u32) -> u32 {
    let mut c = 0;
    let a = color & 0xFF;
    c |= (a - (a * y) / 16) & 0xFF;
    let a = color & 0xFF00;
    c |= (a - (a * y) / 16) & 0xFF00;
    let a = color & 0xFF0000;
    c |= (a - (a * y) / 16) & 0xFF0000;
    c
}

#[inline(always)]
fn vram32(vram: &[u16], byte_off: usize) -> u32 {
    let i = (byte_off >> 1) % vram.len();
    let j = (i + 1) % vram.len();
    vram[i] as u32 | ((vram[j] as u32) << 16)
}

#[inline(always)]
fn vram8(vram: &[u16], byte_off: usize) -> u32 {
    let i = (byte_off >> 1) % vram.len();
    ((vram[i] >> ((byte_off & 1) * 8)) & 0xFF) as u32
}

#[derive(Clone, Copy, Default, Debug)]
struct WindowControl {
    packed: u8,
    priority: i8,
}

impl WindowControl {
    #[inline]
    fn bg_enable(&self, i: usize) -> bool {
        self.packed & (1 << i) != 0
    }
    #[inline]
    fn obj_enable(&self) -> bool {
        self.packed & 0x10 != 0
    }
    #[inline]
    fn blend_enable(&self) -> bool {
        self.packed & 0x20 != 0
    }
}

#[derive(Clone, Copy, Default, Debug)]
struct Window {
    end_x: u8,
    control: WindowControl,
}

#[derive(Clone, Copy, Default, Debug)]
struct WindowRegion {
    end: u8,
    start: u8,
}

#[derive(Clone, Copy, Default, Debug)]
struct WindowN {
    h: WindowRegion,
    v: WindowRegion,
    control: WindowControl,
    #[allow(dead_code)]
    offset_x: i16,
    offset_y: i16,
    on: bool,
}

#[derive(Clone, Copy, Default, Debug)]
struct Background {
    index: u32,
    enabled: i32,
    priority: u32,
    char_base: u32,
    mosaic: bool,
    multipalette: bool,
    screen_base: u32,
    overflow: bool,
    size: u32,
    target1: bool,
    target2: bool,
    x: u16,
    y: u16,
    refx: i32,
    refy: i32,
    dx: i16,
    dmx: i16,
    dy: i16,
    dmy: i16,
    sx: i32,
    sy: i32,
    flags: u32,
    objwin_flags: u32,
    objwin_force_enable: bool,
    objwin_only: bool,
    variant: bool,
    offset_x: i32,
    offset_y: i32,
}

#[derive(Clone, Copy, Default, Debug)]
struct Obj {
    a: u16,
    b: u16,
    c: u16,
}

impl Obj {
    fn y(&self) -> i32 {
        (self.a & 0xFF) as i32
    }
    fn transformed(&self) -> bool {
        self.a & 0x100 != 0
    }
    fn disable(&self) -> bool {
        self.a & 0x200 != 0
    }
    fn double_size(&self) -> u32 {
        ((self.a >> 9) & 1) as u32
    }
    fn mode(&self) -> u16 {
        (self.a >> 10) & 3
    }
    fn mosaic(&self) -> bool {
        self.a & 0x1000 != 0
    }
    fn is_256(&self) -> bool {
        self.a & 0x2000 != 0
    }
    fn shape(&self) -> usize {
        ((self.a >> 14) & 3) as usize
    }
    fn x(&self) -> u32 {
        (self.b & 0x1FF) as u32
    }
    fn mat_index(&self) -> usize {
        ((self.b >> 9) & 0x1F) as usize
    }
    fn hflip(&self) -> bool {
        self.b & 0x1000 != 0
    }
    fn vflip(&self) -> bool {
        self.b & 0x2000 != 0
    }
    fn size(&self) -> usize {
        ((self.b >> 14) & 3) as usize
    }
    fn tile(&self) -> u32 {
        (self.c & 0x3FF) as u32
    }
    fn priority(&self) -> u32 {
        ((self.c >> 10) & 3) as u32
    }
    fn palette(&self) -> usize {
        ((self.c >> 12) & 0xF) as usize
    }
}

#[derive(Clone, Copy, Default, Debug)]
struct Sprite {
    obj: Obj,
    y: i32,
    end_y: i32,
    cycles: i32,
    index: i32,
}

#[derive(Clone, Copy)]
struct ScanlineCache {
    io: [u16; 0x30],
    scale: [[i32; 2]; 2],
    window_on: [bool; 2],
}

impl Default for ScanlineCache {
    fn default() -> Self {
        ScanlineCache { io: [0; 0x30], scale: [[0; 2]; 2], window_on: [false; 2] }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pal {
    Normal,
    Variant,
}

/// Per-background drawing context (the locals set up by BACKGROUND_BITMAP_INIT / PREPARE_OBJWIN).
#[derive(Clone, Copy)]
struct BgCtx {
    flags: u32,
    objwin_flags: u32,
    palette: Pal,
    objwin_palette: Pal,
    objwin_slow: bool,
    blend: bool,
    objwin_force_enable: bool,
    objwin_only: bool,
}

/// `struct GBAVideoSoftwareRenderer`
#[derive(Clone)]
pub struct SoftwareRenderer {
    output: Vec<u32>,
    dispcnt: u16,
    row: [u32; ROW_LEN],
    sprite_layer: [u32; ROW_LEN],
    sprite_cycles_remaining: i32,

    target1_obj: bool,
    target1_bd: bool,
    target2_obj: bool,
    target2_bd: bool,
    blend_dirty: bool,
    blend_effect: u32,
    normal_palette: [u32; 512],
    variant_palette: [u32; 512],

    blda: u32,
    bldb: u32,
    bldy: u32,

    mosaic: u16,
    stereo: bool,

    win_n: [WindowN; 2],
    winout: WindowControl,
    objwin: WindowControl,
    current_window: WindowControl,
    n_windows: usize,
    windows: [Window; MAX_WINDOW + 1],

    bg: [Background; 4],

    force_target1: bool,
    oam_dirty: bool,
    oam_max: usize,
    sprites: [Sprite; 128],
    obj_offset_x: i32,
    obj_offset_y: i32,

    scanline_dirty: [u32; 5],
    next_io: [u16; 0x30],
    cache: Vec<ScanlineCache>,
    next_y: usize,

    start: i32,
    end: i32,

    /// Layer toggles exposed to the frontend (renderer->d.disableBG etc.).
    pub disable_bg: [bool; 4],
    pub disable_obj: bool,
    pub disable_win: [bool; 2],
    pub disable_objwin: bool,
}

impl Default for SoftwareRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl SoftwareRenderer {
    pub fn new() -> SoftwareRenderer {
        let mut r = SoftwareRenderer {
            output: vec![COLOR_WHITE; W * H],
            dispcnt: 0x80,
            row: [0; ROW_LEN],
            sprite_layer: [0; ROW_LEN],
            sprite_cycles_remaining: 0,
            target1_obj: false,
            target1_bd: false,
            target2_obj: false,
            target2_bd: false,
            blend_dirty: false,
            blend_effect: BLEND_NONE,
            normal_palette: [0; 512],
            variant_palette: [0; 512],
            blda: 0,
            bldb: 0,
            bldy: 0,
            mosaic: 0,
            stereo: false,
            win_n: [WindowN::default(); 2],
            winout: WindowControl::default(),
            objwin: WindowControl::default(),
            current_window: WindowControl::default(),
            n_windows: 1,
            windows: [Window::default(); MAX_WINDOW + 1],
            bg: [Background::default(); 4],
            force_target1: false,
            oam_dirty: true,
            oam_max: 0,
            sprites: [Sprite::default(); 128],
            obj_offset_x: 0,
            obj_offset_y: 0,
            scanline_dirty: [0xFFFFFFFF; 5],
            next_io: [0; 0x30],
            cache: vec![ScanlineCache::default(); H],
            next_y: 0,
            start: 0,
            end: 0,
            disable_bg: [false; 4],
            disable_obj: false,
            disable_win: [false; 2],
            disable_objwin: false,
        };
        r.reset();
        r
    }

    /// The completed frame: 240x160 pixels, 0x00BBGGRR.
    pub fn output(&self) -> &[u32] {
        &self.output
    }

    #[inline(always)]
    fn pal(&self, p: Pal) -> &[u32; 512] {
        match p {
            Pal::Normal => &self.normal_palette,
            Pal::Variant => &self.variant_palette,
        }
    }

    fn dirty_all(&mut self) {
        self.scanline_dirty = [0xFFFFFFFF; 5];
    }

    fn dirty_scanline(&mut self, y: usize) {
        self.scanline_dirty[y >> 5] |= 1 << (y & 0x1F);
    }

    /// `GBAVideoSoftwareRendererReset`. The palette is re-derived by `sync_palette`.
    pub fn reset(&mut self) {
        self.dispcnt = 0x0080;
        self.target1_obj = false;
        self.target1_bd = false;
        self.target2_obj = false;
        self.target2_bd = false;
        self.blend_effect = BLEND_NONE;
        self.blend_dirty = false;
        self.update_palettes();

        self.blda = 0;
        self.bldb = 0;
        self.bldy = 0;

        self.win_n[0] = WindowN { control: WindowControl { packed: 0, priority: 0 }, ..Default::default() };
        self.win_n[1] = WindowN { control: WindowControl { packed: 0, priority: 1 }, ..Default::default() };
        self.objwin = WindowControl { packed: 0, priority: 2 };
        self.winout = WindowControl { packed: 0, priority: 3 };
        self.oam_dirty = true;
        self.oam_max = 0;

        self.mosaic = 0;
        self.stereo = false;
        self.next_y = 0;

        self.obj_offset_x = 0;
        self.obj_offset_y = 0;

        self.dirty_all();
        for c in self.cache.iter_mut() {
            *c = ScanlineCache::default();
        }
        self.next_io = [0; 0x30];

        for i in 0..4 {
            self.bg[i] = Background { index: i as u32, dx: 256, dmy: 256, ..Default::default() };
        }
    }

    /// Re-run the palette write hook over the whole palette.
    pub fn sync_palette(&mut self, palette: &[u16]) {
        for (i, &entry) in palette.iter().enumerate().take(512) {
            self.write_palette((i * 2) as u32, entry);
        }
    }

    /// `GBAVideoSoftwareRendererWriteVideoRegister`
    pub fn write_video_register(&mut self, address: u32, mut value: u16) -> u16 {
        match address {
            DISPCNT => {
                value &= 0xFFF7;
                self.dispcnt = value;
                self.update_dispcnt();
            }
            STEREOCNT => self.stereo = value & 1 != 0,
            BG0CNT | BG1CNT => {
                value &= 0xDFFF;
                self.write_bgcnt(((address - BG0CNT) >> 1) as usize, value);
            }
            BG2CNT | BG3CNT => {
                self.write_bgcnt(((address - BG0CNT) >> 1) as usize, value);
            }
            BG0HOFS | BG1HOFS | BG2HOFS | BG3HOFS => {
                value &= 0x01FF;
                self.bg[((address - BG0HOFS) >> 2) as usize].x = value;
            }
            BG0VOFS | BG1VOFS | BG2VOFS | BG3VOFS => {
                value &= 0x01FF;
                self.bg[((address - BG0VOFS) >> 2) as usize].y = value;
            }
            BG2PA => self.bg[2].dx = value as i16,
            BG2PB => self.bg[2].dmx = value as i16,
            BG2PC => self.bg[2].dy = value as i16,
            BG2PD => self.bg[2].dmy = value as i16,
            BG3PA => self.bg[3].dx = value as i16,
            BG3PB => self.bg[3].dmx = value as i16,
            BG3PC => self.bg[3].dy = value as i16,
            BG3PD => self.bg[3].dmy = value as i16,
            BG2X_LO | BG2X_HI | BG2Y_LO | BG2Y_HI | BG3X_LO | BG3X_HI | BG3Y_LO | BG3Y_HI => {
                let i = if address < BG3X_LO { 2 } else { 3 };
                let ny = self.next_y;
                let bg = &mut self.bg[i];
                let is_x = matches!(address, BG2X_LO | BG2X_HI | BG3X_LO | BG3X_HI);
                let hi = matches!(address, BG2X_HI | BG2Y_HI | BG3X_HI | BG3Y_HI);
                let r = if is_x { &mut bg.refx } else { &mut bg.refy };
                if hi {
                    *r = (*r & 0x0000FFFF) | ((value as i32) << 16);
                    *r <<= 4;
                    *r >>= 4;
                } else {
                    *r = (*r & 0xFFFF0000u32 as i32) | value as i32;
                }
                let rv = *r;
                if is_x {
                    bg.sx = rv;
                } else {
                    bg.sy = rv;
                }
                let cached = self.cache[ny].scale[i - 2][if is_x { 0 } else { 1 }];
                if rv != cached {
                    self.dirty_scanline(ny);
                }
            }
            BLDCNT => {
                self.write_bldcnt(value);
                value &= 0x3FFF;
            }
            BLDALPHA => {
                self.blda = (value & 0x1F).min(0x10) as u32;
                self.bldb = ((value >> 8) & 0x1F).min(0x10) as u32;
                value &= 0x1F1F;
            }
            BLDY => {
                value &= 0x1F;
                if value > 0x10 {
                    value = 0x10;
                }
                if self.bldy != value as u32 {
                    self.bldy = value as u32;
                    self.blend_dirty = true;
                }
            }
            WIN0H | WIN1H => {
                let w = &mut self.win_n[((address - WIN0H) >> 1) as usize];
                w.h.end = value as u8;
                w.h.start = (value >> 8) as u8;
                if w.h.start as usize > W && w.h.start > w.h.end {
                    w.h.start = 0;
                }
                if w.h.end as usize > W {
                    w.h.end = W as u8;
                    if w.h.start as usize > W {
                        w.h.start = W as u8;
                    }
                }
            }
            WIN0V | WIN1V => {
                let w = &mut self.win_n[((address - WIN0V) >> 1) as usize];
                w.v.end = value as u8;
                w.v.start = (value >> 8) as u8;
            }
            WININ => {
                value &= 0x3F3F;
                self.win_n[0].control.packed = value as u8;
                self.win_n[1].control.packed = (value >> 8) as u8;
            }
            WINOUT => {
                value &= 0x3F3F;
                self.winout.packed = value as u8;
                self.objwin.packed = (value >> 8) as u8;
            }
            MOSAIC => self.mosaic = value,
            _ => {}
        }
        let idx = (address >> 1) as usize;
        if idx < 0x30 {
            self.next_io[idx] = value;
            if self.cache[self.next_y].io[idx] != value {
                self.cache[self.next_y].io[idx] = value;
                let ny = self.next_y;
                self.dirty_scanline(ny);
            }
        }
        value
    }

    /// `GBAVideoSoftwareRendererWriteVRAM`
    #[inline]
    pub fn write_vram(&mut self, _address: u32) {
        self.dirty_all();
    }

    /// `GBAVideoSoftwareRendererWriteOAM`
    pub fn write_oam(&mut self, _oam: u32) {
        self.oam_dirty = true;
        self.dirty_all();
    }

    /// `GBAVideoSoftwareRendererWritePalette`
    pub fn write_palette(&mut self, address: u32, value: u16) {
        let color = color_from_555(value);
        let i = ((address >> 1) & 0x1FF) as usize;
        self.normal_palette[i] = color;
        if self.blend_effect == BLEND_BRIGHTEN {
            self.variant_palette[i] = brighten(color, self.bldy);
        } else if self.blend_effect == BLEND_DARKEN {
            self.variant_palette[i] = darken(color, self.bldy);
        }
        self.dirty_all();
    }

    fn write_bgcnt(&mut self, i: usize, value: u16) {
        let bg = &mut self.bg[i];
        bg.priority = (value & 3) as u32;
        bg.char_base = (((value >> 2) & 3) as u32) << 14;
        bg.mosaic = value & 0x40 != 0;
        bg.multipalette = value & 0x80 != 0;
        bg.screen_base = (((value >> 8) & 0x1F) as u32) << 11;
        bg.overflow = value & 0x2000 != 0;
        bg.size = ((value >> 14) & 3) as u32;
        self.update_flags(i);
    }

    fn write_bldcnt(&mut self, value: u16) {
        let old_effect = self.blend_effect;
        for i in 0..4 {
            self.bg[i].target1 = value & (1 << i) != 0;
            self.bg[i].target2 = value & (0x100 << i) != 0;
        }
        self.blend_effect = ((value >> 6) & 3) as u32;
        self.target1_obj = value & 0x10 != 0;
        self.target1_bd = value & 0x20 != 0;
        self.target2_obj = value & 0x1000 != 0;
        self.target2_bd = value & 0x2000 != 0;
        if old_effect != self.blend_effect {
            self.blend_dirty = true;
        }
    }

    fn enable_bg(&mut self, bg: usize, active: bool) {
        let was_active = self.bg[bg].enabled;
        if !active {
            if self.next_y == 0 || (was_active > 0 && was_active < ENABLED_MAX) {
                self.bg[bg].enabled = 0;
            } else if was_active == ENABLED_MAX {
                self.bg[bg].enabled = -2;
            }
        } else if was_active == 0 && active {
            if self.next_y == 0 {
                self.bg[bg].enabled = ENABLED_MAX;
            } else if self.dispcnt & 7 > 2 {
                self.bg[bg].enabled = 2;
            } else {
                self.bg[bg].enabled = 1;
            }
        } else if was_active < 0 && active {
            self.bg[bg].enabled = ENABLED_MAX;
        }
    }

    fn update_dispcnt(&mut self) {
        for i in 0..4 {
            let on = self.dispcnt & (0x100 << i) != 0;
            self.enable_bg(i, on);
        }
    }

    fn update_palettes(&mut self) {
        match self.blend_effect {
            BLEND_BRIGHTEN => {
                for i in 0..512 {
                    self.variant_palette[i] = brighten(self.normal_palette[i], self.bldy);
                }
            }
            BLEND_DARKEN => {
                for i in 0..512 {
                    self.variant_palette[i] = darken(self.normal_palette[i], self.bldy);
                }
            }
            _ => self.variant_palette = self.normal_palette,
        }
    }

    /// `_updateFlags`
    fn update_flags(&mut self, i: usize) {
        let bg = self.bg[i];
        let mut flags = (bg.priority << OFFSET_PRIORITY) | (bg.index << OFFSET_INDEX) | FLAG_IS_BACKGROUND;
        if bg.target2 {
            flags |= FLAG_TARGET_2;
        }
        let mut objwin_flags = flags;
        if self.blend_effect == BLEND_ALPHA {
            if self.blda == 0x10 && self.bldb == 0 {
                flags &= !FLAG_TARGET_2;
                objwin_flags &= !FLAG_TARGET_2;
            } else if bg.target1 {
                if self.current_window.blend_enable() {
                    flags |= FLAG_TARGET_1;
                }
                if self.objwin.blend_enable() {
                    objwin_flags |= FLAG_TARGET_1;
                }
            }
        }
        let bgm = &mut self.bg[i];
        bgm.flags = flags;
        bgm.objwin_flags = objwin_flags;
        bgm.variant = bg.target1
            && self.current_window.blend_enable()
            && (self.blend_effect == BLEND_BRIGHTEN || self.blend_effect == BLEND_DARKEN);
    }

    fn break_window(&mut self, win: WindowN) {
        if win.h.end as usize > W || win.h.end < win.h.start {
            let mut a = win;
            let mut b = win;
            a.h.start = 0;
            b.h.end = W as u8;
            self.break_window_inner(a);
            self.break_window_inner(b);
        } else {
            self.break_window_inner(win);
        }
    }

    fn break_window_inner(&mut self, win: WindowN) {
        let mut start_x = 0;
        if win.h.end > 0 {
            let mut active = 0;
            while active < self.n_windows {
                if win.h.start < self.windows[active].end_x {
                    // Insert a window before the end of the active window
                    let old_window = self.windows[active];
                    if win.h.start as i32 > start_x {
                        // And after the start of the active window
                        let mut next = self.n_windows;
                        self.n_windows += 1;
                        while next > active {
                            if next < self.windows.len() {
                                self.windows[next] = self.windows[next - 1];
                            }
                            next -= 1;
                        }
                        self.windows[active].end_x = win.h.start;
                        active += 1;
                    }
                    if active >= self.windows.len() {
                        return;
                    }
                    self.windows[active].control = win.control;
                    self.windows[active].end_x = win.h.end;
                    if win.h.end >= old_window.end_x {
                        // Trim off extra windows we've overwritten
                        active += 1;
                        while self.n_windows > active + 1 && win.h.end >= self.windows[active].end_x {
                            if active + 1 >= self.windows.len() {
                                return;
                            }
                            self.windows[active] = self.windows[active + 1];
                            self.n_windows -= 1;
                            active += 1;
                        }
                    } else {
                        active += 1;
                        let mut next = self.n_windows;
                        self.n_windows += 1;
                        while next > active {
                            if next < self.windows.len() {
                                self.windows[next] = self.windows[next - 1];
                            }
                            next -= 1;
                        }
                        if active < self.windows.len() {
                            self.windows[active] = old_window;
                        }
                    }
                    break;
                }
                start_x = self.windows[active].end_x as i32;
                active += 1;
            }
        }
        if self.n_windows > MAX_WINDOW {
            self.n_windows = MAX_WINDOW;
        }
    }

    fn prepare_window(&mut self) {
        if self.dispcnt & 0x8000 != 0 {
            for i in 0..4 {
                self.bg[i].objwin_force_enable = self.objwin.bg_enable(i) && self.current_window.bg_enable(i);
                self.bg[i].objwin_only = !self.objwin.bg_enable(i);
            }
        }
        let upd = |r: &mut Self, i: usize| {
            if r.bg[i].enabled == ENABLED_MAX {
                r.update_flags(i);
            }
        };
        match self.dispcnt & 7 {
            0 => {
                upd(self, 0);
                upd(self, 1);
                upd(self, 3);
                upd(self, 2);
            }
            2 => {
                upd(self, 3);
                upd(self, 2);
            }
            3..=5 => upd(self, 2),
            1 => {
                upd(self, 0);
                upd(self, 1);
                upd(self, 2);
            }
            _ => {}
        }
    }

    fn step_window(&mut self, y: usize) {
        for w in self.win_n.iter_mut() {
            if y as i32 == w.v.start as i32 + w.offset_y as i32 {
                w.on = true;
            }
            if y as i32 == w.v.end as i32 + w.offset_y as i32 {
                w.on = false;
            }
        }
    }

    fn test_layer_enabled(&self, x: usize, priority: u32) -> bool {
        !self.disable_bg[x]
            && self.bg[x].enabled == ENABLED_MAX
            && (self.current_window.bg_enable(x) || (self.dispcnt & 0x8000 != 0 && self.objwin.bg_enable(x)))
            && self.bg[x].priority == priority
    }

    /// `GBAVideoSoftwareRendererDrawScanline`
    pub fn draw_scanline(&mut self, y: usize, vram: &[u16], oam: &[u16]) {
        self.next_y = if y == H - 1 { 0 } else { y + 1 };

        let mut dirty = self.scanline_dirty[y >> 5] & (1 << (y & 0x1F)) != 0;
        if self.next_io != self.cache[y].io {
            self.cache[y].io = self.next_io;
            dirty = true;
        }
        if self.dispcnt & 7 != 0 {
            let c = &self.cache[y];
            if c.scale[0][0] != self.bg[2].sx
                || c.scale[0][1] != self.bg[2].sy
                || c.scale[1][0] != self.bg[3].sx
                || c.scale[1][1] != self.bg[3].sy
            {
                dirty = true;
            }
        }
        self.cache[y].scale = [[self.bg[2].sx, self.bg[2].sy], [self.bg[3].sx, self.bg[3].sy]];

        self.step_window(y);
        if self.cache[y].window_on[0] != self.win_n[0].on || self.cache[y].window_on[1] != self.win_n[1].on {
            dirty = true;
        }
        self.cache[y].window_on = [self.win_n[0].on, self.win_n[1].on];

        if !dirty {
            self.advance_affine();
            return;
        }

        self.scanline_dirty[y >> 5] &= !(1 << (y & 0x1F));

        if self.dispcnt & 0x80 != 0 {
            self.output[y * W..(y + 1) * W].fill(COLOR_WHITE);
            return;
        }

        self.preprocess_buffer();
        self.sprite_cycles_remaining = if self.dispcnt & 0x20 != 0 { OBJ_HBLANK_FREE_LENGTH } else { OBJ_LENGTH };
        let sprite_layers = self.preprocess_sprite_layer(y as i32, vram, oam);

        self.end = 0;
        let mode = self.dispcnt & 7;
        for w in 0..self.n_windows {
            self.start = self.end;
            self.end = self.windows[w].end_x as i32;
            self.current_window = self.windows[w].control;
            self.prepare_window();
            for priority in 0..4u32 {
                if sprite_layers & (1 << priority) != 0 {
                    self.postprocess_sprite(priority);
                }
                if self.test_layer_enabled(0, priority) && mode < 2 {
                    self.draw_background_mode0(0, y as i32, vram);
                }
                if self.test_layer_enabled(1, priority) && mode < 2 {
                    self.draw_background_mode0(1, y as i32, vram);
                }
                if self.test_layer_enabled(2, priority) {
                    match mode {
                        0 => self.draw_background_mode0(2, y as i32, vram),
                        1 | 2 => self.draw_background_mode2(2, y as i32, vram),
                        3 => self.draw_background_mode3(2, y as i32, vram),
                        4 => self.draw_background_mode4(2, y as i32, vram),
                        5 => self.draw_background_mode5(2, y as i32, vram),
                        _ => {}
                    }
                }
                if self.test_layer_enabled(3, priority) {
                    match mode {
                        0 => self.draw_background_mode0(3, y as i32, vram),
                        2 => self.draw_background_mode2(3, y as i32, vram),
                        _ => {}
                    }
                }
            }
        }

        self.postprocess_buffer();
        self.advance_affine();

        for i in 0..4 {
            if self.bg[i].enabled != 0 && self.bg[i].enabled < ENABLED_MAX {
                self.bg[i].enabled += 1;
                self.dirty_scanline(y);
            }
        }

        let row = &mut self.output[y * W..(y + 1) * W];
        if self.stereo {
            let mut x = 0;
            while x < W {
                let r = &self.row;
                row[x] = (r[x] & (COLOR_RED | COLOR_BLUE)) | (r[x + 1] & COLOR_GREEN);
                row[x + 1] = (r[x + 1] & (COLOR_RED | COLOR_BLUE)) | (r[x] & COLOR_GREEN);
                row[x + 2] = (r[x + 2] & (COLOR_RED | COLOR_BLUE)) | (r[x + 3] & COLOR_GREEN);
                row[x + 3] = (r[x + 3] & (COLOR_RED | COLOR_BLUE)) | (r[x + 2] & COLOR_GREEN);
                x += 4;
            }
        } else {
            for x in 0..W {
                row[x] = self.row[x] & 0x00FFFFFF;
            }
        }
    }

    fn advance_affine(&mut self) {
        if self.dispcnt & 7 != 0 {
            for i in 2..4 {
                if self.bg[i].enabled == ENABLED_MAX {
                    self.bg[i].sx = self.bg[i].sx.wrapping_add(self.bg[i].dmx as i32);
                    self.bg[i].sy = self.bg[i].sy.wrapping_add(self.bg[i].dmy as i32);
                }
            }
        }
    }

    /// `GBAVideoSoftwareRendererFinishFrame`
    pub fn finish_frame(&mut self) {
        self.next_y = 0;
        for i in 2..4 {
            self.bg[i].sx = self.bg[i].refx;
            self.bg[i].sy = self.bg[i].refy;
        }
        for i in 0..4 {
            if self.bg[i].enabled > 0 {
                self.bg[i].enabled = ENABLED_MAX;
            }
        }
        for win in self.win_n.iter_mut() {
            if win.v.end as usize >= H && (win.v.end as i32) < VIDEO_VERTICAL_TOTAL_PIXELS {
                win.on = false;
            }
            if win.v.start as usize >= H && (win.v.start as i32) < VIDEO_VERTICAL_TOTAL_PIXELS && win.v.start > win.v.end {
                win.on = true;
            }
        }
    }

    fn backdrop_color(&self, w: usize) -> u32 {
        if !self.target1_bd
            || self.blend_effect == BLEND_NONE
            || self.blend_effect == BLEND_ALPHA
            || !self.windows[w].control.blend_enable()
        {
            self.normal_palette[0]
        } else {
            self.variant_palette[0]
        }
    }

    fn preprocess_buffer(&mut self) {
        for x in self.sprite_layer.iter_mut() {
            *x = FLAG_UNWRITTEN;
        }

        self.windows[0].end_x = W as u8;
        self.n_windows = 1;
        if self.dispcnt & 0xE000 != 0 {
            self.windows[0].control = self.winout;
            if self.dispcnt & 0x4000 != 0 && !self.disable_win[1] && self.win_n[1].on {
                let w = self.win_n[1];
                self.break_window(w);
            }
            if self.dispcnt & 0x2000 != 0 && !self.disable_win[0] && self.win_n[0].on {
                let w = self.win_n[0];
                self.break_window(w);
            }
        } else {
            self.windows[0].control.packed = 0xFF;
        }

        self.update_dispcnt();

        if self.blend_dirty {
            self.update_palettes();
            self.blend_dirty = false;
        }
        self.force_target1 = false;

        let mut x = 0usize;
        for w in 0..self.n_windows {
            let backdrop = FLAG_UNWRITTEN | FLAG_PRIORITY | FLAG_IS_BACKGROUND | self.backdrop_color(w);
            let end = (self.windows[w].end_x as usize).min(W);
            while x < end {
                self.row[x] = backdrop;
                x += 1;
            }
        }
    }

    fn postprocess_buffer(&mut self) {
        if (self.force_target1 || self.bg.iter().any(|b| b.target1)) && self.target2_bd {
            let mut x = 0usize;
            for w in 0..self.n_windows {
                let backdrop = self.backdrop_color(w);
                let end = (self.windows[w].end_x as usize).min(W);
                while x < end {
                    let color = self.row[x];
                    if color & FLAG_TARGET_1 != 0 {
                        self.row[x] = mix_5bit(self.bldb, backdrop, self.blda, color);
                    }
                    x += 1;
                }
            }
        }
        if self.force_target1 && (self.blend_effect == BLEND_DARKEN || self.blend_effect == BLEND_BRIGHTEN) {
            let mut x = 0usize;
            for w in 0..self.n_windows {
                let end = (self.windows[w].end_x as usize).min(W);
                let mut mask = FLAG_REBLEND | FLAG_IS_BACKGROUND;
                let mut match_ = FLAG_REBLEND;
                let obj_blend = self.objwin.blend_enable();
                let win_blend = self.windows[w].control.blend_enable();
                if self.dispcnt & 0x8000 != 0 && obj_blend != win_blend {
                    mask |= FLAG_OBJWIN;
                    if obj_blend {
                        match_ |= FLAG_OBJWIN;
                    }
                } else if !win_blend {
                    x = end;
                    continue;
                }
                while x < end {
                    let color = self.row[x];
                    if color & mask == match_ {
                        self.row[x] = if self.blend_effect == BLEND_DARKEN {
                            darken(color, self.bldy)
                        } else {
                            brighten(color, self.bldy)
                        };
                    }
                    x += 1;
                }
            }
        }
    }

    // ----- Compositing (software-private.h) -----

    #[inline(always)]
    fn composite(&mut self, idx: usize, color: u32, current: u32, blend: bool, objwin: bool) {
        let out = if blend {
            if color >= current {
                if current & FLAG_TARGET_1 != 0 && color & FLAG_TARGET_2 != 0 {
                    mix_5bit(self.blda, current, self.bldb, color)
                } else {
                    current & (0x00FFFFFF | FLAG_REBLEND | FLAG_OBJWIN)
                }
            } else if objwin {
                (color & !FLAG_TARGET_2) | (current & FLAG_OBJWIN)
            } else {
                color & !FLAG_TARGET_2
            }
        } else if objwin {
            if color < current {
                color | (current & FLAG_OBJWIN)
            } else {
                current & (0x00FFFFFF | FLAG_REBLEND | FLAG_OBJWIN)
            }
        } else if color >= current {
            current & (0x00FFFFFF | FLAG_REBLEND | FLAG_OBJWIN)
        } else {
            color
        };
        self.row[idx] = out;
    }

    /// COMPOSITE_16/256_{OBJWIN,NO_OBJWIN} for a paletted pixel (index already includes palette bank).
    #[inline(always)]
    fn composite_indexed(&mut self, ctx: &BgCtx, idx: usize, color_index: usize, current: u32) {
        if ctx.objwin_slow {
            if ctx.objwin_force_enable || (current & FLAG_OBJWIN == 0) == ctx.objwin_only {
                let mut merged = ctx.flags;
                let color = if current & FLAG_OBJWIN != 0 {
                    merged = ctx.objwin_flags;
                    self.pal(ctx.objwin_palette)[color_index]
                } else if current & (FLAG_IS_BACKGROUND | FLAG_REBLEND) == FLAG_REBLEND {
                    self.normal_palette[color_index]
                } else {
                    self.pal(ctx.palette)[color_index]
                };
                self.composite(idx, color | merged, current, ctx.blend, true);
            }
        } else {
            let color = if current & (FLAG_IS_BACKGROUND | FLAG_REBLEND) == FLAG_REBLEND {
                self.normal_palette[color_index]
            } else {
                self.pal(ctx.palette)[color_index]
            };
            self.composite(idx, color | ctx.flags, current, ctx.blend, false);
        }
    }

    /// BACKGROUND_DRAW_PIXEL_16
    #[inline(always)]
    fn draw_pixel_16(&mut self, ctx: &BgCtx, idx: i32, tile_data: &mut u32, palette_data: usize) {
        let pixel_data = (*tile_data & 0xF) as usize;
        if (0..ROW_LEN as i32).contains(&idx) {
            let current = self.row[idx as usize];
            if pixel_data != 0 && is_writable(current) {
                self.composite_indexed(ctx, idx as usize, palette_data | pixel_data, current);
            }
        }
        *tile_data >>= 4;
    }

    /// BACKGROUND_DRAW_PIXEL_256
    #[inline(always)]
    fn draw_pixel_256(&mut self, ctx: &BgCtx, idx: i32, tile_data: &mut u32) {
        let pixel_data = (*tile_data & 0xFF) as usize;
        if (0..ROW_LEN as i32).contains(&idx) {
            let current = self.row[idx as usize];
            if pixel_data != 0 && is_writable(current) {
                self.composite_indexed(ctx, idx as usize, pixel_data, current);
            }
        }
        *tile_data >>= 8;
    }

    fn bg_ctx(&self, i: usize) -> BgCtx {
        let bg = &self.bg[i];
        let objwin_slow = self.dispcnt & 0x8000 != 0;
        let mut objwin_palette = Pal::Normal;
        if objwin_slow
            && bg.target1
            && self.objwin.blend_enable()
            && (self.blend_effect == BLEND_BRIGHTEN || self.blend_effect == BLEND_DARKEN)
        {
            objwin_palette = Pal::Variant;
        }
        BgCtx {
            flags: bg.flags,
            objwin_flags: bg.objwin_flags,
            palette: if bg.variant { Pal::Variant } else { Pal::Normal },
            objwin_palette,
            objwin_slow,
            blend: bg.flags & FLAG_TARGET_2 != 0,
            objwin_force_enable: bg.objwin_force_enable,
            objwin_only: bg.objwin_only,
        }
    }

    // ----- Mode 0 (software-mode0.c) -----

    #[allow(unused_assignments)]
    fn draw_background_mode0(&mut self, i: usize, y: i32, vram: &[u16]) {
        let bg = self.bg[i];
        let mut in_x = (self.start + bg.x as i32 - bg.offset_x) & 0x1FF;
        let mut length = self.end - self.start;
        let mut y = y;
        if bg.mosaic {
            let mosaic_v = ((self.mosaic >> 4) & 0xF) as i32 + 1;
            y -= y % mosaic_v;
        }
        let in_y = y + bg.y as i32 - bg.offset_y;

        let mut y_base = (in_y & 0xF8) as u32;
        if bg.size == 2 {
            y_base += (in_y & 0x100) as u32;
        } else if bg.size == 3 {
            y_base += ((in_y & 0x100) << 1) as u32;
        }
        let y_base = (bg.screen_base >> 1) + (y_base << 2);

        let ctx = self.bg_ctx(i);
        let size = bg.size;
        let select_character = |local_x: i32| -> u16 {
            let mut x_base = (local_x & 0xF8) as u32;
            if size & 1 != 0 {
                x_base += ((local_x & 0x100) << 5) as u32;
            }
            let screen_base = y_base + (x_base >> 3);
            vram[(screen_base as usize) % vram.len()]
        };
        let map_cache = |local_x: i32| -> u16 { select_character(((local_x >> 3) & 0x3F) << 3) };

        let mut out_x = self.start;
        let mut tile_x: i32 = 0;
        let tile_end = ((length + in_x) >> 3) - (in_x >> 3);
        let mut pixel: i32 = out_x;
        let mut local_x: i32;
        let mut local_y: i32;
        let mut tile_data: u32;
        let mut map_data: u16;
        let mut palette_data: usize;
        let mut char_base: usize;
        let char_base_of = |map: u16, ly: i32, shift: u32, row_shift: u32| -> usize {
            (bg.char_base as usize + (((map & 0x3FF) as usize) << shift)) + ((ly as usize) << row_shift)
        };
        let vflip = |map: u16| map & 0x0800 != 0;
        let hflip = |map: u16| map & 0x0400 != 0;
        let pal_of = |map: u16| (((map & 0xF000) >> 12) as usize) << 4;

        if !bg.multipalette {
            // ---- 16 colors ----
            if bg.mosaic && (self.mosaic & 0xF) != 0 {
                let mosaic_h = (self.mosaic & 0xF) as i32 + 1;
                let mut mosaic_wait = (mosaic_h - out_x + W as i32 * mosaic_h) % mosaic_h;
                let mut carry_data: u32 = 0;
                palette_data = 0;
                let mut x = in_x & 7;
                if mosaic_wait != 0 {
                    let mut base_x = x - (mosaic_h - mosaic_wait);
                    if base_x < 0 {
                        let disturb_x = (16 + base_x) >> 3;
                        in_x -= disturb_x << 3;
                        local_x = tile_x * 8 + in_x;
                        map_data = select_character(local_x);
                        local_y = in_y & 7;
                        if vflip(map_data) {
                            local_y = 7 - local_y;
                        }
                        base_x -= disturb_x << 3;
                        in_x += disturb_x << 3;
                    } else {
                        local_x = tile_x * 8 + in_x;
                        map_data = select_character(local_x);
                        local_y = in_y & 7;
                        if vflip(map_data) {
                            local_y = 7 - local_y;
                        }
                    }
                    char_base = char_base_of(map_data, local_y, 5, 2);
                    if char_base >= 0x10000 {
                        carry_data = 0;
                    } else {
                        palette_data = pal_of(map_data);
                        tile_data = vram32(vram, char_base);
                        if !hflip(map_data) {
                            tile_data = tile_data.wrapping_shr((4 * base_x) as u32);
                        } else {
                            tile_data = tile_data.wrapping_shr((4 * (7 - base_x)) as u32);
                        }
                        tile_data &= 0xF;
                        tile_data |= tile_data << 4;
                        tile_data |= tile_data << 8;
                        tile_data |= tile_data << 16;
                        carry_data = tile_data;
                    }
                }
                local_x = tile_x * 8 + in_x;
                while length > 0 {
                    map_data = map_cache(local_x);
                    local_x += 8;
                    local_y = in_y & 7;
                    if vflip(map_data) {
                        local_y = 7 - local_y;
                    }
                    char_base = char_base_of(map_data, local_y, 5, 2);
                    tile_data = carry_data;
                    while x < 8 && length > 0 {
                        if mosaic_wait == 0 {
                            if char_base >= 0x10000 {
                                carry_data = 0;
                            } else {
                                palette_data = pal_of(map_data);
                                tile_data = vram32(vram, char_base);
                                if !hflip(map_data) {
                                    tile_data >>= x * 4;
                                } else {
                                    tile_data >>= (7 - x) * 4;
                                }
                                tile_data &= 0xF;
                                tile_data |= tile_data << 4;
                                tile_data |= tile_data << 8;
                                tile_data |= tile_data << 16;
                                carry_data = tile_data;
                            }
                            mosaic_wait = mosaic_h;
                        }
                        mosaic_wait -= 1;
                        self.draw_pixel_16(&ctx, pixel, &mut tile_data, palette_data);
                        pixel += 1;
                        x += 1;
                        length -= 1;
                    }
                    x = 0;
                    tile_x += 1;
                }
                return;
            }

            if in_x & 7 != 0 {
                local_x = tile_x * 8 + in_x;
                map_data = select_character(local_x);
                local_y = in_y & 7;
                if vflip(map_data) {
                    local_y = 7 - local_y;
                }
                let mod8 = in_x & 7;
                let mut end = out_x + 8 - mod8;
                if end > self.end {
                    end = self.end;
                }
                if end <= out_x {
                    return;
                }
                // DRAW_BACKGROUND_MODE_0_TILE_SUFFIX_16
                palette_data = pal_of(map_data);
                char_base = char_base_of(map_data, local_y, 5, 2);
                tile_data = vram32(vram, char_base);
                if !hflip(map_data) {
                    tile_data >>= 4 * mod8;
                    while out_x < end {
                        self.draw_pixel_16(&ctx, pixel, &mut tile_data, palette_data);
                        out_x += 1;
                        pixel += 1;
                    }
                } else {
                    out_x = end - 1;
                    while out_x >= self.start {
                        self.draw_pixel_16(&ctx, out_x, &mut tile_data, palette_data);
                        out_x -= 1;
                    }
                }
                out_x = end;
                if tile_x < tile_end {
                    tile_x += 1;
                }
                length -= end - self.start;
            }
            pixel = out_x;
            out_x += (tile_end - tile_x) * 8;
            local_x = (tile_x * 8 + in_x) & 0x1FF;
            // DRAW_BACKGROUND_MODE_0_TILES_16
            while tile_x < tile_end {
                map_data = map_cache(local_x);
                local_x += 8;
                local_y = in_y & 7;
                if vflip(map_data) {
                    local_y = 7 - local_y;
                }
                palette_data = pal_of(map_data);
                char_base = char_base_of(map_data, local_y, 5, 2);
                if char_base >= 0x10000 {
                    pixel += 8;
                    tile_x += 1;
                    continue;
                }
                if !hflip(map_data) {
                    tile_data = vram32(vram, char_base);
                } else {
                    tile_data = vram32(vram, char_base).swap_bytes();
                    tile_data = ((tile_data & 0xF0F0F0F0) >> 4) | ((tile_data & 0x0F0F0F0F) << 4);
                }
                if tile_data != 0 {
                    for k in 0..8 {
                        self.draw_pixel_16(&ctx, pixel + k, &mut tile_data, palette_data);
                    }
                }
                pixel += 8;
                tile_x += 1;
            }
            if length & 7 != 0 {
                local_x = tile_x * 8 + in_x;
                map_data = select_character(local_x);
                local_y = in_y & 7;
                if vflip(map_data) {
                    local_y = 7 - local_y;
                }
                let mod8 = length & 7;
                // DRAW_BACKGROUND_MODE_0_TILE_PREFIX_16
                char_base = char_base_of(map_data, local_y, 5, 2);
                tile_data = vram32(vram, char_base);
                palette_data = pal_of(map_data);
                pixel = out_x;
                if !hflip(map_data) {
                    if out_x < self.start {
                        tile_data >>= 4 * (self.start - out_x);
                        out_x = self.start;
                        pixel = out_x;
                    }
                    while out_x < self.end {
                        self.draw_pixel_16(&ctx, pixel, &mut tile_data, palette_data);
                        out_x += 1;
                        pixel += 1;
                    }
                } else {
                    tile_data = tile_data.wrapping_shr((4 * (8 - mod8)) as u32);
                    let mut end = self.end - 8;
                    if end < -1 {
                        end = -1;
                    }
                    out_x = self.end - 1;
                    pixel = out_x;
                    while out_x > end {
                        self.draw_pixel_16(&ctx, pixel, &mut tile_data, palette_data);
                        out_x -= 1;
                        pixel -= 1;
                    }
                }
            }
        } else {
            // ---- 256 colors ----
            if bg.mosaic && (self.mosaic & 0xF) != 0 {
                let mosaic_h = (self.mosaic & 0xF) as i32 + 1;
                let mut mosaic_wait = (mosaic_h - out_x + W as i32 * mosaic_h) % mosaic_h;
                let mut carry_data: u32 = 0;
                let mut x = in_x & 7;
                let load256 = |char_base: usize, x: i32, flip: bool| -> u32 {
                    let t = if !flip {
                        if x >= 4 {
                            vram32(vram, char_base + 4).wrapping_shr(((x - 4) * 8) as u32)
                        } else {
                            vram32(vram, char_base).wrapping_shr((x * 8) as u32)
                        }
                    } else if x >= 4 {
                        vram32(vram, char_base).wrapping_shr(((7 - x) * 8) as u32)
                    } else {
                        vram32(vram, char_base + 4).wrapping_shr(((3 - x) * 8) as u32)
                    };
                    t & 0xFF
                };
                if mosaic_wait != 0 {
                    let mut base_x = x - (mosaic_h - mosaic_wait);
                    if base_x < 0 {
                        let disturb_x = (16 + base_x) >> 3;
                        in_x -= disturb_x << 3;
                        local_x = tile_x * 8 + in_x;
                        map_data = select_character(local_x);
                        local_y = in_y & 7;
                        if vflip(map_data) {
                            local_y = 7 - local_y;
                        }
                        base_x -= disturb_x << 3;
                        in_x += disturb_x << 3;
                    } else {
                        local_x = tile_x * 8 + in_x;
                        map_data = select_character(local_x);
                        local_y = in_y & 7;
                        if vflip(map_data) {
                            local_y = 7 - local_y;
                        }
                    }
                    let _ = base_x;
                    char_base = char_base_of(map_data, local_y, 6, 3);
                    if char_base >= 0x10000 {
                        carry_data = 0;
                    } else {
                        carry_data = load256(char_base, x, hflip(map_data));
                    }
                }
                local_x = tile_x * 8 + in_x;
                while length > 0 {
                    map_data = map_cache(local_x);
                    local_x += 8;
                    local_y = in_y & 7;
                    if vflip(map_data) {
                        local_y = 7 - local_y;
                    }
                    char_base = char_base_of(map_data, local_y, 6, 3);
                    tile_data = carry_data;
                    while x < 8 && length > 0 {
                        if mosaic_wait == 0 {
                            if char_base >= 0x10000 {
                                carry_data = 0;
                            } else {
                                tile_data = load256(char_base, x, hflip(map_data));
                                carry_data = tile_data;
                            }
                            mosaic_wait = mosaic_h;
                        }
                        tile_data |= tile_data << 8;
                        mosaic_wait -= 1;
                        self.draw_pixel_256(&ctx, pixel, &mut tile_data);
                        pixel += 1;
                        x += 1;
                        length -= 1;
                    }
                    x = 0;
                    tile_x += 1;
                }
                return;
            }

            if in_x & 7 != 0 {
                local_x = tile_x * 8 + in_x;
                map_data = select_character(local_x);
                local_y = in_y & 7;
                if vflip(map_data) {
                    local_y = 7 - local_y;
                }
                let mod8 = in_x & 7;
                let mut end = out_x + 8 - mod8;
                if end > self.end {
                    end = self.end;
                }
                if end <= out_x {
                    return;
                }
                // DRAW_BACKGROUND_MODE_0_TILE_SUFFIX_256
                char_base = char_base_of(map_data, local_y, 6, 3);
                let end2 = end - 4;
                if !hflip(map_data) {
                    let mut shift = in_x & 3;
                    if char_base < 0x10000 && end2 > out_x {
                        tile_data = vram32(vram, char_base).wrapping_shr((8 * shift) as u32);
                        shift = 0;
                        while out_x < end2 {
                            self.draw_pixel_256(&ctx, pixel, &mut tile_data);
                            out_x += 1;
                            pixel += 1;
                        }
                    }
                    if char_base < 0x10000 {
                        tile_data = vram32(vram, char_base + 4).wrapping_shr((8 * shift) as u32);
                        while out_x < end {
                            self.draw_pixel_256(&ctx, pixel, &mut tile_data);
                            out_x += 1;
                            pixel += 1;
                        }
                    }
                } else {
                    let start = out_x;
                    out_x = end - 1;
                    pixel = out_x;
                    if char_base < 0x10000 && end2 > start {
                        tile_data = vram32(vram, char_base);
                        while out_x >= end2 {
                            self.draw_pixel_256(&ctx, pixel, &mut tile_data);
                            out_x -= 1;
                            pixel -= 1;
                        }
                        char_base += 4;
                    }
                    if char_base < 0x10000 {
                        tile_data = vram32(vram, char_base);
                        while out_x >= self.start {
                            self.draw_pixel_256(&ctx, pixel, &mut tile_data);
                            out_x -= 1;
                            pixel -= 1;
                        }
                    }
                }
                out_x = end;
                if tile_x < tile_end {
                    tile_x += 1;
                }
                length -= end - self.start;
            }
            pixel = out_x;
            out_x += (tile_end - tile_x) * 8;
            local_x = (tile_x * 8 + in_x) & 0x1FF;
            // DRAW_BACKGROUND_MODE_0_TILES_256
            while tile_x < tile_end {
                map_data = map_cache(local_x);
                local_x += 8;
                local_y = in_y & 7;
                if vflip(map_data) {
                    local_y = 7 - local_y;
                }
                char_base = char_base_of(map_data, local_y, 6, 3);
                if char_base >= 0x10000 {
                    pixel += 8;
                    tile_x += 1;
                    continue;
                }
                if !hflip(map_data) {
                    tile_data = vram32(vram, char_base);
                    if tile_data != 0 {
                        for k in 0..4 {
                            self.draw_pixel_256(&ctx, pixel + k, &mut tile_data);
                        }
                    }
                    pixel += 4;
                    tile_data = vram32(vram, char_base + 4);
                    if tile_data != 0 {
                        for k in 0..4 {
                            self.draw_pixel_256(&ctx, pixel + k, &mut tile_data);
                        }
                    }
                    pixel += 4;
                } else {
                    tile_data = vram32(vram, char_base + 4);
                    if tile_data != 0 {
                        for k in (0..4).rev() {
                            self.draw_pixel_256(&ctx, pixel + k, &mut tile_data);
                        }
                    }
                    pixel += 4;
                    tile_data = vram32(vram, char_base);
                    if tile_data != 0 {
                        for k in (0..4).rev() {
                            self.draw_pixel_256(&ctx, pixel + k, &mut tile_data);
                        }
                    }
                    pixel += 4;
                }
                tile_x += 1;
            }
            if length & 7 != 0 {
                local_x = tile_x * 8 + in_x;
                map_data = select_character(local_x);
                local_y = in_y & 7;
                if vflip(map_data) {
                    local_y = 7 - local_y;
                }
                let mod8 = length & 7;
                // DRAW_BACKGROUND_MODE_0_TILE_PREFIX_256
                char_base = char_base_of(map_data, local_y, 6, 3);
                if char_base >= 0x10000 {
                    return;
                }
                let end = mod8 - 4;
                pixel = out_x;
                if !hflip(map_data) {
                    if end > 0 {
                        tile_data = vram32(vram, char_base);
                        while out_x < self.end - end {
                            self.draw_pixel_256(&ctx, pixel, &mut tile_data);
                            out_x += 1;
                            pixel += 1;
                        }
                        char_base += 4;
                    }
                    tile_data = vram32(vram, char_base);
                    while out_x < self.end {
                        self.draw_pixel_256(&ctx, pixel, &mut tile_data);
                        out_x += 1;
                        pixel += 1;
                    }
                } else {
                    let mut shift = (8 - mod8) & 3;
                    let start = out_x;
                    out_x = self.end - 1;
                    pixel = out_x;
                    if end > 0 {
                        tile_data = vram32(vram, char_base).wrapping_shr((8 * shift) as u32);
                        while out_x >= start + 4 {
                            self.draw_pixel_256(&ctx, pixel, &mut tile_data);
                            out_x -= 1;
                            pixel -= 1;
                        }
                        shift = 0;
                    }
                    tile_data = vram32(vram, char_base + 4).wrapping_shr((8 * shift) as u32);
                    while out_x >= start {
                        self.draw_pixel_256(&ctx, pixel, &mut tile_data);
                        out_x -= 1;
                        pixel -= 1;
                    }
                }
            }
        }
    }

    // ----- Bitmap / affine modes (software-bg.c) -----

    /// BACKGROUND_BITMAP_INIT. Returns (x, y, mosaic_h, mosaic_wait, local_x, local_y).
    fn bitmap_init(&self, i: usize, in_y: i32) -> (i32, i32, i32, i32, i32, i32) {
        let bg = &self.bg[i];
        let mut x = bg.sx.wrapping_add((self.start - 1).wrapping_mul(bg.dx as i32));
        let mut y = bg.sy.wrapping_add((self.start - 1).wrapping_mul(bg.dy as i32));
        let mut mosaic_h = 0;
        let mut mosaic_wait = 0;
        let mut local_x = 0;
        let mut local_y = 0;
        if bg.mosaic {
            let mosaic_v = ((self.mosaic >> 4) & 0xF) as i32 + 1;
            mosaic_h = (self.mosaic & 0xF) as i32 + 1;
            mosaic_wait = (mosaic_h - self.start + W as i32 * mosaic_h) % mosaic_h;
            let start_x = self.start - (self.start % mosaic_h);
            mosaic_h -= 1;
            local_x = -(in_y % mosaic_v) * bg.dmx as i32;
            local_y = -(in_y % mosaic_v) * bg.dmy as i32;
            x = x.wrapping_add(local_x);
            y = y.wrapping_add(local_y);
            local_x = local_x.wrapping_add(bg.sx).wrapping_add(start_x.wrapping_mul(bg.dx as i32));
            local_y = local_y.wrapping_add(bg.sy).wrapping_add(start_x.wrapping_mul(bg.dy as i32));
        }
        (x, y, mosaic_h, mosaic_wait, local_x, local_y)
    }

    fn draw_background_mode2(&mut self, i: usize, in_y: i32, vram: &[u16]) {
        let bg = self.bg[i];
        let size_adjusted: i32 = 0x8000 << bg.size;
        let (mut x, mut y, mosaic_h, mut mosaic_wait, mut local_x, mut local_y) = self.bitmap_init(i, in_y);
        let ctx = self.bg_ctx(i);
        let screen_base = bg.screen_base as usize;
        let char_base = bg.char_base as usize;
        let mut pixel_data: u32 = 0;

        let fetch = |lx: i32, ly: i32| -> u32 {
            let map_data = vram8(vram, screen_base + ((lx >> 11) + (((ly >> 7) & 0x7F0) << bg.size)) as usize);
            vram8(vram, char_base + ((map_data << 6) as usize) + (((ly & 0x700) >> 5) as usize) + (((lx & 0x700) >> 8) as usize))
        };

        if mosaic_h > 1 {
            if bg.overflow {
                local_x &= size_adjusted - 1;
                local_y &= size_adjusted - 1;
                pixel_data = fetch(local_x, local_y);
            } else if (x | y) & !(size_adjusted - 1) == 0 {
                local_x &= size_adjusted - 1;
                local_y &= size_adjusted - 1;
                pixel_data = fetch(local_x, local_y);
            }
        }

        for out_x in self.start..self.end {
            x = x.wrapping_add(bg.dx as i32);
            y = y.wrapping_add(bg.dy as i32);
            let current = self.row[out_x as usize];
            let mut sample = || -> bool {
                if bg.overflow {
                    local_x = x & (size_adjusted - 1);
                    local_y = y & (size_adjusted - 1);
                } else {
                    if (x | y) & !(size_adjusted - 1) != 0 {
                        return false;
                    }
                    local_x = x;
                    local_y = y;
                }
                true
            };
            if mosaic_h > 1 {
                if mosaic_wait == 0 {
                    if !sample() {
                        continue;
                    }
                    pixel_data = fetch(local_x, local_y);
                    mosaic_wait = mosaic_h;
                } else {
                    mosaic_wait -= 1;
                }
            } else {
                if !sample() {
                    continue;
                }
                pixel_data = fetch(local_x, local_y);
            }
            if pixel_data != 0 && is_writable(current) {
                self.composite_indexed(&ctx, out_x as usize, pixel_data as usize, current);
            }
        }
    }

    /// Shared by modes 3 and 5 (direct color).
    fn draw_direct(&mut self, i: usize, in_y: i32, vram: &[u16], w: i32, h: i32, fetch: &dyn Fn(i32, i32) -> u32) {
        let bg = self.bg[i];
        let (mut x, mut y, mosaic_h, mut mosaic_wait, mut local_x, mut local_y) = self.bitmap_init(i, in_y);
        let ctx = self.bg_ctx(i);
        let variant = bg.variant;
        let _ = vram;
        let mut color = self.normal_palette[0];
        if mosaic_wait != 0 && local_x >= 0 && local_y >= 0 && (local_x >> 8) < w && (local_y >> 8) < h {
            color = color_from_555(fetch(local_x >> 8, local_y >> 8) as u16);
        }
        for out_x in self.start..self.end {
            x = x.wrapping_add(bg.dx as i32);
            y = y.wrapping_add(bg.dy as i32);
            if (x < 0 || y < 0 || (x >> 8) >= w || (y >> 8) >= h) && mosaic_wait == 0 {
                continue;
            }
            local_x = x;
            local_y = y;
            if mosaic_wait == 0 {
                color = color_from_555(fetch(local_x >> 8, local_y >> 8) as u16);
                mosaic_wait = mosaic_h;
            } else {
                mosaic_wait -= 1;
            }
            let current = self.row[out_x as usize];
            if !ctx.objwin_slow || (current & FLAG_OBJWIN == 0) != ctx.objwin_only {
                let merged = if current & FLAG_OBJWIN != 0 { ctx.objwin_flags } else { ctx.flags };
                let c = if !variant {
                    Some(color)
                } else if self.blend_effect == BLEND_BRIGHTEN {
                    Some(brighten(color, self.bldy))
                } else if self.blend_effect == BLEND_DARKEN {
                    Some(darken(color, self.bldy))
                } else {
                    None
                };
                if let Some(c) = c {
                    self.composite(out_x as usize, c | merged, current, true, true);
                }
            }
        }
    }

    fn draw_background_mode3(&mut self, i: usize, in_y: i32, vram: &[u16]) {
        let fetch = |x: i32, y: i32| -> u32 { vram[((x + y * W as i32) as usize) % vram.len()] as u32 };
        self.draw_direct(i, in_y, vram, W as i32, H as i32, &fetch);
    }

    fn draw_background_mode5(&mut self, i: usize, in_y: i32, vram: &[u16]) {
        let offset = if self.dispcnt & 0x10 != 0 { 0xA000 } else { 0 };
        let fetch = |x: i32, y: i32| -> u32 { vram[((offset + x * 2 + y * 320) as usize >> 1) % vram.len()] as u32 };
        self.draw_direct(i, in_y, vram, 160, 128, &fetch);
    }

    fn draw_background_mode4(&mut self, i: usize, in_y: i32, vram: &[u16]) {
        let (mut x, mut y, mosaic_h, mut mosaic_wait, mut local_x, mut local_y) = self.bitmap_init(i, in_y);
        let bg = self.bg[i];
        let ctx = self.bg_ctx(i);
        let offset: usize = if self.dispcnt & 0x10 != 0 { 0xA000 } else { 0 };
        let fetch = |x: i32, y: i32| -> u32 { vram8(vram, offset + (x + y * W as i32) as usize) };
        let mut color: u32 = 0;
        if mosaic_wait != 0 && local_x >= 0 && local_y >= 0 && (local_x >> 8) < W as i32 && (local_y >> 8) < H as i32 {
            color = fetch(local_x >> 8, local_y >> 8);
        }
        for out_x in self.start..self.end {
            x = x.wrapping_add(bg.dx as i32);
            y = y.wrapping_add(bg.dy as i32);
            if (x < 0 || y < 0 || (x >> 8) >= W as i32 || (y >> 8) >= H as i32) && mosaic_wait == 0 {
                continue;
            }
            local_x = x;
            local_y = y;
            if mosaic_wait == 0 {
                color = fetch(local_x >> 8, local_y >> 8);
                mosaic_wait = mosaic_h;
            } else {
                mosaic_wait -= 1;
            }
            let current = self.row[out_x as usize];
            if color != 0 && is_writable(current) {
                if !ctx.objwin_slow {
                    let c = self.pal(ctx.palette)[color as usize];
                    self.composite(out_x as usize, c | ctx.flags, current, true, false);
                } else if ctx.objwin_force_enable || (current & FLAG_OBJWIN == 0) == ctx.objwin_only {
                    let p = if current & FLAG_OBJWIN != 0 { ctx.objwin_palette } else { ctx.palette };
                    let merged = if current & FLAG_OBJWIN != 0 { ctx.objwin_flags } else { ctx.flags };
                    let c = self.pal(p)[color as usize];
                    self.composite(out_x as usize, c | merged, current, true, true);
                }
            }
        }
    }

    // ----- Sprites (software-obj.c, common.c) -----

    /// `GBAVideoRendererCleanOAM`
    fn clean_oam(&mut self, oam: &[u16]) {
        let mut oam_max = 0;
        for i in 0..128 {
            let obj = Obj { a: oam[i * 4], b: oam[i * 4 + 1], c: oam[i * 4 + 2] };
            if obj.transformed() || !obj.disable() {
                let sz = OBJ_SIZES[obj.shape() * 4 + obj.size()];
                let mut width = sz[0];
                let mut height = sz[1];
                let x = ((obj.x() << 23) as i32) >> 23;
                let mut cycles;
                if obj.transformed() {
                    height <<= obj.double_size();
                    width <<= obj.double_size();
                    cycles = 8 + width * 2;
                    if x < 0 {
                        cycles += x;
                    }
                } else {
                    cycles = width - 2;
                    if x < 0 {
                        if x + width < 0 {
                            continue;
                        }
                        cycles += x >> 1;
                    }
                }
                if obj.y() >= H as i32 && obj.y() + height < VIDEO_VERTICAL_TOTAL_PIXELS {
                    continue;
                }
                if obj.x() >= W as u32 && obj.x() as i32 + width < 512 {
                    continue;
                }
                let mut y = obj.y() + self.obj_offset_y;
                if y + height > 256 {
                    y -= 256;
                }
                self.sprites[oam_max] = Sprite { obj, y, end_y: y + height, cycles, index: i as i32 };
                oam_max += 1;
            }
        }
        self.oam_max = oam_max;
    }

    fn preprocess_sprite_layer(&mut self, y: i32, vram: &[u16], oam: &[u16]) -> u32 {
        let mut sprite_layers = 0;
        if self.dispcnt & 0x1000 != 0 && !self.disable_obj {
            if self.oam_dirty {
                self.clean_oam(oam);
                self.oam_dirty = false;
            }
            let mosaic_v = ((self.mosaic >> 12) & 0xF) as i32 + 1;
            let mosaic_y = y - (y % mosaic_v);
            let mut last_index = 0;
            for i in 0..self.oam_max {
                let sprite = self.sprites[i];
                let mut local_y = y;
                self.end = 0;
                self.sprite_cycles_remaining -= 2 * (sprite.index - last_index);
                last_index = sprite.index;
                if self.sprite_cycles_remaining <= 0 {
                    break;
                }
                if y < sprite.y || y >= sprite.end_y {
                    continue;
                }
                if sprite.obj.mosaic() && mosaic_v > 1 {
                    local_y = mosaic_y;
                    if local_y < sprite.y && sprite.y < H as i32 {
                        local_y = sprite.y;
                    }
                    if local_y >= (sprite.end_y & 0xFF) {
                        local_y = sprite.end_y - 1;
                    }
                }
                for w in 0..self.n_windows {
                    self.current_window = self.windows[w].control;
                    self.start = self.end;
                    self.end = self.windows[w].end_x as i32;
                    // TODO: partial sprite drawing
                    if !self.current_window.obj_enable() && self.dispcnt & 0x8000 == 0 {
                        continue;
                    }
                    let drawn = self.preprocess_sprite(&sprite.obj, sprite.index as usize, local_y, vram, oam);
                    sprite_layers |= drawn << sprite.obj.priority();
                }
                self.sprite_cycles_remaining -= sprite.cycles;
            }
        }
        sprite_layers
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn sprite_pixel(
        &mut self,
        out_x: i32,
        tile_data: u32,
        flags: u32,
        kind: u8,
        palette: Pal,
        pal_base: usize,
        objwin_palette: Pal,
        objwin_pal_base: usize,
    ) {
        let ox = out_x as usize;
        if ox >= ROW_LEN {
            return;
        }
        let keep = FLAG_ORDER_MASK | FLAG_REBLEND | FLAG_TARGET_1;
        match kind {
            // OBJWIN
            0 => {
                if tile_data != 0 {
                    self.row[ox] |= FLAG_OBJWIN;
                } else {
                    let current = self.sprite_layer[ox];
                    if current != FLAG_UNWRITTEN && (current & FLAG_ORDER_MASK) > flags {
                        self.sprite_layer[ox] = (current & !keep) | (flags & keep);
                    }
                }
            }
            // NORMAL / NORMAL_OBJWIN
            _ => {
                let current = self.sprite_layer[ox];
                if (current & FLAG_ORDER_MASK) > flags {
                    if tile_data != 0 {
                        let color = if kind == 2 && self.row[ox] & FLAG_OBJWIN != 0 {
                            self.pal(objwin_palette)[objwin_pal_base + tile_data as usize]
                        } else {
                            self.pal(palette)[pal_base + tile_data as usize]
                        };
                        self.sprite_layer[ox] = color | flags;
                    } else if current != FLAG_UNWRITTEN {
                        self.sprite_layer[ox] = (current & !keep) | (flags & keep);
                    }
                }
            }
        }
    }

    /// `GBAVideoSoftwareRendererPreprocessSprite`
    fn preprocess_sprite(&mut self, sprite: &Obj, index: usize, y: i32, vram: &[u16], oam: &[u16]) -> u32 {
        let _ = index;
        let sz = OBJ_SIZES[sprite.shape() * 4 + sprite.size()];
        let width = sz[0];
        let height = sz[1];
        let start = self.start;
        let end = self.end;
        let mut flags = sprite.priority() << OFFSET_PRIORITY;
        if (self.current_window.blend_enable() && self.target1_obj && self.blend_effect == BLEND_ALPHA)
            || sprite.mode() == OBJ_MODE_SEMITRANSPARENT
        {
            flags |= FLAG_TARGET_1;
        }
        if sprite.mode() == OBJ_MODE_OBJWIN {
            flags |= FLAG_OBJWIN;
        }
        if flags & FLAG_OBJWIN != 0 && (self.current_window.priority < self.objwin.priority || self.disable_objwin) {
            return 0;
        }
        let mut x = ((sprite.x() << 23) as i32) >> 23;
        x += self.obj_offset_x;
        let obj_mapping_1d = self.dispcnt & 0x40 != 0;
        let align = (sprite.is_256() && !obj_mapping_1d) as u32;
        let char_base = (sprite.tile() & !align) * 0x20;
        let mask_lo: u32 = if obj_mapping_1d { 0x7FFE } else { 0x3FE };
        let mask_hi: u32 = if obj_mapping_1d { 0 } else { char_base & 0x7C00 };
        if self.dispcnt & 7 >= 3 && sprite.tile() < 512 {
            return 0;
        }

        let objwin_slow_path = self.dispcnt & 0x8000 != 0 && self.objwin.blend_enable() != self.current_window.blend_enable();
        let mut variant = self.target1_obj
            && self.current_window.blend_enable()
            && (self.blend_effect == BLEND_BRIGHTEN || self.blend_effect == BLEND_DARKEN);
        if sprite.mode() == OBJ_MODE_SEMITRANSPARENT || (self.target1_obj && self.blend_effect == BLEND_ALPHA) || objwin_slow_path {
            let mut target2 = self.target2_bd;
            for b in &self.bg {
                target2 |= b.target2 && b.enabled != 0;
            }
            if target2 {
                self.force_target1 = true;
                flags |= FLAG_REBLEND;
                variant = false;
            } else {
                flags &= !FLAG_TARGET_1;
            }
        }

        let mut palette = Pal::Normal;
        let mut objwin_palette = Pal::Normal;
        if variant {
            palette = Pal::Variant;
            if self.objwin.blend_enable() {
                objwin_palette = palette;
            }
        }
        let mut pal_base = 0x100usize;
        let mut objwin_pal_base = 0x100usize;

        let mut in_y = y - (sprite.y() + self.obj_offset_y);
        let stride: i32 = if obj_mapping_1d { width >> (!sprite.is_256() as i32) } else { 0x80 };
        let is256 = sprite.is_256();
        let kind: u8 = if flags & FLAG_OBJWIN != 0 {
            0
        } else if objwin_slow_path {
            2
        } else {
            1
        };

        let vram_base = BASE_TILE >> 1;
        let load = |y_base: u32, x_base: u32| -> u32 {
            let off = ((y_base.wrapping_add((x_base.wrapping_add(char_base)) & mask_lo)) & 0x7FFE) as usize;
            vram[(vram_base + (off >> 1)) % vram.len()] as u32
        };
        let ybase = |ly: i32| -> u32 {
            if is256 {
                ((ly & !7) * stride + (ly & 7) * 8) as u32 + mask_hi
            } else {
                ((ly & !7) * stride + (ly & 7) * 4) as u32 + mask_hi
            }
        };
        let xbase = |lx: i32| -> u32 {
            if is256 {
                ((lx & !7) * 8 + (lx & 6)) as u32
            } else {
                ((lx & !7) * 4 + ((lx >> 1) & 2)) as u32
            }
        };
        let texel = |raw: u32, lx: i32| -> u32 {
            if is256 {
                (raw >> ((lx & 1) << 3)) & 0xFF
            } else {
                (raw >> ((lx & 3) << 2)) & 0xF
            }
        };
        if !is256 {
            pal_base += sprite.palette() << 4;
            if kind == 2 {
                objwin_pal_base += sprite.palette() << 4;
            }
        }

        if sprite.transformed() {
            let total_width = width << sprite.double_size();
            let total_height = height << sprite.double_size();
            let m = sprite.mat_index();
            let mat_a = oam[m * 16 + 3] as i16 as i32;
            let mat_b = oam[m * 16 + 7] as i16 as i32;
            let mat_c = oam[m * 16 + 11] as i16 as i32;
            let mat_d = oam[m * 16 + 15] as i16 as i32;

            if in_y < 0 {
                in_y += 256;
            }
            let mut out_x = if x >= start { x } else { start };
            let mut condition = x + total_width;
            let mut in_x = out_x - x;
            if end < condition {
                condition = end;
            }
            let mut mosaic_h = 1;
            if sprite.mosaic() {
                mosaic_h = ((self.mosaic >> 8) & 0xF) as i32 + 1;
                if condition != end && condition % mosaic_h != 0 {
                    condition += mosaic_h - (condition % mosaic_h);
                }
            }

            let mut x_accum = mat_a
                .wrapping_mul(in_x - 1 - (total_width >> 1))
                .wrapping_add(mat_b.wrapping_mul(in_y - (total_height >> 1)))
                .wrapping_add(width << 7);
            let mut y_accum = mat_c
                .wrapping_mul(in_x - 1 - (total_width >> 1))
                .wrapping_add(mat_d.wrapping_mul(in_y - (total_height >> 1)))
                .wrapping_add(height << 7);

            // Clip off early pixels
            if mat_a != 0 {
                if (x_accum >> 8) < 0 {
                    let diff_x = -x_accum - 1;
                    let s = diff_x / mat_a;
                    x_accum += mat_a * s;
                    y_accum += mat_c * s;
                    out_x += s;
                    in_x += s;
                } else if (x_accum >> 8) >= width {
                    let diff_x = (width << 8) - x_accum;
                    let s = diff_x / mat_a;
                    x_accum += mat_a * s;
                    y_accum += mat_c * s;
                    out_x += s;
                    in_x += s;
                }
            }
            if mat_c != 0 {
                if (y_accum >> 8) < 0 {
                    let diff_y = -y_accum - 1;
                    let s = diff_y / mat_c;
                    x_accum += mat_a * s;
                    y_accum += mat_c * s;
                    out_x += s;
                    in_x += s;
                } else if (y_accum >> 8) >= height {
                    let diff_y = (height << 8) - y_accum;
                    let s = diff_y / mat_c;
                    x_accum += mat_a * s;
                    y_accum += mat_c * s;
                    out_x += s;
                    in_x += s;
                }
            }
            let _ = in_x;

            if out_x < start || out_x >= condition {
                return 0;
            }

            let width_mask = !(width - 1);
            let height_mask = !(height - 1);
            if kind != 0 && mosaic_h > 1 {
                // SPRITE_TRANSFORMED_MOSAIC_LOOP
                let mut local_x = x_accum >> 8;
                let mut local_y = y_accum >> 8;
                while out_x < condition {
                    x_accum = x_accum.wrapping_add(mat_a);
                    y_accum = y_accum.wrapping_add(mat_c);
                    if out_x % mosaic_h == 0 {
                        local_x = x_accum >> 8;
                        local_y = y_accum >> 8;
                    }
                    if local_x & width_mask == 0 && local_y & height_mask == 0 {
                        let raw = load(ybase(local_y), xbase(local_x));
                        let t = texel(raw, local_x);
                        self.sprite_pixel(out_x, t, flags, kind, palette, pal_base, objwin_palette, objwin_pal_base);
                    }
                    out_x += 1;
                }
            } else {
                // SPRITE_TRANSFORMED_LOOP
                while out_x < condition {
                    x_accum = x_accum.wrapping_add(mat_a);
                    y_accum = y_accum.wrapping_add(mat_c);
                    let local_x = x_accum >> 8;
                    let local_y = y_accum >> 8;
                    if local_x & width_mask != 0 || local_y & height_mask != 0 {
                        break;
                    }
                    let raw = load(ybase(local_y), xbase(local_x));
                    let t = texel(raw, local_x);
                    self.sprite_pixel(out_x, t, flags, kind, palette, pal_base, objwin_palette, objwin_pal_base);
                    out_x += 1;
                }
            }
        } else {
            let mut out_x = if x >= start { x } else { start };
            let mut condition = x + width;
            let mut mosaic_h = 1;
            if sprite.mosaic() {
                mosaic_h = ((self.mosaic >> 8) & 0xF) as i32 + 1;
                if condition % mosaic_h != 0 {
                    condition += mosaic_h - (condition % mosaic_h);
                }
            }
            if sprite.y() + height - 256 >= 0 {
                in_y += 256;
            }
            if sprite.vflip() {
                in_y = height - in_y - 1;
            }
            if end < condition {
                condition = end;
            }
            let mut in_x = out_x - x;
            let mut x_offset = 1;
            if sprite.hflip() {
                in_x = width - in_x - 1;
                x_offset = -1;
            }
            let y_base = ybase(in_y);
            if kind != 0 && mosaic_h > 1 {
                // SPRITE_MOSAIC_LOOP
                while out_x < condition {
                    let mut local_x = in_x - x_offset * (out_x % mosaic_h);
                    if local_x < 0 {
                        local_x = 0;
                    } else if local_x > width - 1 {
                        local_x = width - 1;
                    }
                    let raw = load(y_base, xbase(local_x));
                    let t = texel(raw, local_x);
                    self.sprite_pixel(out_x, t, flags, kind, palette, pal_base, objwin_palette, objwin_pal_base);
                    out_x += 1;
                    in_x += x_offset;
                }
            } else {
                // SPRITE_NORMAL_LOOP
                while out_x < condition {
                    let raw = load(y_base, xbase(in_x));
                    let t = texel(raw, in_x);
                    self.sprite_pixel(out_x, t, flags, kind, palette, pal_base, objwin_palette, objwin_pal_base);
                    out_x += 1;
                    in_x += x_offset;
                }
            }
        }
        1
    }

    /// `GBAVideoSoftwareRendererPostprocessSprite`
    fn postprocess_sprite(&mut self, priority: u32) {
        let flags = if self.target2_obj { FLAG_TARGET_2 } else { 0 };
        let start = self.start.max(0) as usize;
        let end = (self.end.max(0) as usize).min(W);
        let objwin_slow_path = self.dispcnt & 0x8000 != 0;
        if objwin_slow_path {
            let objwin_disable = !self.objwin.obj_enable();
            let objwin_only = !objwin_disable && !self.current_window.obj_enable();
            if objwin_disable && !self.current_window.obj_enable() {
                return;
            }
            for x in start..end {
                let color = self.sprite_layer[x] & !FLAG_OBJWIN;
                let current = self.row[x];
                let ok = (color & FLAG_UNWRITTEN) != FLAG_UNWRITTEN && (color & FLAG_PRIORITY) >> OFFSET_PRIORITY == priority;
                let pass = if objwin_disable {
                    current & FLAG_OBJWIN == 0
                } else if objwin_only {
                    current & FLAG_OBJWIN != 0
                } else {
                    true
                };
                if ok && pass {
                    self.composite(x, color | flags, current, true, true);
                }
            }
            return;
        } else if !self.current_window.obj_enable() {
            return;
        }
        for x in start..end {
            let color = self.sprite_layer[x] & !FLAG_OBJWIN;
            let current = self.row[x];
            if (color & FLAG_UNWRITTEN) != FLAG_UNWRITTEN && (color & FLAG_PRIORITY) >> OFFSET_PRIORITY == priority {
                self.composite(x, color | flags, current, true, false);
            }
        }
    }
}
