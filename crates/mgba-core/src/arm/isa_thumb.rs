// Copyright (c) 2013-2014 Jeffrey Pfau
// Rust port of mGBA's Thumb instruction set (src/arm/isa-thumb.c and
// include/mgba/internal/arm/emitter-thumb.h).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

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
fn neutral_s(cpu: &mut ArmCore, d: u32) {
    cpu.cpsr.set_n(sign(d));
    cpu.cpsr.set_z(d == 0);
}

#[inline(always)]
fn add(cpu: &mut ArmCore, rd: usize, m: u32, n: u32) {
    let d = m.wrapping_add(n);
    cpu.gprs[rd] = d;
    addition_s(cpu, m, n, d);
}

#[inline(always)]
fn sub(cpu: &mut ArmCore, rd: usize, m: u32, n: u32) {
    let d = m.wrapping_sub(n);
    cpu.gprs[rd] = d;
    subtraction_s(cpu, m, n, d);
}

#[inline(always)]
fn load_store_post<B: ArmBus>(bus: &mut B, cc: &mut i32) {
    let cpu = bus.cpu_ref();
    *cc += cpu.memory.active_nonseq_cycles16 - cpu.memory.active_seq_cycles16;
}

#[inline(always)]
pub fn execute<B: ArmBus>(bus: &mut B, opcode: u32) {
    let mut cc = bus.cpu_ref().thumb_prefetch_cycles();
    let index = opcode >> 6;
    match index >> 4 {
        // 0x000-0x05F: LSL1, LSR1, ASR1 (immediate shifts)
        0x0..=0x5 => {
            let immediate = (opcode >> 6) & 0x1F;
            let rd = (opcode & 7) as usize;
            let rm = ((opcode >> 3) & 7) as usize;
            let cpu = bus.cpu();
            let v = cpu.gprs[rm];
            match index >> 5 {
                0 => {
                    if immediate == 0 {
                        cpu.gprs[rd] = v;
                    } else {
                        cpu.cpsr.set_c((v >> (32 - immediate)) & 1 != 0);
                        cpu.gprs[rd] = v << immediate;
                    }
                }
                1 => {
                    if immediate == 0 {
                        cpu.cpsr.set_c(sign(v));
                        cpu.gprs[rd] = 0;
                    } else {
                        cpu.cpsr.set_c((v >> (immediate - 1)) & 1 != 0);
                        cpu.gprs[rd] = v >> immediate;
                    }
                }
                _ => {
                    if immediate == 0 {
                        cpu.cpsr.set_c(sign(v));
                        cpu.gprs[rd] = if sign(v) { 0xFFFFFFFF } else { 0 };
                    } else {
                        cpu.cpsr.set_c((v >> (immediate - 1)) & 1 != 0);
                        cpu.gprs[rd] = ((v as i32) >> immediate) as u32;
                    }
                }
            }
            let d = cpu.gprs[rd];
            neutral_s(cpu, d);
        }
        // 0x060-0x07F: ADD3, SUB3, ADD1, SUB1
        0x6 | 0x7 => {
            let rd = (opcode & 7) as usize;
            let rn = ((opcode >> 3) & 7) as usize;
            let field = (opcode >> 6) & 7;
            let cpu = bus.cpu();
            let n = cpu.gprs[rn];
            match (index >> 3) & 3 {
                0 => {
                    let m = cpu.gprs[field as usize];
                    add(cpu, rd, n, m)
                }
                1 => {
                    let m = cpu.gprs[field as usize];
                    sub(cpu, rd, n, m)
                }
                2 => add(cpu, rd, n, field),
                _ => sub(cpu, rd, n, field),
            }
        }
        // 0x080-0x0FF: MOV1, CMP1, ADD2, SUB2
        0x8..=0xF => {
            let rd = ((opcode >> 8) & 7) as usize;
            let immediate = opcode & 0xFF;
            let cpu = bus.cpu();
            let d = cpu.gprs[rd];
            match (index >> 5) & 3 {
                0 => {
                    cpu.gprs[rd] = immediate;
                    neutral_s(cpu, immediate);
                }
                1 => {
                    let alu_out = d.wrapping_sub(immediate);
                    subtraction_s(cpu, d, immediate, alu_out);
                }
                2 => add(cpu, rd, d, immediate),
                _ => sub(cpu, rd, d, immediate),
            }
        }
        0x10 | 0x11 => {
            if index < 0x110 {
                data_processing(bus, opcode, &mut cc);
            } else {
                high_register(bus, opcode, &mut cc);
            }
        }
        // 0x120-0x13F: LDR3 (PC-relative)
        0x12 | 0x13 => {
            let rd = ((opcode >> 8) & 7) as usize;
            let immediate = (opcode & 0xFF) << 2;
            let addr = (bus.cpu_ref().gprs[ARM_PC] & 0xFFFFFFFC).wrapping_add(immediate);
            let v = bus.load32(addr, Some(&mut cc));
            bus.cpu().gprs[rd] = v;
            load_store_post(bus, &mut cc);
        }
        // 0x140-0x17F: load/store with register offset
        0x14..=0x17 => {
            let rm = ((opcode >> 6) & 7) as usize;
            let rd = (opcode & 7) as usize;
            let rn = ((opcode >> 3) & 7) as usize;
            let cpu = bus.cpu_ref();
            let addr = cpu.gprs[rn].wrapping_add(cpu.gprs[rm]);
            let d = cpu.gprs[rd];
            match (index >> 3) & 7 {
                0 => bus.store32(addr, d, Some(&mut cc)),
                1 => bus.store16(addr, d as u16, Some(&mut cc)),
                2 => bus.store8(addr, d as u8, Some(&mut cc)),
                3 => {
                    let v = bus.load8(addr, Some(&mut cc)) as u8 as i8 as i32 as u32;
                    bus.cpu().gprs[rd] = v;
                }
                4 => {
                    let v = bus.load32(addr, Some(&mut cc));
                    bus.cpu().gprs[rd] = v;
                }
                5 => {
                    let v = bus.load16(addr, Some(&mut cc));
                    bus.cpu().gprs[rd] = v;
                }
                6 => {
                    let v = bus.load8(addr, Some(&mut cc));
                    bus.cpu().gprs[rd] = v;
                }
                _ => {
                    let v = bus.load16(addr, Some(&mut cc));
                    let v = if addr & 1 != 0 { v as u8 as i8 as i32 as u32 } else { v as u16 as i16 as i32 as u32 };
                    bus.cpu().gprs[rd] = v;
                }
            }
            load_store_post(bus, &mut cc);
        }
        // 0x180-0x23F: load/store with immediate offset
        0x18..=0x23 => {
            let immediate = (opcode >> 6) & 0x1F;
            let rd = (opcode & 7) as usize;
            let rm = ((opcode >> 3) & 7) as usize;
            let cpu = bus.cpu_ref();
            let base = cpu.gprs[rm];
            let d = cpu.gprs[rd];
            match (index >> 5) - 0xC {
                0 => bus.store32(base.wrapping_add(immediate * 4), d, Some(&mut cc)),
                1 => {
                    let v = bus.load32(base.wrapping_add(immediate * 4), Some(&mut cc));
                    bus.cpu().gprs[rd] = v;
                }
                2 => bus.store8(base.wrapping_add(immediate), d as u8, Some(&mut cc)),
                3 => {
                    let v = bus.load8(base.wrapping_add(immediate), Some(&mut cc));
                    bus.cpu().gprs[rd] = v;
                }
                4 => bus.store16(base.wrapping_add(immediate * 2), d as u16, Some(&mut cc)),
                _ => {
                    let v = bus.load16(base.wrapping_add(immediate * 2), Some(&mut cc));
                    bus.cpu().gprs[rd] = v;
                }
            }
            load_store_post(bus, &mut cc);
        }
        // 0x240-0x27F: STR3 / LDR4 (SP-relative)
        0x24..=0x27 => {
            let rd = ((opcode >> 8) & 7) as usize;
            let immediate = (opcode & 0xFF) << 2;
            let cpu = bus.cpu_ref();
            let addr = cpu.gprs[ARM_SP].wrapping_add(immediate);
            if index < 0x260 {
                let d = cpu.gprs[rd];
                bus.store32(addr, d, Some(&mut cc));
            } else {
                let v = bus.load32(addr, Some(&mut cc));
                bus.cpu().gprs[rd] = v;
            }
            load_store_post(bus, &mut cc);
        }
        // 0x280-0x2BF: ADD5 (PC) / ADD6 (SP)
        0x28..=0x2B => {
            let rd = ((opcode >> 8) & 7) as usize;
            let immediate = (opcode & 0xFF) << 2;
            let cpu = bus.cpu();
            cpu.gprs[rd] = if index < 0x2A0 {
                (cpu.gprs[ARM_PC] & 0xFFFFFFFC).wrapping_add(immediate)
            } else {
                cpu.gprs[ARM_SP].wrapping_add(immediate)
            };
        }
        // 0x2C0-0x2FF: miscellaneous
        0x2C..=0x2F => misc(bus, opcode, index, &mut cc),
        // 0x300-0x33F: STMIA / LDMIA
        0x30..=0x33 => {
            let rn = ((opcode >> 8) & 7) as usize;
            let rs = opcode & 0xFF;
            let address = bus.cpu_ref().gprs[rn];
            if index < 0x320 {
                let address = bus.store_multiple(address, rs, LSM_IA, &mut cc);
                load_store_post(bus, &mut cc);
                bus.cpu().gprs[rn] = address;
            } else {
                let address = bus.load_multiple(address, rs, LSM_IA, &mut cc);
                load_store_post(bus, &mut cc);
                if rs == 0 {
                    cc += thumb_write_pc(bus);
                }
                if (1u32 << rn) & rs == 0 {
                    bus.cpu().gprs[rn] = address;
                }
            }
        }
        // 0x340-0x37F: conditional branches, ILL, SWI
        0x34..=0x37 => {
            let cond = (index >> 2) & 0xF;
            match cond {
                0xE => bus.hit_illegal(opcode),
                0xF => bus.swi16(opcode & 0xFF),
                _ => {
                    if bus.cpu_ref().test_condition(cond) {
                        let immediate = (opcode as u8) as i8 as i32;
                        let cpu = bus.cpu();
                        cpu.gprs[ARM_PC] = cpu.gprs[ARM_PC].wrapping_add((immediate << 1) as u32);
                        cc += thumb_write_pc(bus);
                    }
                }
            }
        }
        // 0x380-0x39F: B
        0x38 | 0x39 => {
            let immediate = (((opcode & 0x07FF) << 5) as u16) as i16 as i32;
            let cpu = bus.cpu();
            cpu.gprs[ARM_PC] = cpu.gprs[ARM_PC].wrapping_add((immediate >> 4) as u32);
            cc += thumb_write_pc(bus);
        }
        // 0x3A0-0x3BF: ILL
        0x3A | 0x3B => bus.hit_illegal(opcode),
        // 0x3C0-0x3DF: BL1
        0x3C | 0x3D => {
            let immediate = (((opcode & 0x07FF) << 5) as u16) as i16 as i32;
            let cpu = bus.cpu();
            cpu.gprs[ARM_LR] = cpu.gprs[ARM_PC].wrapping_add((immediate << 7) as u32);
        }
        // 0x3E0-0x3FF: BL2
        _ => {
            let immediate = (opcode & 0x07FF) << 1;
            let cpu = bus.cpu();
            let pc = cpu.gprs[ARM_PC];
            cpu.gprs[ARM_PC] = cpu.gprs[ARM_LR].wrapping_add(immediate);
            cpu.gprs[ARM_LR] = pc.wrapping_sub(1);
            cc += thumb_write_pc(bus);
        }
    }
    bus.cpu().cycles += cc;
}

