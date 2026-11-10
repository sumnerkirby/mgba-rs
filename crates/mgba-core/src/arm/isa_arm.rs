// Copyright (c) 2013-2014 Jeffrey Pfau
// Rust port of mGBA's ARM-mode instruction set (src/arm/isa-arm.c and
// include/mgba/internal/arm/emitter-arm.h).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

const PSR_USER_MASK: u32 = 0xF0000000;
const PSR_PRIV_MASK: u32 = 0x000000CF;
const PSR_STATE_MASK: u32 = 0x00000020;

/// Decoded instruction class for a 12-bit table index (bits 27-20 and 7-4 of the opcode).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum Op {
    Ill,
    AluReg,
    AluImm,
    Mul,
    MulLong,
    Halfword,
    Swp,
    Swpb,
    Mrs,
    Mrsr,
    Msr,
    Msrr,
    Msri,
    Msrri,
    Bx,
    Bkpt,
    SingleTransfer,
    BlockTransfer,
    B,
    Bl,
    Cdp,
    Mcr,
    Mrc,
    Ldc,
    Stc,
    Swi,
}

const fn decode(index: usize) -> Op {
    let hi = index >> 4;
    let lo = index & 0xF;
    match hi {
        0x00..=0x1F => {
            match hi {
                0x10 => match lo {
                    0x0 => return Op::Mrs,
                    0x9 => return Op::Swp,
                    0xB => return Op::Halfword,
                    _ => return Op::Ill,
                },
                0x12 => match lo {
                    0x0 => return Op::Msr,
                    0x1 => return Op::Bx,
                    0x7 => return Op::Bkpt,
                    0xB => return Op::Halfword,
                    _ => return Op::Ill,
                },
                0x14 => match lo {
                    0x0 => return Op::Mrsr,
                    0x9 => return Op::Swpb,
                    0xB => return Op::Halfword,
                    _ => return Op::Ill,
                },
                0x16 => match lo {
                    0x0 => return Op::Msrr,
                    0xB => return Op::Halfword,
                    _ => return Op::Ill,
                },
                _ => {}
            }
            if lo < 8 || lo & 1 == 0 {
                return Op::AluReg;
            }
            let load = hi & 1 != 0;
            match lo {
                0x9 => {
                    if hi >= 0x10 {
                        Op::Ill
                    } else {
                        match hi >> 1 {
                            0 | 1 => Op::Mul,
                            4..=7 => Op::MulLong,
                            _ => Op::Ill,
                        }
                    }
                }
                0xB => Op::Halfword,
                _ => {
                    if load {
                        Op::Halfword
                    } else {
                        Op::Ill
                    }
                }
            }
        }
        0x20..=0x3F => match hi {
            0x32 => Op::Msri,
            0x36 => Op::Msrri,
            _ => Op::AluImm,
        },
        0x40..=0x5F => Op::SingleTransfer,
        0x60..=0x7F => {
            if lo & 1 != 0 {
                Op::Ill
            } else {
                Op::SingleTransfer
            }
        }
        0x80..=0x9F => Op::BlockTransfer,
        0xA0..=0xAF => Op::B,
        0xB0..=0xBF => Op::Bl,
        0xC0..=0xDF => {
            if hi & 1 != 0 {
                Op::Ldc
            } else {
                Op::Stc
            }
        }
        0xE0..=0xEF => {
            if lo & 1 == 0 {
                Op::Cdp
            } else if hi & 1 != 0 {
                Op::Mrc
            } else {
                Op::Mcr
            }
        }
        _ => Op::Swi,
    }
}

const fn build_table() -> [Op; 0x1000] {
    let mut table = [Op::Ill; 0x1000];
    let mut i = 0;
    while i < 0x1000 {
        table[i] = decode(i);
        i += 1;
    }
    table
}

static ARM_TABLE: [Op; 0x1000] = build_table();

/// The instruction class at a decoder-table index (shared with the disassembler).
pub(crate) fn class_of(index: usize) -> Op {
    ARM_TABLE[index]
}

