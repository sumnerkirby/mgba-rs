// Copyright (c) 2013-2014 Jeffrey Pfau
// Rust port of mGBA's ARM/Thumb instruction decoder and disassembler
// (src/arm/decoder.c, decoder-arm.c, decoder-thumb.c,
// include/mgba/internal/arm/decoder.h).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! mGBA picks a decoder function from a 4096-entry (ARM) or 1024-entry
//! (Thumb) table built by its emitter macros; here the same choice is made
//! from the opcode's bits, using the interpreter's instruction classes for ARM.

use super::isa_arm::{class_of, Op};
use super::{ARM_LR, ARM_PC, ARM_SP};

pub const OPERAND_NONE: u32 = 0x0000_0000;
pub const OPERAND_REGISTER_1: u32 = 0x0000_0001;
pub const OPERAND_IMMEDIATE_1: u32 = 0x0000_0002;
pub const OPERAND_MEMORY_1: u32 = 0x0000_0004;
pub const OPERAND_AFFECTED_1: u32 = 0x0000_0008;
pub const OPERAND_SHIFT_REGISTER_1: u32 = 0x0000_0010;
pub const OPERAND_SHIFT_IMMEDIATE_1: u32 = 0x0000_0020;
pub const OPERAND_1: u32 = 0x0000_00FF;
pub const OPERAND_REGISTER_2: u32 = 0x0000_0100;
pub const OPERAND_IMMEDIATE_2: u32 = 0x0000_0200;
pub const OPERAND_MEMORY_2: u32 = 0x0000_0400;
pub const OPERAND_AFFECTED_2: u32 = 0x0000_0800;
pub const OPERAND_SHIFT_REGISTER_2: u32 = 0x0000_1000;
pub const OPERAND_SHIFT_IMMEDIATE_2: u32 = 0x0000_2000;
pub const OPERAND_2: u32 = 0x0000_FF00;
pub const OPERAND_REGISTER_3: u32 = 0x0001_0000;
pub const OPERAND_IMMEDIATE_3: u32 = 0x0002_0000;
pub const OPERAND_MEMORY_3: u32 = 0x0004_0000;
pub const OPERAND_AFFECTED_3: u32 = 0x0008_0000;
pub const OPERAND_SHIFT_REGISTER_3: u32 = 0x0010_0000;
pub const OPERAND_SHIFT_IMMEDIATE_3: u32 = 0x0020_0000;
pub const OPERAND_3: u32 = 0x00FF_0000;
pub const OPERAND_REGISTER_4: u32 = 0x0100_0000;
pub const OPERAND_IMMEDIATE_4: u32 = 0x0200_0000;
pub const OPERAND_MEMORY_4: u32 = 0x0400_0000;
pub const OPERAND_SHIFT_REGISTER_4: u32 = 0x1000_0000;
pub const OPERAND_SHIFT_IMMEDIATE_4: u32 = 0x2000_0000;
pub const OPERAND_4: u32 = 0xFF00_0000;
pub const OPERAND_MEMORY: u32 = OPERAND_MEMORY_1 | OPERAND_MEMORY_2 | OPERAND_MEMORY_3 | OPERAND_MEMORY_4;

pub const MEMORY_REGISTER_BASE: u16 = 0x0001;
pub const MEMORY_IMMEDIATE_OFFSET: u16 = 0x0002;
pub const MEMORY_REGISTER_OFFSET: u16 = 0x0004;
pub const MEMORY_SHIFTED_OFFSET: u16 = 0x0008;
pub const MEMORY_PRE_INCREMENT: u16 = 0x0010;
pub const MEMORY_POST_INCREMENT: u16 = 0x0020;
pub const MEMORY_OFFSET_SUBTRACT: u16 = 0x0040;
pub const MEMORY_WRITEBACK: u16 = 0x0080;
pub const MEMORY_DECREMENT_AFTER: u16 = 0x0000;
pub const MEMORY_INCREMENT_AFTER: u16 = 0x0100;
pub const MEMORY_DECREMENT_BEFORE: u16 = 0x0200;
pub const MEMORY_INCREMENT_BEFORE: u16 = 0x0300;
pub const MEMORY_SPSR_SWAP: u16 = 0x0400;
pub const MEMORY_STORE: u16 = 0x1000;
pub const MEMORY_LOAD: u16 = 0x2000;
pub const MEMORY_SWAP: u16 = 0x3000;

pub const PSR_C: u8 = 1;
pub const PSR_X: u8 = 2;
pub const PSR_S: u8 = 4;
pub const PSR_F: u8 = 8;
pub const PSR_MASK: u8 = 0xF;

pub const CONDITION_AL: u8 = 0xE;
pub const CONDITION_NV: u8 = 0xF;

pub const SHIFT_NONE: u8 = 0;
pub const SHIFT_LSL: u8 = 1;
pub const SHIFT_LSR: u8 = 2;
pub const SHIFT_ASR: u8 = 3;
pub const SHIFT_ROR: u8 = 4;
pub const SHIFT_RRX: u8 = 5;

pub const ACCESS_WORD: u8 = 4;
pub const ACCESS_HALFWORD: u8 = 2;
pub const ACCESS_SIGNED_HALFWORD: u8 = 10;
pub const ACCESS_BYTE: u8 = 1;
pub const ACCESS_SIGNED_BYTE: u8 = 9;
pub const ACCESS_TRANSLATED_WORD: u8 = 20;
pub const ACCESS_TRANSLATED_BYTE: u8 = 17;

pub const BRANCH_NONE: u8 = 0;
pub const BRANCH: u8 = 1;
pub const BRANCH_INDIRECT: u8 = 2;
pub const BRANCH_LINKED: u8 = 4;

/// `ARM_CPSR` / `ARM_SPSR` pseudo-registers in operands.
pub const ARM_CPSR: u8 = 16;
pub const ARM_SPSR: u8 = 17;

/// `enum ARMMnemonic`
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum Mnemonic {
    #[default]
    Ill = 0,
    Adc,
    Add,
    And,
    Asr,
    B,
    Bic,
    Bkpt,
    Bl,
    Bx,
    Cmn,
    Cmp,
    Eor,
    Ldm,
    Ldr,
    Lsl,
    Lsr,
    Mla,
    Mov,
    Mrs,
    Msr,
    Mul,
    Mvn,
    Neg,
    Orr,
    Ror,
    Rsb,
    Rsc,
    Sbc,
    Smlal,
    Smull,
    Stm,
    Str,
    Sub,
    Swi,
    Swp,
    Teq,
    Tst,
    Umlal,
    Umull,
}

/// `union ARMOperand`. mGBA overlays `shifterReg`, `shifterImm` and
/// `psrBits` in one byte (`shifter` here) and the whole struct with
/// `immediate`; decoders only ever use one view of an operand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Operand {
    pub reg: u8,
    pub shifter_op: u8,
    pub shifter: u8,
    pub immediate: i32,
}

/// `struct ARMMemoryAccess`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryAccess {
    pub base_reg: u8,
    pub width: u8,
    pub format: u16,
    pub offset: Operand,
}

