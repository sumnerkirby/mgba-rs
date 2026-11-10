// Copyright (c) 2013-2015 Jeffrey Pfau
// Rust port of mGBA's GBA system (src/gba/gba.c, src/gba/core.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub mod audio;
pub mod bios;
pub mod dma;
pub mod gpio;
pub mod io;
pub mod memory;
pub mod overrides;
pub mod psg;
pub mod regs;
pub mod renderer;
pub mod savedata;
pub mod serialize;
pub mod sio;
pub mod sio_lockstep;
pub mod timer;
pub mod video;

use crate::arm::{self, ArmBus, ArmCore, ExecutionMode, ARM_PC, ARM_SP, BASE_RESET, MODE_IRQ, MODE_SUPERVISOR, MODE_SYSTEM, WORD_SIZE_ARM};
use crate::timing::{EventId, Timing};

pub use audio::Audio;
pub use memory::Memory;
pub use renderer::SoftwareRenderer;
pub use savedata::SavedataType;
pub use sio::Sio;
pub use sio_lockstep::{GbaSioDriver, GbaSioLockstepCoordinator};
pub use timer::Timer;
pub use video::Video;

pub const ARM7TDMI_FREQUENCY: u32 = 0x1000000;
pub const VIDEO_HORIZONTAL_PIXELS: usize = 240;
pub const VIDEO_VERTICAL_PIXELS: usize = 160;

pub const IRQ_VBLANK: u32 = 0x0;
pub const IRQ_HBLANK: u32 = 0x1;
pub const IRQ_VCOUNTER: u32 = 0x2;
pub const IRQ_TIMER0: u32 = 0x3;
pub const IRQ_SIO: u32 = 0x7;
pub const IRQ_DMA0: u32 = 0x8;
pub const IRQ_KEYPAD: u32 = 0xC;
pub const IRQ_GAMEPAK: u32 = 0xD;

const GBA_IRQ_DELAY: i32 = 7;

pub const SP_BASE_SYSTEM: u32 = 0x03007F00;
pub const SP_BASE_IRQ: u32 = 0x03007FA0;
pub const SP_BASE_SUPERVISOR: u32 = 0x03007FE0;

pub const IDLE_LOOP_NONE: u32 = 0xFFFFFFFF;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum IdleLoopOptimization {
    Ignore = -1,
    Remove = 0,
    Detect = 1,
}