#[inline(always)]
pub fn execute<B: ArmBus>(bus: &mut B, opcode: u32) {
    let index = (((opcode >> 16) & 0xFF0) | ((opcode >> 4) & 0x00F)) as usize;
    let mut cc = bus.cpu_ref().arm_prefetch_cycles();
    match ARM_TABLE[index] {
        Op::AluReg => alu(bus, opcode, &mut cc, false),
        Op::AluImm => alu(bus, opcode, &mut cc, true),
        Op::Mul => multiply(bus, opcode, &mut cc),
        Op::MulLong => multiply_long(bus, opcode, &mut cc),
        Op::Halfword => halfword_transfer(bus, opcode, &mut cc),
        Op::SingleTransfer => single_transfer(bus, opcode, &mut cc),
        Op::BlockTransfer => block_transfer(bus, opcode, &mut cc),
        Op::Swp => {
            let rm = (opcode & 0xF) as usize;
            let rd = ((opcode >> 12) & 0xF) as usize;
            let rn = ((opcode >> 16) & 0xF) as usize;
            let addr = bus.cpu_ref().gprs[rn];
            let d = bus.load32(addr, Some(&mut cc));
            let v = bus.cpu_ref().gprs[rm];
            let addr = bus.cpu_ref().gprs[rn];
            bus.store32(addr, v, Some(&mut cc));
            bus.cpu().gprs[rd] = d;
        }
        Op::Swpb => {
            let rm = (opcode & 0xF) as usize;
            let rd = ((opcode >> 12) & 0xF) as usize;
            let rn = ((opcode >> 16) & 0xF) as usize;
            let addr = bus.cpu_ref().gprs[rn];
            let d = bus.load8(addr, Some(&mut cc));
            let v = bus.cpu_ref().gprs[rm];
            let addr = bus.cpu_ref().gprs[rn];
            bus.store8(addr, v as u8, Some(&mut cc));
            bus.cpu().gprs[rd] = d;
        }
        Op::B => {
            let offset = ((opcode << 8) as i32) >> 6;
            let cpu = bus.cpu();
            cpu.gprs[ARM_PC] = cpu.gprs[ARM_PC].wrapping_add(offset as u32);
            cc += arm_write_pc(bus);
        }
        Op::Bl => {
            let immediate = ((opcode & 0x00FFFFFF) << 8) as i32;
            let cpu = bus.cpu();
            cpu.gprs[ARM_LR] = cpu.gprs[ARM_PC].wrapping_sub(WORD_SIZE_ARM);
            cpu.gprs[ARM_PC] = cpu.gprs[ARM_PC].wrapping_add((immediate >> 6) as u32);
            cc += arm_write_pc(bus);
        }
        Op::Bx => {
            let rm = (opcode & 0xF) as usize;
            let cpu = bus.cpu();
            let target = cpu.gprs[rm];
            cpu.set_execution_mode(if target & 1 != 0 { ExecutionMode::Thumb } else { ExecutionMode::Arm });
            cpu.gprs[ARM_PC] = target & 0xFFFFFFFE;
            cc += write_pc(bus);
        }
        Op::Mrs => {
            let rd = ((opcode >> 12) & 0xF) as usize;
            let cpu = bus.cpu();
            cpu.gprs[rd] = cpu.cpsr.0;
        }
        Op::Mrsr => {
            let rd = ((opcode >> 12) & 0xF) as usize;
            let cpu = bus.cpu();
            cpu.gprs[rd] = cpu.spsr.0;
        }
        Op::Msr => {
            let operand = bus.cpu_ref().gprs[(opcode & 0xF) as usize];
            msr(bus, opcode, operand);
        }
        Op::Msri => {
            let rotate = (opcode & 0x00000F00) >> 7;
            let operand = (opcode & 0xFF).rotate_right(rotate);
            msr(bus, opcode, operand);
        }
        Op::Msrr => {
            let operand = bus.cpu_ref().gprs[(opcode & 0xF) as usize];
            msrr(bus, opcode, operand);
        }
        Op::Msrri => {
            let rotate = (opcode & 0x00000F00) >> 7;
            let operand = (opcode & 0xFF).rotate_right(rotate);
            msrr(bus, opcode, operand);
        }
        Op::Bkpt => {
            bus.bkpt32(((opcode >> 4) & 0xFFF0) | (opcode & 0xF));
            cc = 0;
        }
        Op::Cdp | Op::Mcr | Op::Mrc => {
            let op1 = (opcode >> 21) & 7;
            let op2 = (opcode >> 5) & 7;
            let rd = ((opcode >> 12) & 0xF) as usize;
            let cp = (opcode >> 8) & 0xF;
            let crn = (opcode >> 16) & 0xF;
            let crm = opcode & 0xF;
            match ARM_TABLE[index] {
                Op::Mrc => match bus.cp_mrc(cp, crn, crm, op1, op2) {
                    Some(v) => bus.cpu().gprs[rd] = v,
                    None => bus.hit_illegal(opcode),
                },
                Op::Mcr => {
                    let v = bus.cpu_ref().gprs[rd];
                    if !bus.cp_mcr(cp, crn, crm, op1, op2, v) {
                        bus.hit_illegal(opcode);
                    }
                }
                _ => {
                    if !bus.cp_cdp(cp, crn, crm, rd as u32, op1, op2) {
                        bus.hit_illegal(opcode);
                    }
                }
            }
        }
        Op::Ldc | Op::Stc => bus.hit_stub(opcode),
        Op::Swi => bus.swi32(opcode & 0xFFFFFF),
        Op::Ill => bus.hit_illegal(opcode),
    }
    bus.cpu().cycles += cc;
}