/// `struct ARMInstructionInfo`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InstructionInfo {
    pub opcode: u32,
    pub op1: Operand,
    pub op2: Operand,
    pub op3: Operand,
    pub op4: Operand,
    pub memory: MemoryAccess,
    pub operand_format: u32,
    pub thumb: bool,
    pub traps: bool,
    pub affects_cpsr: bool,
    pub branch_type: u8,
    pub condition: u8,
    pub mnemonic: Mnemonic,
    pub i_cycles: u8,
    pub c_cycles: u8,
    pub s_instruction_cycles: u8,
    pub n_instruction_cycles: u8,
    pub s_data_cycles: u16,
    pub n_data_cycles: u16,
}

/// `ARMInstructionIsBranch`
pub fn is_branch(mnemonic: Mnemonic) -> bool {
    matches!(mnemonic, Mnemonic::B | Mnemonic::Bl | Mnemonic::Bx)
}

impl InstructionInfo {
    /// `LOAD_CYCLES`
    fn load_cycles(&mut self) {
        self.i_cycles = 1;
        self.n_data_cycles = 1;
    }

    /// `STORE_CYCLES`
    fn store_cycles(&mut self) {
        self.s_instruction_cycles = 0;
        self.n_instruction_cycles = 1;
        self.n_data_cycles = 1;
    }
}

// ----- decoder-arm.c -----

const ALU_MNEMONICS: [Mnemonic; 16] = [
    Mnemonic::And,
    Mnemonic::Eor,
    Mnemonic::Sub,
    Mnemonic::Rsb,
    Mnemonic::Add,
    Mnemonic::Adc,
    Mnemonic::Sbc,
    Mnemonic::Rsc,
    Mnemonic::Tst,
    Mnemonic::Teq,
    Mnemonic::Cmp,
    Mnemonic::Cmn,
    Mnemonic::Orr,
    Mnemonic::Mov,
    Mnemonic::Bic,
    Mnemonic::Mvn,
];

/// `ADDR_MODE_1_*`: the third operand of a data-processing instruction.
fn addr_mode_1(opcode: u32, info: &mut InstructionInfo, immediate: bool) {
    if immediate {
        let rotate = (opcode & 0x0000_0F00) >> 7;
        let imm = opcode & 0xFF;
        info.op3.immediate = imm.rotate_right(rotate) as i32;
        info.operand_format |= OPERAND_IMMEDIATE_3;
        return;
    }
    let op = [SHIFT_LSL, SHIFT_LSR, SHIFT_ASR, SHIFT_ROR][((opcode >> 5) & 3) as usize];
    info.op3.reg = (opcode & 0xF) as u8;
    info.op3.shifter_op = op;
    info.operand_format |= OPERAND_REGISTER_3;
    if opcode & 0x10 != 0 {
        info.op3.shifter = ((opcode >> 8) & 0xF) as u8;
        info.i_cycles += 1;
        info.operand_format |= OPERAND_SHIFT_REGISTER_3;
    } else {
        info.op3.shifter = ((opcode >> 7) & 0x1F) as u8;
        if info.op3.shifter == 0 && (op == SHIFT_LSR || op == SHIFT_ASR) {
            info.op3.shifter = 32;
        }
        info.operand_format |= OPERAND_SHIFT_IMMEDIATE_3;
    }
    if info.operand_format & OPERAND_SHIFT_IMMEDIATE_3 != 0 && info.op3.shifter == 0 {
        if op == SHIFT_LSL {
            info.operand_format &= !OPERAND_SHIFT_IMMEDIATE_3;
            info.op3.shifter_op = SHIFT_NONE;
        } else if op == SHIFT_ROR {
            info.op3.shifter_op = SHIFT_RRX;
        }
    }
}

/// `DEFINE_ALU_DECODER_EX_ARM`
fn decode_alu(opcode: u32, info: &mut InstructionInfo, immediate: bool) {
    let alu = ((opcode >> 21) & 0xF) as usize;
    info.mnemonic = ALU_MNEMONICS[alu];
    // TST, TEQ, CMP and CMN only exist with S and skip their destination;
    // MOV and MVN skip their first source.
    let (other_affected, skipped) = match alu {
        8..=11 => (OPERAND_NONE, 1),
        13 | 15 => (OPERAND_AFFECTED_1, 2),
        _ => (OPERAND_AFFECTED_1, 0),
    };
    info.op1.reg = ((opcode >> 12) & 0xF) as u8;
    info.op2.reg = ((opcode >> 16) & 0xF) as u8;
    info.operand_format = OPERAND_REGISTER_1 | other_affected | OPERAND_REGISTER_2;
    info.affects_cpsr = opcode & 0x0010_0000 != 0;
    addr_mode_1(opcode, info, immediate);
    if skipped == 1 {
        info.op1 = info.op2;
        info.op2 = info.op3;
        info.operand_format >>= 8;
    } else if skipped == 2 {
        info.op2 = info.op3;
        info.operand_format |= info.operand_format >> 8;
        info.operand_format &= !OPERAND_3;
    }
    if info.op1.reg as usize == ARM_PC && other_affected & OPERAND_AFFECTED_1 != 0 {
        info.branch_type = BRANCH_INDIRECT;
    }
}

/// `DEFINE_MULTIPLY_DECODER_EX_ARM`
fn decode_multiply(opcode: u32, info: &mut InstructionInfo) {
    let accumulate = opcode & 0x0020_0000 != 0;
    info.mnemonic = if accumulate { Mnemonic::Mla } else { Mnemonic::Mul };
    info.op1.reg = ((opcode >> 16) & 0xF) as u8;
    info.op2.reg = (opcode & 0xF) as u8;
    info.op3.reg = ((opcode >> 8) & 0xF) as u8;
    info.op4.reg = ((opcode >> 12) & 0xF) as u8;
    info.operand_format = OPERAND_REGISTER_1
        | OPERAND_AFFECTED_1
        | OPERAND_REGISTER_2
        | OPERAND_REGISTER_3
        | if accumulate { OPERAND_REGISTER_4 } else { OPERAND_NONE };
    info.affects_cpsr = opcode & 0x0010_0000 != 0;
    if info.op1.reg as usize == ARM_PC {
        info.branch_type = BRANCH_INDIRECT;
    }
}

/// `DEFINE_LONG_MULTIPLY_DECODER_EX_ARM`
fn decode_multiply_long(opcode: u32, info: &mut InstructionInfo) {
    info.mnemonic = match (opcode >> 21) & 3 {
        0 => Mnemonic::Umull,
        1 => Mnemonic::Umlal,
        2 => Mnemonic::Smull,
        _ => Mnemonic::Smlal,
    };
    info.op1.reg = ((opcode >> 12) & 0xF) as u8;
    info.op2.reg = ((opcode >> 16) & 0xF) as u8;
    info.op3.reg = (opcode & 0xF) as u8;
    info.op4.reg = ((opcode >> 8) & 0xF) as u8;
    info.operand_format = OPERAND_REGISTER_1
        | OPERAND_AFFECTED_1
        | OPERAND_REGISTER_2
        | OPERAND_AFFECTED_2
        | OPERAND_REGISTER_3
        | OPERAND_REGISTER_4;
    info.affects_cpsr = opcode & 0x0010_0000 != 0;
    if info.op1.reg as usize == ARM_PC {
        info.branch_type = BRANCH_INDIRECT;
    }
}

