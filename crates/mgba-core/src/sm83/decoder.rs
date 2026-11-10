// Copyright (c) 2013-2017 Jeffrey Pfau
// Rust port of mGBA's SM83 instruction decoder and disassembler
// (src/sm83/decoder.c, include/mgba/internal/sm83/decoder.h).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! mGBA builds 256-entry decoder tables with its emitter macros; the same
//! choices are made here from the opcode's bit fields.

/// `enum SM83Condition`
pub const COND_NONE: u8 = 0;
pub const COND_C: u8 = 1;
pub const COND_Z: u8 = 2;
pub const COND_NC: u8 = 3;
pub const COND_NZ: u8 = 4;

/// `enum SM83Register`
pub const REG_B: u8 = 1;
pub const REG_C: u8 = 2;
pub const REG_D: u8 = 3;
pub const REG_E: u8 = 4;
pub const REG_H: u8 = 5;
pub const REG_L: u8 = 6;
pub const REG_A: u8 = 7;
pub const REG_F: u8 = 8;
pub const REG_BC: u8 = 9;
pub const REG_DE: u8 = 10;
pub const REG_HL: u8 = 11;
pub const REG_AF: u8 = 12;
pub const REG_SP: u8 = 13;
pub const REG_PC: u8 = 14;

pub const OP_FLAG_IMPLICIT: u8 = 1;
pub const OP_FLAG_MEMORY: u8 = 2;
pub const OP_FLAG_INCREMENT: u8 = 4;
pub const OP_FLAG_DECREMENT: u8 = 8;
pub const OP_FLAG_RELATIVE: u8 = 16;

/// `enum SM83Mnemonic`
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum Mnemonic {
    #[default]
    Ill = 0,
    Adc,
    Add,
    And,
    Bit,
    Call,
    Ccf,
    Cp,
    Cpl,
    Daa,
    Dec,
    Di,
    Ei,
    Halt,
    Inc,
    Jp,
    Jr,
    Ld,
    Ldh,
    Nop,
    Or,
    Pop,
    Push,
    Res,
    Ret,
    Reti,
    Rl,
    Rlc,
    Rr,
    Rrc,
    Rst,
    Sbc,
    Scf,
    Set,
    Sla,
    Sra,
    Srl,
    Stop,
    Sub,
    Swap,
    Xor,
}

/// `struct SM83Operand`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Operand {
    pub reg: u8,
    pub flags: u8,
    pub immediate: u16,
}

/// `struct SM83InstructionInfo`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InstructionInfo {
    pub opcode: [u8; 3],
    pub opcode_size: u8,
    pub op1: Operand,
    pub op2: Operand,
    pub mnemonic: Mnemonic,
    pub condition: u8,
}

/// Register operand for the 3-bit field used throughout the opcode map
/// (B, C, D, E, H, L, (HL), A).
fn reg8(field: u8) -> Operand {
    match field & 7 {
        6 => Operand { reg: REG_HL, flags: OP_FLAG_MEMORY, immediate: 0 },
        7 => Operand { reg: REG_A, ..Default::default() },
        f => Operand { reg: REG_B + f, ..Default::default() },
    }
}

fn reg(r: u8) -> Operand {
    Operand { reg: r, ..Default::default() }
}

const COND_FROM_FIELD: [u8; 4] = [COND_NZ, COND_Z, COND_NC, COND_C];
const WIDE: [u8; 4] = [REG_BC, REG_DE, REG_HL, REG_SP];
const ALU: [Mnemonic; 8] =
    [Mnemonic::Add, Mnemonic::Adc, Mnemonic::Sub, Mnemonic::Sbc, Mnemonic::And, Mnemonic::Xor, Mnemonic::Or, Mnemonic::Cp];

