// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's debugger core (src/debugger/debugger.c,
// include/mgba/debugger/debugger.h) and the breakpoint/watchpoint handling of
// its ARM and SM83 platforms (src/arm/debugger/debugger.c,
// src/arm/debugger/memory-debugger.c, src/sm83/debugger/debugger.c,
// src/sm83/debugger/memory-debugger.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! mGBA attaches the debugger to the CPU as a component and swaps the CPU's
//! memory functions for watchpoint shims. Here a core carries a [`Platform`]
//! while a debugger is attached; its memory accessors check the watchpoints
//! in it, and anything that should stop execution is queued in it for the
//! [`Debugger`] to act on once the current instruction finishes.

pub mod cli;
pub mod gdb;
pub mod parser;
pub mod symbols;

use std::collections::VecDeque;
use std::sync::Arc;

use crate::core::MCore;
use parser::{EvalEnv, ParseTree};
use symbols::Symbols;

/// The `BKPT` immediate the debugger plants (`CPU_COMPONENT_DEBUGGER`).
pub const DEBUGGER_BKPT: u32 = 0;

/// `enum mWatchpointType`
pub const WATCHPOINT_WRITE: u32 = 1;
pub const WATCHPOINT_READ: u32 = 2;
pub const WATCHPOINT_RW: u32 = 3;
pub const WATCHPOINT_CHANGE: u32 = 4;
pub const WATCHPOINT_WRITE_CHANGE: u32 = 5;

/// `enum mBreakpointType`
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BreakpointType {
    #[default]
    Hardware,
    Software,
}

/// `enum mDebuggerEntryReason`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryReason {
    Manual,
    Attached,
    Breakpoint,
    Watchpoint,
    IllegalOp,
    Stack,
}

/// `struct mDebuggerEntryInfo`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntryInfo {
    pub address: u32,
    pub segment: i32,
    pub width: i32,
    pub old_value: u32,
    pub new_value: u32,
    pub watch_type: u32,
    pub access_type: u32,
    pub opcode: u32,
    pub break_type: BreakpointType,
    pub point_id: isize,
}

impl EntryInfo {
    /// A `BKPT` the debugger planted was hit at `address`.
    pub fn software_breakpoint(address: u32) -> EntryInfo {
        EntryInfo { address, break_type: BreakpointType::Software, point_id: -1, segment: -1, ..Default::default() }
    }
}

/// `struct mBreakpoint`
#[derive(Clone, Debug)]
pub struct Breakpoint {
    pub id: isize,
    pub address: u32,
    pub segment: i32,
    pub ty: BreakpointType,
    pub condition: Option<ParseTree>,
    pub disabled: bool,
    pub is_temporary: bool,
}

impl Breakpoint {
    pub fn new(address: u32, segment: i32) -> Breakpoint {
        Breakpoint {
            id: 0,
            address,
            segment,
            ty: BreakpointType::Hardware,
            condition: None,
            disabled: false,
            is_temporary: false,
        }
    }
}

/// `struct mWatchpoint`
#[derive(Clone, Debug)]
pub struct Watchpoint {
    pub id: isize,
    pub segment: i32,
    pub min_address: u32,
    pub max_address: u32,
    pub ty: u32,
    pub condition: Option<ParseTree>,
    pub disabled: bool,
}

impl Watchpoint {
    pub fn new(min_address: u32, max_address: u32, segment: i32, ty: u32) -> Watchpoint {
        Watchpoint { id: 0, segment, min_address, max_address, ty, condition: None, disabled: false }
    }
}

/// A software breakpoint (`struct ARMDebugBreakpoint` with its `sw` part).
#[derive(Clone, Debug)]
pub struct SoftwareBreakpoint {
    pub d: Breakpoint,
    pub opcode: u32,
    pub thumb: bool,
}

/// The per-core debugger state: `struct ARMDebugger` / `struct SM83Debugger`
/// minus the CPU pointer.
#[derive(Debug, Clone)]
pub struct Platform {
    pub breakpoints: Vec<Breakpoint>,
    pub sw_breakpoints: Vec<SoftwareBreakpoint>,
    pub watchpoints: Vec<Watchpoint>,
    pub next_id: isize,
    bp_bloom: [u64; 4],
    /// Entries raised while an instruction ran, waiting for `mDebuggerEnter`.
    pub pending: VecDeque<(EntryReason, EntryInfo)>,
    pub symbols: Option<Arc<Symbols>>,
}