/// `DEFINE_LOAD_STORE_DECODER_EX_ARM`, shared tail.
fn load_store_common(opcode: u32, info: &mut InstructionInfo, load: bool, width: u8, format: u16) {
    info.mnemonic = if load { Mnemonic::Ldr } else { Mnemonic::Str };
    let other_affected = if load { OPERAND_AFFECTED_1 } else { OPERAND_AFFECTED_2 };
    info.op1.reg = ((opcode >> 12) & 0xF) as u8;
    info.memory.base_reg = ((opcode >> 16) & 0xF) as u8;
    info.memory.width = width;
    info.operand_format = OPERAND_REGISTER_1 | other_affected | OPERAND_MEMORY_2;
    info.memory.format = MEMORY_REGISTER_BASE | format | if load { MEMORY_LOAD } else { MEMORY_STORE };
}

fn load_store_tail(info: &mut InstructionInfo, load: bool) {
    let other_affected = if load { OPERAND_AFFECTED_1 } else { OPERAND_AFFECTED_2 };
    if info.op1.reg as usize == ARM_PC && other_affected & OPERAND_AFFECTED_1 != 0 {
        info.branch_type = BRANCH_INDIRECT;
    }
    if info.memory.format & (MEMORY_WRITEBACK | MEMORY_REGISTER_OFFSET) == (MEMORY_WRITEBACK | MEMORY_REGISTER_OFFSET)
        && info.memory.offset.reg as usize == ARM_PC
    {
        info.branch_type = BRANCH_INDIRECT;
    }
    if load {
        info.load_cycles();
    } else {
        info.store_cycles();
    }
}

/// The P/U/W variants of `DEFINE_LOAD_STORE_DECODER_SET_ARM`.
fn index_format(opcode: u32) -> u16 {
    let pre = opcode & 0x0100_0000 != 0;
    let up = opcode & 0x0080_0000 != 0;
    let writeback = opcode & 0x0020_0000 != 0;
    let mut format = 0;
    if !pre {
        format |= MEMORY_POST_INCREMENT | MEMORY_WRITEBACK;
    } else if writeback {
        format |= MEMORY_PRE_INCREMENT | MEMORY_WRITEBACK;
    }
    if !up {
        format |= MEMORY_OFFSET_SUBTRACT;
    }
    format
}

/// LDR/STR/LDRB/STRB and their translated forms (`ADDR_MODE_2_*`).
fn decode_single_transfer(opcode: u32, info: &mut InstructionInfo) {
    let load = opcode & 0x0010_0000 != 0;
    let byte = opcode & 0x0040_0000 != 0;
    let translated = opcode & 0x0100_0000 == 0 && opcode & 0x0020_0000 != 0;
    let width = match (byte, translated) {
        (false, false) => ACCESS_WORD,
        (true, false) => ACCESS_BYTE,
        (false, true) => ACCESS_TRANSLATED_WORD,
        (true, true) => ACCESS_TRANSLATED_BYTE,
    };
    load_store_common(opcode, info, load, width, index_format(opcode));
    if opcode & 0x0200_0000 == 0 {
        // ADDR_MODE_2_IMM
        info.memory.format |= MEMORY_IMMEDIATE_OFFSET;
        info.memory.offset.immediate = (opcode & 0xFFF) as i32;
    } else {
        let op = [SHIFT_LSL, SHIFT_LSR, SHIFT_ASR, SHIFT_ROR][((opcode >> 5) & 3) as usize];
        info.memory.format |= MEMORY_REGISTER_OFFSET | MEMORY_SHIFTED_OFFSET;
        info.memory.offset.shifter_op = op;
        info.memory.offset.shifter = ((opcode >> 7) & 0x1F) as u8;
        info.memory.offset.reg = (opcode & 0xF) as u8;
        if info.memory.offset.shifter == 0 {
            match op {
                SHIFT_LSL => {
                    info.memory.format &= !MEMORY_SHIFTED_OFFSET;
                    info.memory.offset.shifter_op = SHIFT_NONE;
                }
                SHIFT_LSR | SHIFT_ASR => info.memory.offset.shifter = 32,
                _ => info.memory.offset.shifter_op = SHIFT_RRX,
            }
        }
    }
    load_store_tail(info, load);
}

/// LDRH/STRH/LDRSB/LDRSH (`ADDR_MODE_3_*`).
fn decode_halfword(opcode: u32, info: &mut InstructionInfo) {
    let load = opcode & 0x0010_0000 != 0;
    let width = match (opcode >> 5) & 3 {
        1 => ACCESS_HALFWORD,
        2 => ACCESS_SIGNED_BYTE,
        _ => ACCESS_SIGNED_HALFWORD,
    };
    load_store_common(opcode, info, load, width, index_format(opcode));
    if opcode & 0x0040_0000 != 0 {
        info.memory.format |= MEMORY_IMMEDIATE_OFFSET;
        info.memory.offset.immediate = ((opcode & 0xF) | ((opcode & 0xF00) >> 4)) as i32;
    } else {
        info.memory.format |= MEMORY_REGISTER_OFFSET;
        info.memory.offset.reg = (opcode & 0xF) as u8;
    }
    load_store_tail(info, load);
}

/// `DEFINE_LOAD_STORE_MULTIPLE_DECODER_EX_ARM`
fn decode_block_transfer(opcode: u32, info: &mut InstructionInfo) {
    let load = opcode & 0x0010_0000 != 0;
    info.mnemonic = if load { Mnemonic::Ldm } else { Mnemonic::Stm };
    info.memory.base_reg = ((opcode >> 16) & 0xF) as u8;
    info.op1.immediate = (opcode & 0xFFFF) as i32;
    if info.op1.immediate & (1 << ARM_PC) != 0 {
        info.branch_type = BRANCH_INDIRECT;
    }
    info.operand_format = OPERAND_MEMORY_1;
    let direction = match (opcode >> 23) & 3 {
        0 => MEMORY_DECREMENT_AFTER,
        1 => MEMORY_INCREMENT_AFTER,
        2 => MEMORY_DECREMENT_BEFORE,
        _ => MEMORY_INCREMENT_BEFORE,
    };
    let mut format = MEMORY_REGISTER_BASE | direction | if load { MEMORY_LOAD } else { MEMORY_STORE };
    if opcode & 0x0020_0000 != 0 {
        format |= MEMORY_WRITEBACK;
    }
    if opcode & 0x0040_0000 != 0 {
        format |= MEMORY_SPSR_SWAP;
    }
    info.memory.format = format;
}