/// Format 4: ALU operations (0x100-0x10F)
fn data_processing<B: ArmBus>(bus: &mut B, opcode: u32, cc: &mut i32) {
    let rd = (opcode & 7) as usize;
    let rn = ((opcode >> 3) & 7) as usize;
    let op = (opcode >> 6) & 0xF;
    if op == 0xD {
        // MUL
        let w = wait_smul(bus.cpu_ref().gprs[rd], 0);
        *cc += bus.stall(w);
        let cpu = bus.cpu();
        cpu.gprs[rd] = cpu.gprs[rd].wrapping_mul(cpu.gprs[rn]);
        let d = cpu.gprs[rd];
        neutral_s(cpu, d);
        *cc += cpu.memory.active_nonseq_cycles16 - cpu.memory.active_seq_cycles16;
        return;
    }
    let cpu = bus.cpu();
    let d = cpu.gprs[rd];
    let n = cpu.gprs[rn];
    match op {
        0x0 => {
            cpu.gprs[rd] = d & n;
            neutral_s(cpu, d & n);
        }
        0x1 => {
            cpu.gprs[rd] = d ^ n;
            neutral_s(cpu, d ^ n);
        }
        0x2 => {
            // LSL2
            let rs = n & 0xFF;
            if rs != 0 {
                if rs < 32 {
                    cpu.cpsr.set_c((d >> (32 - rs)) & 1 != 0);
                    cpu.gprs[rd] = d << rs;
                } else {
                    if rs > 32 {
                        cpu.cpsr.set_c(false);
                    } else {
                        cpu.cpsr.set_c(d & 1 != 0);
                    }
                    cpu.gprs[rd] = 0;
                }
            }
            *cc += 1;
            let r = cpu.gprs[rd];
            neutral_s(cpu, r);
        }
        0x3 => {
            // LSR2
            let rs = n & 0xFF;
            if rs != 0 {
                if rs < 32 {
                    cpu.cpsr.set_c((d >> (rs - 1)) & 1 != 0);
                    cpu.gprs[rd] = d >> rs;
                } else {
                    if rs > 32 {
                        cpu.cpsr.set_c(false);
                    } else {
                        cpu.cpsr.set_c(sign(d));
                    }
                    cpu.gprs[rd] = 0;
                }
            }
            *cc += 1;
            let r = cpu.gprs[rd];
            neutral_s(cpu, r);
        }
        0x4 => {
            // ASR2
            let rs = n & 0xFF;
            if rs != 0 {
                if rs < 32 {
                    cpu.cpsr.set_c((d >> (rs - 1)) & 1 != 0);
                    cpu.gprs[rd] = ((d as i32) >> rs) as u32;
                } else {
                    cpu.cpsr.set_c(sign(d));
                    cpu.gprs[rd] = if sign(d) { 0xFFFFFFFF } else { 0 };
                }
            }
            *cc += 1;
            let r = cpu.gprs[rd];
            neutral_s(cpu, r);
        }
        0x5 => {
            // ADC
            let r = d.wrapping_add(n).wrapping_add(cpu.cpsr.c() as u32);
            cpu.gprs[rd] = r;
            addition_s(cpu, d, n, r);
        }
        0x6 => {
            // SBC
            let c = (!cpu.cpsr.c()) as u32;
            let r = d.wrapping_sub(n).wrapping_sub(c);
            cpu.gprs[rd] = r;
            cpu.cpsr.set_n(sign(r));
            cpu.cpsr.set_z(r == 0);
            cpu.cpsr.set_c(borrow_from_carry(d, n, c));
            cpu.cpsr.set_v(v_subtraction(d, n, r));
        }
        0x7 => {
            // ROR
            let rs = n & 0xFF;
            if rs != 0 {
                let r4 = rs & 0x1F;
                if r4 > 0 {
                    cpu.cpsr.set_c((d >> (r4 - 1)) & 1 != 0);
                    cpu.gprs[rd] = d.rotate_right(r4);
                } else {
                    cpu.cpsr.set_c(sign(d));
                }
            }
            *cc += 1;
            let r = cpu.gprs[rd];
            neutral_s(cpu, r);
        }
        0x8 => neutral_s(cpu, d & n), // TST
        0x9 => sub(cpu, rd, 0, n),    // NEG
        0xA => {
            // CMP2
            let r = d.wrapping_sub(n);
            subtraction_s(cpu, d, n, r);
        }
        0xB => {
            // CMN
            let r = d.wrapping_add(n);
            addition_s(cpu, d, n, r);
        }
        0xC => {
            cpu.gprs[rd] = d | n;
            neutral_s(cpu, d | n);
        }
        0xE => {
            cpu.gprs[rd] = d & !n;
            neutral_s(cpu, d & !n);
        }
        _ => {
            cpu.gprs[rd] = !n;
            neutral_s(cpu, !n);
        }
    }
}

