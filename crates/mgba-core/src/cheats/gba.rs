// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's GBA cheat formats: src/gba/cheats.c (autodetection, VBA),
// src/gba/cheats/gameshark.c (GameShark / Action Replay v1-v2),
// src/gba/cheats/parv3.c (Pro Action Replay v3) and
// src/gba/cheats/codebreaker.c (CodeBreaker).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::tables::*;
use super::{hex, Cheat, CheatPatch, CheatSet, CheatType, SetKind};

const BASE_ROM0: u32 = 0x08000000;
const BASE_IO: u32 = 0x04000000;
const BASE_SRAM: u32 = 0x0E000000;
const SIZE_ROM0: u32 = 0x02000000;
const OFFSET_MASK: u32 = 0x00FFFFFF;
const REG_KEYINPUT: u32 = 0x130;

pub const GAMESHARK_SEEDS: [u32; 4] = [0x09F4FBBD, 0x9681884A, 0x352027E9, 0xF3DEE5A7];
pub const PRO_ACTION_REPLAY_SEEDS: [u32; 4] = [0x7AA9648F, 0x7FAE6994, 0xC0EFAAD5, 0x42712C57];

/// `enum GBACheatGameSharkVersion`
pub const GS_NOT_SET: i32 = 0;
pub const GS_GSAV1: i32 = 1;
pub const GS_GSAV1_RAW: i32 = 2;
pub const GS_PARV3: i32 = 3;
pub const GS_PARV3_RAW: i32 = 4;

// PARv3 field masks and values
const PAR3_COND: u32 = 0x38000000;
const PAR3_WIDTH: u32 = 0x06000000;
const PAR3_ACTION: u32 = 0xC0000000;
const PAR3_BASE: u32 = 0xC0000000;
const PAR3_WIDTH_BASE: u32 = 25;

/// The GBA-specific part of `struct GBACheatSet`.
#[derive(Clone, Debug)]
pub struct GbaSetState {
    /// Address the codes want to run at (`hook`); see the module notes.
    pub hook: Option<u32>,
    incomplete_cheat: Option<usize>,
    incomplete_patch: Option<usize>,
    current_block: Option<usize>,
    pub gsa_version: i32,
    gsa_seeds: [u32; 4],
    cb_rng_state: u32,
    cb_master: u32,
    cb_table: [u8; 0x30],
    cb_seeds: [u32; 4],
    remaining_addresses: i32,
}

impl GbaSetState {
    pub fn new() -> GbaSetState {
        GbaSetState {
            hook: None,
            incomplete_cheat: None,
            incomplete_patch: None,
            current_block: None,
            gsa_version: GS_NOT_SET,
            gsa_seeds: [0; 4],
            cb_rng_state: 0,
            cb_master: 0,
            cb_table: [0; 0x30],
            cb_seeds: [0; 4],
            remaining_addresses: 0,
        }
    }

    /// `GBACheatSetCopyProperties`
    pub(super) fn copy_properties(&mut self, old: &GbaSetState) {
        self.gsa_version = old.gsa_version;
        self.gsa_seeds = old.gsa_seeds;
        self.cb_rng_state = old.cb_rng_state;
        self.cb_master = old.cb_master;
        self.cb_seeds = old.cb_seeds;
        self.cb_table = old.cb_table;
        if old.hook.is_some() {
            self.hook = old.hook;
        }
    }

    /// `GBACheatParseDirectives`
    pub(super) fn parse_directives(&mut self, directives: &[String]) {
        for d in directives {
            match d.as_str() {
                "GSAv1" => self.set_gameshark_version(GS_GSAV1),
                "GSAv1 raw" => self.set_gameshark_version(GS_GSAV1_RAW),
                "PARv3" => self.set_gameshark_version(GS_PARV3),
                "PARv3 raw" => self.set_gameshark_version(GS_PARV3_RAW),
                _ => {}
            }
        }
    }

    /// `GBACheatDumpDirectives`
    pub(super) fn dump_directives(&self) -> Vec<String> {
        match self.gsa_version {
            GS_GSAV1 => vec!["GSAv1".into()],
            GS_GSAV1_RAW => vec!["GSAv1 raw".into()],
            GS_PARV3 => vec!["PARv3".into()],
            GS_PARV3_RAW => vec!["PARv3 raw".into()],
            _ => vec![],
        }
    }