impl Default for Platform {
    fn default() -> Self {
        Platform::new()
    }
}

impl Platform {
    /// `ARMDebuggerInit` / `SM83DebuggerInit`
    pub fn new() -> Platform {
        Platform {
            breakpoints: Vec::new(),
            sw_breakpoints: Vec::new(),
            watchpoints: Vec::new(),
            next_id: 1,
            bp_bloom: [0; 4],
            pending: VecDeque::new(),
            symbols: None,
        }
    }

    /// `_rebuildBpBloom`
    fn rebuild_bp_bloom(&mut self) {
        self.bp_bloom = [0; 4];
        for bp in &self.breakpoints {
            if bp.disabled {
                continue;
            }
            for j in 0..4 {
                self.bp_bloom[j] |= 1u64 << ((bp.address >> (4 * j + 1)) & 0x3F);
            }
        }
    }

    /// `_checkBpBloom`
    fn check_bp_bloom(&self, address: u32) -> bool {
        (0..4).all(|i| self.bp_bloom[i] & (1u64 << ((address >> (4 * i + 1)) & 0x3F)) != 0)
    }

    /// `ARMDebuggerSetBreakpoint` / `SM83DebuggerSetBreakpoint`. ARM clears
    /// the Thumb bit; pass `arm` to do so.
    pub fn set_breakpoint(&mut self, info: Breakpoint, arm: bool) -> isize {
        let id = self.next_id;
        self.next_id += 1;
        let mut bp = info;
        if arm {
            // Clear Thumb bit since it's not part of a valid address
            bp.address &= !1;
        }
        bp.id = id;
        self.breakpoints.push(bp);
        self.rebuild_bp_bloom();
        id
    }

    /// `ARMDebuggerSetWatchpoint` / `SM83DebuggerSetWatchpoint`
    pub fn set_watchpoint(&mut self, info: Watchpoint) -> isize {
        let id = self.next_id;
        self.next_id += 1;
        let mut wp = info;
        wp.id = id;
        self.watchpoints.push(wp);
        id
    }

    /// `ARMDebuggerToggleBreakpoint`
    pub fn toggle_breakpoint(&mut self, id: isize, status: bool) -> bool {
        if let Some(bp) = self.breakpoints.iter_mut().find(|b| b.id == id) {
            bp.disabled = !status;
            self.rebuild_bp_bloom();
            return true;
        }
        if let Some(bp) = self.sw_breakpoints.iter_mut().find(|b| b.d.id == id) {
            bp.d.disabled = !status;
            return true;
        }
        if let Some(wp) = self.watchpoints.iter_mut().find(|w| w.id == id) {
            wp.disabled = !status;
            return true;
        }
        false
    }

    /// `ARMDebuggerListBreakpoints`: hardware and software breakpoints in id order.
    pub fn list_breakpoints(&self) -> Vec<Breakpoint> {
        let mut list: Vec<Breakpoint> = self.breakpoints.clone();
        list.extend(self.sw_breakpoints.iter().map(|b| b.d.clone()));
        list.sort_by_key(|b| b.id);
        list
    }

    /// `ARMDebuggerHasBreakpoints`
    pub fn has_breakpoints(&self) -> bool {
        !self.breakpoints.is_empty() || !self.watchpoints.is_empty()
    }

    pub fn has_watchpoints(&self) -> bool {
        !self.watchpoints.is_empty()
    }

    /// Queue a debugger entry for after the current instruction.
    pub fn enter(&mut self, reason: EntryReason, info: EntryInfo) {
        self.pending.push_back((reason, info));
    }
}

/// Expression environment over a core: `mDebuggerLookupIdentifier` and memory reads.
pub struct CoreEnv<'a> {
    pub core: &'a mut dyn MCore,
    pub symbols: Option<Arc<Symbols>>,
}

impl EvalEnv for CoreEnv<'_> {
    /// `mDebuggerLookupIdentifier`
    fn lookup(&mut self, name: &str) -> Option<(i32, i32)> {
        if let Some(s) = &self.symbols {
            if let Some(v) = s.lookup(name) {
                return Some(v);
            }
        }
        if let Some(v) = self.core.lookup_identifier(name) {
            return Some(v);
        }
        self.core.read_register(name).map(|v| (v, -1))
    }

    fn read8(&mut self, address: u32, segment: i32) -> u32 {
        if segment < 0 {
            self.core.bus_read8(address)
        } else {
            self.core.raw_read8(address, segment)
        }
    }
}

