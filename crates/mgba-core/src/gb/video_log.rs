// Copyright (c) 2013-2017 Jeffrey Pfau
// Playback of mGBA video logs (.mvl) recorded from a Game Boy: the video-log
// player core's state load (`_GBVLPLoadState` in src/gb/core.c) and the proxy
// renderer's packet handler (`_parsePacket` in src/gb/extra/proxy.c).
// Recording logs isn't supported.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::{io, model, sgb, Gb};

/// `enum mVideoLoggerDirtyType`
pub mod dirty {
    pub const DUMMY: u32 = 0;
    pub const FLUSH: u32 = 1;
    pub const SCANLINE: u32 = 2;
    pub const REGISTER: u32 = 3;
    pub const OAM: u32 = 4;
    pub const PALETTE: u32 = 5;
    pub const VRAM: u32 = 6;
    pub const FRAME: u32 = 7;
    pub const RANGE: u32 = 8;
    pub const BUFFER: u32 = 9;
}

const BUFFER_SGB: u32 = 2;

// Offsets in `struct GBSerializedState` (include/mgba/internal/gb/serialize.h)
const STATE_MODEL: usize = 0x8;
const STATE_PALETTE: usize = 0xD4;
const STATE_OAM: usize = 0x260;
const STATE_IO: usize = 0x300;
const STATE_VRAM: usize = 0x400;
const STATE_SGB: usize = 0xC710;
const STATE_SIZE: usize = 0x11800;

impl Gb {
    /// Set up rendering state from a video log's initial state (an mGBA
    /// `GBSerializedState`), like `_GBVLPLoadState`. Returns false if the
    /// state is too short.
    pub fn load_mvl_state(&mut self, state: &[u8]) -> bool {
        // Older states end after WRAM; only the SGB block needs the full size.
        if state.len() < STATE_VRAM + 0x4000 {
            return false;
        }
        self.model = state[STATE_MODEL];
        self.timing.clear();
        // GBVideoReset
        self.video_reset();
        self.timing.clear();

        // GBVideoDeserialize (the renderer-facing parts)
        let (m, borders) = (self.model, self.video.sgb_borders);
        self.renderer.init(m, borders);
        for i in 0..64 {
            let c = u16::from_le_bytes([state[STATE_PALETTE + i * 2], state[STATE_PALETTE + i * 2 + 1]]);
            self.video.palette[i] = c;
            self.renderer.write_palette(i, c);
        }
        self.video.vram.copy_from_slice(&state[STATE_VRAM..STATE_VRAM + 0x4000]);
        self.video.oam.copy_from_slice(&state[STATE_OAM..STATE_OAM + 0xA0]);

        // GBIODeserialize (the renderer-facing parts)
        self.memory.io.copy_from_slice(&state[STATE_IO..STATE_IO + 0x80]);
        for r in [io::REG_LCDC, io::REG_SCY, io::REG_SCX, io::REG_WY, io::REG_WX] {
            let v = state[STATE_IO + r];
            self.renderer.write_video_register(r, v);
        }
        if self.model == model::SGB {
            for r in [io::REG_BGP, io::REG_OBP0, io::REG_OBP1] {
                let v = state[STATE_IO + r];
                self.renderer.write_video_register(r, v);
            }
        }
        self.video.stat = state[STATE_IO + io::REG_STAT];

        if self.model & model::SGB != 0 && state.len() >= STATE_SIZE {
            // GBSGBDeserialize
            let s = &state[STATE_SGB..];
            self.video.sgb_command_header = s[90];
            self.sgb_bit = s[91] as i32;
            let flags = u32::from_le_bytes([s[92], s[93], s[94], s[95]]);
            self.current_sgb_bits = (flags & 3) as i32;
            self.renderer.sgb.render_mode = ((flags >> 2) & 3) as i32;
            self.video.sgb_buffer_index = ((flags >> 4) & 7) as i32;
            self.sgb_current_controller = ((flags >> 7) & 3) as i32;
            self.sgb_controllers = ((flags >> 9) & 3) as i32;
            self.sgb_increment = flags & (1 << 11) != 0;
            self.sgb_packet.copy_from_slice(&s[96..112]);
            self.video.sgb_packet_buffer.copy_from_slice(&s[112..240]);
            let sgb_ram = &mut self.renderer.sgb;
            sgb_ram.char_ram = s[240..240 + 0x2000].to_vec();
            sgb_ram.map_ram = s[0x20F0..0x30F0].to_vec();
            sgb_ram.pal_ram = s[0x30F0..0x40F0].to_vec();
            sgb_ram.attribute_files = s[0x40F0..0x50F0].to_vec();
            let mut attrs = vec![0u8; 90 * 45];
            attrs[..90].copy_from_slice(&s[..90]);
            sgb_ram.attributes = attrs;
            let mut packet = [0u8; 16];
            packet[0] = (sgb::ATRC_EN << 3) | 1;
            self.video_write_sgb_packet(&packet);
        }
        true
    }

    /// Apply one logged packet to the renderer (`_parsePacket`). `payload` is
    /// the data that follows VRAM and buffer packets. Returns false at a FLUSH
    /// (the end of a frame's packets) or an unknown packet.
    pub fn mvl_packet(&mut self, ty: u32, address: u32, value: u32, value2: u32, payload: &[u8]) -> bool {
        match ty {
            dirty::REGISTER => {
                self.renderer.write_video_register(address as usize, value as u8);
            }
            dirty::PALETTE => {
                if address < 64 {
                    self.renderer.write_palette(address as usize, value as u16);
                }
            }
            dirty::OAM => {
                if (address as usize) < 0xA0 {
                    self.video.oam[address as usize] = value as u8;
                }
            }
            dirty::VRAM => {
                let a = address as usize;
                if a <= 0x4000 - 0x1000 && payload.len() >= 0x1000 {
                    self.video.vram[a..a + 0x1000].copy_from_slice(&payload[..0x1000]);
                }
            }
            dirty::SCANLINE => {
                if (address as usize) < super::VIDEO_VERTICAL_PIXELS {
                    self.renderer.finish_scanline(address as i32);
                }
            }
            dirty::RANGE => {
                if value < value2 && value2 as usize <= super::VIDEO_HORIZONTAL_PIXELS && (address as usize) < super::VIDEO_VERTICAL_PIXELS {
                    self.renderer.draw_range(&self.video.vram, &self.video.oam, value as i32, value2 as i32, address as i32);
                }
            }
            dirty::FRAME => self.renderer.finish_frame(),
            dirty::BUFFER => {
                if address == BUFFER_SGB && self.model & model::SGB != 0 && payload.len() >= 16 {
                    let mut packet = [0u8; 128];
                    packet[..16].copy_from_slice(&payload[..16]);
                    self.renderer.write_sgb_packet(&packet);
                }
            }
            _ => return false,
        }
        true
    }
}
