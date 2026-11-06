// Copyright (c) 2013-2014 Jeffrey Pfau
// Rust port of mGBA's ARM7TDMI core (src/arm/arm.c, isa-inlines.h).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub mod isa_arm;
pub mod isa_thumb;

pub const ARM_SP: usize = 13;
pub const ARM_LR: usize = 14;
pub const ARM_PC: usize = 15;

pub const WORD_SIZE_ARM: u32 = 4;
pub const WORD_SIZE_THUMB: u32 = 2;

pub const BASE_RESET: u32 = 0x00000000;
pub const BASE_UNDEF: u32 = 0x00000004;
pub const BASE_SWI: u32 = 0x00000008;
pub const BASE_PABT: u32 = 0x0000000C;
pub const BASE_DABT: u32 = 0x00000010;
pub const BASE_IRQ: u32 = 0x00000018;
pub const BASE_FIQ: u32 = 0x0000001C;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExecutionMode {
    Arm = 0,
    Thumb = 1,
}

pub const MODE_USER: u32 = 0x10;
pub const MODE_FIQ: u32 = 0x11;
pub const MODE_IRQ: u32 = 0x12;
pub const MODE_SUPERVISOR: u32 = 0x13;
pub const MODE_ABORT: u32 = 0x17;
pub const MODE_UNDEFINED: u32 = 0x1B;
pub const MODE_SYSTEM: u32 = 0x1F;

pub const BANK_NONE: usize = 0;
pub const BANK_FIQ: usize = 1;
pub const BANK_IRQ: usize = 2;
pub const BANK_SUPERVISOR: usize = 3;
pub const BANK_ABORT: usize = 4;
pub const BANK_UNDEFINED: usize = 5;

/// Load/store-multiple direction bits.
pub const LSM_B: u32 = 1;
pub const LSM_D: u32 = 2;
pub const LSM_IA: u32 = 0;
pub const LSM_IB: u32 = 1;
pub const LSM_DA: u32 = 2;
pub const LSM_DB: u32 = 3;

/// Program status register (CPSR/SPSR), stored packed.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Psr(pub u32);

macro_rules! psr_bit {
    ($get:ident, $set:ident, $bit:expr) => {
        #[inline(always)]
        pub fn $get(self) -> bool {
            (self.0 >> $bit) & 1 != 0
        }
        #[inline(always)]
        pub fn $set(&mut self, v: bool) {
            self.0 = (self.0 & !(1 << $bit)) | ((v as u32) << $bit);
        }
    };
}

impl Psr {
    psr_bit!(n, set_n, 31);
    psr_bit!(z, set_z, 30);
    psr_bit!(c, set_c, 29);
    psr_bit!(v, set_v, 28);
    psr_bit!(i, set_i, 7);
    psr_bit!(f, set_f, 6);
    psr_bit!(t, set_t, 5);

    #[inline(always)]
    pub fn priv_mode(self) -> u32 {
        self.0 & 0x1F
    }
    #[inline(always)]
    pub fn set_priv(&mut self, mode: u32) {
        self.0 = (self.0 & !0x1F) | (mode & 0x1F);
    }
    /// The top byte (NZCV + unused).
    #[inline(always)]
    pub fn flags(self) -> u32 {
        self.0 >> 24
    }
    #[inline(always)]
    pub fn set_flags(&mut self, flags: u8) {
        self.0 = (self.0 & 0x00FF_FFFF) | ((flags as u32) << 24);
    }
}