/// Evaluate a breakpoint or watchpoint condition: true when it holds.
pub fn condition_holds(core: &mut dyn MCore, symbols: Option<Arc<Symbols>>, cond: &ParseTree) -> bool {
    let mut env = CoreEnv { core, symbols };
    match cond.evaluate(&mut env) {
        Some((value, segment)) => value != 0 || segment >= 0,
        None => false,
    }
}

/// `ARMDebuggerCheckBreakpoints` / `SM83DebuggerCheckBreakpoints` for the
/// instruction about to run.
pub fn check_breakpoints(core: &mut dyn MCore) {
    let Some(mut platform) = core.debug_platform().take() else { return };
    let pc = core.instruction_address();
    if platform.breakpoints.len() > 3 && !platform.check_bp_bloom(pc) {
        *core.debug_platform() = Some(platform);
        return;
    }
    let segment = core.current_segment(pc);
    let mut i = 0;
    while i < platform.breakpoints.len() {
        let bp = &platform.breakpoints[i];
        if bp.address != pc || bp.disabled || (segment >= 0 && bp.segment >= 0 && bp.segment != segment) {
            i += 1;
            continue;
        }
        if let Some(cond) = bp.condition.clone() {
            if !condition_holds(core, platform.symbols.clone(), &cond) {
                i += 1;
                continue;
            }
        }
        let info = EntryInfo {
            address: bp.address,
            segment,
            point_id: bp.id,
            break_type: BreakpointType::Hardware,
            ..Default::default()
        };
        platform.enter(EntryReason::Breakpoint, info);
        if platform.breakpoints[i].is_temporary {
            platform.breakpoints.remove(i);
            platform.rebuild_bp_bloom();
            continue;
        }
        i += 1;
    }
    *core.debug_platform() = Some(platform);
}

/// `_checkWatchpoints`, called by a core's memory accessors while
/// watchpoints are set (the "memory shim"). Returns whether one was hit.
pub fn check_watchpoints(core: &mut dyn MCore, address: u32, access: u32, new_value: u32, width: u32) -> bool {
    let Some(mut platform) = core.debug_platform().take() else { return false };
    let before = platform.pending.len();
    let min_address = address & !(width - 1);
    let max_address = min_address.wrapping_add(width);
    let segment = core.current_segment(address);
    for i in 0..platform.watchpoints.len() {
        let wp = &platform.watchpoints[i];
        if wp.ty & access == 0 || !(wp.min_address < max_address && min_address < wp.max_address) {
            continue;
        }
        if segment >= 0 && wp.segment >= 0 && wp.segment != segment {
            continue;
        }
        if wp.disabled {
            continue;
        }
        if let Some(cond) = wp.condition.clone() {
            if !condition_holds(core, platform.symbols.clone(), &cond) {
                continue;
            }
        }
        // The original memory functions read the old value.
        let old = match width {
            1 => core.bus_read8(address),
            2 => core.bus_read16(address),
            _ => core.bus_read32(address),
        };
        let wp = &platform.watchpoints[i];
        if wp.ty & WATCHPOINT_CHANGE != 0 && new_value == old {
            continue;
        }
        let info = EntryInfo {
            old_value: old,
            new_value,
            watch_type: wp.ty,
            access_type: access,
            address,
            segment: segment.max(0),
            width: width as i32,
            point_id: wp.id,
            ..Default::default()
        };
        platform.enter(EntryReason::Watchpoint, info);
    }
    let hit = platform.pending.len() != before;
    *core.debug_platform() = Some(platform);
    hit
}

/// `enum mDebuggerState`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Created,
    Paused,
    Running,
    Callback,
    Shutdown,
}

/// What a module (`struct mDebuggerModule`) can do with the debugger it is
/// attached to while it runs.
pub struct Context<'a> {
    pub core: &'a mut dyn DebugCore,
    pub shutdown: bool,
}

