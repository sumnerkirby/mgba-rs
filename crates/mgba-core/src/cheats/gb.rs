// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's Game Boy cheat formats (src/gb/cheats.c):
// GameShark (`01xxyyyy`), CodeBreaker (`xxxx-yy` style), Game Genie
// (`ABC-DEF` or `ABC-DEF-GHI`) and VBA (`xxxx:yy`).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::{hex, Cheat, CheatPatch, CheatSet, CheatType};

/// `GBCheatAddCodebreaker`
fn add_codebreaker(set: &mut CheatSet, address: u16, data: u8) -> bool {
    set.list.push(Cheat { ty: CheatType::Assign, width: 1, address: address as u32, operand: data as u32, repeat: 1, ..Default::default() });
    true
}

/// `GBCheatAddGameShark`
fn add_gameshark(set: &mut CheatSet, op: u32) -> bool {
    add_codebreaker(set, (((op & 0xFF) << 8) | ((op >> 8) & 0xFF)) as u16, ((op >> 16) & 0xFF) as u8)
}

/// `GBCheatAddGameGenieLine`
fn add_game_genie_line(set: &mut CheatSet, line: &str) -> bool {
    let Some((op1, rest)) = hex(line, 3) else { return false };
    let Some(rest) = rest.strip_prefix('-') else { return false };
    let Some((op2, mut rest)) = hex(rest, 3) else { return false };
    let mut op3 = 0x1000u32;
    if let Some(r) = rest.strip_prefix('-') {
        let Some((v, r)) = hex(r, 3) else { return false };
        op3 = v;
        rest = r;
    }
    if !rest.is_empty() {
        return false;
    }
    let mut address = (op1 & 0xF) << 8;
    address |= (op2 >> 4) & 0xFF;
    address |= ((op2 & 0xF) ^ 0xF) << 12;
    let mut patch = CheatPatch { address, value: op1 >> 4, width: 1, segment: -1, ..Default::default() };
    if op3 < 0x1000 {
        let mut value = ((op3 & 0xF00) << 20) | (op3 & 0xF);
        value = value.rotate_right(2);
        value |= value >> 24;
        value ^= 0xBA;
        patch.check_value = value & 0xFF;
        patch.check = true;
    }
    set.rom_patches.push(patch);
    true
}

/// `GBCheatAddVBALine`. mGBA reads the value from the start of the line
/// (`hex8(line, ...)`), which makes these codes write the address's high byte;
/// this port reads the value after the colon as the format intends.
fn add_vba_line(set: &mut CheatSet, line: &str) -> bool {
    let Some((address, rest)) = hex(line, 4) else { return false };
    let Some(rest) = rest.strip_prefix(':') else { return false };
    let Some((value, _)) = hex(rest, 2) else { return false };
    set.list.push(Cheat { ty: CheatType::Assign, width: 1, address, operand: value, repeat: 1, ..Default::default() });
    true
}

/// `GBCheatAddLine` with `GB_CHEAT_AUTODETECT`
pub fn add_line(set: &mut CheatSet, line: &str) -> bool {
    let Some((op1, rest)) = hex(line, 4) else {
        return add_game_genie_line(set, line);
    };
    if rest.starts_with(':') {
        return add_vba_line(set, line);
    }
    let Some((op2, mut rest)) = hex(rest, 2) else { return false };
    let mut codebreaker = false;
    if let Some(r) = rest.strip_prefix('-') {
        codebreaker = true;
        rest = r;
    }
    let Some((op3, _)) = hex(rest, 2) else { return false };
    if codebreaker {
        add_codebreaker(set, ((op1 << 8) | op2) as u16, op3 as u8)
    } else {
        add_gameshark(set, (op1 << 16) | (op2 << 8) | op3)
    }
}