// Addressing mode 1

#[inline(always)]
fn shift_register<B: ArmBus>(bus: &mut B, opcode: u32) {
    let cpu = bus.cpu();
    let rm = (opcode & 0xF) as usize;
    let shift_type = (opcode >> 5) & 3;
    if opcode & 0x10 != 0 {
        let rs = ((opcode >> 8) & 0xF) as usize;
        cpu.cycles += 1;
        let mut shift_val = cpu.gprs[rm];
        if rm == ARM_PC {
            shift_val = shift_val.wrapping_add(4);
        }
        let shift = cpu.gprs[rs] & 0xFF;
        let c = cpu.cpsr.c() as u32;
        let (op, carry) = match shift_type {
            0 => {
                if shift == 0 {
                    (shift_val, c)
                } else if shift < 32 {
                    (shift_val << shift, (shift_val >> (32 - shift)) & 1)
                } else if shift == 32 {
                    (0, shift_val & 1)
                } else {
                    (0, 0)
                }
            }
            1 => {
                if shift == 0 {
                    (shift_val, c)
                } else if shift < 32 {
                    (shift_val >> shift, (shift_val >> (shift - 1)) & 1)
                } else if shift == 32 {
                    (0, shift_val >> 31)
                } else {
                    (0, 0)
                }
            }
            2 => {
                if shift == 0 {
                    (shift_val, c)
                } else if shift < 32 {
                    (((shift_val as i32) >> shift) as u32, (shift_val >> (shift - 1)) & 1)
                } else if cpu.gprs[rm] >> 31 != 0 {
                    (0xFFFFFFFF, 1)
                } else {
                    (0, 0)
                }
            }
            _ => {
                let rotate = shift & 0x1F;
                if shift == 0 {
                    (shift_val, c)
                } else if rotate != 0 {
                    (shift_val.rotate_right(rotate), (shift_val >> (rotate - 1)) & 1)
                } else {
                    (shift_val, shift_val >> 31)
                }
            }
        };
        cpu.shifter_operand = op;
        cpu.shifter_carry_out = carry;
    } else {
        let immediate = (opcode & 0x00000F80) >> 7;
        let v = cpu.gprs[rm];
        let (op, carry) = match shift_type {
            0 => {
                if immediate == 0 {
                    (v, cpu.cpsr.c() as u32)
                } else {
                    (v << immediate, (v >> (32 - immediate)) & 1)
                }
            }
            1 => {
                if immediate != 0 {
                    (v >> immediate, (v >> (immediate - 1)) & 1)
                } else {
                    (0, v >> 31)
                }
            }
            2 => {
                if immediate != 0 {
                    (((v as i32) >> immediate) as u32, (v >> (immediate - 1)) & 1)
                } else {
                    let s = v >> 31;
                    (if s != 0 { 0xFFFFFFFF } else { 0 }, s)
                }
            }
            _ => {
                if immediate != 0 {
                    (v.rotate_right(immediate), (v >> (immediate - 1)) & 1)
                } else {
                    // RRX
                    (((cpu.cpsr.c() as u32) << 31) | (v >> 1), v & 1)
                }
            }
        };
        cpu.shifter_operand = op;
        cpu.shifter_carry_out = carry;
    }
}