/// The `_sm83DecoderTable` entry for `opcode`; returns how many immediate bytes follow.
fn decode_main(opcode: u8, info: &mut InstructionInfo) -> usize {
    // DEFINE_DECODER_SM83 starts every decoder by setting RST.
    info.mnemonic = Mnemonic::Rst;
    let x = opcode >> 6;
    let y = (opcode >> 3) & 7;
    let z = opcode & 7;
    match x {
        1 => {
            if opcode == 0x76 {
                info.mnemonic = Mnemonic::Halt;
            } else {
                info.mnemonic = Mnemonic::Ld;
                info.op1 = reg8(y);
                info.op2 = reg8(z);
            }
            return 0;
        }
        2 => {
            // DEFINE_ALU_DECODER_SM83 (register and (HL) forms)
            info.mnemonic = ALU[y as usize];
            info.op1 = reg8(z);
            return 0;
        }
        _ => {}
    }
    match opcode {
        0x00 => info.mnemonic = Mnemonic::Nop,
        0x01 | 0x11 | 0x21 | 0x31 => {
            info.mnemonic = Mnemonic::Ld;
            info.op1 = reg(WIDE[(y >> 1) as usize]);
            return 2;
        }
        0x02 | 0x12 => {
            info.mnemonic = Mnemonic::Ld;
            info.op1 = Operand { reg: if opcode == 0x02 { REG_BC } else { REG_DE }, flags: OP_FLAG_MEMORY, immediate: 0 };
            info.op2 = reg(REG_A);
        }
        0x0A | 0x1A => {
            info.mnemonic = Mnemonic::Ld;
            info.op1 = reg(REG_A);
            info.op2 = Operand { reg: if opcode == 0x0A { REG_BC } else { REG_DE }, flags: OP_FLAG_MEMORY, immediate: 0 };
        }
        0x22 | 0x32 => {
            info.mnemonic = Mnemonic::Ld;
            let step = if opcode == 0x22 { OP_FLAG_INCREMENT } else { OP_FLAG_DECREMENT };
            info.op1 = Operand { reg: REG_HL, flags: step | OP_FLAG_MEMORY, immediate: 0 };
            info.op2 = reg(REG_A);
        }
        0x2A | 0x3A => {
            info.mnemonic = Mnemonic::Ld;
            let step = if opcode == 0x2A { OP_FLAG_INCREMENT } else { OP_FLAG_DECREMENT };
            info.op1 = reg(REG_A);
            info.op2 = Operand { reg: REG_HL, flags: step | OP_FLAG_MEMORY, immediate: 0 };
        }
        0x03 | 0x13 | 0x23 | 0x33 => {
            info.mnemonic = Mnemonic::Inc;
            info.op1 = reg(WIDE[(y >> 1) as usize]);
        }
        0x0B | 0x1B | 0x2B | 0x3B => {
            info.mnemonic = Mnemonic::Dec;
            info.op1 = reg(WIDE[(y >> 1) as usize]);
        }
        0x09 | 0x19 | 0x29 | 0x39 => {
            info.mnemonic = Mnemonic::Add;
            info.op1 = reg(REG_HL);
            info.op2 = reg(WIDE[(y >> 1) as usize]);
        }
        _ if x == 0 && z == 4 => {
            info.mnemonic = Mnemonic::Inc;
            info.op1 = reg8(y);
        }
        _ if x == 0 && z == 5 => {
            info.mnemonic = Mnemonic::Dec;
            info.op1 = reg8(y);
        }
        _ if x == 0 && z == 6 => {
            info.mnemonic = Mnemonic::Ld;
            info.op1 = reg8(y);
            return 1;
        }
        0x07 | 0x0F | 0x17 | 0x1F => {
            // RLCA_, RRCA_, RLA_, RRA_
            info.mnemonic = [Mnemonic::Rlc, Mnemonic::Rrc, Mnemonic::Rl, Mnemonic::Rr][y as usize];
            info.op1 = Operand { reg: REG_A, flags: OP_FLAG_IMPLICIT, immediate: 0 };
        }
        0x08 => {
            info.mnemonic = Mnemonic::Ld;
            info.op1.flags = OP_FLAG_MEMORY;
            info.op2 = reg(REG_SP);
            return 2;
        }
        0x10 => info.mnemonic = Mnemonic::Stop,
        0x18 | 0x20 | 0x28 | 0x30 | 0x38 => {
            info.mnemonic = Mnemonic::Jr;
            info.condition = if opcode == 0x18 { COND_NONE } else { COND_FROM_FIELD[(y - 4) as usize] };
            info.op1.flags = OP_FLAG_RELATIVE;
            return 1;
        }
        0x27 => info.mnemonic = Mnemonic::Daa,
        0x2F => info.mnemonic = Mnemonic::Cpl,
        0x37 => info.mnemonic = Mnemonic::Scf,
        0x3F => info.mnemonic = Mnemonic::Ccf,
        0xC0 | 0xC8 | 0xD0 | 0xD8 => {
            info.mnemonic = Mnemonic::Ret;
            info.condition = COND_FROM_FIELD[y as usize];
        }
        0xC9 => info.mnemonic = Mnemonic::Ret,
        0xD9 => info.mnemonic = Mnemonic::Reti,
        0xC1 | 0xD1 | 0xE1 | 0xF1 | 0xC5 | 0xD5 | 0xE5 | 0xF5 => {
            info.mnemonic = if z == 1 { Mnemonic::Pop } else { Mnemonic::Push };
            info.op1 = reg([REG_BC, REG_DE, REG_HL, REG_AF][(y >> 1) as usize]);
        }
        0xC2 | 0xCA | 0xD2 | 0xDA | 0xC3 => {
            info.mnemonic = Mnemonic::Jp;
            info.condition = if opcode == 0xC3 { COND_NONE } else { COND_FROM_FIELD[y as usize] };
            return 2;
        }
        0xC4 | 0xCC | 0xD4 | 0xDC | 0xCD => {
            info.mnemonic = Mnemonic::Call;
            info.condition = if opcode == 0xCD { COND_NONE } else { COND_FROM_FIELD[y as usize] };
            return 2;
        }
        0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => {
            info.mnemonic = ALU[y as usize];
            info.op1 = Operand { reg: REG_A, flags: OP_FLAG_IMPLICIT, immediate: 0 };
            return 1;
        }
        _ if x == 3 && z == 7 => {
            // RST: mnemonic is already RST
            info.op1.immediate = (y as u16) * 8;
        }
        0xCB => return 1,
        0xE0 => {
            info.mnemonic = Mnemonic::Ldh;
            info.op1 = Operand { reg: 0, flags: OP_FLAG_MEMORY, immediate: 0xFF00 };
            info.op2 = reg(REG_A);
            return 1;
        }
        0xF0 => {
            info.mnemonic = Mnemonic::Ldh;
            info.op1 = reg(REG_A);
            info.op2 = Operand { reg: 0, flags: OP_FLAG_MEMORY, immediate: 0xFF00 };
            return 1;
        }
        0xE2 => {
            info.mnemonic = Mnemonic::Ldh;
            info.op1 = Operand { reg: REG_C, flags: OP_FLAG_MEMORY, immediate: 0xFF00 };
            info.op2 = reg(REG_A);
        }
        0xF2 => {
            info.mnemonic = Mnemonic::Ldh;
            info.op1 = reg(REG_A);
            info.op2 = Operand { reg: REG_C, flags: OP_FLAG_MEMORY, immediate: 0xFF00 };
        }
        0xE8 => {
            info.mnemonic = Mnemonic::Add;
            info.op1 = reg(REG_SP);
            return 1;
        }
        0xF8 => {
            info.mnemonic = Mnemonic::Ld;
            info.op1 = reg(REG_HL);
            info.op2 = reg(REG_SP);
            return 1;
        }
        0xE9 => {
            info.mnemonic = Mnemonic::Jp;
            info.op1 = reg(REG_HL);
        }
        0xF9 => {
            info.mnemonic = Mnemonic::Ld;
            info.op1 = reg(REG_SP);
            info.op2 = reg(REG_HL);
        }
        0xEA => {
            info.mnemonic = Mnemonic::Ld;
            info.op1.flags = OP_FLAG_MEMORY;
            info.op2 = reg(REG_A);
            return 2;
        }
        0xFA => {
            info.mnemonic = Mnemonic::Ld;
            info.op1 = reg(REG_A);
            info.op2.flags = OP_FLAG_MEMORY;
            return 2;
        }
        0xF3 => info.mnemonic = Mnemonic::Di,
        0xFB => info.mnemonic = Mnemonic::Ei,
        _ => info.mnemonic = Mnemonic::Ill,
    }
    0
}

