// Copyright (c) 2013-2015 Jeffrey Pfau
// GBA I/O register offsets (include/mgba/internal/gba/io.h).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
#![allow(dead_code)]

pub const DISPCNT: u32 = 0x000;
pub const STEREOCNT: u32 = 0x002;
pub const DISPSTAT: u32 = 0x004;
pub const VCOUNT: u32 = 0x006;
pub const BG0CNT: u32 = 0x008;
pub const BG1CNT: u32 = 0x00A;
pub const BG2CNT: u32 = 0x00C;
pub const BG3CNT: u32 = 0x00E;
pub const BG0HOFS: u32 = 0x010;
pub const BG0VOFS: u32 = 0x012;
pub const BG1HOFS: u32 = 0x014;
pub const BG1VOFS: u32 = 0x016;
pub const BG2HOFS: u32 = 0x018;
pub const BG2VOFS: u32 = 0x01A;
pub const BG3HOFS: u32 = 0x01C;
pub const BG3VOFS: u32 = 0x01E;
pub const BG2PA: u32 = 0x020;
pub const BG2PB: u32 = 0x022;
pub const BG2PC: u32 = 0x024;
pub const BG2PD: u32 = 0x026;
pub const BG2X_LO: u32 = 0x028;
pub const BG2X_HI: u32 = 0x02A;
pub const BG2Y_LO: u32 = 0x02C;
pub const BG2Y_HI: u32 = 0x02E;
pub const BG3PA: u32 = 0x030;
pub const BG3PB: u32 = 0x032;
pub const BG3PC: u32 = 0x034;
pub const BG3PD: u32 = 0x036;
pub const BG3X_LO: u32 = 0x038;
pub const BG3X_HI: u32 = 0x03A;
pub const BG3Y_LO: u32 = 0x03C;
pub const BG3Y_HI: u32 = 0x03E;
pub const WIN0H: u32 = 0x040;
pub const WIN1H: u32 = 0x042;
pub const WIN0V: u32 = 0x044;
pub const WIN1V: u32 = 0x046;
pub const WININ: u32 = 0x048;
pub const WINOUT: u32 = 0x04A;
pub const MOSAIC: u32 = 0x04C;
pub const BLDCNT: u32 = 0x050;
pub const BLDALPHA: u32 = 0x052;
pub const BLDY: u32 = 0x054;
pub const SOUND1CNT_LO: u32 = 0x060;
pub const SOUND1CNT_HI: u32 = 0x062;
pub const SOUND1CNT_X: u32 = 0x064;
pub const SOUND2CNT_LO: u32 = 0x068;
pub const SOUND2CNT_HI: u32 = 0x06C;
pub const SOUND3CNT_LO: u32 = 0x070;
pub const SOUND3CNT_HI: u32 = 0x072;
pub const SOUND3CNT_X: u32 = 0x074;
pub const SOUND4CNT_LO: u32 = 0x078;
pub const SOUND4CNT_HI: u32 = 0x07C;
pub const SOUNDCNT_LO: u32 = 0x080;
pub const SOUNDCNT_HI: u32 = 0x082;
pub const SOUNDCNT_X: u32 = 0x084;
pub const SOUNDBIAS: u32 = 0x088;
pub const WAVE_RAM0_LO: u32 = 0x090;
pub const WAVE_RAM0_HI: u32 = 0x092;
pub const WAVE_RAM1_LO: u32 = 0x094;
pub const WAVE_RAM1_HI: u32 = 0x096;
pub const WAVE_RAM2_LO: u32 = 0x098;
pub const WAVE_RAM2_HI: u32 = 0x09A;
pub const WAVE_RAM3_LO: u32 = 0x09C;
pub const WAVE_RAM3_HI: u32 = 0x09E;
pub const FIFO_A_LO: u32 = 0x0A0;
pub const FIFO_A_HI: u32 = 0x0A2;
pub const FIFO_B_LO: u32 = 0x0A4;
pub const FIFO_B_HI: u32 = 0x0A6;
pub const DMA0SAD_LO: u32 = 0x0B0;
pub const DMA0SAD_HI: u32 = 0x0B2;
pub const DMA0DAD_LO: u32 = 0x0B4;
pub const DMA0DAD_HI: u32 = 0x0B6;
pub const DMA0CNT_LO: u32 = 0x0B8;
pub const DMA0CNT_HI: u32 = 0x0BA;
pub const DMA1SAD_LO: u32 = 0x0BC;
pub const DMA1SAD_HI: u32 = 0x0BE;
pub const DMA1DAD_LO: u32 = 0x0C0;
pub const DMA1DAD_HI: u32 = 0x0C2;
pub const DMA1CNT_LO: u32 = 0x0C4;
pub const DMA1CNT_HI: u32 = 0x0C6;
pub const DMA2SAD_LO: u32 = 0x0C8;
pub const DMA2SAD_HI: u32 = 0x0CA;
pub const DMA2DAD_LO: u32 = 0x0CC;
pub const DMA2DAD_HI: u32 = 0x0CE;
pub const DMA2CNT_LO: u32 = 0x0D0;
pub const DMA2CNT_HI: u32 = 0x0D2;
pub const DMA3SAD_LO: u32 = 0x0D4;
pub const DMA3SAD_HI: u32 = 0x0D6;
pub const DMA3DAD_LO: u32 = 0x0D8;
pub const DMA3DAD_HI: u32 = 0x0DA;
pub const DMA3CNT_LO: u32 = 0x0DC;
pub const DMA3CNT_HI: u32 = 0x0DE;
pub const TM0CNT_LO: u32 = 0x100;
pub const TM0CNT_HI: u32 = 0x102;
pub const TM1CNT_LO: u32 = 0x104;
pub const TM1CNT_HI: u32 = 0x106;
pub const TM2CNT_LO: u32 = 0x108;
pub const TM2CNT_HI: u32 = 0x10A;
pub const TM3CNT_LO: u32 = 0x10C;
pub const TM3CNT_HI: u32 = 0x10E;
pub const SIODATA32_LO: u32 = 0x120;
pub const SIOMULTI0: u32 = 0x120;
pub const SIODATA32_HI: u32 = 0x122;
pub const SIOMULTI1: u32 = 0x122;
pub const SIOMULTI2: u32 = 0x124;
pub const SIOMULTI3: u32 = 0x126;
pub const SIOCNT: u32 = 0x128;
pub const SIOMLT_SEND: u32 = 0x12A;
pub const SIODATA8: u32 = 0x12A;
pub const RCNT: u32 = 0x134;
pub const JOYCNT: u32 = 0x140;
pub const JOY_RECV_LO: u32 = 0x150;
pub const JOY_RECV_HI: u32 = 0x152;
pub const JOY_TRANS_LO: u32 = 0x154;
pub const JOY_TRANS_HI: u32 = 0x156;
pub const JOYSTAT: u32 = 0x158;
pub const KEYINPUT: u32 = 0x130;
pub const KEYCNT: u32 = 0x132;
pub const IE: u32 = 0x200;
pub const IF: u32 = 0x202;
pub const WAITCNT: u32 = 0x204;
pub const IME: u32 = 0x208;
pub const MAX: u32 = 0x20A;
pub const POSTFLG: u32 = 0x300;
pub const HALTCNT: u32 = 0x301;
pub const EXWAITCNT_LO: u32 = 0x800;
pub const EXWAITCNT_HI: u32 = 0x802;
pub const INTERNAL_EXWAITCNT_LO: u32 = 0x210;
pub const INTERNAL_EXWAITCNT_HI: u32 = 0x212;
pub const INTERNAL_MAX: u32 = 0x214;
pub const DEBUG_STRING: u32 = 0xFFF600;
pub const DEBUG_FLAGS: u32 = 0xFFF700;
pub const DEBUG_ENABLE: u32 = 0xFFF780;