#[inline(always)]
fn shift_immediate(cpu: &mut ArmCore, opcode: u32) {
    let rotate = (opcode & 0x00000F00) >> 7;
    let immediate = opcode & 0x000000FF;
    if rotate == 0 {
        cpu.shifter_operand = immediate;
        cpu.shifter_carry_out = cpu.cpsr.c() as u32;
    } else {
        cpu.shifter_operand = immediate.rotate_right(rotate);
        cpu.shifter_carry_out = cpu.shifter_operand >> 31;
    }
}

#[inline(always)]
fn addition_s(cpu: &mut ArmCore, m: u32, n: u32, d: u32) {
    cpu.cpsr.set_flags(0);
    cpu.cpsr.set_n(sign(d));
    cpu.cpsr.set_z(d == 0);
    cpu.cpsr.set_c(carry_from(m, n, d));
    cpu.cpsr.set_v(v_addition(m, n, d));
}

#[inline(always)]
fn subtraction_s(cpu: &mut ArmCore, m: u32, n: u32, d: u32) {
    cpu.cpsr.set_flags(0);
    cpu.cpsr.set_n(sign(d));
    cpu.cpsr.set_z(d == 0);
    cpu.cpsr.set_c(borrow_from(m, n));
    cpu.cpsr.set_v(v_subtraction(m, n, d));
}

#[inline(always)]
fn subtraction_carry_s(cpu: &mut ArmCore, m: u32, n: u32, d: u32, c: u32) {
    cpu.cpsr.set_n(sign(d));
    cpu.cpsr.set_z(d == 0);
    cpu.cpsr.set_c(borrow_from_carry(m, n, c));
    cpu.cpsr.set_v(v_subtraction(m, n, d));
}

#[inline(always)]
fn neutral_s(cpu: &mut ArmCore, d: u32) {
    cpu.cpsr.set_n(sign(d));
    cpu.cpsr.set_z(d == 0);
    cpu.cpsr.set_c(cpu.shifter_carry_out & 1 != 0);
}

/// Data processing instructions (both register-shifted and immediate operands).
#[inline(always)]
fn alu<B: ArmBus>(bus: &mut B, opcode: u32, cc: &mut i32, immediate: bool) {
    if immediate {
        shift_immediate(bus.cpu(), opcode);
    } else {
        shift_register(bus, opcode);
    }
    let op = (opcode >> 21) & 0xF;
    // Compare ops (TST/TEQ/CMP/CMN) always set flags.
    let s = opcode & 0x00100000 != 0 || (8..=11).contains(&op);
    let rd = ((opcode >> 12) & 0xF) as usize;
    let rn = ((opcode >> 16) & 0xF) as usize;
    let cpu = bus.cpu();
    let mut n = cpu.gprs[rn];
    if rn == ARM_PC && (opcode & 0x02000010) == 0x00000010 {
        n = n.wrapping_add(WORD_SIZE_ARM);
    }
    let so = cpu.shifter_operand;
    let c = cpu.cpsr.c() as u32;

    // Flag kinds: 0 = neutral, 1 = addition, 2 = subtraction, 3 = subtraction w/ carry
    let (d, write, kind, fm, fn_, carry) = match op {
        0x0 => (n & so, true, 0, 0, 0, 0),                                   // AND
        0x1 => (n ^ so, true, 0, 0, 0, 0),                                   // EOR
        0x2 => (n.wrapping_sub(so), true, 2, n, so, 0),                      // SUB
        0x3 => (so.wrapping_sub(n), true, 2, so, n, 0),                      // RSB
        0x4 => (n.wrapping_add(so), true, 1, n, so, 0),                      // ADD
        0x5 => (n.wrapping_add(so).wrapping_add(c), true, 1, n, so, 0),      // ADC
        0x6 => (n.wrapping_sub(so).wrapping_sub(1 - c), true, 3, n, so, 1 - c), // SBC
        0x7 => (so.wrapping_sub(n).wrapping_sub(1 - c), true, 3, so, n, 1 - c), // RSC
        0x8 => (n & so, false, 0, 0, 0, 0),                                  // TST
        0x9 => (n ^ so, false, 0, 0, 0, 0),                                  // TEQ
        0xA => (n.wrapping_sub(so), false, 2, n, so, 0),                     // CMP
        0xB => (n.wrapping_add(so), false, 1, n, so, 0),                     // CMN
        0xC => (n | so, true, 0, 0, 0, 0),                                   // ORR
        0xD => (so, true, 0, 0, 0, 0),                                       // MOV
        0xE => (n & !so, true, 0, 0, 0, 0),                                  // BIC
        _ => (!so, true, 0, 0, 0, 0),                                        // MVN
    };
    if write {
        cpu.gprs[rd] = d;
    }
    if s {
        if rd == ARM_PC && mode_has_spsr(cpu.cpsr.priv_mode()) {
            cpu.cpsr = cpu.spsr;
            read_cpsr(bus);
        } else {
            let cpu = bus.cpu();
            match kind {
                0 => neutral_s(cpu, d),
                1 => addition_s(cpu, fm, fn_, d),
                2 => subtraction_s(cpu, fm, fn_, d),
                _ => subtraction_carry_s(cpu, fm, fn_, d, carry),
            }
        }
    }
    if write && rd == ARM_PC {
        *cc += write_pc(bus);
    }
}

