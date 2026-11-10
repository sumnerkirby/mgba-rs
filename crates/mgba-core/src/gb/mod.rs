// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's Game Boy / Game Boy Color system
// (src/gb/gb.c, src/gb/core.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub mod audio;
pub mod io;
pub mod mbc;
pub mod mbc_extra;
pub mod mcore;
pub mod memory;
pub mod renderer;
pub mod serialize;
pub mod sgb_border;
pub mod sio_lockstep;
pub mod timer;
pub mod video;
pub mod video_log;

use std::sync::Arc;

use crate::sm83::{self, Sm83Bus, Sm83Core, CORE_FETCH, CORE_HALT_BUG};
use crate::timing::{Timing, TimingEvent};

pub use crate::gba::LogLevel;
pub use audio::GbAudio;
pub use io::GbSio;
pub use sio_lockstep::{GbSioDriver, GbSioLockstep};
pub use memory::GbMemory;
pub use renderer::GbRenderer;
pub use timer::GbTimer;
pub use video::GbVideo;

pub const DMG_SM83_FREQUENCY: u32 = 0x400000;
pub const CGB_SM83_FREQUENCY: u32 = 0x800000;
pub const SGB_SM83_FREQUENCY: u32 = 0x418B1E;

pub const VIDEO_HORIZONTAL_PIXELS: usize = video::HORIZONTAL_PIXELS;
pub const VIDEO_VERTICAL_PIXELS: usize = video::VERTICAL_PIXELS;

pub const IRQ_VBLANK: u8 = 0x0;
pub const IRQ_LCDSTAT: u8 = 0x1;
pub const IRQ_TIMER: u8 = 0x2;
pub const IRQ_SIO: u8 = 0x3;
pub const IRQ_KEYPAD: u8 = 0x4;

/// `enum GBModel`
pub mod model {
    pub const AUTODETECT: u8 = 0xFF;
    pub const DMG: u8 = 0x00;
    pub const SGB: u8 = 0x20;
    pub const MGB: u8 = 0x40;
    pub const SGB2: u8 = MGB | SGB;
    pub const CGB: u8 = 0x80;
    pub const SCGB: u8 = CGB | SGB;
    pub const AGB: u8 = 0xC0;

    /// `GBNameToModel`
    pub fn from_name(name: &str) -> u8 {
        match name.to_ascii_uppercase().as_str() {
            "DMG" | "GB" => DMG,
            "CGB" | "GBC" => CGB,
            "AGB" | "GBA" => AGB,
            "SGB" => SGB,
            "MGB" => MGB,
            "SGB2" => SGB2,
            "SCGB" | "SGBC" => SCGB,
            _ => AUTODETECT,
        }
    }

    /// `GBModelToName`
    pub fn name(model: u8) -> &'static str {
        match model {
            DMG => "DMG",
            SGB => "SGB",
            MGB => "MGB",
            SGB2 => "SGB2",
            CGB => "CGB",
            AGB => "AGB",
            SCGB => "SCGB",
            _ => "Autodetect",
        }
    }
}

/// `enum GBSGBCommand`
pub mod sgb {
    pub const PAL01: u8 = 0;
    pub const PAL23: u8 = 1;
    pub const PAL03: u8 = 2;
    pub const PAL12: u8 = 3;
    pub const ATTR_BLK: u8 = 4;
    pub const ATTR_LIN: u8 = 5;
    pub const ATTR_DIV: u8 = 6;
    pub const ATTR_CHR: u8 = 7;
    pub const SOUND: u8 = 8;
    pub const SOU_TRN: u8 = 9;
    pub const PAL_SET: u8 = 10;
    pub const PAL_TRN: u8 = 11;
    pub const ATRC_EN: u8 = 12;
    pub const TEST_EN: u8 = 13;
    pub const PICON_EN: u8 = 14;
    pub const DATA_SND: u8 = 15;
    pub const DATA_TRN: u8 = 16;
    pub const MLT_REQ: u8 = 17;
    pub const JUMP: u8 = 18;
    pub const CHR_TRN: u8 = 19;
    pub const PCT_TRN: u8 = 20;
    pub const ATTR_TRN: u8 = 21;
    pub const ATTR_SET: u8 = 22;
    pub const MASK_EN: u8 = 23;
    pub const OBJ_TRN: u8 = 24;
}

/// The Game Boy's scheduled events (one `struct mTimingEvent` each in mGBA).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum GbEvent {
    VideoMode,
    VideoFrame,
    AudioSample,
    TimerIrq,
    Timer,
    Sio,
    Dma,
    Hdma,
    Ei,
    /// The link-cable node's event (`GBSIOLockstepNode::event`); past
    /// `GB_EVENT_COUNT`, so it has no slot in the saved `Timing::whens`.
    SioLockstep,
}

pub const GB_EVENT_COUNT: usize = 9;

impl TimingEvent for GbEvent {
    #[inline]
    fn index(self) -> usize {
        self as usize
    }
    fn priority(self) -> u32 {
        match self {
            GbEvent::Ei => 0,
            GbEvent::VideoMode => 8,
            GbEvent::VideoFrame => 9,
            GbEvent::AudioSample => 0x18,
            GbEvent::TimerIrq => 0x20,
            GbEvent::Timer => 0x21,
            GbEvent::Sio => 0x30,
            GbEvent::Dma => 0x40,
            GbEvent::Hdma => 0x41,
            GbEvent::SioLockstep => 0x80,
        }
    }
}

/// Model selection as mGBA's `gb.model`, `sgb.model`, `cgb.model`,
/// `cgb.hybridModel` and `cgb.sgbModel` settings (applied by cartridge type).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelConfig {
    pub gb: Option<u8>,
    pub sgb: Option<u8>,
    pub cgb: Option<u8>,
    pub cgb_hybrid: Option<u8>,
    pub cgb_sgb: Option<u8>,
}