/// `ARMDecodeARM`
pub fn decode_arm(opcode: u32) -> InstructionInfo {
    let mut info = InstructionInfo {
        thumb: false,
        opcode,
        branch_type: BRANCH_NONE,
        condition: (opcode >> 28) as u8,
        s_instruction_cycles: 1,
        ..Default::default()
    };
    let index = (((opcode >> 16) & 0xFF0) | ((opcode >> 4) & 0xF)) as usize;
    let i = &mut info;
    match class_of(index) {
        Op::AluReg => decode_alu(opcode, i, false),
        Op::AluImm => decode_alu(opcode, i, true),
        Op::Mul => decode_multiply(opcode, i),
        Op::MulLong => decode_multiply_long(opcode, i),
        Op::Halfword => decode_halfword(opcode, i),
        Op::SingleTransfer => decode_single_transfer(opcode, i),
        Op::BlockTransfer => decode_block_transfer(opcode, i),
        Op::Swp | Op::Swpb => {
            i.mnemonic = Mnemonic::Swp;
            i.memory.base_reg = ((opcode >> 16) & 0xF) as u8;
            i.op1.reg = ((opcode >> 12) & 0xF) as u8;
            i.op2.reg = (opcode & 0xF) as u8;
            i.operand_format =
                OPERAND_REGISTER_1 | OPERAND_AFFECTED_1 | OPERAND_REGISTER_2 | OPERAND_MEMORY_3 | OPERAND_AFFECTED_3;
            i.memory.format = MEMORY_REGISTER_BASE | MEMORY_SWAP;
            i.memory.width = if class_of(index) == Op::Swp { ACCESS_WORD } else { ACCESS_BYTE };
        }
        Op::B | Op::Bl => {
            let offset = (opcode << 8) as i32;
            i.mnemonic = if class_of(index) == Op::B { Mnemonic::B } else { Mnemonic::Bl };
            i.op1.immediate = offset >> 6;
            i.operand_format = OPERAND_IMMEDIATE_1;
            i.branch_type = if class_of(index) == Op::B { BRANCH } else { BRANCH_LINKED };
        }
        Op::Bx => {
            i.mnemonic = Mnemonic::Bx;
            i.op1.reg = (opcode & 0xF) as u8;
            i.operand_format = OPERAND_REGISTER_1;
            i.branch_type = BRANCH_INDIRECT;
        }
        Op::Cdp | Op::Ldc | Op::Stc | Op::Mcr | Op::Mrc => {
            i.mnemonic = Mnemonic::Ill;
            i.operand_format = OPERAND_NONE;
        }
        Op::Bkpt => {
            // Not strictly in ARMv4T, but here for convenience
            i.mnemonic = Mnemonic::Bkpt;
            i.operand_format = OPERAND_NONE;
            i.traps = true;
        }
        Op::Ill => {
            i.mnemonic = Mnemonic::Ill;
            i.operand_format = OPERAND_NONE;
            i.traps = true;
        }
        Op::Msr | Op::Msrr => {
            i.mnemonic = Mnemonic::Msr;
            i.affects_cpsr = class_of(index) == Op::Msr;
            i.op1.reg = if class_of(index) == Op::Msr { ARM_CPSR } else { ARM_SPSR };
            i.op1.shifter = ((opcode >> 16) as u8) & PSR_MASK;
            i.op2.reg = (opcode & 0xF) as u8;
            i.operand_format = OPERAND_REGISTER_1 | OPERAND_AFFECTED_1 | OPERAND_REGISTER_2;
        }
        Op::Mrs | Op::Mrsr => {
            i.mnemonic = Mnemonic::Mrs;
            i.affects_cpsr = class_of(index) == Op::Mrs;
            i.op1.reg = ((opcode >> 12) & 0xF) as u8;
            i.op2.reg = if class_of(index) == Op::Mrs { ARM_CPSR } else { ARM_SPSR };
            i.op2.shifter = 0;
            i.operand_format = OPERAND_REGISTER_1 | OPERAND_AFFECTED_1 | OPERAND_REGISTER_2;
        }
        Op::Msri | Op::Msrri => {
            let rotate = (opcode & 0xF00) >> 7;
            i.mnemonic = Mnemonic::Msr;
            i.affects_cpsr = class_of(index) == Op::Msri;
            i.op1.reg = if class_of(index) == Op::Msri { ARM_CPSR } else { ARM_SPSR };
            i.op1.shifter = ((opcode >> 16) as u8) & PSR_MASK;
            i.op2.immediate = (opcode & 0xFF).rotate_right(rotate) as i32;
            i.operand_format = OPERAND_REGISTER_1 | OPERAND_AFFECTED_1 | OPERAND_IMMEDIATE_2;
        }
        Op::Swi => {
            i.mnemonic = Mnemonic::Swi;
            i.op1.immediate = (opcode & 0xFF_FFFF) as i32;
            i.operand_format = OPERAND_IMMEDIATE_1;
            i.traps = true;
        }
    }
    info
}

// ----- decoder-thumb.c -----

const THUMB_DATA_FORM_5: [(Mnemonic, bool); 16] = [
    (Mnemonic::And, true),
    (Mnemonic::Eor, true),
    (Mnemonic::Lsl, true),
    (Mnemonic::Lsr, true),
    (Mnemonic::Asr, true),
    (Mnemonic::Adc, true),
    (Mnemonic::Sbc, true),
    (Mnemonic::Ror, true),
    (Mnemonic::Tst, false),
    (Mnemonic::Neg, true),
    (Mnemonic::Cmp, false),
    (Mnemonic::Cmn, false),
    (Mnemonic::Orr, true),
    (Mnemonic::Mul, true),
    (Mnemonic::Bic, true),
    (Mnemonic::Mvn, true),
];

/// `DEFINE_IMMEDIATE_5_DECODER_MEM_THUMB`
fn thumb_imm5_mem(opcode: u16, info: &mut InstructionInfo, load: bool, width: u8) {
    let opcode = opcode as u32;
    info.mnemonic = if load { Mnemonic::Ldr } else { Mnemonic::Str };
    info.op1.reg = (opcode & 7) as u8;
    info.memory.base_reg = ((opcode >> 3) & 7) as u8;
    info.memory.offset.immediate = (((opcode >> 6) & 0x1F) * width as u32) as i32;
    info.memory.width = width;
    info.operand_format =
        OPERAND_REGISTER_1 | if load { OPERAND_AFFECTED_1 } else { OPERAND_AFFECTED_2 } | OPERAND_MEMORY_2;
    info.memory.format =
        MEMORY_REGISTER_BASE | MEMORY_IMMEDIATE_OFFSET | if load { MEMORY_LOAD } else { MEMORY_STORE };
    if load {
        info.load_cycles();
    } else {
        info.store_cycles();
    }
}

/// `DEFINE_LOAD_STORE_WITH_REGISTER_THUMB`
fn thumb_reg_mem(opcode: u16, info: &mut InstructionInfo, load: bool, width: u8) {
    let opcode = opcode as u32;
    info.mnemonic = if load { Mnemonic::Ldr } else { Mnemonic::Str };
    info.memory.offset.reg = ((opcode >> 6) & 7) as u8;
    info.op1.reg = (opcode & 7) as u8;
    info.memory.base_reg = ((opcode >> 3) & 7) as u8;
    info.memory.width = width;
    info.operand_format =
        OPERAND_REGISTER_1 | if load { OPERAND_AFFECTED_1 } else { OPERAND_AFFECTED_2 } | OPERAND_MEMORY_2;
    info.memory.format =
        MEMORY_REGISTER_BASE | MEMORY_REGISTER_OFFSET | if load { MEMORY_LOAD } else { MEMORY_STORE };
    if load {
        info.load_cycles();
    } else {
        info.store_cycles();
    }
}