fn multiply<B: ArmBus>(bus: &mut B, opcode: u32, cc: &mut i32) {
    let accumulate = opcode & 0x00200000 != 0;
    let s = opcode & 0x00100000 != 0;
    let rs = ((opcode >> 8) & 0xF) as usize;
    let rm = (opcode & 0xF) as usize;
    if !accumulate {
        // MUL: rd in bits 19-16
        let rd = ((opcode >> 16) & 0xF) as usize;
        if rd != ARM_PC {
            let w = wait_smul(bus.cpu_ref().gprs[rs], 0);
            *cc += bus.stall(w);
            let cpu = bus.cpu();
            cpu.gprs[rd] = cpu.gprs[rm].wrapping_mul(cpu.gprs[rs]);
            if s {
                let d = cpu.gprs[rd];
                neutral_s(cpu, d);
            }
        }
    } else {
        // MLA: rdHi (destination) in 19-16, rd (accumulate) in 15-12
        let rd = ((opcode >> 12) & 0xF) as usize;
        let rd_hi = ((opcode >> 16) & 0xF) as usize;
        if rd_hi != ARM_PC && rd != ARM_PC {
            let w = wait_smul(bus.cpu_ref().gprs[rs], 1);
            *cc += bus.stall(w);
            let cpu = bus.cpu();
            cpu.gprs[rd_hi] = cpu.gprs[rm].wrapping_mul(cpu.gprs[rs]).wrapping_add(cpu.gprs[rd]);
            if s {
                let d = cpu.gprs[rd_hi];
                neutral_s(cpu, d);
            }
        }
    }
    let cpu = bus.cpu_ref();
    *cc += cpu.memory.active_nonseq_cycles32 - cpu.memory.active_seq_cycles32;
}

fn multiply_long<B: ArmBus>(bus: &mut B, opcode: u32, cc: &mut i32) {
    let signed = opcode & 0x00400000 != 0;
    let accumulate = opcode & 0x00200000 != 0;
    let s = opcode & 0x00100000 != 0;
    let rd = ((opcode >> 12) & 0xF) as usize;
    let rd_hi = ((opcode >> 16) & 0xF) as usize;
    let rs = ((opcode >> 8) & 0xF) as usize;
    let rm = (opcode & 0xF) as usize;
    if rd_hi != ARM_PC && rd != ARM_PC {
        let wait = if accumulate { 2 } else { 1 };
        let r = bus.cpu_ref().gprs[rs];
        let w = if signed { wait_smul(r, wait) } else { wait_umul(r, wait) };
        *cc += bus.stall(w);
        let cpu = bus.cpu();
        let (lo, hi) = if signed {
            let d = (cpu.gprs[rm] as i32 as i64).wrapping_mul(cpu.gprs[rs] as i32 as i64);
            if accumulate {
                let d = d.wrapping_add(cpu.gprs[rd] as u64 as i64);
                let d_hi = cpu.gprs[rd_hi].wrapping_add((d >> 32) as u32);
                (d as u32, d_hi)
            } else {
                (d as u32, (d >> 32) as u32)
            }
        } else {
            let d = (cpu.gprs[rm] as u64).wrapping_mul(cpu.gprs[rs] as u64);
            if accumulate {
                let d = d.wrapping_add(cpu.gprs[rd] as u64);
                let d_hi = cpu.gprs[rd_hi].wrapping_add((d >> 32) as u32);
                (d as u32, d_hi)
            } else {
                (d as u32, (d >> 32) as u32)
            }
        };
        cpu.gprs[rd] = lo;
        cpu.gprs[rd_hi] = hi;
        if s {
            let lo = cpu.gprs[rd];
            cpu.cpsr.set_n(sign(hi));
            cpu.cpsr.set_z((hi | lo) == 0);
        }
    }
    let cpu = bus.cpu_ref();
    *cc += cpu.memory.active_nonseq_cycles32 - cpu.memory.active_seq_cycles32;
}

