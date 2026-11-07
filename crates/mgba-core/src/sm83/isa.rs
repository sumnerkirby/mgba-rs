// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's SM83 instruction set (src/sm83/isa-sm83.c,
// include/mgba/internal/sm83/emitter-sm83.h).
//
// mGBA keeps the next micro-op as a function pointer in `cpu->instruction`.
// Here it is the `Instr` enum (so save states can hold it); `Instr::Op(n)` is
// entry `n` of `_sm83InstructionTable` and `Instr::Cb(n)` entry `n` of the CB
// table, and the other variants are the continuation steps.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

/// Register operand index in opcode order: B, C, D, E, H, L, (HL), A.
pub const REG_B: u8 = 0;
pub const REG_C: u8 = 1;
pub const REG_D: u8 = 2;
pub const REG_E: u8 = 3;
pub const REG_H: u8 = 4;
pub const REG_L: u8 = 5;
pub const REG_HL: u8 = 6;
pub const REG_A: u8 = 7;

/// ALU operation in opcode order (0x80 + 8 * op).
pub const ALU_ADD: u8 = 0;
pub const ALU_ADC: u8 = 1;
pub const ALU_SUB: u8 = 2;
pub const ALU_SBC: u8 = 3;
pub const ALU_AND: u8 = 4;
pub const ALU_XOR: u8 = 5;
pub const ALU_OR: u8 = 6;
pub const ALU_CP: u8 = 7;

/// 16-bit register pair: BC, DE, HL, and SP or AF depending on the instruction.
pub const PAIR_BC: u8 = 0;
pub const PAIR_DE: u8 = 1;
pub const PAIR_HL: u8 = 2;
pub const PAIR_SP_AF: u8 = 3;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Instr {
    /// `_sm83InstructionTable[n]`
    Op(u8),
    /// `_sm83CBInstructionTable[n]`
    Cb(u8),
    JpFinish,
    JpDelay,
    JrFinish,
    CallUpdateSpl,
    CallUpdateSph,
    CallUpdatePch,
    CallUpdatePcl,
    RetFinish,
    RetUpdateSpl,
    RetUpdateSph,
    LdHlSpDelay,
    /// `LDx_Bus`: load the bus into a register (`REG_HL` is `LDHL_Bus`, a store to (HL)).
    LdRBus(u8),
    /// `ANDBus`, `ADDBus`, ...
    AluBus(u8),
    AddSpFinish,
    AddSpDelay,
    LdBcDelay,
    LdDeDelay,
    LdHlDelay,
    LdSpFinish,
    LdSpDelay,
    LdIaFinish,
    LdIaDelay,
    LdAiFinish,
    LdAiDelay,
    LdAioDelay,
    LdIoaDelay,
    LdIspStoreH,
    LdIspStoreL,
    LdIspReadAddr,
    AddHlFinish(u8),
    IncHlDelay,
    DecHlDelay,
    PopDelay(u8),
    PushFinish,
    PushDelay(u8),
    /// `xxxHLDelay` of a CB-prefixed instruction operating on (HL)
    CbHlDelay(u8),
    RstUpdateSpl(u8),
    RstUpdateSph(u8),
    Stop2,
    CbDelegate,
    IrqStall,
    IrqFinish,
    IrqDelay,
    Irq,
}

#[inline(always)]
fn read_reg(cpu: &Sm83Core, r: u8) -> u8 {
    match r {
        REG_B => cpu.b,
        REG_C => cpu.c,
        REG_D => cpu.d,
        REG_E => cpu.e,
        REG_H => cpu.h,
        REG_L => cpu.l,
        REG_A => cpu.a,
        _ => unreachable!(),
    }
}

#[inline(always)]
fn write_reg(cpu: &mut Sm83Core, r: u8, v: u8) {
    match r {
        REG_B => cpu.b = v,
        REG_C => cpu.c = v,
        REG_D => cpu.d = v,
        REG_E => cpu.e = v,
        REG_H => cpu.h = v,
        REG_L => cpu.l = v,
        REG_A => cpu.a = v,
        _ => unreachable!(),
    }
}

#[inline(always)]
fn read_pair(cpu: &Sm83Core, p: u8) -> u16 {
    match p {
        PAIR_BC => cpu.bc(),
        PAIR_DE => cpu.de(),
        PAIR_HL => cpu.hl(),
        _ => cpu.sp,
    }
}