/// `DEFINE_IMMEDIATE_WITH_REGISTER_MEM_THUMB`
fn thumb_word_mem(opcode: u16, info: &mut InstructionInfo, load: bool, base: usize) {
    let opcode = opcode as u32;
    info.mnemonic = if load { Mnemonic::Ldr } else { Mnemonic::Str };
    info.op1.reg = ((opcode >> 8) & 7) as u8;
    info.memory.base_reg = base as u8;
    info.memory.offset.immediate = ((opcode & 0xFF) << 2) as i32;
    info.memory.width = ACCESS_WORD;
    info.operand_format =
        OPERAND_REGISTER_1 | if load { OPERAND_AFFECTED_1 } else { OPERAND_AFFECTED_2 } | OPERAND_MEMORY_2;
    info.memory.format =
        MEMORY_REGISTER_BASE | MEMORY_IMMEDIATE_OFFSET | if load { MEMORY_LOAD } else { MEMORY_STORE };
    if load {
        info.load_cycles();
    } else {
        info.store_cycles();
    }
}

/// `DEFINE_LOAD_STORE_MULTIPLE_EX_THUMB`
fn thumb_multiple(info: &mut InstructionInfo, rn: u8, list: i32, load: bool, direction: u16) {
    info.mnemonic = if load { Mnemonic::Ldm } else { Mnemonic::Stm };
    info.memory.base_reg = rn;
    info.op1.immediate = list;
    if info.op1.immediate & (1 << ARM_PC) != 0 {
        info.branch_type = BRANCH_INDIRECT;
    }
    info.operand_format = OPERAND_MEMORY_1 | OPERAND_AFFECTED_1;
    info.memory.format =
        MEMORY_REGISTER_BASE | MEMORY_WRITEBACK | if load { MEMORY_LOAD } else { MEMORY_STORE } | direction;
}

fn thumb_ill(info: &mut InstructionInfo) {
    info.mnemonic = Mnemonic::Ill;
    info.operand_format = OPERAND_NONE;
    info.traps = true;
}

/// `ARMDecodeThumb`
pub fn decode_thumb(opcode: u16) -> InstructionInfo {
    let mut info = InstructionInfo {
        thumb: true,
        opcode: opcode as u32,
        branch_type: BRANCH_NONE,
        condition: CONDITION_AL,
        s_instruction_cycles: 1,
        ..Default::default()
    };
    let i = &mut info;
    let op = opcode as u32;
    let index = op >> 6;
    match index {
        // DEFINE_IMMEDIATE_5_DECODER_DATA_THUMB: LSL1, LSR1, ASR1
        0x000..=0x05F => {
            i.mnemonic = [Mnemonic::Lsl, Mnemonic::Lsr, Mnemonic::Asr][(index >> 5) as usize];
            i.op3.immediate = ((op >> 6) & 0x1F) as i32;
            i.op1.reg = (op & 7) as u8;
            i.op2.reg = ((op >> 3) & 7) as u8;
            i.affects_cpsr = true;
            i.operand_format = OPERAND_REGISTER_1 | OPERAND_AFFECTED_1 | OPERAND_REGISTER_2 | OPERAND_IMMEDIATE_3;
        }
        // DEFINE_DATA_FORM_1_DECODER_THUMB: ADD3, SUB3; FORM_2: ADD1, SUB1
        0x060..=0x07F => {
            i.mnemonic = if index & 8 == 0 { Mnemonic::Add } else { Mnemonic::Sub };
            i.op1.reg = (op & 7) as u8;
            i.op2.reg = ((op >> 3) & 7) as u8;
            i.affects_cpsr = true;
            if index < 0x070 {
                i.op3.reg = ((op >> 6) & 7) as u8;
                i.operand_format = OPERAND_REGISTER_1 | OPERAND_AFFECTED_1 | OPERAND_REGISTER_2 | OPERAND_REGISTER_3;
            } else {
                i.op3.immediate = ((op >> 6) & 7) as i32;
                i.operand_format = OPERAND_REGISTER_1 | OPERAND_AFFECTED_1 | OPERAND_REGISTER_2 | OPERAND_IMMEDIATE_3;
            }
        }
        // DEFINE_DATA_FORM_3_DECODER_THUMB: MOV1, CMP1, ADD2, SUB2
        0x080..=0x0FF => {
            let (mn, affected) = [
                (Mnemonic::Mov, OPERAND_AFFECTED_1),
                (Mnemonic::Cmp, OPERAND_NONE),
                (Mnemonic::Add, OPERAND_AFFECTED_1),
                (Mnemonic::Sub, OPERAND_AFFECTED_1),
            ][((index >> 5) & 3) as usize];
            i.mnemonic = mn;
            i.op1.reg = ((op >> 8) & 7) as u8;
            i.op2.immediate = (op & 0xFF) as i32;
            i.affects_cpsr = true;
            i.operand_format = OPERAND_REGISTER_1 | affected | OPERAND_IMMEDIATE_2;
        }
        // DEFINE_DATA_FORM_5_DECODER_THUMB
        0x100..=0x10F => {
            let (mn, affected) = THUMB_DATA_FORM_5[(index & 0xF) as usize];
            i.mnemonic = mn;
            i.op1.reg = (op & 7) as u8;
            i.op2.reg = ((op >> 3) & 7) as u8;
            i.affects_cpsr = true;
            i.operand_format =
                OPERAND_REGISTER_1 | if affected { OPERAND_AFFECTED_1 } else { OPERAND_NONE } | OPERAND_REGISTER_2;
        }
        // DEFINE_DECODER_WITH_HIGH_THUMB: ADD4, CMP3, MOV3
        0x110..=0x11B => {
            let (mn, affected, cpsr) = [
                (Mnemonic::Add, OPERAND_AFFECTED_1, false),
                (Mnemonic::Cmp, OPERAND_NONE, true),
                (Mnemonic::Mov, OPERAND_AFFECTED_1, false),
            ][((index - 0x110) >> 2) as usize];
            let h1 = if index & 2 != 0 { 8 } else { 0 };
            let h2 = if index & 1 != 0 { 8 } else { 0 };
            i.mnemonic = mn;
            i.op1.reg = ((op & 7) | h1) as u8;
            i.op2.reg = (((op >> 3) & 7) | h2) as u8;
            if i.op1.reg as usize == ARM_PC {
                i.branch_type = BRANCH_INDIRECT;
            }
            i.affects_cpsr = cpsr;
            i.operand_format = OPERAND_REGISTER_1 | affected | OPERAND_REGISTER_2;
        }
        0x11C | 0x11D => {
            i.mnemonic = Mnemonic::Bx;
            i.op1.reg = ((op >> 3) & 0xF) as u8;
            i.operand_format = OPERAND_REGISTER_1;
            i.branch_type = BRANCH_INDIRECT;
        }
        0x11E | 0x11F => thumb_ill(i),
        0x120..=0x13F => thumb_word_mem(opcode, i, true, ARM_PC),
        0x140..=0x17F => {
            let (load, width) = [
                (false, ACCESS_WORD),
                (false, ACCESS_HALFWORD),
                (false, ACCESS_BYTE),
                (true, ACCESS_SIGNED_BYTE),
                (true, ACCESS_WORD),
                (true, ACCESS_HALFWORD),
                (true, ACCESS_BYTE),
                (true, ACCESS_SIGNED_HALFWORD),
            ][((index >> 3) & 7) as usize];
            thumb_reg_mem(opcode, i, load, width);
        }
        0x180..=0x23F => {
            let (load, width) = [(false, 4), (true, 4), (false, 1), (true, 1), (false, 2), (true, 2)]
                [((index - 0x180) >> 5) as usize];
            thumb_imm5_mem(opcode, i, load, width);
        }
        0x240..=0x25F => thumb_word_mem(opcode, i, false, ARM_SP),
        0x260..=0x27F => thumb_word_mem(opcode, i, true, ARM_SP),
        // DEFINE_IMMEDIATE_WITH_REGISTER_DATA_THUMB: ADD5 (pc), ADD6 (sp)
        0x280..=0x2BF => {
            i.mnemonic = Mnemonic::Add;
            i.op1.reg = ((op >> 8) & 7) as u8;
            i.op2.reg = if index < 0x2A0 { ARM_PC } else { ARM_SP } as u8;
            i.op3.immediate = ((op & 0xFF) << 2) as i32;
            i.operand_format = OPERAND_REGISTER_1 | OPERAND_AFFECTED_1 | OPERAND_REGISTER_2 | OPERAND_IMMEDIATE_3;
        }
        // DEFINE_SP_MODIFY_THUMB: ADD7, SUB4
        0x2C0..=0x2C3 => {
            i.mnemonic = if index < 0x2C2 { Mnemonic::Add } else { Mnemonic::Sub };
            i.op1.reg = ARM_SP as u8;
            i.op2.immediate = ((op & 0x7F) << 2) as i32;
            i.operand_format = OPERAND_REGISTER_1 | OPERAND_AFFECTED_1 | OPERAND_IMMEDIATE_2;
        }
        0x2D0..=0x2D7 => {
            let extra = if index >= 0x2D4 { 1 << ARM_LR } else { 0 };
            thumb_multiple(i, ARM_SP as u8, (op & 0xFF) as i32 | extra, false, MEMORY_DECREMENT_BEFORE);
        }
        0x2F0..=0x2F7 => {
            let extra = if index >= 0x2F4 { 1 << ARM_PC } else { 0 };
            thumb_multiple(i, ARM_SP as u8, (op & 0xFF) as i32 | extra, true, MEMORY_INCREMENT_AFTER);
        }
        0x2F8..=0x2FB => {
            i.mnemonic = Mnemonic::Bkpt;
            i.operand_format = OPERAND_NONE;
            i.traps = true;
        }
        0x300..=0x33F => {
            thumb_multiple(i, ((op >> 8) & 7) as u8, (op & 0xFF) as i32, index >= 0x320, MEMORY_INCREMENT_AFTER);
        }
        // DEFINE_CONDITIONAL_BRANCH_THUMB
        0x340..=0x377 => {
            i.mnemonic = Mnemonic::B;
            i.op1.immediate = ((op as u8 as i8) as i32) << 1;
            i.branch_type = BRANCH;
            i.condition = ((index - 0x340) >> 2) as u8;
            i.operand_format = OPERAND_IMMEDIATE_1;
        }
        0x37C..=0x37F => {
            i.mnemonic = Mnemonic::Swi;
            i.op1.immediate = (op & 0xFF) as i32;
            i.operand_format = OPERAND_IMMEDIATE_1;
            i.traps = true;
        }
        0x380..=0x39F => {
            i.mnemonic = Mnemonic::B;
            let immediate = ((op & 0x7FF) << 5) as u16 as i16;
            i.op1.immediate = (immediate as i32) >> 4;
            i.operand_format = OPERAND_IMMEDIATE_1;
            i.branch_type = BRANCH;
        }
        0x3C0..=0x3DF => {
            i.mnemonic = Mnemonic::Bl;
            let immediate = ((op & 0x7FF) << 5) as u16 as i16;
            i.op1.reg = ARM_LR as u8;
            i.op2.reg = ARM_PC as u8;
            i.op3.immediate = (immediate as i32) << 7;
            i.operand_format =
                OPERAND_REGISTER_1 | OPERAND_AFFECTED_1 | OPERAND_REGISTER_2 | OPERAND_AFFECTED_2 | OPERAND_IMMEDIATE_3;
        }
        0x3E0..=0x3FF => {
            i.mnemonic = Mnemonic::Bl;
            i.op1.reg = ARM_PC as u8;
            i.op2.reg = ARM_LR as u8;
            i.op3.immediate = ((op & 0x7FF) << 1) as i32;
            i.operand_format = OPERAND_REGISTER_1 | OPERAND_AFFECTED_1 | OPERAND_REGISTER_2 | OPERAND_IMMEDIATE_3;
            i.branch_type = BRANCH_LINKED;
        }
        _ => thumb_ill(i),
    }
    info
}