macro_rules! glog {
    ($gb:expr, $level:expr, $($arg:tt)*) => {
        if $gb.log_enabled {
            $gb.push_log($level, format!($($arg)*));
        }
    };
}
pub(crate) use glog;

/// `struct GB`
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Gb {
    pub cpu: Sm83Core,
    pub timing: Timing<GbEvent, GB_EVENT_COUNT>,
    pub memory: GbMemory,
    pub video: GbVideo,
    pub renderer: GbRenderer,
    pub timer: GbTimer,
    pub audio: GbAudio,
    pub sio: GbSio,
    /// `sio.driver`: the link cable, when one is attached.
    #[serde(skip)]
    pub sio_driver: Option<GbSioDriver>,
    /// The debugger's breakpoints and watchpoints while one is attached.
    #[serde(skip)]
    pub debugger: Option<Box<crate::debugger::Platform>>,
    pub model: u8,

    pub pristine_rom_size: usize,
    pub yanked_rom_size: usize,
    pub yanked_mbc: i32,
    pub rom_crc32: u32,
    pub has_rom: bool,
    #[serde(skip)]
    pub bios: Option<Arc<Vec<u8>>>,
    pub sram_size: usize,
    /// The clock block at the end of the save file (MBC3, HuC-3, TAMA5), as
    /// mGBA would re-read it on reset.
    pub rtc_file: Vec<u8>,
    pub sram_dirty: i32,
    pub sram_dirt_age: u32,

    pub sgb_bit: i32,
    pub current_sgb_bits: i32,
    pub sgb_packet: [u8; 16],
    pub sgb_controllers: i32,
    pub sgb_current_controller: i32,
    pub sgb_increment: bool,

    pub cpu_blocked: bool,
    pub early_exit: bool,
    pub double_speed: u32,

    pub allow_opposing_directions: bool,
    /// The frontend's key state (`*gb->keySource`), GBA KEYINPUT bit order.
    pub keys: u16,
    /// Rumble motor state (MBC5 rumble carts).
    pub rumble: bool,
    /// Image the Pocket Camera captures: 128x112, 8-bit grayscale, row-major.
    /// `None` leaves captures untouched, as mGBA does with no camera attached.
    #[serde(skip)]
    pub camera_image: Option<Vec<u8>>,

    /// Model the frontend asked for, per cartridge type (unset follows the cartridge).
    pub model_config: ModelConfig,
    /// Run `GBSkipBIOS` after reset when a boot ROM is loaded (mGBA's `skipBios` option).
    pub skip_bios_on_reset: bool,
    /// `RTC_FAKE_EPOCH` with this base (milliseconds); `None` uses the wall clock.
    pub rtc_fake_epoch_ms: Option<i64>,

    /// Active cheats (`CPU_COMPONENT_CHEAT_DEVICE`), applied at the end of each frame.
    #[serde(skip)]
    pub cheats: crate::cheats::CheatDevice,
    #[serde(skip)]
    pub log: Vec<(LogLevel, String)>,
    #[serde(skip)]
    pub log_enabled: bool,
    /// Frontend callback flag: a save write has settled and should be flushed.
    pub savedata_updated: bool,
}

const CGB_BIOS_HRAM: [u8; memory::SIZE_HRAM] = [
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D, 0x00, 0x08, 0x11,
    0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99, 0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E,
    0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x71, 0x02, 0x4D, 0x01, 0xC1, 0xFF, 0x0D, 0x00, 0xD3, 0x05, 0xF9, 0x00, 0x00,
];

const REGISTERED_TRADEMARK: [u8; 8] = [0x3C, 0x42, 0xB9, 0xA5, 0xB9, 0xA5, 0x42, 0x3C];

const DMG0_BIOS_CHECKSUM: u32 = 0xC2F5CC97;
const DMG_BIOS_CHECKSUM: u32 = 0x59C8598E;
const MGB_BIOS_CHECKSUM: u32 = 0xE6920754;
const SGB_BIOS_CHECKSUM: u32 = 0xEC8A83B9;
const SGB2_BIOS_CHECKSUM: u32 = 0x53D0DD63;
const CGB_BIOS_CHECKSUM: u32 = 0x41884E46;
const CGB0_BIOS_CHECKSUM: u32 = 0xE8EF5318;
const CGBE_BIOS_CHECKSUM: u32 = 0xE95DC95D;
const AGB_BIOS_CHECKSUM: u32 = 0xFFD6B0F1;
const AGB0_BIOS_CHECKSUM: u32 = 0x570337EA;
const FORTUNE_BIOS_CHECKSUM: u32 = 0x66CC6D94;
const GAMEFIGHTER_BIOS_CHECKSUM: u32 = 0x908BA8DE;
const KONGFENG_GBBC_BIOS_CHECKSUM: u32 = 0x69236128;
const MAXSTATION_BIOS_CHECKSUM: u32 = 0x783E69C2;

/// `GBIsBIOS`
pub fn is_bios(data: &[u8]) -> bool {
    if data.is_empty() || data.len() > memory::SIZE_CART_BANK0 {
        return false;
    }
    matches!(
        crc32(data),
        DMG_BIOS_CHECKSUM
            | DMG0_BIOS_CHECKSUM
            | MGB_BIOS_CHECKSUM
            | SGB_BIOS_CHECKSUM
            | SGB2_BIOS_CHECKSUM
            | CGB_BIOS_CHECKSUM
            | CGB0_BIOS_CHECKSUM
            | CGBE_BIOS_CHECKSUM
            | AGB_BIOS_CHECKSUM
            | AGB0_BIOS_CHECKSUM
            | FORTUNE_BIOS_CHECKSUM
            | GAMEFIGHTER_BIOS_CHECKSUM
            | KONGFENG_GBBC_BIOS_CHECKSUM
            | MAXSTATION_BIOS_CHECKSUM
    )
}