#[inline(always)]
fn writeback<B: ArmBus>(bus: &mut B, rn: usize, addr: u32, cc: &mut i32) {
    bus.cpu().gprs[rn] = addr;
    if rn == ARM_PC {
        *cc += arm_write_pc(bus);
    }
}

#[inline(always)]
fn load_post_body<B: ArmBus>(bus: &mut B, rd: usize, cc: &mut i32) {
    let cpu = bus.cpu_ref();
    *cc += cpu.memory.active_nonseq_cycles32 - cpu.memory.active_seq_cycles32;
    if rd == ARM_PC {
        *cc += arm_write_pc(bus);
    }
}

#[inline(always)]
fn store_post_body<B: ArmBus>(bus: &mut B, cc: &mut i32) {
    let cpu = bus.cpu_ref();
    *cc += cpu.memory.active_nonseq_cycles32 - cpu.memory.active_seq_cycles32;
}

/// LDR/STR/LDRB/STRB (and their T variants).
fn single_transfer<B: ArmBus>(bus: &mut B, opcode: u32, cc: &mut i32) {
    let register_offset = opcode & 0x02000000 != 0;
    let pre = opcode & 0x01000000 != 0;
    let up = opcode & 0x00800000 != 0;
    let byte = opcode & 0x00400000 != 0;
    let w = opcode & 0x00200000 != 0;
    let load = opcode & 0x00100000 != 0;
    let translate = !pre && w;

    let rn = ((opcode >> 16) & 0xF) as usize;
    let rd = ((opcode >> 12) & 0xF) as usize;
    let rm = (opcode & 0xF) as usize;
    let cpu = bus.cpu_ref();
    let mut d = cpu.gprs[rd];
    if rd == ARM_PC {
        d = d.wrapping_add(WORD_SIZE_ARM);
    }
    let offset = if register_offset {
        let i = (opcode & 0x00000F80) >> 7;
        let v = cpu.gprs[rm];
        match (opcode >> 5) & 3 {
            0 => v << i,
            1 => {
                if i != 0 {
                    v >> i
                } else {
                    0
                }
            }
            2 => {
                if i != 0 {
                    ((v as i32) >> i) as u32
                } else {
                    ((v as i32) >> 31) as u32
                }
            }
            _ => {
                if i != 0 {
                    v.rotate_right(i)
                } else {
                    ((cpu.cpsr.c() as u32) << 31) | (v >> 1)
                }
            }
        }
    } else {
        opcode & 0x00000FFF
    };
    let base = cpu.gprs[rn];
    let indexed = if up { base.wrapping_add(offset) } else { base.wrapping_sub(offset) };
    let (address, wb) = if pre {
        (indexed, if w { Some(indexed) } else { None })
    } else {
        (base, Some(indexed))
    };

    if load {
        if let Some(a) = wb {
            writeback(bus, rn, a, cc);
        }
        let value = if translate {
            let priv_mode = bus.cpu_ref().privilege_mode;
            bus.cpu().set_privilege_mode(MODE_USER);
            let r = if byte { bus.load8(address, Some(cc)) } else { bus.load32(address, Some(cc)) };
            bus.cpu().set_privilege_mode(priv_mode);
            r
        } else if byte {
            bus.load8(address, Some(cc))
        } else {
            bus.load32(address, Some(cc))
        };
        bus.cpu().gprs[rd] = value;
        load_post_body(bus, rd, cc);
    } else {
        if translate {
            let priv_mode = bus.cpu_ref().privilege_mode;
            bus.cpu().set_privilege_mode(MODE_USER);
            if byte {
                bus.store8(address, d as u8, Some(cc));
            } else {
                bus.store32(address, d, Some(cc));
            }
            bus.cpu().set_privilege_mode(priv_mode);
        } else if byte {
            bus.store8(address, d as u8, Some(cc));
        } else {
            bus.store32(address, d, Some(cc));
        }
        store_post_body(bus, cc);
        if let Some(a) = wb {
            writeback(bus, rn, a, cc);
        }
    }
}