/// GBA keypad bits (KEYINPUT order).
pub mod keys {
    pub const A: u16 = 1 << 0;
    pub const B: u16 = 1 << 1;
    pub const SELECT: u16 = 1 << 2;
    pub const START: u16 = 1 << 3;
    pub const RIGHT: u16 = 1 << 4;
    pub const LEFT: u16 = 1 << 5;
    pub const UP: u16 = 1 << 6;
    pub const DOWN: u16 = 1 << 7;
    pub const R: u16 = 1 << 8;
    pub const L: u16 = 1 << 9;
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogLevel {
    Fatal,
    Error,
    Warn,
    Info,
    Debug,
    Stub,
    GameError,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Gba {
    pub cpu: ArmCore,
    pub timing: Timing,
    pub memory: Memory,
    pub video: Video,
    #[serde(skip)]
    pub renderer: SoftwareRenderer,
    pub audio: Audio,
    pub sio: Sio,
    /// `sio.driver`: the link cable, when one is attached.
    #[serde(skip)]
    pub sio_driver: Option<GbaSioDriver>,
    pub timers: [Timer; 4],

    pub bus: u32,
    pub performing_dma: u32,

    pub keys_active: u16,
    pub keys_last: u16,

    pub rom_crc32: u32,
    pub pristine_rom_size: usize,
    pub yanked_rom_size: u32,
    pub has_rom: bool,
    pub bios_checksum: u32,

    pub idle_optimization: IdleLoopOptimization,
    pub idle_loop: u32,
    pub last_jump: u32,
    pub halt_pending: bool,
    pub cpu_blocked: bool,
    pub early_exit: bool,
    pub dma_pc: u32,
    pub bios_stall: u32,

    pub vba_bug_compat: bool,
    pub hard_crash: bool,
    pub allow_opposing_directions: bool,

    pub debug: bool,
    #[serde(with = "serde_big_array::BigArray")]
    pub debug_string: [u8; 0x100],
    pub debug_flags: u16,

    /// Set when the game hits a fatal condition (e.g. jumped to an invalid address).
    pub crashed: bool,
    /// Messages logged by the core since the frontend last drained them.
    /// Active cheats (`CPU_COMPONENT_CHEAT_DEVICE`), applied at the end of each frame.
    #[serde(skip)]
    pub cheats: crate::cheats::CheatDevice,
    #[serde(skip)]
    pub log: Vec<(LogLevel, String)>,
    #[serde(skip)]
    pub log_enabled: bool,
    /// Frontend callback flag: a save write has settled and should be flushed.
    pub savedata_updated: bool,
    /// Jump straight to the cartridge entry point on reset instead of running the BIOS intro.
    pub skip_bios_on_reset: bool,
    /// User-forced cartridge override (save type / hardware), replacing the built-in table.
    pub forced_override: Option<overrides::Override>,
}

const LOGO_CRC32: u32 = 0xD0BEB55E;

impl Default for Gba {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! glog {
    ($gba:expr, $level:expr, $($arg:tt)*) => {
        if $gba.log_enabled {
            $gba.push_log($level, format!($($arg)*));
        }
    };
}
pub(crate) use glog;

impl Gba {
    /// `GBACreate` + `GBAInit`
    pub fn new() -> Gba {
        let mut gba = Gba {
            cpu: ArmCore::default(),
            timing: Timing::default(),
            memory: Memory::new(),
            video: Video::new(),
            renderer: SoftwareRenderer::new(),
            audio: Audio::new(),
            sio: Sio::default(),
            sio_driver: None,
            timers: [Timer::default(); 4],
            bus: 0,
            performing_dma: 0,
            keys_active: 0,
            keys_last: 0x400,
            rom_crc32: 0,
            pristine_rom_size: 0,
            yanked_rom_size: 0,
            has_rom: false,
            bios_checksum: 0,
            idle_optimization: IdleLoopOptimization::Remove,
            idle_loop: IDLE_LOOP_NONE,
            last_jump: 0,
            halt_pending: false,
            cpu_blocked: false,
            early_exit: false,
            dma_pc: 0,
            bios_stall: 0,
            vba_bug_compat: true,
            hard_crash: false,
            allow_opposing_directions: true,
            debug: false,
            debug_string: [0; 0x100],
            debug_flags: 0,
            crashed: false,
            cheats: crate::cheats::CheatDevice::new(),
            log: Vec::new(),
            log_enabled: false,
            savedata_updated: false,
            skip_bios_on_reset: false,
            forced_override: None,
        };
        gba.bios_checksum = bios::checksum(&gba.memory.bios);
        gba
    }

    pub(crate) fn push_log(&mut self, level: LogLevel, msg: String) {
        if self.log.len() < 1000 {
            self.log.push((level, msg));
        }
    }

    // ----- Scheduler helpers (mTiming bound to this CPU) -----

    #[inline]
    pub fn schedule(&mut self, id: EventId, when: i32) {
        let relative = self.cpu.cycles;
        self.timing.schedule(id, when, relative, &mut self.cpu.next_event);
    }

    #[inline]
    pub fn schedule_absolute(&mut self, id: EventId, when: i32) {
        let now = self.current_time();
        self.schedule(id, when.wrapping_sub(now));
    }

    #[inline]
    pub fn deschedule(&mut self, id: EventId) {
        self.timing.deschedule(id);
    }

    #[inline]
    pub fn current_time(&self) -> i32 {
        self.timing.current_time(self.cpu.cycles)
    }

    #[inline]
    pub fn until(&self, id: EventId) -> i32 {
        self.timing.until(id, self.cpu.cycles)
    }

    /// `mTimingTick`
    fn timing_tick(&mut self, cycles: i32) -> i32 {
        self.timing.master_cycles = self.timing.master_cycles.wrapping_add(cycles as u32);
        self.timing.global_cycles = self.timing.global_cycles.wrapping_add(cycles as i64 as u64);
        let master = self.timing.master_cycles;
        while let Some(next) = self.timing.root.first().copied() {
            let next_when = next.when.wrapping_sub(master) as i32;
            if next_when > 0 {
                return next_when;
            }
            self.timing.root.remove(0);
            self.dispatch_event(next.id, (-next_when) as u32);
        }
        if !self.timing.reroot.is_empty() {
            self.timing.root = std::mem::take(&mut self.timing.reroot);
            self.cpu.next_event = self.timing.next_event(self.cpu.cycles);
        }
        self.cpu.next_event
    }

    fn dispatch_event(&mut self, id: EventId, cycles_late: u32) {
        match id {
            EventId::Irq => self.trigger_irq(cycles_late),
            EventId::Video => self.video_event(cycles_late),
            EventId::AudioFrame => self.audio_update_frame_event(cycles_late),
            EventId::AudioSample => self.audio_sample_event(cycles_late),
            EventId::Timer0 => self.timer_update(0, cycles_late),
            EventId::Timer1 => self.timer_update(1, cycles_late),
            EventId::Timer2 => self.timer_update(2, cycles_late),
            EventId::Timer3 => self.timer_update(3, cycles_late),
            EventId::Dma => self.dma_event(cycles_late),
            EventId::SavedataDust => {}
            EventId::Sio => self.sio_finish(cycles_late),
            EventId::SioLockstep => self.sio_lockstep_event(cycles_late),
        }
    }

    // ----- gba.c -----

    /// `GBAReset` (the CPU's irqh.reset), called after `ARMReset`.
    fn gba_reset(&mut self) {
        self.cpu.set_privilege_mode(MODE_IRQ);
        self.cpu.gprs[ARM_SP] = SP_BASE_IRQ;
        self.cpu.set_privilege_mode(MODE_SUPERVISOR);
        self.cpu.gprs[ARM_SP] = SP_BASE_SUPERVISOR;
        self.cpu.set_privilege_mode(MODE_SYSTEM);
        self.cpu.gprs[ARM_SP] = SP_BASE_SYSTEM;

        self.cpu_blocked = false;
        self.early_exit = false;
        self.dma_pc = 0;
        self.bios_stall = 0;
        self.keys_last = 0x400;
        if self.yanked_rom_size != 0 {
            self.memory.rom_size = self.yanked_rom_size;
            self.memory.rom_mask = to_pow2(self.memory.rom_size) - 1;
            self.yanked_rom_size = 0;
        }
        self.timing.clear();
        self.memory_reset();
        self.video_reset();
        self.audio_reset();
        self.io_init();
        self.timer_init();
        self.sio_reset();

        self.last_jump = 0;
        self.halt_pending = false;
        self.debug = false;
        self.debug_string = [0; 0x100];
        self.crashed = false;
    }

    /// `_GBACoreReset`: full system reset as the frontend sees it.
    pub fn reset(&mut self) {
        self.apply_overrides();
        arm::reset(self);
        self.gba_reset();
        let mut force_skip = self.skip_bios_on_reset && self.has_rom;
        if !force_skip && self.has_rom && self.pristine_rom_size >= 0xA0 && self.memory.full_bios {
            if crc32(&self.memory.rom[1..0x9D]) != LOGO_CRC32 {
                glog!(self, LogLevel::Warn, "Invalid logo, skipping BIOS");
                force_skip = true;
            }
        }
        if force_skip {
            self.skip_bios();
        }
        self.timing.interrupt();
    }

    /// `GBASkipBIOS`
    pub fn skip_bios(&mut self) {
        if self.cpu.gprs[ARM_PC] == BASE_RESET + WORD_SIZE_ARM {
            if self.has_rom {
                self.cpu.gprs[ARM_PC] = memory::BASE_ROM0;
            } else if self.memory.wram[0x30] != 0 {
                self.cpu.gprs[ARM_PC] = memory::BASE_EWRAM + 0xC0;
            } else {
                self.cpu.gprs[ARM_PC] = memory::BASE_EWRAM;
            }
            self.video.vcount = 0x7E;
            self.memory.io[(regs::VCOUNT >> 1) as usize] = 0x7E;
            self.deschedule(EventId::Video);
            self.schedule(EventId::Video, 117);
            self.memory.io[(regs::POSTFLG >> 1) as usize] = 1;
            arm::arm_write_pc(self);
        }
    }

    /// `GBAProcessEvents`
    fn gba_process_events(&mut self) {
        self.bus = self.cpu.prefetch[1];
        if self.cpu.execution_mode == ExecutionMode::Thumb {
            self.bus |= self.cpu.prefetch[1] << 16;
        }

        let mut next_event = self.cpu.next_event;
        while self.cpu.cycles >= next_event {
            self.cpu.next_event = i32::MAX;
            loop {
                let cycles = self.cpu.cycles;
                self.cpu.cycles = 0;
                next_event = self.timing_tick(if cycles < next_event { next_event } else { cycles });
                if !(self.cpu_blocked && !self.early_exit) {
                    break;
                }
            }
            self.cpu.next_event = next_event;
            if self.cpu.halted {
                self.cpu.cycles = next_event;
                if self.memory.io[(regs::IME >> 1) as usize] == 0 || self.memory.io[(regs::IE >> 1) as usize] == 0 {
                    break;
                }
            }
            if self.early_exit {
                break;
            }
        }
        self.early_exit = false;
        if self.cpu_blocked {
            self.cpu.cycles = self.cpu.next_event;
        }
    }

    /// `GBARaiseIRQ`
    pub fn raise_irq(&mut self, irq: u32, cycles_late: u32) {
        self.memory.io[(regs::IF >> 1) as usize] |= 1 << irq;
        self.test_irq(cycles_late);
    }

    /// `GBATestIRQ`
    pub fn test_irq(&mut self, cycles_late: u32) {
        if self.memory.io[(regs::IE >> 1) as usize] & self.memory.io[(regs::IF >> 1) as usize] != 0
            && !self.timing.is_scheduled(EventId::Irq)
        {
            self.schedule(EventId::Irq, GBA_IRQ_DELAY - cycles_late as i32);
        }
    }

    /// `GBAHalt`
    pub fn halt(&mut self) {
        self.cpu.next_event = self.cpu.cycles;
        self.cpu.halted = true;
    }

    /// `GBAStop`
    pub fn stop(&mut self) {
        glog!(self, LogLevel::Info, "Stop requested");
        self.cpu.next_event = self.cpu.cycles;
    }

    /// `GBADebug`
    fn gba_debug(&mut self, flags: u16) {
        self.debug_flags = flags;
        if flags & 0x100 != 0 {
            let end = self.debug_string.iter().position(|&b| b == 0).unwrap_or(0x100);
            let s = String::from_utf8_lossy(&self.debug_string[..end]).into_owned();
            self.debug_string = [0; 0x100];
            self.push_log(LogLevel::Info, format!("[GBA Debug] {}", s));
        }
        self.debug_flags &= !0x100;
    }

    /// `GBAInterrupt`
    pub fn interrupt(&mut self) {
        self.early_exit = true;
        self.timing.interrupt();
    }

    /// `_triggerIRQ`
    fn trigger_irq(&mut self, _cycles_late: u32) {
        self.cpu.halted = false;
        if self.memory.io[(regs::IE >> 1) as usize] & self.memory.io[(regs::IF >> 1) as usize] == 0 {
            return;
        }
        if self.memory.io[(regs::IME >> 1) as usize] != 0 && !self.cpu.cpsr.i() {
            arm::raise_irq(self);
        }
    }

    /// `GBATestKeypadIRQ`
    pub fn test_keypad_irq(&mut self) {
        let keys_last = self.keys_last;
        let keys_active = self.keys_active;
        let mut keycnt = self.memory.io[(regs::KEYCNT >> 1) as usize];
        if keycnt & 0x4000 == 0 {
            return;
        }
        self.keys_last = keys_active;
        let is_and = keycnt & 0x8000 != 0;
        keycnt &= 0x3FF;
        if is_and && keycnt == (keys_active & keycnt) {
            if keys_last == keys_active {
                return;
            }
            self.raise_irq(IRQ_KEYPAD, 0);
        } else if !is_and && (keys_active & keycnt) != 0 {
            self.raise_irq(IRQ_KEYPAD, 0);
        } else {
            self.keys_last = 0x400;
        }
    }

    /// `GBAFrameStarted`
    pub(crate) fn frame_started(&mut self) {
        self.test_keypad_irq();
    }

    /// `GBAFrameEnded`
    pub(crate) fn frame_ended(&mut self) {
        let was_dirty = self.memory.savedata.dirty != 0;
        let frame = self.video.frame_counter;
        self.memory.savedata.clean(frame);
        if was_dirty && self.memory.savedata.dirty == 0 {
            self.savedata_updated = true;
        }
        if !self.cheats.sets.is_empty() {
            let mut cheats = std::mem::take(&mut self.cheats);
            cheats.refresh_all(self);
            self.cheats = cheats;
        }
    }

    // ----- Frontend-facing API (mCore) -----

    /// `GBALoadROM`
    pub fn load_rom(&mut self, data: Vec<u8>) -> bool {
        if data.is_empty() {
            return false;
        }
        self.pristine_rom_size = data.len();
        let mut rom = data;
        let rom_size;
        if rom.len() > memory::SIZE_ROM0 as usize {
            rom.truncate(memory::SIZE_ROM0 as usize);
            rom_size = memory::SIZE_ROM0;
            self.pristine_rom_size = memory::SIZE_ROM0 as usize;
        } else if rom.len() == 0x00100000 {
            // 1 MiB ROMs (e.g. Classic NES) all appear as 4x mirrored, but not more
            let mut mirrored = rom.clone();
            for _ in 0..3 {
                mirrored.extend_from_slice(&rom);
            }
            rom = mirrored;
            rom_size = 0x00400000;
        } else {
            rom_size = rom.len() as u32;
        }
        self.rom_crc32 = crc32(&rom[..self.pristine_rom_size.min(rom.len())]);
        let (rom_size, rom_mask) = if rom_size.count_ones() != 1 {
            // This ROM is either a bad dump or homebrew. Emulate flash cart behavior.
            (memory::SIZE_ROM0, memory::SIZE_ROM0 - 1)
        } else {
            (rom_size, to_pow2(rom_size) - 1)
        };
        // Pad with the open-bus pattern isn't needed: out-of-bounds reads check rom_size.
        rom.resize((rom_mask as usize + 1).max(rom.len()), 0);
        self.memory.rom = std::sync::Arc::from(rom);
        self.memory.rom_size = rom_size;
        self.memory.rom_mask = rom_mask;
        self.yanked_rom_size = 0;
        self.has_rom = true;
        if self.memory.active_region >= memory::REGION_ROM0 as i32 {
            let pc = self.cpu.gprs[ARM_PC];
            self.set_active_region_impl(pc);
        }
        self.memory.hw.init();
        true
    }

    /// `GBALoadBIOS`. Returns false if the image is unusable.
    pub fn load_bios(&mut self, data: &[u8]) -> bool {
        if data.len() != memory::SIZE_BIOS as usize {
            return false;
        }
        let checksum = bios::checksum(data);
        if checksum == bios::GBA_DEBUG_BIOS_CHECKSUM {
            return false;
        }
        self.memory.bios.copy_from_slice(data);
        self.memory.full_bios = true;
        self.bios_checksum = checksum;
        true
    }

    /// Load existing save data (e.g. from a .sav file).
    pub fn load_save(&mut self, data: &[u8]) {
        self.memory.savedata.load(data);
    }

    pub fn savedata(&self) -> Option<&[u8]> {
        self.memory.savedata.data()
    }

    /// `_GBACoreSetKeys`
    pub fn set_keys(&mut self, keys: u16) {
        self.keys_active = keys & 0x3FF;
        self.test_keypad_irq();
    }

    /// `_GBACoreRunFrame`
    pub fn run_frame(&mut self) {
        let frame_counter = self.video.frame_counter;
        let start = self.current_time();
        while self.video.frame_counter == frame_counter
            && self.current_time().wrapping_sub(start) < video::VIDEO_TOTAL_LENGTH + video::VIDEO_HORIZONTAL_LENGTH
        {
            arm::run_loop(self);
        }
    }

    /// The most recent completed frame, 240x160, 0x00RRGGBB.
    pub fn framebuffer(&self) -> &[u32] {
        self.renderer.output()
    }

    /// Move produced audio samples (stereo, 32768 Hz by default) into `out`.
    pub fn drain_audio(&mut self, out: &mut Vec<[i16; 2]>) {
        out.extend(self.audio.buffer.drain(..));
    }

    /// Native audio rate (depends on SOUNDBIAS resolution).
    pub fn audio_sample_rate(&self) -> u32 {
        ARM7TDMI_FREQUENCY / self.audio.sample_interval as u32
    }

    pub fn frame_counter(&self) -> u32 {
        self.video.frame_counter
    }

    /// Internal game title from the cartridge header.
    pub fn game_title(&self) -> String {
        if self.memory.rom.len() < 0xC0 {
            return String::new();
        }
        let raw = &self.memory.rom[0xA0..0xAC];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        String::from_utf8_lossy(&raw[..end]).trim().to_string()
    }

    /// 4-character game code from the cartridge header.
    pub fn game_code(&self) -> String {
        if self.memory.rom.len() < 0xC0 {
            return String::new();
        }
        String::from_utf8_lossy(&self.memory.rom[0xAC..0xB0]).to_string()
    }

    pub fn drain_log(&mut self) -> Vec<(LogLevel, String)> {
        std::mem::take(&mut self.log)
    }
}

impl ArmBus for Gba {
    #[inline(always)]
    fn cpu(&mut self) -> &mut ArmCore {
        &mut self.cpu
    }
    #[inline(always)]
    fn cpu_ref(&self) -> &ArmCore {
        &self.cpu
    }
    #[inline]
    fn load32(&mut self, address: u32, cc: Option<&mut i32>) -> u32 {
        self.mem_load32(address, cc)
    }
    #[inline]
    fn load16(&mut self, address: u32, cc: Option<&mut i32>) -> u32 {
        self.mem_load16(address, cc)
    }
    #[inline]
    fn load8(&mut self, address: u32, cc: Option<&mut i32>) -> u32 {
        self.mem_load8(address, cc)
    }
    #[inline]
    fn store32(&mut self, address: u32, value: u32, cc: Option<&mut i32>) {
        self.mem_store32(address, value, cc)
    }
    #[inline]
    fn store16(&mut self, address: u32, value: u16, cc: Option<&mut i32>) {
        self.mem_store16(address, value, cc)
    }
    #[inline]
    fn store8(&mut self, address: u32, value: u8, cc: Option<&mut i32>) {
        self.mem_store8(address, value, cc)
    }
    fn load_multiple(&mut self, address: u32, mask: u32, direction: u32, cc: &mut i32) -> u32 {
        self.mem_load_multiple(address, mask, direction, Some(cc))
    }
    fn store_multiple(&mut self, address: u32, mask: u32, direction: u32, cc: &mut i32) -> u32 {
        self.mem_store_multiple(address, mask, direction, Some(cc))
    }
    #[inline(always)]
    fn fetch32(&self, address: u32) -> u32 {
        self.active_fetch32(address)
    }
    #[inline(always)]
    fn fetch16(&self, address: u32) -> u32 {
        self.active_fetch16(address)
    }
    #[inline]
    fn stall(&mut self, wait: i32) -> i32 {
        self.memory_stall(wait)
    }
    fn set_active_region(&mut self, address: u32) {
        self.set_active_region_impl(address)
    }
    fn process_events(&mut self) {
        self.gba_process_events()
    }
    fn swi16(&mut self, immediate: u32) {
        self.bios_swi16(immediate)
    }
    fn swi32(&mut self, immediate: u32) {
        self.bios_swi16(immediate >> 16)
    }
    fn hit_illegal(&mut self, opcode: u32) {
        if self.cpu.execution_mode == ExecutionMode::Thumb && (opcode & 0xFFC0) == 0xE800 {
            glog!(self, LogLevel::Info, "Hit Wii U VC opcode: {:08x}", opcode);
            return;
        }
        if self.yanked_rom_size == 0 {
            glog!(self, LogLevel::Warn, "Illegal opcode: {:08x}", opcode);
        }
        arm::raise_undefined(self);
    }
    fn hit_stub(&mut self, opcode: u32) {
        glog!(self, LogLevel::Error, "Stub opcode: {:08x}", opcode);
    }
    fn bkpt16(&mut self, _immediate: u32) {
        arm::raise_undefined(self);
    }
    fn bkpt32(&mut self, _immediate: u32) {
        arm::raise_undefined(self);
    }
    fn read_cpsr(&mut self) {
        self.test_irq(0);
    }
    fn cp_mrc(&mut self, cp: u32, _crn: u32, _crm: u32, _op1: u32, _op2: u32) -> Option<u32> {
        if cp == 14 {
            glog!(self, LogLevel::GameError, "Read from missing CP14");
            Some(self.load_bad())
        } else {
            None
        }
    }
    fn cp_mcr(&mut self, cp: u32, _crn: u32, _crm: u32, _op1: u32, _op2: u32, _value: u32) -> bool {
        if cp == 14 {
            glog!(self, LogLevel::GameError, "Write to missing CP14");
            true
        } else {
            false
        }
    }
    fn cp_cdp(&mut self, cp: u32, _crn: u32, _crm: u32, _crd: u32, _op1: u32, _op2: u32) -> bool {
        cp == 0
    }
}

pub fn to_pow2(x: u32) -> u32 {
    if x <= 1 {
        return 1;
    }
    x.next_power_of_two()
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (!(crc & 1)).wrapping_add(1);
            crc = (crc >> 1) ^ (0xEDB88320 & mask);
        }
    }
    !crc
}

/// Heuristic ROM check (`GBAIsROM`).
pub fn is_rom(data: &[u8]) -> bool {
    data.len() >= 0xC0 && data[3] == 0xEA && data[0xB2] == 0x96
}