/// The `_sm83CBDecoderTable` entry for `opcode`.
fn decode_cb(opcode: u8, info: &mut InstructionInfo) -> usize {
    let x = opcode >> 6;
    let y = (opcode >> 3) & 7;
    let z = opcode & 7;
    if x == 0 {
        info.mnemonic = [
            Mnemonic::Rlc,
            Mnemonic::Rrc,
            Mnemonic::Rl,
            Mnemonic::Rr,
            Mnemonic::Sla,
            Mnemonic::Sra,
            Mnemonic::Swap,
            Mnemonic::Srl,
        ][y as usize];
        info.op1 = reg8(z);
    } else {
        info.mnemonic = [Mnemonic::Ill, Mnemonic::Bit, Mnemonic::Res, Mnemonic::Set][x as usize];
        info.op1.immediate = y as u16;
        info.op2 = reg8(z);
    }
    0
}

/// `SM83Decode`: feed one byte; returns how many more bytes the instruction needs.
pub fn decode_byte(opcode: u8, info: &mut InstructionInfo) -> usize {
    if info.opcode_size as usize == info.opcode.len() {
        return 0;
    }
    info.opcode[info.opcode_size as usize] = opcode;
    match info.opcode_size {
        0 => {
            info.opcode_size += 1;
            decode_main(opcode, info)
        }
        1 if info.opcode[0] == 0xCB => {
            info.opcode_size += 1;
            decode_cb(opcode, info)
        }
        _ => {
            info.opcode_size += 1;
            let shift = (info.opcode_size as u32 - 2) * 8;
            if info.op1.reg != 0 {
                info.op2.immediate |= (opcode as u16) << shift;
            } else {
                info.op1.immediate |= (opcode as u16) << shift;
            }
            0
        }
    }
}