    /// `GBACheatSetGameSharkVersion`
    pub fn set_gameshark_version(&mut self, version: i32) {
        self.gsa_version = version;
        match version {
            GS_GSAV1 | GS_GSAV1_RAW => self.gsa_seeds = GAMESHARK_SEEDS,
            GS_PARV3 | GS_PARV3_RAW => self.gsa_seeds = PRO_ACTION_REPLAY_SEEDS,
            _ => {}
        }
    }
}

impl Default for GbaSetState {
    fn default() -> Self {
        Self::new()
    }
}

fn parts(set: &mut CheatSet) -> (&mut Vec<Cheat>, &mut Vec<CheatPatch>, &mut GbaSetState) {
    match &mut set.kind {
        SetKind::Gba(st) => (&mut set.list, &mut set.rom_patches, st),
        SetKind::Gb => panic!("GBA code parser used on a Game Boy cheat set"),
    }
}

/// `GBACheatEncryptGameShark` (TEA)
pub fn encrypt_gameshark(op1: &mut u32, op2: &mut u32, seeds: &[u32; 4]) {
    let mut sum: u32 = 0;
    for _ in 0..32 {
        sum = sum.wrapping_add(0x9E3779B9);
        *op1 = op1.wrapping_add(
            ((*op2 << 4).wrapping_add(seeds[0])) ^ (op2.wrapping_add(sum)) ^ ((*op2 >> 5).wrapping_add(seeds[1])),
        );
        *op2 = op2.wrapping_add(
            ((*op1 << 4).wrapping_add(seeds[2])) ^ (op1.wrapping_add(sum)) ^ ((*op1 >> 5).wrapping_add(seeds[3])),
        );
    }
}

/// `GBACheatDecryptGameShark` (TEA)
pub fn decrypt_gameshark(op1: &mut u32, op2: &mut u32, seeds: &[u32; 4]) {
    let mut sum: u32 = 0xC6EF3720;
    for _ in 0..32 {
        *op2 = op2.wrapping_sub(
            ((*op1 << 4).wrapping_add(seeds[2])) ^ (op1.wrapping_add(sum)) ^ ((*op1 >> 5).wrapping_add(seeds[3])),
        );
        *op1 = op1.wrapping_sub(
            ((*op2 << 4).wrapping_add(seeds[0])) ^ (op2.wrapping_add(sum)) ^ ((*op2 >> 5).wrapping_add(seeds[1])),
        );
        sum = sum.wrapping_sub(0x9E3779B9);
    }
}

/// `GBACheatReseedGameShark`
fn reseed_gameshark(seeds: &mut [u32; 4], params: u16, t1: &[u8; 256], t2: &[u8; 256]) {
    let s0 = (params >> 8) as usize;
    let s1 = (params & 0xFF) as usize;
    for (y, seed) in seeds.iter_mut().enumerate() {
        for x in 0..4 {
            let z = t1[(s0 + x) & 0xFF].wrapping_add(t2[(s1 + y) & 0xFF]);
            *seed <<= 8;
            *seed |= z as u32;
        }
    }
}

/// `GBACheatAddressIsReal`
pub fn address_is_real(address: u32) -> i32 {
    let off = address & OFFSET_MASK;
    match address >> 24 {
        0x0 => -0x80,
        0x2 => {
            if off > 0x40000 {
                -0x40
            } else {
                0x20
            }
        }
        0x3 => {
            if off > 0x8000 {
                -0x40
            } else {
                0x20
            }
        }
        0x4 => {
            if off > 0x400 {
                -0x80
            } else {
                0x10
            }
        }
        0x5 | 0x7 => {
            if off > 0x400 {
                -0x80
            } else {
                -0x8
            }
        }
        0x6 => {
            if off > 0x18000 {
                -0x80
            } else {
                -0x8
            }
        }
        0x8..=0xD => -0x8,
        0xE | 0xF => {
            if off > 0x10000 {
                -0x80
            } else {
                -0x8
            }
        }
        _ => -0xC0,
    }
}

// ----- GameShark (gameshark.c) -----