/// `GBAIORegisterNames`: register names for the debugger's identifier lookup.
pub const IO_REGISTER_NAMES: &[(&str, u32)] = &[
    ("DISPCNT", DISPCNT),
    ("DISPSTAT", DISPSTAT),
    ("VCOUNT", VCOUNT),
    ("BG0CNT", BG0CNT),
    ("BG1CNT", BG1CNT),
    ("BG2CNT", BG2CNT),
    ("BG3CNT", BG3CNT),
    ("BG0HOFS", BG0HOFS),
    ("BG0VOFS", BG0VOFS),
    ("BG1HOFS", BG1HOFS),
    ("BG1VOFS", BG1VOFS),
    ("BG2HOFS", BG2HOFS),
    ("BG2VOFS", BG2VOFS),
    ("BG3HOFS", BG3HOFS),
    ("BG3VOFS", BG3VOFS),
    ("BG2PA", BG2PA),
    ("BG2PB", BG2PB),
    ("BG2PC", BG2PC),
    ("BG2PD", BG2PD),
    ("BG2X_LO", BG2X_LO),
    ("BG2X_HI", BG2X_HI),
    ("BG2Y_LO", BG2Y_LO),
    ("BG2Y_HI", BG2Y_HI),
    ("BG3PA", BG3PA),
    ("BG3PB", BG3PB),
    ("BG3PC", BG3PC),
    ("BG3PD", BG3PD),
    ("BG3X_LO", BG3X_LO),
    ("BG3X_HI", BG3X_HI),
    ("BG3Y_LO", BG3Y_LO),
    ("BG3Y_HI", BG3Y_HI),
    ("WIN0H", WIN0H),
    ("WIN1H", WIN1H),
    ("WIN0V", WIN0V),
    ("WIN1V", WIN1V),
    ("WININ", WININ),
    ("WINOUT", WINOUT),
    ("MOSAIC", MOSAIC),
    ("BLDCNT", BLDCNT),
    ("BLDALPHA", BLDALPHA),
    ("BLDY", BLDY),
    ("SOUND1CNT_LO", SOUND1CNT_LO),
    ("SOUND1CNT_HI", SOUND1CNT_HI),
    ("SOUND1CNT_X", SOUND1CNT_X),
    ("SOUND2CNT_LO", SOUND2CNT_LO),
    ("SOUND2CNT_HI", SOUND2CNT_HI),
    ("SOUND3CNT_LO", SOUND3CNT_LO),
    ("SOUND3CNT_HI", SOUND3CNT_HI),
    ("SOUND3CNT_X", SOUND3CNT_X),
    ("SOUND4CNT_LO", SOUND4CNT_LO),
    ("SOUND4CNT_HI", SOUND4CNT_HI),
    ("SOUNDCNT_LO", SOUNDCNT_LO),
    ("SOUNDCNT_HI", SOUNDCNT_HI),
    ("SOUNDCNT_X", SOUNDCNT_X),
    ("SOUNDBIAS", SOUNDBIAS),
    ("WAVE_RAM0_LO", WAVE_RAM0_LO),
    ("WAVE_RAM0_HI", WAVE_RAM0_HI),
    ("WAVE_RAM1_LO", WAVE_RAM1_LO),
    ("WAVE_RAM1_HI", WAVE_RAM1_HI),
    ("WAVE_RAM2_LO", WAVE_RAM2_LO),
    ("WAVE_RAM2_HI", WAVE_RAM2_HI),
    ("WAVE_RAM3_LO", WAVE_RAM3_LO),
    ("WAVE_RAM3_HI", WAVE_RAM3_HI),
    ("FIFO_A_LO", FIFO_A_LO),
    ("FIFO_A_HI", FIFO_A_HI),
    ("FIFO_B_LO", FIFO_B_LO),
    ("FIFO_B_HI", FIFO_B_HI),
    ("DMA0SAD_LO", DMA0SAD_LO),
    ("DMA0SAD_HI", DMA0SAD_HI),
    ("DMA0DAD_LO", DMA0DAD_LO),
    ("DMA0DAD_HI", DMA0DAD_HI),
    ("DMA0CNT_LO", DMA0CNT_LO),
    ("DMA0CNT_HI", DMA0CNT_HI),
    ("DMA1SAD_LO", DMA1SAD_LO),
    ("DMA1SAD_HI", DMA1SAD_HI),
    ("DMA1DAD_LO", DMA1DAD_LO),
    ("DMA1DAD_HI", DMA1DAD_HI),
    ("DMA1CNT_LO", DMA1CNT_LO),
    ("DMA1CNT_HI", DMA1CNT_HI),
    ("DMA2SAD_LO", DMA2SAD_LO),
    ("DMA2SAD_HI", DMA2SAD_HI),
    ("DMA2DAD_LO", DMA2DAD_LO),
    ("DMA2DAD_HI", DMA2DAD_HI),
    ("DMA2CNT_LO", DMA2CNT_LO),
    ("DMA2CNT_HI", DMA2CNT_HI),
    ("DMA3SAD_LO", DMA3SAD_LO),
    ("DMA3SAD_HI", DMA3SAD_HI),
    ("DMA3DAD_LO", DMA3DAD_LO),
    ("DMA3DAD_HI", DMA3DAD_HI),
    ("DMA3CNT_LO", DMA3CNT_LO),
    ("DMA3CNT_HI", DMA3CNT_HI),
    ("TM0CNT_LO", TM0CNT_LO),
    ("TM0CNT_HI", TM0CNT_HI),
    ("TM1CNT_LO", TM1CNT_LO),
    ("TM1CNT_HI", TM1CNT_HI),
    ("TM2CNT_LO", TM2CNT_LO),
    ("TM2CNT_HI", TM2CNT_HI),
    ("TM3CNT_LO", TM3CNT_LO),
    ("TM3CNT_HI", TM3CNT_HI),
    ("SIOMULTI0", SIOMULTI0),
    ("SIOMULTI1", SIOMULTI1),
    ("SIOMULTI2", SIOMULTI2),
    ("SIOMULTI3", SIOMULTI3),
    ("SIOCNT", SIOCNT),
    ("SIOMLT_SEND", SIOMLT_SEND),
    ("KEYINPUT", KEYINPUT),
    ("KEYCNT", KEYCNT),
    ("RCNT", RCNT),
    ("JOYCNT", JOYCNT),
    ("JOY_RECV_LO", JOY_RECV_LO),
    ("JOY_RECV_HI", JOY_RECV_HI),
    ("JOY_TRANS_LO", JOY_TRANS_LO),
    ("JOY_TRANS_HI", JOY_TRANS_HI),
    ("JOYSTAT", JOYSTAT),
    ("IE", IE),
    ("IF", IF),
    ("WAITCNT", WAITCNT),
    ("IME", IME),
];