/// `GBIsCompatibleBIOS`
fn is_compatible_bios(data: &[u8], m: u8) -> bool {
    if data.is_empty() || data.len() > memory::SIZE_CART_BANK0 {
        return false;
    }
    match crc32(data) {
        DMG_BIOS_CHECKSUM | DMG0_BIOS_CHECKSUM | MGB_BIOS_CHECKSUM | SGB_BIOS_CHECKSUM | SGB2_BIOS_CHECKSUM
        | FORTUNE_BIOS_CHECKSUM | GAMEFIGHTER_BIOS_CHECKSUM | MAXSTATION_BIOS_CHECKSUM => m < model::CGB,
        CGB_BIOS_CHECKSUM | CGB0_BIOS_CHECKSUM | CGBE_BIOS_CHECKSUM | AGB_BIOS_CHECKSUM | AGB0_BIOS_CHECKSUM
        | KONGFENG_GBBC_BIOS_CHECKSUM => m >= model::CGB,
        _ => false,
    }
}

/// `GBValidModels`
pub fn valid_models(bank0: &[u8]) -> u8 {
    if bank0.len() < 0x150 {
        return model::MGB;
    }
    let cart = &bank0[0x100..];
    let mut models = match cart[mbc::cart::CGB] {
        0x80 => model::CGB | model::MGB,
        0xC0 => model::CGB,
        _ => model::MGB,
    };
    if cart[mbc::cart::SGB] == 0x03 && cart[mbc::cart::OLD_LICENSEE] == 0x33 {
        models |= model::SGB;
    }
    models
}

/// `GBIsROM`
pub fn is_rom(data: &[u8]) -> bool {
    mbc::is_rom(data)
}

impl Default for Gb {
    fn default() -> Self {
        Self::new()
    }
}

impl Gb {
    /// `GBCreate` + `GBInit`
    pub fn new() -> Gb {
        Gb {
            cpu: Sm83Core::default(),
            timing: Timing::default(),
            memory: GbMemory::new(),
            video: GbVideo::new(),
            renderer: GbRenderer::new(),
            timer: GbTimer::default(),
            audio: GbAudio::new(),
            sio: GbSio::default(),
            sio_driver: None,
            debugger: None,
            model: model::AUTODETECT,
            pristine_rom_size: 0,
            yanked_rom_size: 0,
            yanked_mbc: mbc::MBC_NONE,
            rom_crc32: 0,
            has_rom: false,
            bios: None,
            sram_size: 0,
            rtc_file: Vec::new(),
            sram_dirty: 0,
            sram_dirt_age: 0,
            sgb_bit: -1,
            current_sgb_bits: 0,
            sgb_packet: [0; 16],
            sgb_controllers: 0,
            sgb_current_controller: 0,
            sgb_increment: false,
            cpu_blocked: false,
            early_exit: false,
            double_speed: 0,
            allow_opposing_directions: true,
            keys: 0,
            rumble: false,
            camera_image: None,
            model_config: ModelConfig::default(),
            skip_bios_on_reset: false,
            rtc_fake_epoch_ms: None,
            cheats: crate::cheats::CheatDevice::new(),
            log: Vec::new(),
            log_enabled: false,
            savedata_updated: false,
        }
    }

    pub(crate) fn push_log(&mut self, level: LogLevel, msg: String) {
        if self.log.len() < 1000 {
            self.log.push((level, msg));
        }
    }

    pub fn drain_log(&mut self) -> Vec<(LogLevel, String)> {
        std::mem::take(&mut self.log)
    }

    // ----- Scheduler helpers (mTiming bound to this CPU) -----

    #[inline]
    pub fn schedule(&mut self, id: GbEvent, when: i32) {
        let relative = self.cpu.cycles;
        self.timing.schedule(id, when, relative, &mut self.cpu.next_event);
    }

    #[inline]
    pub fn deschedule(&mut self, id: GbEvent) {
        self.timing.deschedule(id);
    }

    #[inline]
    pub fn current_time(&self) -> i32 {
        self.timing.current_time(self.cpu.cycles)
    }