/// `GBACheatAddGameSharkRaw`
fn add_gameshark_raw(set: &mut CheatSet, op1: u32, op2: u32) -> bool {
    let (list, patches, st) = parts(set);
    if let Some(inc) = st.incomplete_cheat {
        let operand = list[inc].operand;
        for addr in [op1, op2] {
            if st.remaining_addresses > 0 {
                list.push(Cheat { ty: CheatType::Assign, width: 4, address: addr, operand, repeat: 1, ..Default::default() });
                st.remaining_addresses -= 1;
            }
        }
        if st.remaining_addresses == 0 {
            st.incomplete_cheat = None;
        }
        return true;
    }
    let mut cheat = Cheat { operand: op2, repeat: 1, ..Default::default() };
    match op1 >> 28 {
        0x0 | 0x1 | 0x2 => {
            cheat.ty = CheatType::Assign;
            cheat.width = 1 << (op1 >> 28);
            cheat.address = op1 & 0x0FFFFFFF;
        }
        0x3 => {
            // GSA_ASSIGN_LIST
            st.remaining_addresses = (op1 & 0xFFFF) as i32 - 1;
            // The first address is op2; following lines list more addresses
            // that receive this cheat's operand (which mGBA leaves as op2).
            cheat.ty = CheatType::Assign;
            cheat.width = 4;
            cheat.address = op2;
            list.push(cheat);
            st.incomplete_cheat = Some(list.len() - 1);
            return true;
        }
        0x6 => {
            // GSA_PATCH
            patches.push(CheatPatch {
                address: BASE_ROM0 | ((op1 & 0xFFFFFF) << 1),
                value: op2,
                width: 2,
                segment: -1,
                ..Default::default()
            });
            return true;
        }
        0x8 => {
            // GSA_BUTTON
            let width = match op1 & 0x00F00000 {
                0x00100000 => 1,
                0x00200000 => 2,
                _ => return false,
            };
            list.push(Cheat { ty: CheatType::IfButton, repeat: 1, ..Default::default() });
            cheat.ty = CheatType::Assign;
            cheat.width = width;
            cheat.address = op1 & 0x0F0FFFFF;
        }
        0xD => {
            // GSA_IF
            if op1 == 0xDEADFACE {
                reseed_gameshark(&mut st.gsa_seeds, op2 as u16, &GSA1_T1, &GSA1_T2);
                return true;
            }
            cheat.ty = match op2 >> 20 {
                0 => CheatType::IfEq,
                1 => CheatType::IfNe,
                2 => CheatType::IfLe,
                3 => CheatType::IfGe,
                // mGBA leaves the type unset (CHEAT_ASSIGN in practice)
                _ => CheatType::Assign,
            };
            cheat.width = 2;
            cheat.address = op1 & 0x0FFFFFFF;
            cheat.operand = op2 & 0xFFFF;
        }
        0xE => {
            // GSA_IF_RANGE
            cheat.ty = CheatType::IfEq;
            cheat.width = 2;
            cheat.address = op2 & 0x0FFFFFFF;
            cheat.operand = op1 & 0xFFFF;
            cheat.repeat = (op1 >> 16) & 0xFF;
        }
        0xF => {
            // GSA_HOOK
            if st.hook.is_some() {
                return false;
            }
            st.hook = Some(BASE_ROM0 | (op1 & (SIZE_ROM0 - 1)));
            return true;
        }
        _ => return false,
    }
    list.push(cheat);
    true
}

/// `GBACheatAddGameShark`
pub fn add_gameshark(set: &mut CheatSet, op1: u32, op2: u32) -> bool {
    let (_, _, st) = parts(set);
    let (mut o1, mut o2) = (op1, op2);
    match st.gsa_version {
        GS_GSAV1_RAW => {}
        GS_GSAV1 => decrypt_gameshark(&mut o1, &mut o2, &st.gsa_seeds),
        _ => {
            st.set_gameshark_version(GS_GSAV1);
            decrypt_gameshark(&mut o1, &mut o2, &st.gsa_seeds);
        }
    }
    add_gameshark_raw(set, o1, o2)
}

/// `GBACheatGameSharkProbability`
fn gameshark_probability(op1: u32, op2: u32) -> i32 {
    let mut p = 0;
    if op2 == 0x001DC0DE {
        return 0x100;
    }
    let address = op1 & 0x0FFFFFFF;
    match op1 >> 28 {
        0x0 => {
            p += 0x20;
            if op2 & 0xFFFFFF00 != 0 {
                p -= 0x10;
            }
            p += address_is_real(address);
        }
        0x1 => {
            p += 0x20;
            if op2 & 0xFFFF0000 != 0 {
                p -= 0x10;
            }
            p += address_is_real(address);
        }
        0x2 => {
            p += 0x20;
            p += address_is_real(address);
        }
        0x6 => {
            p += 0x20;
            if op2 & 0xCFFF0000 != 0 {
                p -= 0x10;
            }
        }
        0x8 => p += 0x10,
        0xD => {
            p += 0x20;
            if op2 & 0xFFCF0000 != 0 {
                p -= 0x10;
            }
            p += address_is_real(address);
        }
        0xE => {
            p += 0x20;
            p += address_is_real(op2);
            if op1 & 0x0F000000 != 0 {
                p -= 0x10;
            }
        }
        0xF => {
            p += 0x20;
            if op2 & 0xFFFF0000 != 0 {
                p -= 0x10;
            }
        }
        _ => p -= 0x40,
    }
    p
}