/// `ARMDecodeThumbCombine`: fold a BL prefix/suffix pair into one instruction.
pub fn decode_thumb_combine(info1: &InstructionInfo, info2: &InstructionInfo) -> Option<InstructionInfo> {
    if !info1.thumb || info1.mnemonic != Mnemonic::Bl || !info2.thumb || info2.mnemonic != Mnemonic::Bl {
        return None;
    }
    if info1.op1.reg as usize != ARM_LR || info1.op2.reg as usize != ARM_PC {
        return None;
    }
    if info2.op1.reg as usize != ARM_PC || info2.op2.reg as usize != ARM_LR {
        return None;
    }
    let mut out = *info1;
    out.op1.immediate = info1.op3.immediate | info2.op3.immediate;
    out.operand_format = OPERAND_IMMEDIATE_1;
    out.thumb = true;
    out.mnemonic = Mnemonic::Bl;
    out.branch_type = BRANCH_LINKED;
    out.traps = false;
    out.affects_cpsr = false;
    out.condition = CONDITION_AL;
    out.s_data_cycles = 0;
    out.n_data_cycles = 0;
    out.s_instruction_cycles = 2;
    out.n_instruction_cycles = 0;
    out.i_cycles = 0;
    out.c_cycles = 0;
    Some(out)
}

/// `ARMResolveMemoryAccess`
pub fn resolve_memory_access(info: &InstructionInfo, gprs: &[u32; 16], carry: bool, pc: u32) -> u32 {
    let m = &info.memory;
    let mut address = 0u32;
    if m.format & MEMORY_REGISTER_BASE != 0 {
        if m.base_reg as usize == ARM_PC && m.format & MEMORY_IMMEDIATE_OFFSET != 0 {
            address = pc;
        } else {
            address = gprs[m.base_reg as usize & 0xF];
        }
    }
    if m.format & MEMORY_POST_INCREMENT != 0 {
        return address;
    }
    let mut offset: i32 = 0;
    if m.format & MEMORY_IMMEDIATE_OFFSET != 0 {
        offset = m.offset.immediate;
    } else if m.format & MEMORY_REGISTER_OFFSET != 0 {
        offset = if m.offset.reg as usize == ARM_PC { pc } else { gprs[m.offset.reg as usize & 0xF] } as i32;
    }
    if m.format & MEMORY_SHIFTED_OFFSET != 0 {
        let shift = m.offset.shifter as u32;
        offset = match m.offset.shifter_op {
            SHIFT_LSL => offset.wrapping_shl(shift),
            SHIFT_LSR => (offset as u32).checked_shr(shift).unwrap_or(0) as i32,
            SHIFT_ASR => offset >> shift.min(31),
            SHIFT_ROR => (offset as u32).rotate_right(shift) as i32,
            SHIFT_RRX => (((carry as u32) << 31) | ((offset as u32) >> 1)) as i32,
            _ => offset,
        };
    }
    if m.format & MEMORY_OFFSET_SUBTRACT != 0 {
        address.wrapping_sub(offset as u32)
    } else {
        address.wrapping_add(offset as u32)
    }
}