    #[inline]
    pub fn until(&self, id: GbEvent) -> i32 {
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

    fn dispatch_event(&mut self, id: GbEvent, cycles_late: u32) {
        match id {
            GbEvent::VideoMode => self.video_mode_event(cycles_late),
            GbEvent::VideoFrame => self.video_frame_event(cycles_late),
            GbEvent::AudioSample => self.audio_sample_event(cycles_late),
            GbEvent::TimerIrq => self.timer_irq(cycles_late),
            GbEvent::Timer => self.timer_update(cycles_late),
            GbEvent::Sio => self.sio_process_events(cycles_late),
            GbEvent::SioLockstep => self.sio_lockstep_process_events(cycles_late),
            GbEvent::Dma => self.memory_dma_service(cycles_late),
            GbEvent::Hdma => self.memory_hdma_service(cycles_late),
            GbEvent::Ei => {
                // _enableInterrupts
                self.memory.ime = true;
                self.update_irqs();
            }
        }
    }

    // ----- gb.c -----

    /// `GBReset` (the CPU's irqh.reset, called at the end of `SM83Reset`)
    fn gb_reset(&mut self) {
        self.memory.rom_base = 0;
        self.memory.bios_mapped = false;
        self.detect_model();

        self.cpu.b = 0;
        self.cpu.d = 0;

        self.timer.internal_div = 0;

        self.cpu_blocked = false;
        self.early_exit = false;
        self.double_speed = 0;

        if self.yanked_rom_size != 0 {
            self.memory.rom_size = self.yanked_rom_size;
            self.memory.mbc_type = self.yanked_mbc;
            self.yanked_rom_size = 0;
        }

        self.sgb_bit = -1;
        self.sgb_controllers = 0;
        self.sgb_current_controller = 0;
        self.current_sgb_bits = 0;
        self.sgb_increment = false;
        self.sgb_packet = [0; 16];

        self.timing.clear();

        self.memory_reset();

        if let Some(bios) = self.bios.clone() {
            if !is_compatible_bios(&bios, self.model) {
                self.bios = None;
            } else {
                self.map_bios();
                let cpu = &mut self.cpu;
                cpu.a = 0;
                cpu.f = 0;
                cpu.c = 0;
                cpu.e = 0;
                cpu.h = 0;
                cpu.l = 0;
                cpu.sp = 0;
                cpu.pc = 0;
            }
        }

        self.audio.style = match self.model {
            model::MGB | model::SGB2 => audio::STYLE_MGB,
            model::AGB | model::CGB | model::SCGB => audio::STYLE_CGB,
            _ => audio::STYLE_DMG,
        };

        self.video_reset();
        self.timer_reset();
        self.io_reset();
        self.audio_reset();
        if self.bios.is_none() && self.has_rom {
            self.skip_bios();
        } else {
            self.schedule(GbEvent::Timer, 0);
        }

        self.sio_reset();

        let pc = self.cpu.pc;
        self.set_active_region_impl(pc);
    }

    /// `_GBCoreReset` + `SM83Reset`: full system reset as the frontend sees it.
    pub fn reset(&mut self) {
        self.model = model::AUTODETECT;
        if self.has_rom {
            self.apply_overrides();
            if self.model == model::AUTODETECT {
                let c = self.model_config;
                let chosen = match valid_models(&self.memory.rom) {
                    m if m == model::SGB | model::MGB => c.sgb,
                    model::MGB => c.gb,
                    m if m == model::MGB | model::CGB => c.cgb_hybrid,
                    m if m == model::SGB | model::CGB || m == model::MGB | model::SGB | model::CGB => c.cgb_sgb,
                    model::CGB => c.cgb,
                    _ => None,
                };
                if let Some(m) = chosen {
                    self.model = m;
                }
            }
        }
        sm83::reset(&mut self.cpu);
        self.gb_reset();
        if self.skip_bios_on_reset && self.bios.is_some() && self.has_rom {
            self.skip_bios();
        }
        self.timing.interrupt();
    }

    /// The built-in override table (`_overrides` in src/gb/overrides.c).
    fn apply_overrides(&mut self) {
        if self.memory.rom.len() < 0x150 {
            return;
        }
        let crc = crc32(&self.memory.rom[0x100..0x150]);
        let mbc = match crc {
            0xA61F3EE1 => mbc::M161, // Mani 4 in 1 - Tetris + Alleyway + Yakuman + Tennis
            // Pokemon Spaceworld 1997 demo (Gold/Silver, debug and non-debug)
            0x232A067D | 0x630ED957 | 0x5AFF0038 | 0xA61856BD => mbc::MBC3_RTC,
            // Unlicensed bootlegs
            0x30F8F86C => mbc::UNL_PKJD, // Pokemon Jade Version (Telefang Speed bootleg)
            0xE1147E75 | 0xEFF88FAA => mbc::UNL_NT_OLD_1, // Rockman 8; True Color 25 in 1
            // 23 in 1, 29 in 1, Caise Gedou 24/29 in 1, Rockman X4, Sonic Adventure 8,
            // Super Donkey Kong 5, Super Mario Special 3
            0x811925D9 | 0x62A8016A | 0x5758D6D9 | 0x80265A64 | 0x805459DE | 0x0B1B808A | 0x4650EB9A => {
                mbc::UNL_NT_OLD_2
            }
            // Capcom vs SNK, Digimon 02 4, Digimon 2, Digimon Pocket, Harry Potter 3,
            // Pokemon Diamond/Jade (Special Pikachu Edition), Pokemon - Mewtwo Strikes Back,
            // Shuma Baolong 02 4, Street Fighter Zero 4
            0xB289D95A | 0x688D6713 | 0x8931A272 | 0x79083C6B | 0x0C5047EE | 0x8AC634B7 | 0x8628A287 | 0xBC75D7B8
            | 0xFF0B60CC | 0x14A992A6 => mbc::UNL_NT_NEW,
            0x3EF5AFB2 => mbc::UNL_LI_CHENG, // Pokemon Jade Version (Telefang Speed bootleg)
            _ => return,
        };
        self.memory.mbc_type = mbc;
        self.mbc_init();
    }

    /// `GBSkipBIOS`
    pub fn skip_bios(&mut self) {
        let next_div;
        let cart_cgb = self.memory.rom_byte(0x100 + mbc::cart::CGB);
        match self.model {
            model::SGB => {
                let cpu = &mut self.cpu;
                cpu.a = 1;
                cpu.f = 0x00;
                cpu.c = 0x14;
                cpu.e = 0x00;
                cpu.h = 0xC0;
                cpu.l = 0x60;
                self.timer.internal_div = 0xD85;
                next_div = 8;
            }
            model::MGB => {
                let cpu = &mut self.cpu;
                cpu.a = 0xFF;
                cpu.f = 0xB0;
                cpu.c = 0x13;
                cpu.e = 0xD8;
                cpu.h = 1;
                cpu.l = 0x4D;
                self.timer.internal_div = 0xABC;
                next_div = 4;
            }
            model::SGB2 => {
                let cpu = &mut self.cpu;
                cpu.a = 0xFF;
                cpu.f = 0x00;
                cpu.c = 0x14;
                cpu.e = 0x00;
                cpu.h = 0xC0;
                cpu.l = 0x60;
                self.timer.internal_div = 0xD84;
                next_div = 8;
            }
            model::AGB | model::CGB | model::SCGB => {
                if self.model == model::AGB {
                    self.cpu.b = 1;
                }
                self.cpu.a = 0x11;
                self.cpu.f = if self.model == model::AGB { 0x00 } else { 0x80 };
                self.cpu.c = 0;
                self.cpu.h = 0;
                if cart_cgb & 0x80 != 0 {
                    self.cpu.d = 0xFF;
                    self.cpu.e = 0x56;
                    self.cpu.l = 0x0D;
                    self.timer.internal_div = 0x2F0;
                } else {
                    self.cpu.e = 0x08;
                    self.cpu.l = 0x7C;
                    self.timer.internal_div = 0x260;
                    self.model = model::DMG;
                    self.memory.io[io::REG_KEY1] = 0xFF;
                    self.memory.io[io::REG_BCPS] = 0x88; // Faked writing 4 BG palette entries
                    self.memory.io[io::REG_OCPS] = 0x90; // Faked writing 8 OBJ palette entries
                    self.memory.io[io::REG_SVBK] = 0xFF;
                    self.video_disable_cgb();
                }
                self.memory.hram.copy_from_slice(&CGB_BIOS_HRAM);
                next_div = 0xC;
            }
            _ => {
                // GB_MODEL_AUTODETECT falls through to DMG
                self.model = model::DMG;
                let cpu = &mut self.cpu;
                cpu.a = 1;
                cpu.f = 0xB0;
                cpu.c = 0x13;
                cpu.e = 0xD8;
                cpu.h = 1;
                cpu.l = 0x4D;
                self.timer.internal_div = 0xABC;
                next_div = 4;
            }
        }

        for i in 0..48u16 {
            let byte = self.load8(0x104 + i);

            let mut output0: u8 = 0;
            let mut output1: u8 = 0;
            output0 |= byte & 0x80;
            output0 |= (byte & 0x40) >> 1;
            output0 |= (byte & 0x20) >> 2;
            output0 |= (byte & 0x10) >> 3;
            output0 |= output0 >> 1;

            output1 |= (byte & 0x08) << 3;
            output1 |= (byte & 0x04) << 2;
            output1 |= (byte & 0x02) << 1;
            output1 |= byte & 0x01;
            output1 |= output1 << 1;

            self.patch8(0x8010 + i * 8, output0);
            self.patch8(0x8012 + i * 8, output0);
            self.patch8(0x8014 + i * 8, output1);
            self.patch8(0x8016 + i * 8, output1);
        }
        for (i, &b) in REGISTERED_TRADEMARK.iter().enumerate() {
            self.patch8(0x8190 + i as u16 * 2, b);
        }
        if self.memory.mbc_type == mbc::UNL_SACHEN_MMC2 {
            self.memory.mbc_state.sachen.locked = mbc_extra::SACHEN_UNLOCKED;
        }

        if self.model < model::CGB {
            for i in 0..12u16 {
                self.patch8(0x9904 + i, i as u8 + 1);
                self.patch8(0x9924 + i, i as u8 + 13);
            }
            self.patch8(0x9910, 0x19);
        }

        self.cpu.sp = 0xFFFE;
        self.cpu.pc = 0x100;

        self.timer.next_div = timer::DMG_DIV_PERIOD * (16 - next_div);

        self.deschedule(GbEvent::Timer);
        let nd = self.timer.next_div;
        self.schedule(GbEvent::Timer, nd);

        if self.bios.is_some() {
            self.unmap_bios();
        }

        self.io_write(io::REG_NR52, 0xF1);
        self.io_write(io::REG_NR14, 0x3F);
        self.io_write(io::REG_NR10, 0x80);
        self.io_write(io::REG_NR11, 0xBF);
        self.io_write(io::REG_NR12, 0xF3);
        self.io_write(io::REG_NR13, 0xF3);
        self.io_write(io::REG_NR24, 0x3F);
        self.io_write(io::REG_NR21, 0x3F);
        self.io_write(io::REG_NR22, 0x00);
        self.io_write(io::REG_NR34, 0x3F);
        self.io_write(io::REG_NR30, 0x7F);
        self.io_write(io::REG_NR31, 0xFF);
        self.io_write(io::REG_NR32, 0x9F);
        self.io_write(io::REG_NR44, 0x3F);
        self.io_write(io::REG_NR41, 0xFF);
        self.io_write(io::REG_NR42, 0x00);
        self.io_write(io::REG_NR43, 0x00);
        self.io_write(io::REG_NR50, 0x77);
        self.io_write(io::REG_NR51, 0xF3);
        self.io_write(io::REG_LCDC, 0x91);
        self.memory.io[io::REG_BANK] = 0x1;
        self.video_skip_bios();
    }

    /// `GBMapBIOS`
    fn map_bios(&mut self) {
        let Some(bios) = self.bios.clone() else { return };
        let mut base = vec![0u8; memory::SIZE_CART_BANK0];
        let size = bios.len().min(memory::SIZE_CART_BANK0);
        base[..size].copy_from_slice(&bios[..size]);
        if self.has_rom {
            for i in size..memory::SIZE_CART_BANK0 {
                base[i] = self.memory.rom_byte(i);
            }
            if size > 0x100 {
                for i in 0x100..0x200 {
                    base[i] = self.memory.rom_byte(i);
                }
            }
        }
        self.memory.bios_rom = base;
        self.memory.bios_mapped = true;
    }

    /// `GBUnmapBIOS`
    pub(crate) fn unmap_bios(&mut self) {
        if self.memory.io[io::REG_BANK] == 0xFF && (self.memory.bios_mapped || self.memory.rom_base != 0) {
            self.memory.bios_mapped = false;
            if self.memory.mbc_type == mbc::MMM01 {
                let b = (self.memory.rom_size / memory::SIZE_CART_BANK0) as i32 - 2;
                self.mbc_switch_bank0(b);
            } else {
                self.mbc_switch_bank0(0);
            }
        }
        // XXX: Force AGB registers for AGB-mode
        if self.model == model::AGB && self.cpu.pc == 0x100 {
            self.cpu.b = 1;
        }
    }

    /// `GBDetectModel`
    fn detect_model(&mut self) {
        if self.model != model::AUTODETECT {
            return;
        }
        if let Some(bios) = &self.bios {
            match crc32(bios) {
                DMG_BIOS_CHECKSUM | DMG0_BIOS_CHECKSUM | FORTUNE_BIOS_CHECKSUM | GAMEFIGHTER_BIOS_CHECKSUM
                | MAXSTATION_BIOS_CHECKSUM => self.model = model::DMG,
                MGB_BIOS_CHECKSUM => self.model = model::MGB,
                SGB_BIOS_CHECKSUM => self.model = model::SGB,
                SGB2_BIOS_CHECKSUM => self.model = model::SGB2,
                CGB_BIOS_CHECKSUM | CGB0_BIOS_CHECKSUM | CGBE_BIOS_CHECKSUM | KONGFENG_GBBC_BIOS_CHECKSUM => {
                    self.model = model::CGB
                }
                AGB_BIOS_CHECKSUM | AGB0_BIOS_CHECKSUM => self.model = model::AGB,
                _ => self.bios = None,
            }
        }
        if self.model == model::AUTODETECT && self.has_rom {
            let cgb = self.memory.rom_byte(0x100 + mbc::cart::CGB);
            let sgb = self.memory.rom_byte(0x100 + mbc::cart::SGB);
            let lic = self.memory.rom_byte(0x100 + mbc::cart::OLD_LICENSEE);
            self.model = if cgb & 0x80 != 0 {
                model::CGB
            } else if sgb == 0x03 && lic == 0x33 {
                model::SGB
            } else {
                model::DMG
            };
        }
    }

    /// `GBUpdateIRQs`
    pub fn update_irqs(&mut self) {
        let irqs = self.memory.ie & self.memory.io[io::REG_IF] & 0x1F;
        if irqs == 0 {
            self.cpu.irq_pending = false;
            return;
        }
        self.cpu.halted = false;

        if !self.memory.ime {
            self.cpu.irq_pending = false;
            return;
        }
        if self.cpu.irq_pending {
            return;
        }
        sm83::raise_irq(&mut self.cpu);
    }

    /// `_GBAdvanceCycles`
    fn advance_cycles(&mut self) {
        let ds = self.double_speed as i32;
        let state_mask = (4 * (2 - ds)) - 1;
        let state_offset = ((self.cpu.next_event - self.cpu.cycles) & state_mask) >> (ds == 0) as i32;
        self.cpu.cycles = self.cpu.next_event;
        self.cpu.execution_state = ((self.cpu.execution_state as i32 + state_offset) & 3) as u8;
    }

    /// `GBProcessEvents`
    fn gb_process_events(&mut self) {
        let was_halted = self.cpu.halted;
        loop {
            loop {
                let cycles = self.cpu.cycles;
                self.cpu.cycles = 0;
                self.cpu.next_event = i32::MAX;

                let mut next_event = cycles;
                loop {
                    next_event = self.timing_tick(next_event);
                    if !self.cpu_blocked {
                        break;
                    }
                }
                // This loop cannot early exit until the SM83 run loop properly handles mid-M-cycle-exits
                self.cpu.next_event = next_event;

                if self.cpu.halted {
                    self.advance_cycles();
                    if self.memory.ie == 0 || !self.memory.ime {
                        break;
                    }
                }
                if self.early_exit {
                    break;
                }
                if self.cpu.cycles < self.cpu.next_event {
                    break;
                }
            }
            if self.cpu_blocked {
                self.advance_cycles();
            }
            if !was_halted || (self.cpu.execution_state & 3) == CORE_FETCH {
                break;
            }
            let next_fetch = (CORE_FETCH as i32 - self.cpu.execution_state as i32) * self.cpu.t_multiplier;
            if next_fetch < self.cpu.next_event {
                self.cpu.cycles += next_fetch;
                self.cpu.execution_state = CORE_FETCH;
                break;
            }
            self.advance_cycles();
        }
        self.early_exit = false;
    }

    /// `GBSetInterrupts`
    fn gb_set_interrupts(&mut self, enable: bool) {
        self.deschedule(GbEvent::Ei);
        if !enable {
            self.memory.ime = false;
            self.update_irqs();
        } else {
            let t = self.cpu.t_multiplier;
            self.schedule(GbEvent::Ei, 4 * t);
        }
    }

    /// `GBIRQVector`
    fn gb_irq_vector(&mut self) -> u16 {
        let irqs = self.memory.ie & self.memory.io[io::REG_IF];
        for irq in 0..5u8 {
            if irqs & (1 << irq) != 0 {
                self.memory.io[io::REG_IF] &= !(1 << irq);
                return 0x40 + irq as u16 * 8;
            }
        }
        0
    }

    /// `GBHalt`
    fn gb_halt(&mut self) {
        if self.memory.ie & self.memory.io[io::REG_IF] & 0x1F == 0 {
            self.advance_cycles();
            self.cpu.execution_state = (self.cpu.execution_state.wrapping_sub(1)) & 3;
            self.cpu.halted = true;
        } else if !self.memory.ime {
            glog!(self, LogLevel::GameError, "HALT bug");
            self.cpu.execution_state = CORE_HALT_BUG;
        }
    }

    /// `GBStop`
    fn gb_stop(&mut self) {
        if self.model >= model::CGB && self.memory.io[io::REG_KEY1] & 1 != 0 {
            self.double_speed ^= 1;
            self.cpu.t_multiplier = 2 - self.double_speed as i32;
            self.memory.io[io::REG_KEY1] = 0;
            self.memory.io[io::REG_KEY1] |= (self.double_speed << 7) as u8;
        } else {
            // Sleep/shutdown callbacks go to the frontend in mGBA; nothing is attached here.
            glog!(self, LogLevel::Info, "Stop requested");
        }
    }

    /// `GBIllegal`
    fn gb_illegal(&mut self) {
        glog!(self, LogLevel::GameError, "Hit illegal opcode at address {:04X}:{:02X}", self.cpu.pc, self.cpu.bus);
        // Hang forever
        self.memory.ime = false;
        self.cpu.pc = self.cpu.pc.wrapping_sub(1);
    }

    /// `GBFrameStarted`
    fn frame_started(&mut self) {
        self.test_keypad_irq();
    }

    /// `GBFrameEnded`
    fn frame_ended(&mut self) {
        let frame = self.video.frame_counter;
        if self.sram_clean(frame) {
            // GBMBCRTCWrite and friends: the frontend writes savedata() out now
            let suffix = self.save_suffix();
            if !suffix.is_empty() {
                self.rtc_file = suffix;
            }
            self.savedata_updated = true;
        }

        if !self.cheats.sets.is_empty() {
            let mut cheats = std::mem::take(&mut self.cheats);
            cheats.refresh_all(self);
            self.cheats = cheats;
        }
    }

    /// `GBInterrupt`
    pub fn interrupt(&mut self) {
        self.early_exit = true;
        self.timing.interrupt();
    }

    /// The RTC source's current time (`mRTCSource::unixTime`, or `time(0)`).
    pub(crate) fn rtc_unix_time(&self) -> i64 {
        match self.rtc_fake_epoch_ms {
            Some(base) => {
                let frame_cycles = if self.model >= model::CGB && self.model != model::AUTODETECT {
                    (video::TOTAL_LENGTH as i64) << 1
                } else {
                    video::TOTAL_LENGTH as i64
                };
                let frequency = if self.model == model::SGB {
                    SGB_SM83_FREQUENCY
                } else if self.model >= model::CGB && self.model != model::AUTODETECT {
                    CGB_SM83_FREQUENCY
                } else {
                    DMG_SM83_FREQUENCY
                } as i64;
                (base + self.video.frame_counter as i64 * (frame_cycles * 1000) / frequency) / 1000
            }
            None => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        }
    }

    // ----- Frontend-facing API (mCore) -----

    /// `GBLoadROM`
    pub fn load_rom(&mut self, data: Vec<u8>) -> bool {
        if data.is_empty() {
            return false;
        }
        let mut rom = data;
        if rom.len() > memory::SIZE_CART_MAX {
            rom.truncate(memory::SIZE_CART_MAX);
        }
        self.pristine_rom_size = rom.len();
        let rom_size = if rom.len() < memory::SIZE_CART_BANK0 { memory::SIZE_CART_BANK0 } else { rom.len() };
        self.rom_crc32 = crc32(&rom[..self.pristine_rom_size]);
        // Pad so bank pointers stay in bounds; reads past rom_size are open bus anyway.
        let padded = rom_size.max(memory::SIZE_CART_BANK0 * 2).div_ceil(memory::SIZE_CART_BANK0) * memory::SIZE_CART_BANK0;
        rom.resize(padded, 0);
        self.memory.rom = Arc::new(rom);
        self.memory.rom_size = rom_size;
        self.memory.mbc_type = mbc::MBC_AUTODETECT;
        self.memory.sram = Vec::new();
        self.sram_size = 0;
        self.rtc_file = Vec::new();
        self.yanked_rom_size = 0;
        self.has_rom = true;
        self.mbc_reset();
        let pc = self.cpu.pc;
        self.set_active_region_impl(pc);
        true
    }

    /// `GBLoadBIOS`. Returns false if the image isn't a known Game Boy boot ROM.
    pub fn load_bios(&mut self, data: &[u8]) -> bool {
        if !is_bios(data) {
            return false;
        }
        self.bios = Some(Arc::new(data.to_vec()));
        true
    }

    /// Load existing save data (a .sav file in mGBA's layout: SRAM, then the
    /// RTC block for MBC3+RTC carts).
    pub fn load_save(&mut self, data: &[u8]) {
        let size = self.sram_size;
        if size != 0 {
            if self.memory.sram.len() < size {
                self.memory.sram.resize(size, 0xFF);
            }
            let n = data.len().min(size);
            self.memory.sram[..n].copy_from_slice(&data[..n]);
            for b in &mut self.memory.sram[n..size] {
                *b = 0xFF;
            }
            let bank = self.memory.sram_current_bank;
            self.mbc_switch_sram_bank(bank);
        }
        if data.len() > size {
            let suffix = &data[size..];
            match self.memory.mbc_type {
                mbc::MBC3_RTC => self.rtc_load_suffix(suffix),
                mbc::HUC3 => self.huc3_load_suffix(suffix),
                mbc::TAMA5 => self.tama5_load_suffix(suffix),
                _ => return,
            }
            self.rtc_file = suffix.to_vec();
        }
    }

    /// The battery save as mGBA writes it (`None` for carts without SRAM).
    pub fn savedata(&self) -> Option<Vec<u8>> {
        if self.sram_size == 0 || self.memory.sram.is_empty() {
            return None;
        }
        let mut out = self.memory.sram[..self.sram_size.min(self.memory.sram.len())].to_vec();
        out.extend_from_slice(&self.save_suffix());
        Some(out)
    }

    /// The block mGBA appends to the save for carts with a clock (empty otherwise).
    fn save_suffix(&self) -> Vec<u8> {
        match self.memory.mbc_type {
            mbc::MBC3_RTC => self.rtc_save_suffix(),
            mbc::HUC3 => self.huc3_save_suffix(),
            mbc::TAMA5 => self.tama5_save_suffix(),
            _ => Vec::new(),
        }
    }

    /// `_GBCoreSetKeys` (GBA KEYINPUT bit order: A, B, Select, Start, Right, Left, Up, Down)
    pub fn set_keys(&mut self, keys: u16) {
        self.keys = keys & 0xFF;
        self.test_keypad_irq();
    }

    /// `_GBCoreRunFrame`
    pub fn run_frame(&mut self) {
        let frame_counter = self.video.frame_counter;
        while self.video.frame_counter == frame_counter {
            sm83::run(self);
        }
    }

    /// The most recent completed frame, [`Gb::video_size`] pixels, 0x00BBGGRR.
    pub fn framebuffer(&self) -> &[u32] {
        self.renderer.output()
    }

    /// Frame size: 256x224 for a Super Game Boy with borders, otherwise 160x144
    /// (`_GBCoreCurrentVideoSize`).
    pub fn video_size(&self) -> (usize, usize) {
        self.renderer.size()
    }

    /// Show or hide Super Game Boy borders (mGBA's `sgb.borders`, on by default).
    pub fn set_sgb_borders(&mut self, enable: bool) {
        self.video.sgb_borders = enable;
        self.renderer.enable_sgb_border(enable);
    }

    /// Move produced audio samples into `out` at [`Gb::audio_sample_rate`].
    /// mGBA's native rate is 131072 Hz; pairs of four are averaged down to 32768 Hz.
    pub fn drain_audio(&mut self, out: &mut Vec<[i16; 2]>) {
        let buf = &mut self.audio.buffer;
        while buf.len() >= 4 {
            let mut l = 0i32;
            let mut r = 0i32;
            for _ in 0..4 {
                let s = buf.pop_front().unwrap();
                l += s[0] as i32;
                r += s[1] as i32;
            }
            out.push([(l / 4) as i16, (r / 4) as i16]);
        }
    }

    pub fn audio_sample_rate(&self) -> u32 {
        32768
    }

    pub fn frame_counter(&self) -> u32 {
        self.video.frame_counter
    }

    /// CPU cycles per second for the current model (`_GBCoreFrequency`).
    pub fn frequency(&self) -> u32 {
        if self.model == model::SGB {
            SGB_SM83_FREQUENCY
        } else if self.model >= model::CGB && self.model != model::AUTODETECT {
            CGB_SM83_FREQUENCY
        } else {
            DMG_SM83_FREQUENCY
        }
    }

    /// Internal game title from the cartridge header (`GBGetGameInfo`).
    pub fn game_title(&self) -> String {
        if self.memory.rom.len() < 0x150 {
            return String::new();
        }
        let cart = &self.memory.rom[0x100..];
        let raw = if cart[mbc::cart::OLD_LICENSEE] != 0x33 { &cart[0x34..0x44] } else { &cart[0x34..0x3F] };
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        String::from_utf8_lossy(&raw[..end]).trim().to_string()
    }

    /// Game code from the cartridge header (empty for old-licensee carts).
    pub fn game_code(&self) -> String {
        if self.memory.rom.len() < 0x150 {
            return String::new();
        }
        let cart = &self.memory.rom[0x100..];
        if cart[mbc::cart::OLD_LICENSEE] != 0x33 {
            return String::new();
        }
        String::from_utf8_lossy(&cart[0x3F..0x43]).trim_end_matches('\0').to_string()
    }

    /// Whether the loaded cartridge is a Game Boy Color title.
    pub fn is_cgb_rom(&self) -> bool {
        self.memory.rom_byte(0x100 + mbc::cart::CGB) & 0x80 != 0
    }
}

impl Sm83Bus for Gb {
    #[inline(always)]
    fn cpu(&mut self) -> &mut Sm83Core {
        &mut self.cpu
    }
    #[inline(always)]
    fn cpu_load8(&mut self, address: u16) -> u8 {
        self.cpu_load8_impl(address)
    }
    #[inline(always)]
    fn load8(&mut self, address: u16) -> u8 {
        if self.debugger.is_some() {
            self.debug_watch(address, crate::debugger::WATCHPOINT_READ, 0);
        }
        Gb::load8(self, address)
    }
    #[inline(always)]
    fn store8(&mut self, address: u16, value: u8) {
        if self.debugger.is_some() {
            self.debug_watch(address, crate::debugger::WATCHPOINT_WRITE, value);
        }
        Gb::store8(self, address, value)
    }
    #[inline(always)]
    fn set_active_region(&mut self, address: u16) {
        self.set_active_region_impl(address)
    }
    fn process_events(&mut self) {
        self.gb_process_events()
    }
    fn set_interrupts(&mut self, enable: bool) {
        self.gb_set_interrupts(enable)
    }
    fn irq_vector(&mut self) -> u16 {
        self.gb_irq_vector()
    }
    fn halt(&mut self) {
        self.gb_halt()
    }
    fn stop(&mut self) {
        self.gb_stop()
    }
    fn hit_illegal(&mut self) {
        self.gb_illegal()
    }
}

pub fn crc32(data: &[u8]) -> u32 {
    crate::gba::crc32(data)
}