// ----- Pro Action Replay v3 (parv3.c) -----

fn par_addr(x: u32) -> u32 {
    (x & 0xFFFFF) | ((x << 4) & 0x0F000000)
}

/// `_parEndBlock`
fn par_end_block(list: &mut [Cheat], st: &mut GbaSetState) {
    let Some(cb) = st.current_block else { return };
    let size = (list.len() - cb - 1) as u32;
    let block = &mut list[cb];
    if block.repeat != 0 {
        block.negative_repeat = size.wrapping_sub(block.repeat);
    } else {
        block.repeat = size;
    }
    st.current_block = None;
}

/// `_parElseBlock`
fn par_else_block(list: &mut [Cheat], st: &mut GbaSetState) {
    let Some(cb) = st.current_block else { return };
    let size = (list.len() - cb - 1) as u32;
    list[cb].repeat = size;
}

/// `_addPAR3Cond`
fn add_par3_cond(set: &mut CheatSet, op1: u32, op2: u32) -> bool {
    let (list, _, st) = parts(set);
    let width = 1i32 << ((op1 & PAR3_WIDTH) >> PAR3_WIDTH_BASE);
    if (op1 & PAR3_ACTION) == 0xC0000000 {
        // TODO: Codes that disable
        return false;
    }
    let mut cheat = Cheat {
        address: par_addr(op1),
        width,
        operand: if width >= 4 { op2 } else { op2 & (0xFFFFFFFFu32 >> ((4 - width) * 8)) },
        ..Default::default()
    };
    match op1 & PAR3_ACTION {
        0x00000000 => cheat.repeat = 1,
        0x40000000 => cheat.repeat = 2,
        _ => {
            // PAR3_ACTION_BLOCK
            cheat.repeat = 0;
            if st.current_block.is_some() {
                par_end_block(list, st);
            }
            st.current_block = Some(list.len());
        }
    }
    cheat.ty = match op1 & PAR3_COND {
        0x08000000 => CheatType::IfEq,
        0x10000000 => CheatType::IfNe,
        0x18000000 => CheatType::IfLt,
        0x20000000 => CheatType::IfGt,
        0x28000000 => CheatType::IfUlt,
        0x30000000 => CheatType::IfUgt,
        0x38000000 => CheatType::IfAnd,
        _ => {
            cheat.operand = 0;
            CheatType::IfLand
        }
    };
    if width > 4 {
        cheat.width = 0;
        cheat.ty = CheatType::Never;
    }
    list.push(cheat);
    true
}

/// `_addPAR3Special`
fn add_par3_special(set: &mut CheatSet, op2: u32) -> bool {
    let (list, patches, st) = parts(set);
    let mut rom_patch = false;
    match op2 & 0xFF000000 {
        0x08000000 => return false, // TODO: Slowdown
        0x10000000 | 0x12000000 | 0x14000000 => {
            let width = 1 << ((op2 >> 25) & 3);
            list.push(Cheat { ty: CheatType::IfButton, repeat: 1, ..Default::default() });
            list.push(Cheat { ty: CheatType::Assign, width, repeat: 1, address: par_addr(op2), ..Default::default() });
            st.incomplete_cheat = Some(list.len() - 1);
        }
        0x18000000 | 0x1A000000 | 0x1C000000 | 0x1E000000 => rom_patch = true,
        0x40000000 => {
            if st.current_block.is_some() {
                par_end_block(list, st);
                return true;
            }
            return false;
        }
        0x60000000 => {
            if st.current_block.is_some() {
                par_else_block(list, st);
                return true;
            }
            return false;
        }
        0x80000000 | 0x82000000 | 0x84000000 => {
            let width = 1 << ((op2 >> 25) & 3);
            list.push(Cheat { ty: CheatType::Assign, width, address: par_addr(op2), ..Default::default() });
            st.incomplete_cheat = Some(list.len() - 1);
        }
        _ => {}
    }
    if rom_patch {
        patches.push(CheatPatch { address: BASE_ROM0 | ((op2 & 0xFFFFFF) << 1), width: 2, segment: -1, ..Default::default() });
        st.incomplete_patch = Some(patches.len() - 1);
    }
    true
}