/// LDRH/STRH/LDRSB/LDRSH (addressing mode 3).
fn halfword_transfer<B: ArmBus>(bus: &mut B, opcode: u32, cc: &mut i32) {
    let pre = opcode & 0x01000000 != 0;
    let up = opcode & 0x00800000 != 0;
    let imm = opcode & 0x00400000 != 0;
    let w = opcode & 0x00200000 != 0;
    let load = opcode & 0x00100000 != 0;
    let sh = (opcode >> 5) & 3; // 1 = H, 2 = SB, 3 = SH

    let rn = ((opcode >> 16) & 0xF) as usize;
    let rd = ((opcode >> 12) & 0xF) as usize;
    let rm = (opcode & 0xF) as usize;
    let cpu = bus.cpu_ref();
    let mut d = cpu.gprs[rd];
    if rd == ARM_PC {
        d = d.wrapping_add(WORD_SIZE_ARM);
    }
    let offset = if imm { ((opcode & 0x00000F00) >> 4) | (opcode & 0x0000000F) } else { cpu.gprs[rm] };
    let base = cpu.gprs[rn];
    let indexed = if up { base.wrapping_add(offset) } else { base.wrapping_sub(offset) };
    let (address, wb) = if pre {
        (indexed, if w { Some(indexed) } else { None })
    } else {
        (base, Some(indexed))
    };

    if load {
        if let Some(a) = wb {
            writeback(bus, rn, a, cc);
        }
        let value = match sh {
            2 => bus.load8(address, Some(cc)) as u8 as i8 as i32 as u32,
            3 => {
                let v = bus.load16(address, Some(cc));
                if address & 1 != 0 {
                    v as u8 as i8 as i32 as u32
                } else {
                    v as u16 as i16 as i32 as u32
                }
            }
            _ => bus.load16(address, Some(cc)),
        };
        bus.cpu().gprs[rd] = value;
        load_post_body(bus, rd, cc);
    } else {
        bus.store16(address, d as u16, Some(cc));
        store_post_body(bus, cc);
        if let Some(a) = wb {
            writeback(bus, rn, a, cc);
        }
    }
}

/// LDM/STM
fn block_transfer<B: ArmBus>(bus: &mut B, opcode: u32, cc: &mut i32) {
    let pre = opcode & 0x01000000 != 0;
    let up = opcode & 0x00800000 != 0;
    let s = opcode & 0x00400000 != 0;
    let w = opcode & 0x00200000 != 0;
    let load = opcode & 0x00100000 != 0;
    let direction = (if pre { LSM_B } else { 0 }) | (if up { 0 } else { LSM_D });

    let rn = ((opcode >> 16) & 0xF) as usize;
    let rs = opcode & 0x0000FFFF;
    let address = bus.cpu_ref().gprs[rn];

    if load {
        let user_bank = s && (rs & 0x8000) == 0 && rs != 0;
        let mut saved = 0;
        if user_bank {
            saved = bus.cpu_ref().privilege_mode;
            bus.cpu().set_privilege_mode(MODE_SYSTEM);
        }
        let address = bus.load_multiple(address, rs, direction, cc);
        if w && (1u32 << rn) & rs == 0 {
            bus.cpu().gprs[rn] = address;
        }
        if s {
            if user_bank {
                bus.cpu().set_privilege_mode(saved);
            } else if mode_has_spsr(bus.cpu_ref().cpsr.priv_mode()) {
                let cpu = bus.cpu();
                cpu.cpsr = cpu.spsr;
                read_cpsr(bus);
            }
        }
        let cpu = bus.cpu_ref();
        *cc += cpu.memory.active_nonseq_cycles32 - cpu.memory.active_seq_cycles32;
        if (rs & 0x8000) != 0 || rs == 0 {
            *cc += write_pc(bus);
        }
    } else {
        let mut saved = 0;
        if s {
            saved = bus.cpu_ref().privilege_mode;
            bus.cpu().set_privilege_mode(MODE_SYSTEM);
        }
        let address = bus.store_multiple(address, rs, direction, cc);
        if w {
            bus.cpu().gprs[rn] = address;
        }
        if s {
            bus.cpu().set_privilege_mode(saved);
        }
        store_post_body(bus, cc);
    }
}