/// `struct mDebuggerModule`: a debugger front end (CLI, GDB stub).
pub trait Module {
    fn is_paused(&self) -> bool;
    fn set_paused(&mut self, paused: bool);
    fn needs_callback(&self) -> bool {
        false
    }
    /// `init`
    fn init(&mut self, _ctx: &mut Context) {}
    /// `deinit`
    fn deinit(&mut self, _ctx: &mut Context) {}
    /// `paused`: talk to the user (or GDB) while stopped, for up to `timeout_ms`.
    fn paused(&mut self, ctx: &mut Context, timeout_ms: i32);
    /// `entered`
    fn entered(&mut self, ctx: &mut Context, reason: EntryReason, info: Option<&EntryInfo>);
    /// `custom`: called every step while `needs_callback` is set.
    fn custom(&mut self, _ctx: &mut Context) {}
    /// `update`
    fn update(&mut self, _ctx: &mut Context) {}
    /// `interrupt`
    fn interrupt(&mut self, _ctx: &mut Context) {}
}

/// The platform-specific debugger operations a core provides
/// (`mDebuggerPlatform` callbacks and the CLI system hooks).
pub trait DebugCore: MCore {
    /// `trace`: registers and the next instruction on one line.
    fn trace(&mut self) -> String;
    /// The CLI system's `printStatus`.
    fn print_status(&mut self) -> String;
    /// Disassemble `count` instructions from `address` (the current PC when
    /// `None`); `mode` forces ARM (`Some(false)`) or Thumb (`Some(true)`).
    fn disassemble(&mut self, address: Option<u32>, segment: i32, count: u32, mode: Option<bool>) -> Vec<String>;
    /// `ARMDebuggerSetSoftwareBreakpoint`; GBA only.
    fn set_software_breakpoint(&mut self, _address: u32, _thumb: bool) -> isize {
        -1
    }
    /// Remove a breakpoint or watchpoint by id (`clearBreakpoint`), undoing a
    /// software breakpoint's ROM patch.
    fn clear_breakpoint(&mut self, id: isize) -> bool;
    /// Name of the CPU architecture ("ARM", "SM83") and of the system ("GBA", "GB").
    fn platform_name(&self) -> &'static str;
    fn system_name(&self) -> &'static str;
    /// Bytes per register and register count GDB expects (`g` packet layout).
    fn as_mcore(&mut self) -> &mut dyn MCore;
}

/// `struct mDebugger`
pub struct Debugger {
    pub state: State,
    pub modules: Vec<Box<dyn Module>>,
    pub symbols: Option<Arc<Symbols>>,
}

impl Default for Debugger {
    fn default() -> Self {
        Debugger::new()
    }
}

impl Debugger {
    /// `mDebuggerInit`
    pub fn new() -> Debugger {
        Debugger { state: State::Created, modules: Vec::new(), symbols: None }
    }

    /// `mDebuggerAttachModule`
    pub fn attach_module(&mut self, module: Box<dyn Module>) {
        self.modules.push(module);
    }

    /// `mDebuggerAttach` + `_mDebuggerInit`: start debugging `core`.
    pub fn attach(&mut self, core: &mut dyn DebugCore) {
        let mut platform = Platform::new();
        platform.symbols = self.symbols.clone();
        *core.debug_platform() = Some(Box::new(platform));
        self.state = State::Running;
        let mut ctx = Context { core, shutdown: false };
        for m in self.modules.iter_mut() {
            m.init(&mut ctx);
        }
        if ctx.shutdown {
            self.state = State::Shutdown;
        }
    }

    /// `_mDebuggerDeinit`: stop debugging, removing software breakpoints.
    pub fn detach(&mut self, core: &mut dyn DebugCore) {
        self.state = State::Shutdown;
        let mut ctx = Context { core, shutdown: true };
        for m in self.modules.iter_mut() {
            m.deinit(&mut ctx);
        }
        let ids: Vec<isize> = core
            .debug_platform()
            .as_ref()
            .map(|p| p.sw_breakpoints.iter().rev().map(|b| b.d.id).collect())
            .unwrap_or_default();
        for id in ids {
            core.clear_breakpoint(id);
        }
        *core.debug_platform() = None;
    }

    /// Replace the symbol table (`core->symbolTable`).
    pub fn set_symbols(&mut self, core: &mut dyn DebugCore, symbols: Symbols) {
        let symbols = Arc::new(symbols);
        self.symbols = Some(symbols.clone());
        if let Some(p) = core.debug_platform() {
            p.symbols = Some(symbols);
        }
    }