/// `GBACheatAddProActionReplayRaw`
fn add_par3_raw(set: &mut CheatSet, op1: u32, op2: u32) -> bool {
    {
        let (list, patches, st) = parts(set);
        if let Some(p) = st.incomplete_patch.take() {
            patches[p].value = op1;
            return true;
        }
        if let Some(inc) = st.incomplete_cheat {
            let width = list[inc].width;
            list[inc].operand = if width >= 4 { op1 } else { op1 & (0xFFFFFFFFu32 >> ((4 - width) * 8)) };
            if inc > 0 && list[inc - 1].ty == CheatType::IfButton {
                st.incomplete_cheat = None;
                return true;
            }
            let c = &mut list[inc];
            c.operand_offset = (op2 >> 24) as i32;
            c.repeat = (op2 >> 16) & 0xFF;
            c.address_offset = (op2 & 0xFFFF) as i32 * c.width;
            st.incomplete_cheat = None;
            return true;
        }
        if op2 == 0x001DC0DE {
            return true;
        }
        if op1 == 0xDEADFACE {
            reseed_gameshark(&mut st.gsa_seeds, op2 as u16, &PAR3_T1, &PAR3_T2);
            return true;
        }
    }
    if op1 == 0 {
        return add_par3_special(set, op2);
    }
    let (list, _, st) = parts(set);
    if op1 >> 24 == 0xC4 {
        if st.hook.is_some() {
            return false;
        }
        st.hook = Some(BASE_ROM0 | (op1 & (SIZE_ROM0 - 2)));
        return true;
    }
    if op1 & PAR3_COND != 0 {
        return add_par3_cond(set, op1, op2);
    }
    let mut width = 1i32 << ((op1 & PAR3_WIDTH) >> PAR3_WIDTH_BASE);
    let mut cheat = Cheat { address: par_addr(op1), repeat: 1, ..Default::default() };
    match op1 & PAR3_BASE {
        0x00000000 => {
            cheat.ty = CheatType::Assign;
            cheat.address_offset = width;
            if width < 4 {
                cheat.repeat = (op2 >> (width * 8)) + 1;
            }
        }
        0x40000000 => {
            cheat.ty = CheatType::AssignIndirect;
            if width < 4 {
                cheat.address_offset = ((op2 >> (width * 8)) as i32).wrapping_mul(width);
            }
        }
        0x80000000 => cheat.ty = CheatType::Add,
        _ => {
            width = (((op1 >> 24) & 1) + 1) as i32;
            cheat.ty = CheatType::Assign;
            cheat.address = BASE_IO | (op1 & OFFSET_MASK);
        }
    }
    if op1 & 0x01000000 != 0 && (op1 & 0xFE000000) != 0xC6000000 {
        return false;
    }
    cheat.width = width;
    cheat.operand = if width >= 4 { op2 } else { op2 & (0xFFFFFFFFu32 >> ((4 - width) * 8)) };
    list.push(cheat);
    true
}

/// `GBACheatAddProActionReplay`
pub fn add_pro_action_replay(set: &mut CheatSet, op1: u32, op2: u32) -> bool {
    let (_, _, st) = parts(set);
    let (mut o1, mut o2) = (op1, op2);
    match st.gsa_version {
        GS_PARV3_RAW => {}
        GS_PARV3 => decrypt_gameshark(&mut o1, &mut o2, &st.gsa_seeds),
        _ => {
            st.set_gameshark_version(GS_PARV3);
            decrypt_gameshark(&mut o1, &mut o2, &st.gsa_seeds);
        }
    }
    add_par3_raw(set, o1, o2)
}