/// Per-region timing and fetch information maintained by the bus
/// (mirror of the bookkeeping parts of `struct ARMMemory`).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, Debug)]
pub struct ArmMemoryInfo {
    pub active_mask: u32,
    pub active_seq_cycles32: i32,
    pub active_seq_cycles16: i32,
    pub active_nonseq_cycles32: i32,
    pub active_nonseq_cycles16: i32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ArmCore {
    pub gprs: [u32; 16],
    pub cpsr: Psr,
    pub spsr: Psr,

    pub cycles: i32,
    pub next_event: i32,
    pub halted: bool,

    pub banked_registers: [[u32; 7]; 6],
    pub banked_spsrs: [u32; 6],

    pub shifter_operand: u32,
    pub shifter_carry_out: u32,

    pub prefetch: [u32; 2],
    pub execution_mode: ExecutionMode,
    pub privilege_mode: u32,

    pub memory: ArmMemoryInfo,
}

impl Default for ArmCore {
    fn default() -> Self {
        ArmCore {
            gprs: [0; 16],
            cpsr: Psr(MODE_SYSTEM),
            spsr: Psr(0),
            cycles: 0,
            next_event: 0,
            halted: false,
            banked_registers: [[0; 7]; 6],
            banked_spsrs: [0; 6],
            shifter_operand: 0,
            shifter_carry_out: 0,
            prefetch: [0; 2],
            execution_mode: ExecutionMode::Arm,
            privilege_mode: MODE_SYSTEM,
            memory: ArmMemoryInfo::default(),
        }
    }
}

#[inline(always)]
pub fn select_bank(mode: u32) -> usize {
    match mode {
        MODE_USER | MODE_SYSTEM => BANK_NONE,
        MODE_FIQ => BANK_FIQ,
        MODE_IRQ => BANK_IRQ,
        MODE_SUPERVISOR => BANK_SUPERVISOR,
        MODE_ABORT => BANK_ABORT,
        MODE_UNDEFINED => BANK_UNDEFINED,
        _ => BANK_NONE,
    }
}

#[inline(always)]
pub fn mode_has_spsr(mode: u32) -> bool {
    mode != MODE_SYSTEM && mode != MODE_USER
}

impl ArmCore {
    pub fn set_privilege_mode(&mut self, mode: u32) {
        if mode == self.privilege_mode {
            return;
        }
        let new_bank = select_bank(mode);
        let old_bank = select_bank(self.privilege_mode);
        if new_bank != old_bank {
            if mode == MODE_FIQ || self.privilege_mode == MODE_FIQ {
                let old_fiq = (old_bank == BANK_FIQ) as usize;
                let new_fiq = (new_bank == BANK_FIQ) as usize;
                for i in 0..5 {
                    self.banked_registers[old_fiq][2 + i] = self.gprs[8 + i];
                }
                for i in 0..5 {
                    self.gprs[8 + i] = self.banked_registers[new_fiq][2 + i];
                }
            }
            self.banked_registers[old_bank][0] = self.gprs[ARM_SP];
            self.banked_registers[old_bank][1] = self.gprs[ARM_LR];
            self.gprs[ARM_SP] = self.banked_registers[new_bank][0];
            self.gprs[ARM_LR] = self.banked_registers[new_bank][1];

            self.banked_spsrs[old_bank] = self.spsr.0;
            self.spsr = Psr(self.banked_spsrs[new_bank]);
        }
        self.privilege_mode = mode;
    }

    /// `_ARMSetMode`
    #[inline]
    pub fn set_execution_mode(&mut self, mode: ExecutionMode) {
        if mode == self.execution_mode {
            return;
        }
        self.execution_mode = mode;
        match mode {
            ExecutionMode::Arm => {
                self.cpsr.set_t(false);
                self.memory.active_mask &= !2;
            }
            ExecutionMode::Thumb => {
                self.cpsr.set_t(true);
                self.memory.active_mask |= 2;
            }
        }
        self.next_event = self.cycles;
    }

    #[inline(always)]
    pub fn instruction_length(&self) -> u32 {
        if self.cpsr.t() {
            WORD_SIZE_THUMB
        } else {
            WORD_SIZE_ARM
        }
    }

    /// `_ARMPCAddress`: address of the currently executing instruction.
    #[inline(always)]
    pub fn pc_address(&self) -> u32 {
        self.gprs[ARM_PC].wrapping_sub(self.instruction_length() * 2)
    }

    #[inline(always)]
    pub fn arm_prefetch_cycles(&self) -> i32 {
        1 + self.memory.active_seq_cycles32
    }

    #[inline(always)]
    pub fn thumb_prefetch_cycles(&self) -> i32 {
        1 + self.memory.active_seq_cycles16
    }