    fn has_breakpoints(core: &mut dyn DebugCore) -> bool {
        core.debug_platform().as_ref().is_some_and(|p| p.has_breakpoints())
    }

    /// Deliver entries the core queued while it ran.
    fn deliver_pending(&mut self, core: &mut dyn DebugCore) {
        loop {
            let entry = core.debug_platform().as_mut().and_then(|p| p.pending.pop_front());
            let Some((reason, info)) = entry else { break };
            self.enter(core, reason, Some(info));
        }
    }

    /// `mDebuggerRunTimeout`
    pub fn run_timeout(&mut self, core: &mut dyn DebugCore, timeout_ms: i32) {
        match self.state {
            State::Running => {
                if !Self::has_breakpoints(core) {
                    core.run_loop();
                } else {
                    core.step();
                    check_breakpoints(core.as_mcore());
                }
                self.deliver_pending(core);
            }
            State::Callback => {
                core.step();
                check_breakpoints(core.as_mcore());
                self.deliver_pending(core);
                let mut ctx = Context { core, shutdown: false };
                for m in self.modules.iter_mut() {
                    if m.needs_callback() {
                        m.custom(&mut ctx);
                    }
                }
                if ctx.shutdown {
                    self.state = State::Shutdown;
                } else {
                    self.update_paused();
                }
            }
            State::Paused => {
                let mut any_paused = 0;
                let mut ctx = Context { core, shutdown: false };
                for m in self.modules.iter_mut() {
                    if m.is_paused() {
                        m.paused(&mut ctx, timeout_ms);
                        if m.is_paused() {
                            any_paused += 1;
                        }
                    } else if m.needs_callback() {
                        m.custom(&mut ctx);
                    }
                }
                if ctx.shutdown {
                    self.state = State::Shutdown;
                } else if self.state == State::Paused && any_paused == 0 {
                    self.update_paused();
                }
            }
            State::Created | State::Shutdown => {}
        }
    }

    /// `mDebuggerRun`
    pub fn run(&mut self, core: &mut dyn DebugCore) {
        self.run_timeout(core, 50);
    }

    /// `mDebuggerRunFrame`, stopping early while paused so a frontend can
    /// keep drawing (`timeout_ms` is passed to paused modules).
    pub fn run_frame(&mut self, core: &mut dyn DebugCore, timeout_ms: i32) {
        let frame = core.frame_counter();
        loop {
            self.run_timeout(core, timeout_ms);
            if core.frame_counter() != frame || matches!(self.state, State::Paused | State::Shutdown) {
                break;
            }
        }
    }

    /// `mDebuggerEnter`
    pub fn enter(&mut self, core: &mut dyn DebugCore, reason: EntryReason, info: Option<EntryInfo>) {
        let mut ctx = Context { core, shutdown: false };
        for m in self.modules.iter_mut() {
            m.set_paused(true);
            m.entered(&mut ctx, reason, info.as_ref());
        }
        if ctx.shutdown {
            self.state = State::Shutdown;
            return;
        }
        self.update_paused();
    }

    /// `mDebuggerInterrupt`
    pub fn interrupt(&mut self, core: &mut dyn DebugCore) {
        let mut ctx = Context { core, shutdown: false };
        for m in self.modules.iter_mut() {
            m.interrupt(&mut ctx);
        }
    }

    /// `mDebuggerUpdatePaused`
    pub fn update_paused(&mut self) {
        if self.state == State::Shutdown {
            return;
        }
        let any_paused = self.modules.iter().any(|m| m.is_paused());
        let any_callback = self.modules.iter().any(|m| m.needs_callback());
        self.state = if any_paused {
            State::Paused
        } else if any_callback {
            State::Callback
        } else {
            State::Running
        };
    }

    /// `mDebuggerShutdown`
    pub fn shutdown(&mut self) {
        self.state = State::Shutdown;
    }

    /// `mDebuggerIsShutdown`
    pub fn is_shutdown(&self) -> bool {
        self.state == State::Shutdown
    }

    /// `mDebuggerUpdate`
    pub fn update(&mut self, core: &mut dyn DebugCore) {
        let mut ctx = Context { core, shutdown: false };
        for m in self.modules.iter_mut() {
            m.update(&mut ctx);
        }
        if ctx.shutdown {
            self.state = State::Shutdown;
        }
    }
}