/// `GBACheatProActionReplayProbability`
fn par3_probability(op1: u32, op2: u32) -> i32 {
    let mut p = 0;
    if op2 == 0x001DC0DE {
        return 0x100;
    }
    if op1 == 0xDEADFACE && op2 & 0xFFFF0000 == 0 {
        return 0x100;
    }
    if op1 == 0 {
        p += 0x40;
        let address = par_addr(op2);
        match op2 & 0xFE000000 {
            0x80000000 | 0x82000000 | 0x84000000 => p += address_is_real(address),
            0x18000000 | 0x1A000000 | 0x1C000000 | 0x1E000000 => {}
            0x00000000 | 0x08000000 | 0x10000000 | 0x12000000 | 0x14000000 | 0x40000000 | 0x60000000 => {
                if op2 & 0x01000000 != 0 {
                    p -= 0x40;
                }
            }
            _ => p -= 0x40,
        }
        return p;
    }
    let width = ((op1 & PAR3_WIDTH) >> (PAR3_WIDTH_BASE - 3)) as i32;
    if op1 & PAR3_COND != 0 {
        p += 0x20;
        if width >= 24 {
            return 0;
        }
        if op2 & !((1u32 << width) - 1) != 0 {
            p -= 0x10;
        }
    } else {
        let address = par_addr(op1);
        p += 0x20;
        match op1 & PAR3_BASE {
            0x80000000 | 0x00000000 | 0x40000000 => {
                if op1 & PAR3_BASE == 0x80000000 && width < 32 && op2 & !((1u32 << width) - 1) != 0 {
                    p -= 0x10;
                }
                p += address_is_real(address);
                if op1 & 0x01000000 != 0 {
                    return 0;
                }
            }
            _ => {}
        }
    }
    p
}

// ----- CodeBreaker (codebreaker.c) -----

fn cb_load_byteswap(op1: u32, op2: u16) -> [u8; 6] {
    [(op1 >> 24) as u8, (op1 >> 16) as u8, (op1 >> 8) as u8, op1 as u8, (op2 >> 8) as u8, op2 as u8]
}

fn cb_store_byteswap(b: &[u8; 6]) -> (u32, u16) {
    (
        ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | b[3] as u32,
        ((b[4] as u16) << 8) | b[5] as u16,
    )
}

/// `_cbDecrypt`
fn cb_decrypt(st: &GbaSetState, op1: &mut u32, op2: &mut u16) {
    let mut buffer = cb_load_byteswap(*op1, *op2);
    for i in (0..st.cb_table.len()).rev() {
        let offset_x = i >> 3;
        let offset_y = (st.cb_table[i] >> 3) as usize;
        let bit_x = i & 7;
        let bit_y = (st.cb_table[i] & 7) as usize;
        let x = (buffer[offset_x] >> bit_x) & 1;
        let y = (buffer[offset_y] >> bit_y) & 1;
        let mut x2 = buffer[offset_x] & !(1 << bit_x);
        if y != 0 {
            x2 |= 1 << bit_x;
        }
        buffer[offset_x] = x2;
        let mut y2 = buffer[offset_y] & !(1 << bit_y);
        if x != 0 {
            y2 |= 1 << bit_y;
        }
        buffer[offset_y] = y2;
    }
    let (mut o1, mut o2) = cb_store_byteswap(&buffer);
    o1 ^= st.cb_seeds[0];
    o2 ^= st.cb_seeds[1] as u16;
    let mut buffer = cb_load_byteswap(o1, o2);
    let master = st.cb_master;
    for i in 0..5 {
        buffer[i] ^= ((master >> 8) as u8) ^ buffer[i + 1];
    }
    buffer[5] ^= (master >> 8) as u8;
    for i in (1..=5).rev() {
        buffer[i] ^= (master as u8) ^ buffer[i - 1];
    }
    buffer[0] ^= master as u8;
    let (mut o1, mut o2) = cb_store_byteswap(&buffer);
    o1 ^= st.cb_seeds[2];
    o2 ^= st.cb_seeds[3] as u16;
    *op1 = o1;
    *op2 = o2;
}

/// `_cbRand`
fn cb_rand(st: &mut GbaSetState) -> u32 {
    // Roll LCG three times to get enough bits of entropy
    let roll = st.cb_rng_state.wrapping_mul(0x41C64E6D).wrapping_add(0x3039);
    let roll2 = roll.wrapping_mul(0x41C64E6D).wrapping_add(0x3039);
    let roll3 = roll2.wrapping_mul(0x41C64E6D).wrapping_add(0x3039);
    let mut mix = (roll << 14) & 0xC0000000;
    mix |= (roll2 >> 1) & 0x3FFF8000;
    mix |= (roll3 >> 16) & 0x7FFF;
    st.cb_rng_state = roll3;
    mix
}