/// Decode a whole instruction from `read(address)`; returns it and its length.
pub fn decode(mut read: impl FnMut(u16) -> u8, address: u16) -> (InstructionInfo, u16) {
    let mut info = InstructionInfo::default();
    let mut address = address;
    let mut more = decode_byte(read(address), &mut info);
    while more > 0 {
        address = address.wrapping_add(1);
        more = more - 1 + decode_byte(read(address), &mut info);
    }
    let len = info.opcode_size as u16;
    (info, len)
}

const CONDITIONS: [Option<&str>; 5] = [None, Some("c"), Some("z"), Some("nc"), Some("nz")];
const REGISTERS: [&str; 15] = ["", "b", "c", "d", "e", "h", "l", "a", "f", "bc", "de", "hl", "af", "sp", "pc"];
const MNEMONIC_STRINGS: [&str; 42] = [
    "--", "adc", "add", "and", "bit", "call", "ccf", "cp", "cpl", "daa", "dec", "di", "ei", "halt", "inc", "jp",
    "jr", "ld", "ldh", "nop", "or", "pop", "push", "res", "ret", "reti", "rl", "rlc", "rr", "rrc", "rst", "sbc",
    "scf", "set", "sla", "sra", "srl", "stop", "sub", "swap", "xor", "ill",
];

/// `_decodeOperand`
fn decode_operand(op: Operand, pc: u16, out: &mut String) -> usize {
    if op.flags & OP_FLAG_IMPLICIT != 0 {
        return 0;
    }
    let start = out.len();
    out.push(' ');
    if op.flags & OP_FLAG_MEMORY != 0 {
        out.push('[');
    }
    if op.reg != 0 {
        out.push_str(REGISTERS[op.reg as usize]);
    } else if op.flags & OP_FLAG_RELATIVE != 0 {
        out.push_str(&format!("${:04X}", pc.wrapping_add(op.immediate as u8 as i8 as u16)));
    } else {
        out.push_str(&format!("${:02X}", op.immediate));
    }
    if op.flags & OP_FLAG_INCREMENT != 0 {
        out.push('+');
    }
    if op.flags & OP_FLAG_DECREMENT != 0 {
        out.push('-');
    }
    if op.flags & OP_FLAG_MEMORY != 0 {
        out.push(']');
    }
    out.len() - start
}