#[inline(always)]
fn write_pair(cpu: &mut Sm83Core, p: u8, v: u16) {
    match p {
        PAIR_BC => cpu.set_bc(v),
        PAIR_DE => cpu.set_de(v),
        PAIR_HL => cpu.set_hl(v),
        _ => cpu.sp = v,
    }
}

/// The ALU macros (`DEFINE_ADD_INSTRUCTION_SM83` etc.).
#[inline(always)]
fn alu(cpu: &mut Sm83Core, op: u8, operand: u8) {
    let a = cpu.a as i32;
    let o = operand as i32;
    match op {
        ALU_ADD => {
            let diff = a + o;
            cpu.set_fn(false);
            cpu.set_fh((a & 0xF) + (o & 0xF) >= 0x10);
            cpu.set_fc(diff >= 0x100);
            cpu.a = diff as u8;
            cpu.set_fz(cpu.a == 0);
        }
        ALU_ADC => {
            let c = cpu.fc() as i32;
            let diff = a + o + c;
            cpu.set_fn(false);
            cpu.set_fh((a & 0xF) + (o & 0xF) + c >= 0x10);
            cpu.set_fc(diff >= 0x100);
            cpu.a = diff as u8;
            cpu.set_fz(cpu.a == 0);
        }
        ALU_SUB => {
            let diff = a - o;
            cpu.set_fn(true);
            cpu.set_fh((a & 0xF) - (o & 0xF) < 0);
            cpu.set_fc(diff < 0);
            cpu.a = diff as u8;
            cpu.set_fz(cpu.a == 0);
        }
        ALU_SBC => {
            let c = cpu.fc() as i32;
            let diff = a - o - c;
            cpu.set_fn(true);
            cpu.set_fh((a & 0xF) - (o & 0xF) - c < 0);
            cpu.set_fc(diff < 0);
            cpu.a = diff as u8;
            cpu.set_fz(cpu.a == 0);
        }
        ALU_AND => {
            cpu.a &= operand;
            cpu.set_fz(cpu.a == 0);
            cpu.set_fn(false);
            cpu.set_fc(false);
            cpu.set_fh(true);
        }
        ALU_XOR => {
            cpu.a ^= operand;
            cpu.set_fz(cpu.a == 0);
            cpu.set_fn(false);
            cpu.set_fc(false);
            cpu.set_fh(false);
        }
        ALU_OR => {
            cpu.a |= operand;
            cpu.set_fz(cpu.a == 0);
            cpu.set_fn(false);
            cpu.set_fc(false);
            cpu.set_fh(false);
        }
        _ => {
            let diff = a - o;
            cpu.set_fn(true);
            cpu.set_fz(diff & 0xFF == 0);
            cpu.set_fh((a & 0xF) - (o & 0xF) < 0);
            cpu.set_fc(diff < 0);
        }
    }
}

/// The body of a CB-prefixed instruction applied to `reg`. Returns the new value.
#[inline(always)]
fn cb_body(cpu: &mut Sm83Core, op: u8, mut reg: u8) -> u8 {
    let group = op >> 3;
    if group >= 8 {
        let bit = 1u8 << (group & 7);
        match group >> 3 {
            1 => {
                cpu.set_fn(false);
                cpu.set_fh(true);
                cpu.set_fz(reg & bit == 0);
            }
            2 => reg &= !bit,
            _ => reg |= bit,
        }
        return reg;
    }
    match group {
        0 => {
            // RLC
            reg = (reg << 1) | (reg >> 7);
            cpu.set_fc(reg & 1 != 0);
        }
        1 => {
            // RRC
            let low = reg & 1;
            reg = (reg >> 1) | (low << 7);
            cpu.set_fc(low != 0);
        }
        2 => {
            // RL
            let wide = ((reg as u32) << 1) | cpu.fc() as u32;
            reg = wide as u8;
            cpu.set_fc(wide >> 8 != 0);
        }
        3 => {
            // RR
            let low = reg & 1;
            reg = (reg >> 1) | ((cpu.fc() as u8) << 7);
            cpu.set_fc(low != 0);
        }
        4 => {
            // SLA
            cpu.set_fc(reg >> 7 != 0);
            reg <<= 1;
        }
        5 => {
            // SRA
            cpu.set_fc(reg & 1 != 0);
            reg = ((reg as i8) >> 1) as u8;
        }
        6 => {
            // SWAP
            reg = reg.rotate_left(4);
            cpu.set_fc(false);
        }
        _ => {
            // SRL
            cpu.set_fc(reg & 1 != 0);
            reg >>= 1;
        }
    }
    cpu.set_fn(false);
    cpu.set_fh(false);
    cpu.set_fz(reg == 0);
    reg
}