/// `_cbSwapIndex`
fn cb_swap_index(st: &mut GbaSetState) -> usize {
    let mut roll = cb_rand(st);
    let mut count = st.cb_table.len() as u32;
    if roll == count {
        roll = 0;
    }
    if roll < count {
        return roll as usize;
    }
    let mut bit: u32 = 1;
    while count < 0x10000000 && count < roll {
        count <<= 4;
        bit <<= 4;
    }
    while count < 0x80000000 && count < roll {
        count <<= 1;
        bit <<= 1;
    }
    let mut mask;
    loop {
        mask = 0;
        if roll >= count {
            roll -= count;
        }
        if roll >= count >> 1 {
            roll -= count >> 1;
            mask |= bit.rotate_right(1);
        }
        if roll >= count >> 2 {
            roll -= count >> 2;
            mask |= bit.rotate_right(2);
        }
        if roll >= count >> 3 {
            roll -= count >> 3;
            mask |= bit.rotate_right(3);
        }
        if roll == 0 || bit >> 4 == 0 {
            break;
        }
        bit >>= 4;
        count >>= 4;
    }
    mask &= 0xE0000000;
    if mask == 0 || bit & 7 == 0 {
        return roll as usize;
    }
    if mask & bit.rotate_right(3) != 0 {
        roll = roll.wrapping_add(count >> 3);
    }
    if mask & bit.rotate_right(2) != 0 {
        roll = roll.wrapping_add(count >> 2);
    }
    if mask & bit.rotate_right(1) != 0 {
        roll = roll.wrapping_add(count >> 1);
    }
    roll as usize
}

/// `_cbReseed`
fn cb_reseed(st: &mut GbaSetState, op1: u32, op2: u16) {
    st.cb_rng_state = (op2 as u32 & 0xFF) ^ 0x1111;
    // Populate the initial seed table
    for (i, t) in st.cb_table.iter_mut().enumerate() {
        *t = i as u8;
    }
    // Swap pseudo-random table entries based on the input code
    for _ in 0..0x50 {
        let x = cb_swap_index(st) % st.cb_table.len();
        let y = cb_swap_index(st) % st.cb_table.len();
        st.cb_table.swap(x, y);
    }
    // Spin the RNG some to make the initial seed
    st.cb_rng_state = 0x4EFAD1C3;
    for _ in 0..((op1 >> 24) & 0xF) {
        st.cb_rng_state = cb_rand(st);
    }
    st.cb_seeds[2] = cb_rand(st);
    st.cb_seeds[3] = cb_rand(st);
    st.cb_rng_state = (op2 as u32 >> 8) ^ 0xF254;
    for _ in 0..(op2 >> 8) {
        st.cb_rng_state = cb_rand(st);
    }
    st.cb_seeds[0] = cb_rand(st);
    st.cb_seeds[1] = cb_rand(st);
    st.cb_master = op1;
}

fn swap16(h: u32) -> u32 {
    ((h >> 8) | (h << 8)) & 0xFFFF
}

/// `GBACheatAddCodeBreaker`
pub fn add_codebreaker(set: &mut CheatSet, op1: u32, op2: u16) -> bool {
    let (list, _, st) = parts(set);
    let (mut op1, mut op2) = (op1, op2);
    if st.cb_master != 0 {
        cb_decrypt(st, &mut op1, &mut op2);
    }
    if let Some(mut inc) = st.incomplete_cheat {
        if st.remaining_addresses != 0 {
            list[inc].operand = swap16(op1 >> 16);
            st.remaining_addresses -= 1;
            for half in [op1 & 0xFFFF, op2 as u32] {
                if st.remaining_addresses != 0 {
                    let address = list[inc].address.wrapping_add(2);
                    list.push(Cheat { ty: CheatType::Assign, width: 2, address, operand: swap16(half), repeat: 1, ..Default::default() });
                    st.remaining_addresses -= 1;
                    inc = list.len() - 1;
                }
            }
            if st.remaining_addresses == 0 {
                st.incomplete_cheat = None;
            } else {
                let address = list[inc].address.wrapping_add(2);
                list.push(Cheat { ty: CheatType::Assign, width: 2, address, repeat: 1, ..Default::default() });
                st.incomplete_cheat = Some(list.len() - 1);
            }
        } else {
            let c = &mut list[inc];
            c.repeat = op1 & 0xFFFF;
            c.address_offset = op2 as i32;
            c.operand_offset = (op1 >> 16) as i32;
            st.incomplete_cheat = None;
        }
        return true;
    }
    let ty = match op1 >> 28 {
        0x0 => return true, // CB_GAME_ID: TODO: Run checksum
        0x1 => {
            if st.hook.is_some() {
                return false;
            }
            st.hook = Some(BASE_ROM0 | (op1 & (SIZE_ROM0 - 1)));
            return true;
        }
        0x2 => (CheatType::Or, 2),
        0x3 => (CheatType::Assign, 1),
        0x4 | 0x5 => (CheatType::Assign, 2),
        0x6 => (CheatType::And, 2),
        0x7 => (CheatType::IfEq, 2),
        0x8 => (CheatType::Assign, 2),
        0x9 => {
            cb_reseed(st, op1, op2);
            return true;
        }
        0xA => (CheatType::IfNe, 2),
        0xB => (CheatType::IfGt, 2),
        0xC => (CheatType::IfLt, 2),
        0xD => {
            if op1 & 0x0FFFFFFF == 0x20 {
                list.push(Cheat {
                    ty: CheatType::IfNand,
                    width: 2,
                    address: BASE_IO | REG_KEYINPUT,
                    operand: op2 as u32,
                    repeat: 1,
                    ..Default::default()
                });
                return true;
            }
            return false;
        }
        0xE => (CheatType::Add, 2),
        _ => (CheatType::IfAnd, 2),
    };
    list.push(Cheat { ty: ty.0, width: ty.1, address: op1 & 0x0FFFFFFF, operand: op2 as u32, repeat: 1, ..Default::default() });
    match op1 >> 28 {
        0x4 => st.incomplete_cheat = Some(list.len() - 1),
        0x5 => {
            st.incomplete_cheat = Some(list.len() - 1);
            st.remaining_addresses = op2 as i32;
        }
        _ => {}
    }
    true
}

