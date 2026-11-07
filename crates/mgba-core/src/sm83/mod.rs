// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's SM83 (Game Boy CPU) core (src/sm83/sm83.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub mod isa;

pub use isa::Instr;

/// `enum SM83ExecutionState`. The low two bits are the T-cycle phase within
/// an M-cycle; the system code inspects them.
pub const CORE_IDLE_0: u8 = 0;
pub const CORE_IDLE_1: u8 = 1;
pub const CORE_EXECUTE: u8 = 2;
pub const CORE_FETCH: u8 = 3;
pub const CORE_MEMORY_LOAD: u8 = 7;
pub const CORE_MEMORY_STORE: u8 = 11;
pub const CORE_READ_PC: u8 = 15;
pub const CORE_STALL: u8 = 19;
pub const CORE_OP2: u8 = 23;
pub const CORE_HALT_BUG: u8 = 27;

pub const FLAG_Z: u8 = 0x80;
pub const FLAG_N: u8 = 0x40;
pub const FLAG_H: u8 = 0x20;
pub const FLAG_C: u8 = 0x10;

/// `struct SM83Core` (register file and micro-op state).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Sm83Core {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,

    pub index: u16,

    pub t_multiplier: i32,
    pub cycles: i32,
    pub next_event: i32,
    pub execution_state: u8,
    pub halted: bool,

    pub bus: u8,
    pub condition: bool,
    pub instruction: Instr,

    pub irq_pending: bool,
}

impl Default for Sm83Core {
    fn default() -> Self {
        Sm83Core {
            a: 0,
            f: 0,
            b: 0,
            c: 0,
            d: 0,
            e: 0,
            h: 0,
            l: 0,
            sp: 0,
            pc: 0,
            index: 0,
            t_multiplier: 2,
            cycles: 0,
            next_event: 0,
            execution_state: CORE_FETCH,
            halted: false,
            bus: 0,
            condition: false,
            instruction: Instr::Op(0),
            irq_pending: false,
        }
    }
}

macro_rules! flag {
    ($get:ident, $set:ident, $bit:expr) => {
        #[inline(always)]
        pub fn $get(&self) -> bool {
            self.f & $bit != 0
        }
        #[inline(always)]
        pub fn $set(&mut self, v: bool) {
            if v {
                self.f |= $bit;
            } else {
                self.f &= !$bit;
            }
        }
    };
}

impl Sm83Core {
    flag!(fz, set_fz, FLAG_Z);
    flag!(fn_, set_fn, FLAG_N);
    flag!(fh, set_fh, FLAG_H);
    flag!(fc, set_fc, FLAG_C);

    #[inline(always)]
    pub fn af(&self) -> u16 {
        ((self.a as u16) << 8) | self.f as u16
    }
    #[inline(always)]
    pub fn bc(&self) -> u16 {
        ((self.b as u16) << 8) | self.c as u16
    }
    #[inline(always)]
    pub fn de(&self) -> u16 {
        ((self.d as u16) << 8) | self.e as u16
    }
    #[inline(always)]
    pub fn hl(&self) -> u16 {
        ((self.h as u16) << 8) | self.l as u16
    }
    #[inline(always)]
    pub fn set_bc(&mut self, v: u16) {
        self.b = (v >> 8) as u8;
        self.c = v as u8;
    }
    #[inline(always)]
    pub fn set_de(&mut self, v: u16) {
        self.d = (v >> 8) as u8;
        self.e = v as u8;
    }
    #[inline(always)]
    pub fn set_hl(&mut self, v: u16) {
        self.h = (v >> 8) as u8;
        self.l = v as u8;
    }
}

/// The system side of the CPU: `struct SM83Memory` plus `struct SM83InterruptHandler`.
pub trait Sm83Bus {
    fn cpu(&mut self) -> &mut Sm83Core;
    /// `cpu->memory.cpuLoad8` (instruction/operand fetch)
    fn cpu_load8(&mut self, address: u16) -> u8;
    /// `cpu->memory.load8`
    fn load8(&mut self, address: u16) -> u8;
    /// `cpu->memory.store8`
    fn store8(&mut self, address: u16, value: u8);
    /// `cpu->memory.setActiveRegion`
    fn set_active_region(&mut self, address: u16);
    fn process_events(&mut self);
    fn set_interrupts(&mut self, enable: bool);
    fn irq_vector(&mut self) -> u16;
    fn halt(&mut self);
    fn stop(&mut self);
    fn hit_illegal(&mut self);
}