    pub fn test_condition(&self, condition: u32) -> bool {
        let c = self.cpsr;
        match condition {
            0x0 => c.z(),
            0x1 => !c.z(),
            0x2 => c.c(),
            0x3 => !c.c(),
            0x4 => c.n(),
            0x5 => !c.n(),
            0x6 => c.v(),
            0x7 => !c.v(),
            0x8 => c.c() && !c.z(),
            0x9 => !c.c() || c.z(),
            0xA => c.n() == c.v(),
            0xB => c.n() != c.v(),
            0xC => !c.z() && c.n() == c.v(),
            0xD => c.z() || c.n() != c.v(),
            _ => true,
        }
    }
}

/// The system the CPU is attached to (mGBA's `struct ARMMemory` callbacks,
/// `struct ARMInterruptHandler` and coprocessor hooks, rolled into one trait).
pub trait ArmBus: Sized {
    fn cpu(&mut self) -> &mut ArmCore;
    fn cpu_ref(&self) -> &ArmCore;

    fn load32(&mut self, address: u32, cycle_counter: Option<&mut i32>) -> u32;
    fn load16(&mut self, address: u32, cycle_counter: Option<&mut i32>) -> u32;
    fn load8(&mut self, address: u32, cycle_counter: Option<&mut i32>) -> u32;
    fn store32(&mut self, address: u32, value: u32, cycle_counter: Option<&mut i32>);
    fn store16(&mut self, address: u32, value: u16, cycle_counter: Option<&mut i32>);
    fn store8(&mut self, address: u32, value: u8, cycle_counter: Option<&mut i32>);
    fn load_multiple(&mut self, address: u32, mask: u32, direction: u32, cycle_counter: &mut i32) -> u32;
    fn store_multiple(&mut self, address: u32, mask: u32, direction: u32, cycle_counter: &mut i32) -> u32;

    /// Fetch from the active region; `address` is already masked by `active_mask`.
    fn fetch32(&self, address: u32) -> u32;
    fn fetch16(&self, address: u32) -> u32;

    fn stall(&mut self, wait: i32) -> i32;
    fn set_active_region(&mut self, address: u32);

    fn process_events(&mut self);
    fn swi16(&mut self, immediate: u32);
    fn swi32(&mut self, immediate: u32);
    fn hit_illegal(&mut self, opcode: u32);
    fn hit_stub(&mut self, opcode: u32);
    fn bkpt16(&mut self, immediate: u32);
    fn bkpt32(&mut self, immediate: u32);
    fn read_cpsr(&mut self);