/// Format 5: hi register operations / BX (0x110-0x11F)
fn high_register<B: ArmBus>(bus: &mut B, opcode: u32, cc: &mut i32) {
    let index = opcode >> 6;
    let h1 = if index & 2 != 0 { 8 } else { 0 };
    let h2 = if index & 1 != 0 { 8 } else { 0 };
    let rd = ((opcode & 7) | h1) as usize;
    let rm = (((opcode >> 3) & 7) | h2) as usize;
    match (index >> 2) & 3 {
        0 => {
            // ADD4
            let cpu = bus.cpu();
            cpu.gprs[rd] = cpu.gprs[rd].wrapping_add(cpu.gprs[rm]);
            if rd == ARM_PC {
                *cc += thumb_write_pc(bus);
            }
        }
        1 => {
            // CMP3
            let cpu = bus.cpu();
            let (d, m) = (cpu.gprs[rd], cpu.gprs[rm]);
            let r = d.wrapping_sub(m);
            subtraction_s(cpu, d, m, r);
        }
        2 => {
            // MOV3
            let cpu = bus.cpu();
            cpu.gprs[rd] = cpu.gprs[rm];
            if rd == ARM_PC {
                *cc += thumb_write_pc(bus);
            }
        }
        _ => {
            if index & 2 != 0 {
                bus.hit_illegal(opcode);
            } else {
                // BX
                let rm = ((opcode >> 3) & 0xF) as usize;
                let cpu = bus.cpu();
                let target = cpu.gprs[rm];
                cpu.set_execution_mode(if target & 1 != 0 { ExecutionMode::Thumb } else { ExecutionMode::Arm });
                cpu.gprs[ARM_PC] = target & 0xFFFFFFFE;
                *cc += write_pc(bus);
            }
        }
    }
}