/// `SM83Reset` (without the `irqh.reset` call, which the owner does).
pub fn reset(cpu: &mut Sm83Core) {
    cpu.a = 0;
    cpu.f = 0;
    cpu.set_bc(0);
    cpu.set_de(0);
    cpu.set_hl(0);
    cpu.sp = 0;
    cpu.pc = 0;
    cpu.instruction = Instr::Op(0);
    cpu.t_multiplier = 2;
    cpu.cycles = 0;
    cpu.next_event = 0;
    cpu.execution_state = CORE_FETCH;
    cpu.halted = false;
    cpu.irq_pending = false;
}

/// `SM83RaiseIRQ`
#[inline]
pub fn raise_irq(cpu: &mut Sm83Core) {
    cpu.irq_pending = true;
}

/// `_SM83Step`
#[inline(always)]
fn step<B: Sm83Bus>(bus: &mut B) {
    let cpu = bus.cpu();
    cpu.cycles += cpu.t_multiplier;
    let state = cpu.execution_state;
    cpu.execution_state = CORE_IDLE_0;
    match state {
        CORE_FETCH => {
            if cpu.irq_pending {
                cpu.index = cpu.sp;
                cpu.irq_pending = false;
                cpu.instruction = Instr::Irq;
                bus.set_interrupts(false);
                return;
            }
            let pc = cpu.pc;
            let b = bus.cpu_load8(pc);
            let cpu = bus.cpu();
            cpu.bus = b;
            cpu.instruction = Instr::Op(b);
            cpu.pc = cpu.pc.wrapping_add(1);
        }
        CORE_MEMORY_LOAD => {
            let index = cpu.index;
            let b = bus.load8(index);
            bus.cpu().bus = b;
        }
        CORE_MEMORY_STORE => {
            let (index, b) = (cpu.index, cpu.bus);
            bus.store8(index, b);
        }
        CORE_READ_PC => {
            let pc = cpu.pc;
            let b = bus.cpu_load8(pc);
            let cpu = bus.cpu();
            cpu.bus = b;
            cpu.pc = cpu.pc.wrapping_add(1);
        }
        CORE_STALL => {
            cpu.instruction = Instr::Op(0);
        }
        CORE_HALT_BUG => {
            if cpu.irq_pending {
                cpu.index = cpu.sp;
                cpu.irq_pending = false;
                cpu.instruction = Instr::Irq;
                bus.set_interrupts(false);
                return;
            }
            let pc = cpu.pc;
            let b = bus.cpu_load8(pc);
            let cpu = bus.cpu();
            cpu.bus = b;
            cpu.instruction = Instr::Op(b);
        }
        _ => {}
    }
}

/// `_SM83TickInternal`
#[inline(always)]
fn tick_internal<B: Sm83Bus>(bus: &mut B) -> bool {
    let mut running = true;
    step(bus);
    let cpu = bus.cpu();
    let t = cpu.t_multiplier;
    if cpu.cycles + t * 2 >= cpu.next_event {
        if cpu.cycles >= cpu.next_event {
            bus.process_events();
        }
        let cpu = bus.cpu();
        cpu.cycles += t;
        cpu.execution_state = cpu.execution_state.wrapping_add(1);
        if cpu.cycles >= cpu.next_event {
            bus.process_events();
        }
        let cpu = bus.cpu();
        cpu.cycles += t;
        cpu.execution_state = cpu.execution_state.wrapping_add(1);
        if cpu.cycles >= cpu.next_event {
            bus.process_events();
        }
        running = false;
    } else {
        cpu.cycles += t * 2;
    }
    let cpu = bus.cpu();
    cpu.execution_state = CORE_FETCH;
    let instr = cpu.instruction;
    isa::execute(bus, instr);
    let cpu = bus.cpu();
    cpu.cycles += cpu.t_multiplier;
    running
}

/// `SM83Tick`
pub fn tick<B: Sm83Bus>(bus: &mut B) {
    while bus.cpu().cycles >= bus.cpu().next_event {
        bus.process_events();
    }
    tick_internal(bus);
    while bus.cpu().cycles >= bus.cpu().next_event {
        bus.process_events();
    }
}

/// `SM83Run`
pub fn run<B: Sm83Bus>(bus: &mut B) {
    let mut running = true;
    while running || bus.cpu().execution_state != CORE_FETCH {
        if bus.cpu().cycles < bus.cpu().next_event {
            running = tick_internal(bus) && running;
        } else {
            bus.process_events();
            running = false;
        }
    }
}