    fn cp_mrc(&mut self, _cp: u32, _crn: u32, _crm: u32, _op1: u32, _op2: u32) -> Option<u32> {
        None
    }
    fn cp_mcr(&mut self, _cp: u32, _crn: u32, _crm: u32, _op1: u32, _op2: u32, _value: u32) -> bool {
        false
    }
    fn cp_cdp(&mut self, _cp: u32, _crn: u32, _crm: u32, _crd: u32, _op1: u32, _op2: u32) -> bool {
        false
    }
}

/// `ARMWritePC`
#[inline]
pub fn arm_write_pc<B: ArmBus>(bus: &mut B) -> i32 {
    let pc = bus.cpu_ref().gprs[ARM_PC] & !(WORD_SIZE_THUMB - 1);
    bus.set_active_region(pc);
    let mask = bus.cpu_ref().memory.active_mask;
    let p0 = bus.fetch32(pc & mask);
    let p1 = bus.fetch32(pc.wrapping_add(WORD_SIZE_ARM) & mask);
    let cpu = bus.cpu();
    cpu.prefetch[0] = p0;
    cpu.prefetch[1] = p1;
    cpu.gprs[ARM_PC] = pc.wrapping_add(WORD_SIZE_ARM);
    2 + cpu.memory.active_nonseq_cycles32 + cpu.memory.active_seq_cycles32
}

/// `ThumbWritePC`
#[inline]
pub fn thumb_write_pc<B: ArmBus>(bus: &mut B) -> i32 {
    let pc = bus.cpu_ref().gprs[ARM_PC] & !(WORD_SIZE_THUMB - 1);
    bus.set_active_region(pc);
    let mask = bus.cpu_ref().memory.active_mask;
    let p0 = bus.fetch16(pc & mask);
    let p1 = bus.fetch16(pc.wrapping_add(WORD_SIZE_THUMB) & mask);
    let cpu = bus.cpu();
    cpu.prefetch[0] = p0;
    cpu.prefetch[1] = p1;
    cpu.gprs[ARM_PC] = pc.wrapping_add(WORD_SIZE_THUMB);
    2 + cpu.memory.active_nonseq_cycles16 + cpu.memory.active_seq_cycles16
}

/// Write PC in whatever the current execution mode is.
#[inline]
pub fn write_pc<B: ArmBus>(bus: &mut B) -> i32 {
    if bus.cpu_ref().execution_mode == ExecutionMode::Thumb {
        thumb_write_pc(bus)
    } else {
        arm_write_pc(bus)
    }
}

/// `_ARMReadCPSR`
#[inline]
pub fn read_cpsr<B: ArmBus>(bus: &mut B) {
    let cpu = bus.cpu();
    let mode = if cpu.cpsr.t() { ExecutionMode::Thumb } else { ExecutionMode::Arm };
    cpu.set_execution_mode(mode);
    let p = cpu.cpsr.priv_mode();
    cpu.set_privilege_mode(p);
    bus.read_cpsr();
}

pub fn reset<B: ArmBus>(bus: &mut B) {
    {
        let cpu = bus.cpu();
        cpu.gprs = [0; 16];
        cpu.banked_registers = [[0; 7]; 6];
        cpu.banked_spsrs = [0; 6];
        cpu.privilege_mode = MODE_SYSTEM;
        cpu.cpsr = Psr(MODE_SYSTEM);
        cpu.spsr = Psr(0);
        cpu.shifter_operand = 0;
        cpu.shifter_carry_out = 0;
        cpu.execution_mode = ExecutionMode::Thumb;
        cpu.set_execution_mode(ExecutionMode::Arm);
    }
    arm_write_pc(bus);
    let cpu = bus.cpu();
    cpu.cycles = 0;
    cpu.next_event = 0;
    cpu.halted = false;
}

fn raise_exception<B: ArmBus>(bus: &mut B, mode: u32, vector: u32, lr_offset: u32) {
    let cpu = bus.cpu();
    let cpsr = cpu.cpsr;
    let width = if cpu.execution_mode == ExecutionMode::Thumb { WORD_SIZE_THUMB } else { WORD_SIZE_ARM };
    cpu.set_privilege_mode(mode);
    cpu.cpsr.set_priv(mode);
    cpu.gprs[ARM_LR] = cpu.gprs[ARM_PC].wrapping_sub(width).wrapping_add(lr_offset);
    cpu.gprs[ARM_PC] = vector;
    cpu.set_execution_mode(ExecutionMode::Arm);
    let c = arm_write_pc(bus);
    let cpu = bus.cpu();
    cpu.cycles += c;
    cpu.spsr = cpsr;
    cpu.cpsr.set_i(true);
}

pub fn raise_irq<B: ArmBus>(bus: &mut B) {
    if bus.cpu_ref().cpsr.i() {
        return;
    }
    raise_exception(bus, MODE_IRQ, BASE_IRQ, WORD_SIZE_ARM);
    bus.cpu().halted = false;
}

pub fn raise_swi<B: ArmBus>(bus: &mut B) {
    raise_exception(bus, MODE_SUPERVISOR, BASE_SWI, 0);
}

pub fn raise_undefined<B: ArmBus>(bus: &mut B) {
    raise_exception(bus, MODE_UNDEFINED, BASE_UNDEF, 0);
}

const CONDITION_LUT: [u16; 16] = [
    0xF0F0, // EQ
    0x0F0F, // NE
    0xCCCC, // CS
    0x3333, // CC
    0xFF00, // MI
    0x00FF, // PL
    0xAAAA, // VS
    0x5555, // VC
    0x0C0C, // HI
    0xF3F3, // LS
    0xAA55, // GE
    0x55AA, // LT
    0x0A05, // GT
    0xF5FA, // LE
    0xFFFF, // AL
    0x0000, // NV
];

#[inline(always)]
fn arm_step<B: ArmBus>(bus: &mut B) {
    let cpu = bus.cpu();
    let opcode = cpu.prefetch[0];
    cpu.prefetch[0] = cpu.prefetch[1];
    cpu.gprs[ARM_PC] = cpu.gprs[ARM_PC].wrapping_add(WORD_SIZE_ARM);
    let addr = cpu.gprs[ARM_PC] & cpu.memory.active_mask;
    let next = bus.fetch32(addr);
    let cpu = bus.cpu();
    cpu.prefetch[1] = next;

    let condition = opcode >> 28;
    if condition != 0xE {
        let flags = cpu.cpsr.flags() >> 4;
        if CONDITION_LUT[condition as usize] & (1 << flags) == 0 {
            cpu.cycles += cpu.arm_prefetch_cycles();
            return;
        }
    }
    isa_arm::execute(bus, opcode);
}

#[inline(always)]
fn thumb_step<B: ArmBus>(bus: &mut B) {
    let cpu = bus.cpu();
    let opcode = cpu.prefetch[0];
    cpu.prefetch[0] = cpu.prefetch[1];
    cpu.gprs[ARM_PC] = cpu.gprs[ARM_PC].wrapping_add(WORD_SIZE_THUMB);
    let addr = cpu.gprs[ARM_PC] & cpu.memory.active_mask;
    let next = bus.fetch16(addr);
    bus.cpu().prefetch[1] = next;
    isa_thumb::execute(bus, opcode);
}

/// `ARMRun`: execute a single instruction.
pub fn run<B: ArmBus>(bus: &mut B) {
    while bus.cpu_ref().cycles >= bus.cpu_ref().next_event {
        bus.process_events();
    }
    if bus.cpu_ref().execution_mode == ExecutionMode::Thumb {
        thumb_step(bus);
    } else {
        arm_step(bus);
    }
    while bus.cpu_ref().cycles >= bus.cpu_ref().next_event {
        bus.process_events();
    }
}

/// `ARMRunLoop`: execute until the next event, then process events.
pub fn run_loop<B: ArmBus>(bus: &mut B) {
    if bus.cpu_ref().execution_mode == ExecutionMode::Thumb {
        while bus.cpu_ref().cycles < bus.cpu_ref().next_event {
            thumb_step(bus);
        }
    } else {
        while bus.cpu_ref().cycles < bus.cpu_ref().next_event {
            arm_step(bus);
        }
    }
    bus.process_events();
}

/// `ARMRunFake`: inject an opcode into the pipeline.
pub fn run_fake<B: ArmBus>(bus: &mut B, opcode: u32) {
    let cpu = bus.cpu();
    if cpu.execution_mode == ExecutionMode::Arm {
        cpu.gprs[ARM_PC] = cpu.gprs[ARM_PC].wrapping_sub(WORD_SIZE_ARM);
    } else {
        cpu.gprs[ARM_PC] = cpu.gprs[ARM_PC].wrapping_sub(WORD_SIZE_THUMB);
    }
    cpu.prefetch[1] = cpu.prefetch[0];
    cpu.prefetch[0] = opcode;
}

// Shared flag helpers (isa-inlines.h)

#[inline(always)]
pub(crate) fn sign(x: u32) -> bool {
    (x >> 31) != 0
}

#[inline(always)]
pub(crate) fn carry_from(m: u32, n: u32, d: u32) -> bool {
    (m >> 31) + (n >> 31) > (d >> 31)
}

#[inline(always)]
pub(crate) fn borrow_from(m: u32, n: u32) -> bool {
    m >= n
}

#[inline(always)]
pub(crate) fn borrow_from_carry(m: u32, n: u32, c: u32) -> bool {
    (m as u64) >= (n as u64) + (c as u64)
}

#[inline(always)]
pub(crate) fn v_addition(m: u32, n: u32, d: u32) -> bool {
    !sign(m ^ n) && sign(m ^ d)
}

#[inline(always)]
pub(crate) fn v_subtraction(m: u32, n: u32, d: u32) -> bool {
    sign(m ^ n) && sign(m ^ d)
}

#[inline(always)]
pub(crate) fn wait_smul(r: u32, wait: i32) -> i32 {
    if (r & 0xFFFFFF00) == 0xFFFFFF00 || (r & 0xFFFFFF00) == 0 {
        wait + 1
    } else if (r & 0xFFFF0000) == 0xFFFF0000 || (r & 0xFFFF0000) == 0 {
        wait + 2
    } else if (r & 0xFF000000) == 0xFF000000 || (r & 0xFF000000) == 0 {
        wait + 3
    } else {
        wait + 4
    }
}

#[inline(always)]
pub(crate) fn wait_umul(r: u32, wait: i32) -> i32 {
    if (r & 0xFFFFFF00) == 0 {
        wait + 1
    } else if (r & 0xFFFF0000) == 0 {
        wait + 2
    } else if (r & 0xFF000000) == 0 {
        wait + 3
    } else {
        wait + 4
    }
}