#[inline(always)]
fn condition(cpu: &Sm83Core, cc: u8) -> bool {
    match cc {
        0 => !cpu.fz(),
        1 => cpu.fz(),
        2 => !cpu.fc(),
        _ => cpu.fc(),
    }
}

/// `_SM83InstructionRETUpdateSPH`
#[inline(always)]
fn ret_update_sph(cpu: &mut Sm83Core) {
    if cpu.condition {
        cpu.index = cpu.sp;
        cpu.execution_state = CORE_MEMORY_LOAD;
        cpu.instruction = Instr::RetUpdateSpl;
    }
}

/// Execute one micro-op (`cpu->instruction(cpu)`).
pub fn execute<B: Sm83Bus>(bus: &mut B, instr: Instr) {
    match instr {
        Instr::Op(op) => execute_op(bus, op),
        Instr::Cb(op) => execute_cb(bus, op),
        Instr::CbDelegate => {
            let op = bus.cpu().bus;
            execute_cb(bus, op);
        }
        Instr::JpFinish => {
            let cpu = bus.cpu();
            if cpu.condition {
                cpu.pc = ((cpu.bus as u16) << 8) | cpu.index;
                let pc = cpu.pc;
                bus.set_active_region(pc);
                bus.cpu().execution_state = CORE_STALL;
            }
        }
        Instr::JpDelay => {
            let cpu = bus.cpu();
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::JpFinish;
            cpu.index = cpu.bus as u16;
        }
        Instr::JrFinish => {
            let cpu = bus.cpu();
            if cpu.condition {
                cpu.pc = cpu.pc.wrapping_add(cpu.bus as i8 as u16);
                let pc = cpu.pc;
                bus.set_active_region(pc);
                bus.cpu().execution_state = CORE_STALL;
            }
        }
        Instr::CallUpdateSpl => {
            let cpu = bus.cpu();
            cpu.index = cpu.index.wrapping_sub(1);
            cpu.bus = cpu.sp as u8;
            cpu.sp = cpu.index;
            cpu.execution_state = CORE_MEMORY_STORE;
            cpu.instruction = Instr::Op(0);
        }
        Instr::CallUpdateSph => {
            let cpu = bus.cpu();
            cpu.execution_state = CORE_MEMORY_STORE;
            cpu.instruction = Instr::CallUpdateSpl;
        }
        Instr::CallUpdatePch => {
            let cpu = bus.cpu();
            if cpu.condition {
                let new_pc = ((cpu.bus as u16) << 8) | cpu.index;
                cpu.bus = (cpu.pc >> 8) as u8;
                cpu.index = cpu.sp.wrapping_sub(1);
                cpu.sp = cpu.pc; // GROSS
                cpu.pc = new_pc;
                bus.set_active_region(new_pc);
                let cpu = bus.cpu();
                cpu.execution_state = CORE_OP2;
                cpu.instruction = Instr::CallUpdateSph;
            }
        }
        Instr::CallUpdatePcl => {
            let cpu = bus.cpu();
            cpu.execution_state = CORE_READ_PC;
            cpu.index = cpu.bus as u16;
            cpu.instruction = Instr::CallUpdatePch;
        }
        Instr::RetFinish => {
            let cpu = bus.cpu();
            cpu.sp = cpu.sp.wrapping_add(2);
            cpu.pc |= (cpu.bus as u16) << 8;
            let pc = cpu.pc;
            bus.set_active_region(pc);
            bus.cpu().execution_state = CORE_STALL;
        }
        Instr::RetUpdateSpl => {
            let cpu = bus.cpu();
            cpu.index = cpu.sp.wrapping_add(1);
            cpu.pc = cpu.bus as u16;
            cpu.execution_state = CORE_MEMORY_LOAD;
            cpu.instruction = Instr::RetFinish;
        }
        Instr::RetUpdateSph => ret_update_sph(bus.cpu()),
        Instr::LdHlSpDelay => {
            let cpu = bus.cpu();
            let diff = cpu.bus as i8 as i32;
            let sum = cpu.sp as i32 + diff;
            cpu.set_hl(sum as u16);
            cpu.execution_state = CORE_STALL;
            cpu.set_fz(false);
            cpu.set_fn(false);
            cpu.set_fc((diff & 0xFF) + (cpu.sp as i32 & 0xFF) >= 0x100);
            cpu.set_fh((diff & 0xF) + (cpu.sp as i32 & 0xF) >= 0x10);
        }
        Instr::LdRBus(r) => {
            let cpu = bus.cpu();
            if r == REG_HL {
                cpu.index = cpu.hl();
                cpu.execution_state = CORE_MEMORY_STORE;
                cpu.instruction = Instr::Op(0);
            } else {
                let v = cpu.bus;
                write_reg(cpu, r, v);
            }
        }
        Instr::AluBus(op) => {
            let cpu = bus.cpu();
            let v = cpu.bus;
            alu(cpu, op, v);
        }
        Instr::AddSpFinish => {
            let cpu = bus.cpu();
            cpu.sp = cpu.index;
            cpu.execution_state = CORE_STALL;
        }
        Instr::AddSpDelay => {
            let cpu = bus.cpu();
            let diff = cpu.bus as i8 as i32;
            let sum = cpu.sp as i32 + diff;
            cpu.index = sum as u16;
            cpu.execution_state = CORE_OP2;
            cpu.instruction = Instr::AddSpFinish;
            cpu.set_fz(false);
            cpu.set_fn(false);
            cpu.set_fc((diff & 0xFF) + (cpu.sp as i32 & 0xFF) >= 0x100);
            cpu.set_fh((diff & 0xF) + (cpu.sp as i32 & 0xF) >= 0x10);
        }
        Instr::LdBcDelay => {
            let cpu = bus.cpu();
            cpu.c = cpu.bus;
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdRBus(REG_B);
        }
        Instr::LdDeDelay => {
            let cpu = bus.cpu();
            cpu.e = cpu.bus;
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdRBus(REG_D);
        }
        Instr::LdHlDelay => {
            let cpu = bus.cpu();
            cpu.l = cpu.bus;
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdRBus(REG_H);
        }
        Instr::LdSpFinish => {
            let cpu = bus.cpu();
            cpu.sp |= (cpu.bus as u16) << 8;
        }
        Instr::LdSpDelay => {
            let cpu = bus.cpu();
            cpu.sp = cpu.bus as u16;
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdSpFinish;
        }
        Instr::LdIaFinish => {
            let cpu = bus.cpu();
            cpu.index |= (cpu.bus as u16) << 8;
            cpu.bus = cpu.a;
            cpu.execution_state = CORE_MEMORY_STORE;
            cpu.instruction = Instr::Op(0);
        }
        Instr::LdIaDelay => {
            let cpu = bus.cpu();
            cpu.index = cpu.bus as u16;
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdIaFinish;
        }
        Instr::LdAiFinish => {
            let cpu = bus.cpu();
            cpu.index |= (cpu.bus as u16) << 8;
            cpu.execution_state = CORE_MEMORY_LOAD;
            cpu.instruction = Instr::LdRBus(REG_A);
        }
        Instr::LdAiDelay => {
            let cpu = bus.cpu();
            cpu.index = cpu.bus as u16;
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdAiFinish;
        }
        Instr::LdAioDelay => {
            let cpu = bus.cpu();
            cpu.index = 0xFF00 | cpu.bus as u16;
            cpu.execution_state = CORE_MEMORY_LOAD;
            cpu.instruction = Instr::LdRBus(REG_A);
        }
        Instr::LdIoaDelay => {
            let cpu = bus.cpu();
            cpu.index = 0xFF00 | cpu.bus as u16;
            cpu.bus = cpu.a;
            cpu.execution_state = CORE_MEMORY_STORE;
            cpu.instruction = Instr::Op(0);
        }
        Instr::LdIspStoreH => {
            let cpu = bus.cpu();
            cpu.index = cpu.index.wrapping_add(1);
            cpu.bus = (cpu.sp >> 8) as u8;
            cpu.execution_state = CORE_MEMORY_STORE;
            cpu.instruction = Instr::Op(0);
        }
        Instr::LdIspStoreL => {
            let cpu = bus.cpu();
            cpu.index |= (cpu.bus as u16) << 8;
            cpu.bus = cpu.sp as u8;
            cpu.execution_state = CORE_MEMORY_STORE;
            cpu.instruction = Instr::LdIspStoreH;
        }
        Instr::LdIspReadAddr => {
            let cpu = bus.cpu();
            cpu.index = cpu.bus as u16;
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdIspStoreL;
        }
        Instr::AddHlFinish(p) => {
            let cpu = bus.cpu();
            let hi = (read_pair(cpu, p) >> 8) as i32;
            let h = cpu.h as i32;
            let c = cpu.fc() as i32;
            let diff = hi + h + c;
            cpu.set_fn(false);
            cpu.set_fh((hi & 0xF) + (h & 0xF) + c >= 0x10);
            cpu.set_fc(diff >= 0x100);
            cpu.h = diff as u8;
        }
        Instr::IncHlDelay => {
            let cpu = bus.cpu();
            let diff = cpu.bus.wrapping_add(1);
            cpu.set_fn(false);
            cpu.set_fh(cpu.bus & 0xF == 0xF);
            cpu.bus = diff;
            cpu.set_fz(cpu.bus == 0);
            cpu.instruction = Instr::Op(0);
            cpu.execution_state = CORE_MEMORY_STORE;
        }
        Instr::DecHlDelay => {
            let cpu = bus.cpu();
            let diff = cpu.bus.wrapping_sub(1);
            cpu.set_fn(true);
            cpu.set_fh(cpu.bus & 0xF == 0);
            cpu.bus = diff;
            cpu.set_fz(cpu.bus == 0);
            cpu.instruction = Instr::Op(0);
            cpu.execution_state = CORE_MEMORY_STORE;
        }
        Instr::PopDelay(p) => {
            let cpu = bus.cpu();
            let hi_reg = match p {
                PAIR_BC => {
                    cpu.c = cpu.bus;
                    REG_B
                }
                PAIR_DE => {
                    cpu.e = cpu.bus;
                    REG_D
                }
                PAIR_HL => {
                    cpu.l = cpu.bus;
                    REG_H
                }
                _ => {
                    cpu.f = cpu.bus;
                    REG_A
                }
            };
            cpu.f &= 0xF0;
            cpu.index = cpu.sp;
            cpu.sp = cpu.sp.wrapping_add(1);
            cpu.instruction = Instr::LdRBus(hi_reg);
            cpu.execution_state = CORE_MEMORY_LOAD;
        }
        Instr::PushFinish => {
            bus.cpu().execution_state = CORE_STALL;
        }
        Instr::PushDelay(p) => {
            let cpu = bus.cpu();
            cpu.sp = cpu.sp.wrapping_sub(1);
            cpu.index = cpu.sp;
            cpu.bus = match p {
                PAIR_BC => cpu.c,
                PAIR_DE => cpu.e,
                PAIR_HL => cpu.l,
                _ => cpu.f,
            };
            cpu.instruction = Instr::PushFinish;
            cpu.execution_state = CORE_MEMORY_STORE;
        }
        Instr::CbHlDelay(op) => {
            let cpu = bus.cpu();
            let reg = cpu.bus;
            let reg = cb_body(cpu, op, reg);
            cpu.bus = reg;
            cpu.execution_state = if (0x40..0x80).contains(&op) { CORE_FETCH } else { CORE_MEMORY_STORE };
            cpu.instruction = Instr::Op(0);
        }
        Instr::RstUpdateSpl(vec) => {
            let cpu = bus.cpu();
            cpu.sp = cpu.sp.wrapping_sub(1);
            cpu.index = cpu.sp;
            cpu.bus = cpu.pc as u8;
            cpu.pc = vec as u16;
            bus.set_active_region(vec as u16);
            let cpu = bus.cpu();
            cpu.execution_state = CORE_MEMORY_STORE;
            cpu.instruction = Instr::Op(0);
        }
        Instr::RstUpdateSph(vec) => {
            let cpu = bus.cpu();
            cpu.sp = cpu.sp.wrapping_sub(1);
            cpu.index = cpu.sp;
            cpu.bus = (cpu.pc >> 8) as u8;
            cpu.execution_state = CORE_MEMORY_STORE;
            cpu.instruction = Instr::RstUpdateSpl(vec);
        }
        Instr::Stop2 => bus.stop(),
        Instr::IrqStall => {
            bus.cpu().execution_state = CORE_STALL;
        }
        Instr::IrqFinish => {
            let cpu = bus.cpu();
            cpu.execution_state = CORE_OP2;
            cpu.instruction = Instr::IrqStall;
        }
        Instr::IrqDelay => {
            let cpu = bus.cpu();
            cpu.sp = cpu.sp.wrapping_sub(1);
            cpu.index = cpu.sp;
            cpu.bus = cpu.pc as u8;
            cpu.execution_state = CORE_MEMORY_STORE;
            cpu.instruction = Instr::IrqFinish;
            let vector = bus.irq_vector();
            bus.cpu().pc = vector;
            bus.set_active_region(vector);
        }
        Instr::Irq => {
            let cpu = bus.cpu();
            cpu.sp = cpu.sp.wrapping_sub(1);
            cpu.index = cpu.sp;
            cpu.bus = (cpu.pc >> 8) as u8;
            cpu.execution_state = CORE_MEMORY_STORE;
            cpu.instruction = Instr::IrqDelay;
        }
    }
}