fn msr<B: ArmBus>(bus: &mut B, opcode: u32, operand: u32) {
    let c = opcode & 0x00010000 != 0;
    let f = opcode & 0x00080000 != 0;
    let mask = (if c { 0x000000FF } else { 0 }) | (if f { 0xFF000000 } else { 0 });
    let cpu = bus.cpu();
    if mask & PSR_USER_MASK != 0 {
        cpu.cpsr.0 = (cpu.cpsr.0 & !PSR_USER_MASK) | (operand & PSR_USER_MASK);
    }
    if mask & PSR_STATE_MASK != 0 {
        cpu.cpsr.0 = (cpu.cpsr.0 & !PSR_STATE_MASK) | (operand & PSR_STATE_MASK);
    }
    if cpu.privilege_mode != MODE_USER && (mask & PSR_PRIV_MASK) != 0 {
        cpu.set_privilege_mode((operand & 0x0000000F) | 0x00000010);
        cpu.cpsr.0 = (cpu.cpsr.0 & !PSR_PRIV_MASK) | (operand & PSR_PRIV_MASK);
    }
    read_cpsr(bus);
    if bus.cpu_ref().execution_mode == ExecutionMode::Thumb {
        let cpu = bus.cpu();
        cpu.prefetch[0] = 0x46C0; // nop
        cpu.prefetch[1] &= 0xFFFF;
        cpu.gprs[ARM_PC] = cpu.gprs[ARM_PC].wrapping_add(WORD_SIZE_THUMB);
    } else {
        let cpu = bus.cpu_ref();
        let mask = cpu.memory.active_mask;
        let pc = cpu.gprs[ARM_PC];
        let p0 = bus.fetch32(pc.wrapping_sub(WORD_SIZE_ARM) & mask);
        let p1 = bus.fetch32(pc & mask);
        let cpu = bus.cpu();
        cpu.prefetch[0] = p0;
        cpu.prefetch[1] = p1;
    }
}

fn msrr<B: ArmBus>(bus: &mut B, opcode: u32, operand: u32) {
    let c = opcode & 0x00010000 != 0;
    let f = opcode & 0x00080000 != 0;
    let mut mask = (if c { 0x000000FF } else { 0 }) | (if f { 0xFF000000 } else { 0 });
    mask &= PSR_USER_MASK | PSR_PRIV_MASK | PSR_STATE_MASK;
    let cpu = bus.cpu();
    cpu.spsr.0 = (cpu.spsr.0 & !mask) | (operand & mask) | 0x00000010;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_table_matches_emitter() {
        // Spot-check rows of DECLARE_ARM_EMITTER_BLOCK
        assert_eq!(decode(0x009), Op::Mul);
        assert_eq!(decode(0x00B), Op::Halfword);
        assert_eq!(decode(0x00D), Op::Ill);
        assert_eq!(decode(0x01D), Op::Halfword);
        assert_eq!(decode(0x049), Op::Ill);
        assert_eq!(decode(0x089), Op::MulLong);
        assert_eq!(decode(0x100), Op::Mrs);
        assert_eq!(decode(0x109), Op::Swp);
        assert_eq!(decode(0x111), Op::AluReg);
        assert_eq!(decode(0x119), Op::Ill);
        assert_eq!(decode(0x121), Op::Bx);
        assert_eq!(decode(0x127), Op::Bkpt);
        assert_eq!(decode(0x149), Op::Swpb);
        assert_eq!(decode(0x160), Op::Msrr);
        assert_eq!(decode(0x1A9), Op::Ill);
        assert_eq!(decode(0x1AB), Op::Halfword);
        assert_eq!(decode(0x320), Op::Msri);
        assert_eq!(decode(0x300), Op::AluImm);
        assert_eq!(decode(0x611), Op::Ill);
        assert_eq!(decode(0x610), Op::SingleTransfer);
        assert_eq!(decode(0xE01), Op::Mcr);
        assert_eq!(decode(0xE11), Op::Mrc);
        assert_eq!(decode(0xE10), Op::Cdp);
        assert_eq!(decode(0xF00), Op::Swi);
    }
}