// ----- decoder.c: ARMDisassemble -----

/// What the disassembler needs to resolve PC-relative loads and labels.
pub trait DisassemblyContext {
    /// Read `width` (1, 2 or 4) bytes at `address` without side effects.
    fn load(&mut self, address: u32, width: u32) -> u32;
    /// `mDebuggerSymbolReverseLookup`
    fn symbol_at(&self, _address: u32) -> Option<String> {
        None
    }
}

const CONDITIONS: [&str; 16] =
    ["eq", "ne", "cs", "cc", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt", "gt", "le", "al", "nv"];

const MNEMONIC_STRINGS: [&str; 41] = [
    "ill", "adc", "add", "and", "asr", "b", "bic", "bkpt", "bl", "bx", "cmn", "cmp", "eor", "ldm", "ldr", "lsl",
    "lsr", "mla", "mov", "mrs", "msr", "mul", "mvn", "neg", "orr", "ror", "rsb", "rsc", "sbc", "smlal", "smull",
    "stm", "str", "sub", "swi", "swp", "teq", "tst", "umlal", "umull", "ill",
];

const DIRECTION_STRINGS: [&str; 4] = ["da", "ia", "db", "ib"];

const ACCESS_TYPE_STRINGS: [&str; 24] =
    ["", "b", "h", "", "", "", "", "", "", "sb", "sh", "", "", "", "", "", "", "bt", "", "", "t", "", "", ""];

/// `_decodeRegister`
fn decode_register(reg: u8, out: &mut String) {
    match reg as usize {
        ARM_SP => out.push_str("sp"),
        ARM_LR => out.push_str("lr"),
        ARM_PC => out.push_str("pc"),
        16 => out.push_str("cpsr"),
        17 => out.push_str("spsr"),
        _ => out.push_str(&format!("r{reg}")),
    }
}

/// `_decodeRegisterList`
fn decode_register_list(mut list: i32, out: &mut String) {
    out.push('{');
    let mut start: i32 = -1;
    let mut end: i32 = -1;
    for i in 0..=ARM_PC as i32 {
        if list & 1 != 0 {
            if start < 0 {
                start = i;
                end = i;
            } else if end + 1 == i {
                end = i;
            } else {
                if end > start {
                    decode_register(start as u8, out);
                    out.push('-');
                }
                decode_register(end as u8, out);
                out.push(',');
                start = i;
                end = i;
            }
        }
        list >>= 1;
    }
    if start >= 0 {
        if end > start {
            decode_register(start as u8, out);
            out.push('-');
        }
        decode_register(end as u8, out);
    }
    out.push('}');
}

/// `_decodePSR`
fn decode_psr(bits: u8, out: &mut String) {
    if bits == 0 {
        return;
    }
    out.push('_');
    if bits & PSR_C != 0 {
        out.push('c');
    }
    if bits & PSR_X != 0 {
        out.push('x');
    }
    if bits & PSR_S != 0 {
        out.push('s');
    }
    if bits & PSR_F != 0 {
        out.push('f');
    }
}

/// `_decodePCRelative`
fn decode_pc_relative(
    address: u32,
    ctx: Option<&dyn DisassemblyContext>,
    pc: u32,
    thumb_branch: bool,
    out: &mut String,
) {
    let address = address.wrapping_add(pc);
    let mut label = None;
    if let Some(ctx) = ctx {
        label = ctx.symbol_at(address);
        if label.is_none() && thumb_branch {
            label = ctx.symbol_at(address | 1);
        }
    }
    match label {
        Some(l) => out.push_str(&l),
        None => out.push_str(&format!("0x{address:08X}")),
    }
}

/// `_decodeShift`
fn decode_shift(op: Operand, reg: bool, out: &mut String) {
    out.push_str(", ");
    match op.shifter_op {
        SHIFT_LSL => out.push_str("lsl "),
        SHIFT_LSR => out.push_str("lsr "),
        SHIFT_ASR => out.push_str("asr "),
        SHIFT_ROR => out.push_str("ror "),
        SHIFT_RRX => {
            out.push_str("rrx");
            return;
        }
        _ => {}
    }
    if !reg {
        out.push_str(&format!("#{}", op.shifter));
    } else {
        decode_register(op.shifter, out);
    }
}

/// `_decodeMemory`
fn decode_memory(memory: MemoryAccess, ctx: &mut Option<&mut dyn DisassemblyContext>, pc: u32, out: &mut String) {
    let mut elide_close = false;
    let mut comment = String::new();
    if memory.format & MEMORY_REGISTER_BASE != 0 {
        if memory.base_reg as usize == ARM_PC && memory.format & MEMORY_IMMEDIATE_OFFSET != 0 {
            let mut addr_base = if memory.format & MEMORY_OFFSET_SUBTRACT != 0 {
                (memory.offset.immediate as u32).wrapping_neg()
            } else {
                memory.offset.immediate as u32
            };
            if ctx.is_none() || memory.format & MEMORY_STORE != 0 {
                out.push('[');
                decode_pc_relative(addr_base, ctx.as_deref().map(|c| c as &dyn DisassemblyContext), pc & 0xFFFF_FFFC, false, out);
            } else {
                let ctx = ctx.as_deref_mut().unwrap();
                decode_pc_relative(addr_base, Some(&*ctx), pc & 0xFFFF_FFFC, false, &mut comment);
                addr_base = addr_base.wrapping_add(pc & 0xFFFF_FFFC); // Thumb does not have PC-relative LDRH/LDRB
                let value = match memory.width & 7 {
                    1 => ctx.load(addr_base, 1),
                    2 => ctx.load(addr_base, 2),
                    _ => ctx.load(addr_base, 4),
                };
                match ctx.symbol_at(value) {
                    Some(label) => out.push_str(&format!("={label}")),
                    None => out.push_str(&format!("=0x{value:08X}")),
                }
                elide_close = true;
            }
        } else {
            out.push('[');
            decode_register(memory.base_reg, out);
            if memory.format & (MEMORY_REGISTER_OFFSET | MEMORY_IMMEDIATE_OFFSET) != 0
                && memory.format & MEMORY_POST_INCREMENT == 0
            {
                out.push_str(", ");
            }
        }
    } else {
        out.push('[');
    }
    if memory.format & MEMORY_POST_INCREMENT != 0 {
        out.push_str("], ");
        elide_close = true;
    }
    if memory.format & MEMORY_IMMEDIATE_OFFSET != 0 && memory.base_reg as usize != ARM_PC {
        if memory.format & MEMORY_OFFSET_SUBTRACT != 0 {
            out.push_str(&format!("#-{}", memory.offset.immediate));
        } else {
            out.push_str(&format!("#{}", memory.offset.immediate));
        }
    } else if memory.format & MEMORY_REGISTER_OFFSET != 0 {
        if memory.format & MEMORY_OFFSET_SUBTRACT != 0 {
            out.push('-');
        }
        decode_register(memory.offset.reg, out);
    }
    if memory.format & MEMORY_SHIFTED_OFFSET != 0 {
        decode_shift(memory.offset, false, out);
    }
    if !elide_close {
        out.push(']');
    }
    if memory.format & (MEMORY_PRE_INCREMENT | MEMORY_WRITEBACK) == (MEMORY_PRE_INCREMENT | MEMORY_WRITEBACK) {
        out.push('!');
    }
    if !comment.is_empty() {
        out.push_str(&format!("  @ {comment}"));
    }
}

/// `ARMDisassemble`. `pc` is the value the instruction sees as PC (its
/// address plus 8 in ARM mode or 4 in Thumb mode).
pub fn disassemble(info: &InstructionInfo, mut ctx: Option<&mut dyn DisassemblyContext>, pc: u32) -> String {
    let mut out = String::new();
    let mut mnemonic = MNEMONIC_STRINGS[info.mnemonic as usize];
    let mut skip3 = false;
    let cond = if info.condition != CONDITION_AL && info.condition < CONDITION_NV {
        CONDITIONS[info.condition as usize]
    } else {
        ""
    };
    let mut flags = "";
    match info.mnemonic {
        Mnemonic::Ldm | Mnemonic::Stm => flags = DIRECTION_STRINGS[((info.memory.format >> 8) & 3) as usize],
        Mnemonic::Ldr | Mnemonic::Str | Mnemonic::Swp => {
            flags = ACCESS_TYPE_STRINGS.get(info.memory.width as usize).copied().unwrap_or("")
        }
        Mnemonic::Add
        | Mnemonic::Adc
        | Mnemonic::And
        | Mnemonic::Asr
        | Mnemonic::Bic
        | Mnemonic::Eor
        | Mnemonic::Lsl
        | Mnemonic::Lsr
        | Mnemonic::Mla
        | Mnemonic::Mul
        | Mnemonic::Mov
        | Mnemonic::Mvn
        | Mnemonic::Orr
        | Mnemonic::Ror
        | Mnemonic::Rsb
        | Mnemonic::Rsc
        | Mnemonic::Sbc
        | Mnemonic::Smlal
        | Mnemonic::Smull
        | Mnemonic::Sub
        | Mnemonic::Umlal
        | Mnemonic::Umull => {
            if info.mnemonic == Mnemonic::Add
                && info.operand_format & (OPERAND_3 | OPERAND_4) == OPERAND_IMMEDIATE_3
                && info.op3.immediate == 0
                && info.thumb
            {
                skip3 = true;
                mnemonic = "mov";
            }
            if info.affects_cpsr && !info.thumb {
                flags = "s";
            }
        }
        _ => {}
    }
    out.push_str(&format!("{mnemonic}{cond}{flags} "));
    let f = info.operand_format;
    match info.mnemonic {
        Mnemonic::Ldm | Mnemonic::Stm => {
            decode_register(info.memory.base_reg, &mut out);
            if info.memory.format & MEMORY_WRITEBACK != 0 {
                out.push('!');
            }
            out.push_str(", ");
            decode_register_list(info.op1.immediate, &mut out);
            if info.memory.format & MEMORY_SPSR_SWAP != 0 {
                out.push('^');
            }
        }
        Mnemonic::B | Mnemonic::Bl => {
            if f & OPERAND_IMMEDIATE_1 != 0 {
                decode_pc_relative(info.op1.immediate as u32, ctx.as_deref().map(|c| c as &dyn DisassemblyContext), pc, true, &mut out);
            }
        }
        _ => {
            let ops = [info.op1, info.op2, info.op3, info.op4];
            for (n, op) in ops.iter().enumerate() {
                let shift = n * 8;
                let bits = (f >> shift) & 0xFF;
                if n == 2 && skip3 {
                    continue;
                }
                if n > 0 && bits != 0 {
                    out.push_str(", ");
                }
                if bits & OPERAND_IMMEDIATE_1 != 0 {
                    out.push_str(&format!("#{}", op.immediate));
                } else if bits & OPERAND_MEMORY_1 != 0 {
                    decode_memory(info.memory, &mut ctx, pc, &mut out);
                } else if bits & OPERAND_REGISTER_1 != 0 {
                    decode_register(op.reg, &mut out);
                    if n == 0 && op.reg as usize > ARM_PC {
                        decode_psr(op.shifter, &mut out);
                    }
                }
                if bits & OPERAND_SHIFT_REGISTER_1 != 0 {
                    decode_shift(*op, true, &mut out);
                } else if bits & OPERAND_SHIFT_IMMEDIATE_1 != 0 {
                    decode_shift(*op, false, &mut out);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm(op: u32) -> String {
        disassemble(&decode_arm(op), None, 0x0800_0008)
    }
    fn thumb(op: u16) -> String {
        disassemble(&decode_thumb(op), None, 0x0800_0004)
    }

    #[test]
    fn disassembles_like_mgba() {
        assert_eq!(arm(0xE3A0_0001), "mov r0, #1");
        assert_eq!(arm(0xE092_1003), "adds r1, r2, r3");
        assert_eq!(arm(0xE1A0_1102), "mov r1, r2, lsl #2");
        assert_eq!(arm(0xE591_0004), "ldr r0, [r1, #4]");
        assert_eq!(arm(0xE5B1_0004), "ldr r0, [r1, #4]!");
        assert_eq!(arm(0xE491_0004), "ldr r0, [r1], #4");
        assert_eq!(arm(0xE1D1_00B2), "ldrh r0, [r1, #2]");
        assert_eq!(arm(0xE92D_4010), "stmdb sp!, {r4,lr}");
        assert_eq!(arm(0xE8BD_800F), "ldmia sp!, {r0-r3,pc}");
        assert_eq!(arm(0xEAFF_FFFE), "b 0x08000000");
        assert_eq!(arm(0x0A00_0000), "beq 0x08000008");
        assert_eq!(arm(0xE12F_FF1E), "bx lr");
        assert_eq!(arm(0xE10F_0000), "mrs r0, cpsr");
        assert_eq!(arm(0xE129_F000), "msr cpsr_cf, r0");
        assert_eq!(arm(0xE000_0291), "mul r0, r1, r2");
        assert_eq!(arm(0xEF00_0005), "swi #5");
        assert_eq!(thumb(0x2001), "mov r0, #1");
        assert_eq!(thumb(0x1C08), "mov r0, r1");
        assert_eq!(thumb(0x6848), "ldr r0, [r1, #4]");
        assert_eq!(thumb(0xB510), "stmdb sp!, {r4,lr}");
        assert_eq!(thumb(0xBD10), "ldmia sp!, {r4,pc}");
        assert_eq!(thumb(0xD0FE), "beq 0x08000000");
        assert_eq!(thumb(0x4770), "bx lr");
        assert_eq!(thumb(0xDF05), "swi #5");
    }

    #[test]
    fn combines_thumb_bl() {
        let a = decode_thumb(0xF000);
        let b = decode_thumb(0xF802);
        let bl = decode_thumb_combine(&a, &b).unwrap();
        assert_eq!(disassemble(&bl, None, 0x0800_0004), "bl 0x08000008");
    }
}