// ----- cheats.c -----

/// `GBACheatAddAutodetect`
fn add_autodetect(set: &mut CheatSet, op1: u32, op2: u32) -> bool {
    let (_, _, st) = parts(set);
    match st.gsa_version {
        GS_NOT_SET => {
            // Try to detect GameShark version
            let mut max = i32::MIN;
            let (mut o1, mut o2) = (op1, op2);
            decrypt_gameshark(&mut o1, &mut o2, &GAMESHARK_SEEDS);
            let p = gameshark_probability(o1, o2);
            if p > max {
                max = p;
                st.set_gameshark_version(GS_GSAV1);
            }
            let (mut o1, mut o2) = (op1, op2);
            decrypt_gameshark(&mut o1, &mut o2, &PRO_ACTION_REPLAY_SEEDS);
            let p = par3_probability(o1, o2);
            if p > max {
                max = p;
                st.set_gameshark_version(GS_PARV3);
            }
            let p = gameshark_probability(op1, op2);
            if p > max {
                max = p;
                st.set_gameshark_version(GS_GSAV1_RAW);
            }
            let p = par3_probability(op1, op2);
            if p > max {
                st.set_gameshark_version(GS_PARV3_RAW);
            }
            if st.gsa_version < GS_PARV3 {
                add_gameshark(set, op1, op2)
            } else {
                add_pro_action_replay(set, op1, op2)
            }
        }
        GS_GSAV1 | GS_GSAV1_RAW => add_gameshark(set, op1, op2),
        _ => add_pro_action_replay(set, op1, op2),
    }
}

/// `GBACheatAddVBALine`
pub fn add_vba_line(set: &mut CheatSet, line: &str) -> bool {
    let Some((address, rest)) = hex(line, 8) else { return false };
    let Some(mut rest) = rest.strip_prefix(':') else { return false };
    let mut value: u32 = 0;
    let mut width = 0;
    while width < 4 {
        let Some((op, r)) = hex(rest, 2) else { break };
        rest = r;
        value = (value << 8) | op;
        width += 1;
    }
    if width == 0 || width == 3 {
        return false;
    }
    let (list, patches, _) = parts(set);
    if !(BASE_ROM0..BASE_SRAM).contains(&address) {
        list.push(Cheat { ty: CheatType::Assign, width, address, operand: value, repeat: 1, ..Default::default() });
    } else {
        patches.push(CheatPatch { width, address, segment: 0, value, ..Default::default() });
    }
    true
}

/// `GBACheatAddLine` with `GBA_CHEAT_AUTODETECT`
pub fn add_line(set: &mut CheatSet, line: &str) -> bool {
    let Some((op1, rest)) = hex(line, 8) else { return false };
    if rest.starts_with(':') {
        return add_vba_line(set, line);
    }
    let rest = rest.trim_start();
    let Some((op2, rest)) = hex(rest, 4) else { return false };
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        return add_codebreaker(set, op1, op2 as u16);
    }
    let Some((op3, _)) = hex(rest, 4) else { return false };
    add_autodetect(set, op1, (op2 << 16) | op3)
}