/// 0x2C0-0x2FF: ADD7/SUB4, PUSH/POP, BKPT
fn misc<B: ArmBus>(bus: &mut B, opcode: u32, index: u32, cc: &mut i32) {
    match index {
        0x2C0 | 0x2C1 => {
            let cpu = bus.cpu();
            cpu.gprs[ARM_SP] = cpu.gprs[ARM_SP].wrapping_add((opcode & 0x7F) << 2);
        }
        0x2C2 | 0x2C3 => {
            let cpu = bus.cpu();
            cpu.gprs[ARM_SP] = cpu.gprs[ARM_SP].wrapping_sub((opcode & 0x7F) << 2);
        }
        0x2D0..=0x2D3 | 0x2D4..=0x2D7 => {
            // PUSH / PUSHR
            let mut rs = opcode & 0xFF;
            if index >= 0x2D4 {
                rs |= 1 << ARM_LR;
            }
            let address = bus.cpu_ref().gprs[ARM_SP];
            let address = bus.store_multiple(address, rs, LSM_DB, cc);
            load_store_post(bus, cc);
            bus.cpu().gprs[ARM_SP] = address;
        }
        0x2F0..=0x2F3 | 0x2F4..=0x2F7 => {
            // POP / POPR
            let mut rs = opcode & 0xFF;
            let with_pc = index >= 0x2F4;
            if with_pc {
                rs |= 1 << ARM_PC;
            }
            let address = bus.cpu_ref().gprs[ARM_SP];
            let address = bus.load_multiple(address, rs, LSM_IA, cc);
            load_store_post(bus, cc);
            bus.cpu().gprs[ARM_SP] = address;
            if with_pc {
                *cc += thumb_write_pc(bus);
            }
        }
        0x2F8..=0x2FB => {
            bus.bkpt16(opcode & 0xFF);
            *cc = 0;
        }
        _ => bus.hit_illegal(opcode),
    }
}