fn execute_cb<B: Sm83Bus>(bus: &mut B, op: u8) {
    let cpu = bus.cpu();
    let r = op & 7;
    if r == REG_HL {
        cpu.index = cpu.hl();
        cpu.execution_state = CORE_MEMORY_LOAD;
        cpu.instruction = Instr::CbHlDelay(op);
    } else {
        let reg = read_reg(cpu, r);
        let reg = cb_body(cpu, op, reg);
        write_reg(cpu, r, reg);
    }
}

#[inline(always)]
fn execute_op<B: Sm83Bus>(bus: &mut B, op: u8) {
    let cpu = bus.cpu();
    match op {
        0x00 => {}
        // LD rr,nn
        0x01 => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdBcDelay;
        }
        0x11 => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdDeDelay;
        }
        0x21 => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdHlDelay;
        }
        0x31 => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdSpDelay;
        }
        // LD (BC),A / LD (DE),A
        0x02 | 0x12 => {
            cpu.index = if op == 0x02 { cpu.bc() } else { cpu.de() };
            cpu.bus = cpu.a;
            cpu.execution_state = CORE_MEMORY_STORE;
            cpu.instruction = Instr::Op(0);
        }
        // LD (HL+),A / LD (HL-),A
        0x22 | 0x32 => {
            cpu.index = cpu.hl();
            let hl = if op == 0x22 { cpu.index.wrapping_add(1) } else { cpu.index.wrapping_sub(1) };
            cpu.set_hl(hl);
            cpu.bus = cpu.a;
            cpu.execution_state = CORE_MEMORY_STORE;
            cpu.instruction = Instr::Op(0);
        }
        // INC rr / DEC rr
        0x03 | 0x13 | 0x23 | 0x33 | 0x0B | 0x1B | 0x2B | 0x3B => {
            let p = (op >> 4) & 3;
            let v = read_pair(cpu, p);
            let v = if op & 8 == 0 { v.wrapping_add(1) } else { v.wrapping_sub(1) };
            write_pair(cpu, p, v);
            cpu.execution_state = CORE_STALL;
        }
        // INC (HL) / DEC (HL)
        0x34 => {
            cpu.index = cpu.hl();
            cpu.instruction = Instr::IncHlDelay;
            cpu.execution_state = CORE_MEMORY_LOAD;
        }
        0x35 => {
            cpu.index = cpu.hl();
            cpu.instruction = Instr::DecHlDelay;
            cpu.execution_state = CORE_MEMORY_LOAD;
        }
        // INC r
        0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x3C => {
            let r = op >> 3;
            let v = read_reg(cpu, r);
            cpu.set_fh(v & 0xF == 0xF);
            let v = v.wrapping_add(1);
            write_reg(cpu, r, v);
            cpu.set_fn(false);
            cpu.set_fz(v == 0);
        }
        // DEC r
        0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x3D => {
            let r = op >> 3;
            let v = read_reg(cpu, r);
            cpu.set_fh(v & 0xF == 0);
            let v = v.wrapping_sub(1);
            write_reg(cpu, r, v);
            cpu.set_fn(true);
            cpu.set_fz(v == 0);
        }
        // LD r,n / LD (HL),n
        0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x36 | 0x3E => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdRBus(op >> 3);
        }
        0x07 => {
            cpu.a = cpu.a.rotate_left(1);
            cpu.set_fz(false);
            cpu.set_fh(false);
            cpu.set_fn(false);
            cpu.set_fc(cpu.a & 1 != 0);
        }
        0x0F => {
            let low = cpu.a & 1;
            cpu.a = (cpu.a >> 1) | (low << 7);
            cpu.set_fz(false);
            cpu.set_fh(false);
            cpu.set_fn(false);
            cpu.set_fc(low != 0);
        }
        0x17 => {
            let wide = ((cpu.a as u32) << 1) | cpu.fc() as u32;
            cpu.a = wide as u8;
            cpu.set_fz(false);
            cpu.set_fh(false);
            cpu.set_fn(false);
            cpu.set_fc(wide >> 8 != 0);
        }
        0x1F => {
            let low = cpu.a & 1;
            cpu.a = (cpu.a >> 1) | ((cpu.fc() as u8) << 7);
            cpu.set_fz(false);
            cpu.set_fh(false);
            cpu.set_fn(false);
            cpu.set_fc(low != 0);
        }
        // LD (nn),SP
        0x08 => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdIspReadAddr;
        }
        // ADD HL,rr
        0x09 | 0x19 | 0x29 | 0x39 => {
            let p = (op >> 4) & 3;
            let lo = read_pair(cpu, p) as u8 as i32;
            let diff = lo + cpu.l as i32;
            cpu.l = diff as u8;
            cpu.set_fc(diff >= 0x100);
            cpu.execution_state = CORE_OP2;
            cpu.instruction = Instr::AddHlFinish(p);
        }
        // LD A,(BC) / LD A,(DE)
        0x0A | 0x1A => {
            cpu.execution_state = CORE_MEMORY_LOAD;
            cpu.index = if op == 0x0A { cpu.bc() } else { cpu.de() };
            cpu.instruction = Instr::LdRBus(REG_A);
        }
        // LD A,(HL+) / LD A,(HL-)
        0x2A | 0x3A => {
            cpu.index = cpu.hl();
            let hl = if op == 0x2A { cpu.index.wrapping_add(1) } else { cpu.index.wrapping_sub(1) };
            cpu.set_hl(hl);
            cpu.execution_state = CORE_MEMORY_LOAD;
            cpu.instruction = Instr::LdRBus(REG_A);
        }
        0x10 => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::Stop2;
        }
        // JR / JR cc
        0x18 | 0x20 | 0x28 | 0x30 | 0x38 => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::JrFinish;
            cpu.condition = op == 0x18 || condition(cpu, (op >> 3) & 3);
        }
        0x27 => {
            if cpu.fn_() {
                if cpu.fh() {
                    cpu.a = cpu.a.wrapping_add(0xFA);
                }
                if cpu.fc() {
                    cpu.a = cpu.a.wrapping_add(0xA0);
                }
            } else {
                let mut a = cpu.a as i32;
                if (cpu.a & 0xF) > 0x9 || cpu.fh() {
                    a += 0x6;
                }
                if (a & 0x1F0) > 0x90 || cpu.fc() {
                    a += 0x60;
                    cpu.set_fc(true);
                } else {
                    cpu.set_fc(false);
                }
                cpu.a = a as u8;
            }
            cpu.set_fh(false);
            cpu.set_fz(cpu.a == 0);
        }
        0x2F => {
            cpu.a ^= 0xFF;
            cpu.set_fh(true);
            cpu.set_fn(true);
        }
        0x37 => {
            cpu.set_fc(true);
            cpu.set_fh(false);
            cpu.set_fn(false);
        }
        0x3F => {
            let c = cpu.fc();
            cpu.set_fc(!c);
            cpu.set_fh(false);
            cpu.set_fn(false);
        }
        0x76 => {
            bus.halt();
            // XXX: Subtract the cycles that will be added later in the tick function
            let cpu = bus.cpu();
            cpu.cycles -= cpu.t_multiplier;
        }
        // LD (HL),r
        0x70..=0x77 => {
            cpu.bus = read_reg(cpu, op & 7);
            cpu.index = cpu.hl();
            cpu.execution_state = CORE_MEMORY_STORE;
            cpu.instruction = Instr::Op(0);
        }
        // LD r,r' / LD r,(HL)
        0x40..=0x7F => {
            let dst = (op >> 3) & 7;
            let src = op & 7;
            if src == REG_HL {
                cpu.execution_state = CORE_MEMORY_LOAD;
                cpu.index = cpu.hl();
                cpu.instruction = Instr::LdRBus(dst);
            } else {
                let v = read_reg(cpu, src);
                write_reg(cpu, dst, v);
            }
        }
        // ALU A,r / ALU A,(HL)
        0x80..=0xBF => {
            let aop = (op >> 3) & 7;
            let src = op & 7;
            if src == REG_HL {
                cpu.execution_state = CORE_MEMORY_LOAD;
                cpu.index = cpu.hl();
                cpu.instruction = Instr::AluBus(aop);
            } else {
                let v = read_reg(cpu, src);
                alu(cpu, aop, v);
            }
        }
        // ALU A,n
        0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::AluBus((op >> 3) & 7);
        }
        // RET cc
        0xC0 | 0xC8 | 0xD0 | 0xD8 => {
            cpu.condition = condition(cpu, (op >> 3) & 3);
            cpu.execution_state = CORE_OP2;
            cpu.instruction = Instr::RetUpdateSph;
        }
        0xC9 => {
            cpu.condition = true;
            ret_update_sph(cpu);
        }
        0xD9 => {
            cpu.condition = true;
            bus.set_interrupts(true);
            ret_update_sph(bus.cpu());
        }
        // POP rr
        0xC1 | 0xD1 | 0xE1 | 0xF1 => {
            cpu.index = cpu.sp;
            cpu.sp = cpu.sp.wrapping_add(1);
            cpu.instruction = Instr::PopDelay((op >> 4) & 3);
            cpu.execution_state = CORE_MEMORY_LOAD;
        }
        // PUSH rr
        0xC5 | 0xD5 | 0xE5 | 0xF5 => {
            let p = (op >> 4) & 3;
            cpu.sp = cpu.sp.wrapping_sub(1);
            cpu.index = cpu.sp;
            cpu.bus = match p {
                PAIR_BC => cpu.b,
                PAIR_DE => cpu.d,
                PAIR_HL => cpu.h,
                _ => cpu.a,
            };
            cpu.instruction = Instr::PushDelay(p);
            cpu.execution_state = CORE_MEMORY_STORE;
        }
        // JP / JP cc
        0xC3 | 0xC2 | 0xCA | 0xD2 | 0xDA => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::JpDelay;
            cpu.condition = op == 0xC3 || condition(cpu, (op >> 3) & 3);
        }
        // CALL / CALL cc
        0xCD | 0xC4 | 0xCC | 0xD4 | 0xDC => {
            cpu.condition = op == 0xCD || condition(cpu, (op >> 3) & 3);
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::CallUpdatePcl;
        }
        // RST
        0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => {
            cpu.execution_state = CORE_OP2;
            cpu.instruction = Instr::RstUpdateSph(op & 0x38);
        }
        0xCB => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::CbDelegate;
        }
        0xE0 => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdIoaDelay;
        }
        0xF0 => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdAioDelay;
        }
        0xE2 => {
            cpu.index = 0xFF00 | cpu.c as u16;
            cpu.bus = cpu.a;
            cpu.execution_state = CORE_MEMORY_STORE;
            cpu.instruction = Instr::Op(0);
        }
        0xF2 => {
            cpu.index = 0xFF00 | cpu.c as u16;
            cpu.execution_state = CORE_MEMORY_LOAD;
            cpu.instruction = Instr::LdRBus(REG_A);
        }
        0xE8 => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::AddSpDelay;
        }
        0xF8 => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdHlSpDelay;
        }
        0xE9 => {
            cpu.pc = cpu.hl();
            let pc = cpu.pc;
            bus.set_active_region(pc);
        }
        0xF9 => {
            cpu.sp = cpu.hl();
            cpu.execution_state = CORE_STALL;
        }
        0xEA => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdIaDelay;
        }
        0xFA => {
            cpu.execution_state = CORE_READ_PC;
            cpu.instruction = Instr::LdAiDelay;
        }
        0xF3 => bus.set_interrupts(false),
        0xFB => bus.set_interrupts(true),
        // 0xD3, 0xDB, 0xDD, 0xE3, 0xE4, 0xEB, 0xEC, 0xED, 0xF4, 0xFC, 0xFD
        _ => bus.hit_illegal(),
    }
}