/// `SM83Disassemble`. `pc` is the address after the instruction.
pub fn disassemble(info: &InstructionInfo, pc: u16) -> String {
    let mut out = String::from(MNEMONIC_STRINGS[info.mnemonic as usize]);
    let has_operands = info.op1.reg != 0 || info.op1.immediate != 0 || info.op2.reg != 0 || info.op2.immediate != 0;
    if let Some(cond) = CONDITIONS[info.condition as usize] {
        out.push(' ');
        out.push_str(cond);
        if has_operands {
            out.push(',');
        }
    }
    let mut written = 0;
    if has_operands {
        written = decode_operand(info.op1, pc, &mut out);
    }
    if info.op2.reg != 0 || (info.op1.immediate == 0 && info.opcode_size > 1 && info.opcode[0] != 0xCB) {
        if written != 0 {
            out.push(',');
        }
        decode_operand(info.op2, pc, &mut out);
    }
    out
}

const WIDTHS: [u8; 256] = [
    1, 3, 1, 1, 1, 1, 2, 1, 3, 1, 1, 1, 1, 1, 2, 1, //
    2, 3, 1, 1, 1, 1, 2, 1, 2, 1, 1, 1, 1, 1, 2, 1, //
    2, 3, 1, 1, 1, 1, 2, 1, 2, 1, 1, 1, 1, 1, 2, 1, //
    2, 3, 1, 1, 1, 1, 2, 1, 2, 1, 1, 1, 1, 1, 2, 1, //
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, //
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, //
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, //
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, //
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, //
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, //
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, //
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, //
    1, 1, 3, 3, 3, 1, 2, 1, 1, 1, 3, 2, 3, 3, 2, 1, //
    1, 1, 3, 0, 3, 1, 2, 1, 1, 1, 3, 0, 3, 0, 2, 1, //
    2, 1, 1, 0, 0, 1, 2, 1, 2, 1, 3, 0, 0, 0, 2, 1, //
    2, 1, 1, 1, 0, 1, 2, 1, 2, 1, 3, 1, 0, 0, 2, 1, //
];

/// `SM83InstructionLength`
pub fn instruction_length(opcode: u8) -> u8 {
    WIDTHS[opcode as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dis(bytes: &[u8]) -> String {
        let (info, len) = decode(|a| bytes.get(a as usize).copied().unwrap_or(0), 0);
        disassemble(&info, 0x100 + len)
    }

    #[test]
    fn disassembles_like_mgba() {
        assert_eq!(dis(&[0x00]), "nop");
        assert_eq!(dis(&[0x3E, 0x42]), "ld a, $42");
        assert_eq!(dis(&[0x21, 0x34, 0x12]), "ld hl, $1234");
        assert_eq!(dis(&[0xE0, 0x40]), "ldh [$FF40], a");
        assert_eq!(dis(&[0xF0, 0x44]), "ldh a, [$FF44]");
        assert_eq!(dis(&[0x22]), "ld [hl+], a");
        assert_eq!(dis(&[0x7E]), "ld a, [hl]");
        assert_eq!(dis(&[0xC3, 0x50, 0x01]), "jp $150");
        assert_eq!(dis(&[0x20, 0xFE]), "jr nz, $0100");
        assert_eq!(dis(&[0xCB, 0x7F]), "bit $07, a");
        assert_eq!(dis(&[0xCB, 0x37]), "swap a");
        assert_eq!(dis(&[0xFE, 0x10]), "cp $10");
        assert_eq!(dis(&[0xEF]), "rst $28");
        assert_eq!(dis(&[0xC9]), "ret");
        assert_eq!(dis(&[0xD8]), "ret c");
        assert_eq!(dis(&[0x76]), "halt");
        assert_eq!(dis(&[0xEA, 0x00, 0xC0]), "ld [$C000], a");
    }
}
