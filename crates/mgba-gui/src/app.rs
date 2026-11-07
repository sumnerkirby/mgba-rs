//! The desktop frontend: window, menus, input, timing, audio and file handling.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Color32, ColorImage, Key, TextureHandle, TextureOptions};
use mgba_core::gb::{self, model};
use mgba_core::gba::{keys, LogLevel};
use mgba_core::{Gb, Gba};
use serde::{Deserialize, Serialize};

use crate::audio::{AudioOut, Resampler};
use crate::core::{detect_platform, Core, Platform};

/// Screen size shown before a game is loaded (the GBA's).
const GBA_W: usize = 240;
const GBA_H: usize = 160;

/// Game Boy models offered in the Emulation menu (`gb.model` and friends in mGBA).
const GB_MODELS: [(&str, u8); 7] = [
    ("Autodetect", model::AUTODETECT),
    ("Game Boy (DMG)", model::DMG),
    ("Game Boy Pocket (MGB)", model::MGB),
    ("Super Game Boy (SGB)", model::SGB),
    ("Super Game Boy 2 (SGB2)", model::SGB2),
    ("Game Boy Color (CGB)", model::CGB),
    ("Game Boy Advance (AGB)", model::AGB),
];
const SETTINGS_KEY: &str = "mgba-rs-settings";
const MAX_RECENT: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Button {
    A,
    B,
    Select,
    Start,
    Right,
    Left,
    Up,
    Down,
    R,
    L,
}

impl Button {
    pub const ALL: [Button; 10] = [
        Button::Up,
        Button::Down,
        Button::Left,
        Button::Right,
        Button::A,
        Button::B,
        Button::L,
        Button::R,
        Button::Start,
        Button::Select,
    ];

    fn bit(self) -> u16 {
        match self {
            Button::A => keys::A,
            Button::B => keys::B,
            Button::Select => keys::SELECT,
            Button::Start => keys::START,
            Button::Right => keys::RIGHT,
            Button::Left => keys::LEFT,
            Button::Up => keys::UP,
            Button::Down => keys::DOWN,
            Button::R => keys::R,
            Button::L => keys::L,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Button::A => "A",
            Button::B => "B",
            Button::Select => "Select",
            Button::Start => "Start",
            Button::Right => "Right",
            Button::Left => "Left",
            Button::Up => "Up",
            Button::Down => "Down",
            Button::R => "R",
            Button::L => "L",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub scale: f32,
    pub integer_scaling: bool,
    pub smooth: bool,
    pub volume: f32,
    pub muted: bool,
    pub bios_path: Option<PathBuf>,
    /// Game Boy / Super Game Boy boot ROM (`gb.bios`)
    pub gb_bios_path: Option<PathBuf>,
    /// Game Boy Color boot ROM (`gbc.bios`)
    pub gbc_bios_path: Option<PathBuf>,
    /// Forced Game Boy model, `model::AUTODETECT` to follow the cartridge.
    pub gb_model: u8,
    pub skip_bios: bool,
    pub keymap: Vec<(Button, Key)>,
    pub recent_roms: Vec<PathBuf>,
    pub fast_forward_multiplier: u32,
    pub show_fps: bool,
    pub pause_when_unfocused: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            scale: 3.0,
            integer_scaling: true,
            smooth: false,
            volume: 1.0,
            muted: false,
            bios_path: None,
            gb_bios_path: None,
            gbc_bios_path: None,
            gb_model: model::AUTODETECT,
            skip_bios: false,
            // mGBA's default keyboard layout
            keymap: vec![
                (Button::Up, Key::ArrowUp),
                (Button::Down, Key::ArrowDown),
                (Button::Left, Key::ArrowLeft),
                (Button::Right, Key::ArrowRight),
                (Button::A, Key::X),
                (Button::B, Key::Z),
                (Button::L, Key::A),
                (Button::R, Key::S),
                (Button::Start, Key::Enter),
                (Button::Select, Key::Backspace),
            ],
            recent_roms: Vec::new(),
            fast_forward_multiplier: 4,
            show_fps: true,
            pause_when_unfocused: false,
        }
    }
}

impl Settings {
    fn key_for(&self, b: Button) -> Option<Key> {
        self.keymap.iter().find(|(btn, _)| *btn == b).map(|(_, k)| *k)
    }

    fn set_key(&mut self, b: Button, key: Key) {
        self.keymap.retain(|(btn, _)| *btn != b);
        self.keymap.push((b, key));
    }
}

struct Emulator {
    core: Core,
    rom_path: PathBuf,
    save_path: PathBuf,
    title: String,
}

impl Emulator {
    fn state_path(&self, slot: u32) -> PathBuf {
        self.rom_path.with_extension(format!("ss{slot}"))
    }

    fn write_save(&mut self) -> std::io::Result<bool> {
        if let Some(data) = self.core.savedata() {
            std::fs::write(&self.save_path, data)?;
            return Ok(true);
        }
        Ok(false)
    }
}

struct FpsCounter {
    frames: u32,
    since: Instant,
    fps: f64,
}

impl FpsCounter {
    fn tick(&mut self, frames: u32) {
        self.frames += frames;
        let elapsed = self.since.elapsed().as_secs_f64();
        if elapsed >= 0.5 {
            self.fps = self.frames as f64 / elapsed;
            self.frames = 0;
            self.since = Instant::now();
        }
    }
}

pub struct App {
    settings: Settings,
    emu: Option<Emulator>,
    paused: bool,
    fast_forward_toggle: bool,
    texture: Option<TextureHandle>,
    audio: Option<AudioOut>,
    resampler: Resampler,
    audio_scratch: Vec<[i16; 2]>,
    last_tick: Option<Instant>,
    acc: f64,
    fps: FpsCounter,
    status: Option<(String, Instant)>,
    error: Option<String>,
    show_keymap: bool,
    rebinding: Option<Button>,
    show_about: bool,
    show_log: bool,
    log_lines: Vec<String>,
    fullscreen: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, rom: Option<PathBuf>) -> App {
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, SETTINGS_KEY))
            .unwrap_or_default();
        let audio = AudioOut::new();
        let mut app = App {
            settings,
            emu: None,
            paused: false,
            fast_forward_toggle: false,
            texture: None,
            audio,
            resampler: Resampler::new(),
            audio_scratch: Vec::with_capacity(4096),
            last_tick: None,
            acc: 0.0,
            fps: FpsCounter { frames: 0, since: Instant::now(), fps: 0.0 },
            status: None,
            error: None,
            show_keymap: false,
            rebinding: None,
            show_about: false,
            show_log: false,
            log_lines: Vec::new(),
            fullscreen: false,
        };
        if app.audio.is_none() {
            app.set_status("No audio device available; running without sound");
        }
        if let Some(path) = rom {
            app.load_rom(&cc.egui_ctx, &path);
        }
        app
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    // ----- File handling -----

    fn load_rom(&mut self, ctx: &egui::Context, path: &Path) {
        self.close_rom();
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                self.error = Some(format!("Couldn't read {}: {e}", path.display()));
                return;
            }
        };
        let Some(platform) = detect_platform(path, &data) else {
            self.error = Some(format!("{} doesn't look like a Game Boy or Game Boy Advance ROM", path.display()));
            return;
        };
        let save_path = path.with_extension("sav");
        let save = std::fs::read(&save_path).ok();
        let core = match platform {
            Platform::Gba => self.make_gba(data, save.as_deref()),
            Platform::Gb => self.make_gb(data, save.as_deref()),
        };
        let Some(core) = core else {
            self.error = Some(format!("Couldn't load {}", path.display()));
            return;
        };
        let mut title = core.game_title();
        if title.is_empty() {
            title = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("mGBA-rs — {title}")));
        self.emu = Some(Emulator { core, rom_path: path.to_path_buf(), save_path, title: title.clone() });
        self.paused = false;
        self.acc = 0.0;
        self.last_tick = None;
        if let Some(a) = &self.audio {
            a.clear();
        }
        self.settings.recent_roms.retain(|p| p != path);
        self.settings.recent_roms.insert(0, path.to_path_buf());
        self.settings.recent_roms.truncate(MAX_RECENT);
        self.set_status(format!("Loaded {title}"));
    }

    fn make_gba(&mut self, data: Vec<u8>, save: Option<&[u8]>) -> Option<Core> {
        if data.len() < 0xC0 {
            return None;
        }
        let mut gba = Gba::new();
        gba.log_enabled = self.show_log;
        if let Some(bios_path) = &self.settings.bios_path {
            match std::fs::read(bios_path) {
                Ok(bios) if gba.load_bios(&bios) => {}
                Ok(_) => self.set_status("BIOS image rejected (wrong size or known-bad dump); using built-in BIOS"),
                Err(e) => self.set_status(format!("Couldn't read BIOS ({e}); using built-in BIOS")),
            }
        }
        gba.skip_bios_on_reset = self.settings.skip_bios;
        if !gba.load_rom(data) {
            return None;
        }
        // Overrides pick the save type before existing save data is applied.
        gba.reset();
        if let Some(save) = save {
            gba.load_save(save);
            gba.reset();
        }
        Some(Core::Gba(Box::new(gba)))
    }

    fn make_gb(&mut self, data: Vec<u8>, save: Option<&[u8]>) -> Option<Core> {
        let mut gb = Gb::new();
        gb.log_enabled = self.show_log;
        let m = self.settings.gb_model;
        if m != model::AUTODETECT {
            let c = &mut gb.model_config;
            c.gb = Some(m);
            c.sgb = Some(m);
            c.cgb = Some(m);
            c.cgb_hybrid = Some(m);
            c.cgb_sgb = Some(m);
        }
        if !gb.load_rom(data) {
            return None;
        }
        // Boot ROMs are only used with a matching model; prefer the one for this cartridge.
        let cgb_cart = gb.is_cgb_rom() && (m == model::AUTODETECT || m >= model::CGB);
        let bios_path = if cgb_cart || m >= model::CGB && m != model::AUTODETECT {
            self.settings.gbc_bios_path.clone()
        } else {
            self.settings.gb_bios_path.clone()
        };
        if let Some(bios_path) = bios_path {
            match std::fs::read(&bios_path) {
                Ok(bios) if gb.load_bios(&bios) => {}
                Ok(_) => self.set_status("Boot ROM not recognized; starting without it"),
                Err(e) => self.set_status(format!("Couldn't read boot ROM ({e}); starting without it")),
            }
        }
        gb.skip_bios_on_reset = self.settings.skip_bios;
        if let Some(save) = save {
            gb.load_save(save);
        }
        gb.reset();
        Some(Core::Gb(Box::new(gb)))
    }

    fn close_rom(&mut self) {
        if let Some(mut emu) = self.emu.take() {
            if let Err(e) = emu.write_save() {
                self.error = Some(format!("Couldn't write save file {}: {e}", emu.save_path.display()));
            }
        }
    }

    fn open_rom_dialog(&mut self, ctx: &egui::Context) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Open ROM")
            .add_filter("Game Boy / Game Boy Advance ROM", &["gba", "agb", "bin", "gb", "gbc", "sgb", "cgb"])
            .add_filter("Game Boy Advance ROM", &["gba", "agb", "bin"])
            .add_filter("Game Boy (Color) ROM", &["gb", "gbc", "sgb", "cgb"])
            .add_filter("All files", &["*"]);
        if let Some(dir) = self.settings.recent_roms.first().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.load_rom(ctx, &path);
        }
    }

    fn choose_gb_bios_dialog(&mut self, color: bool) {
        let title = if color { "Select Game Boy Color boot ROM (gbc_bios.bin)" } else { "Select Game Boy boot ROM (gb_bios.bin)" };
        if let Some(path) = rfd::FileDialog::new().set_title(title).add_filter("Boot ROM", &["bin", "rom", "gb", "gbc"]).pick_file() {
            match std::fs::read(&path) {
                Ok(data) if gb::is_bios(&data) => {
                    if color {
                        self.settings.gbc_bios_path = Some(path);
                    } else {
                        self.settings.gb_bios_path = Some(path);
                    }
                    self.set_status("Boot ROM set; it will be used the next time a game is loaded");
                }
                Ok(_) => self.error = Some("That file isn't a known Game Boy boot ROM.".into()),
                Err(e) => self.error = Some(format!("Couldn't read boot ROM: {e}")),
            }
        }
    }

    fn choose_bios_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Select GBA BIOS (gba_bios.bin)")
            .add_filter("BIOS image", &["bin", "rom", "gba"])
            .pick_file()
        {
            match std::fs::read(&path) {
                Ok(data) if data.len() == 0x4000 => {
                    self.settings.bios_path = Some(path);
                    self.set_status("BIOS set; it will be used the next time a game is loaded");
                }
                Ok(_) => self.error = Some("That file isn't a 16 KiB GBA BIOS image.".into()),
                Err(e) => self.error = Some(format!("Couldn't read BIOS: {e}")),
            }
        }
    }

    fn save_state(&mut self, slot: u32) {
        let Some(emu) = &self.emu else { return };
        let path = emu.state_path(slot);
        match std::fs::write(&path, emu.core.save_state()) {
            Ok(()) => self.set_status(format!("Saved state to slot {slot}")),
            Err(e) => self.error = Some(format!("Couldn't write {}: {e}", path.display())),
        }
    }

    fn load_state(&mut self, slot: u32) {
        let Some(emu) = &mut self.emu else { return };
        let path = emu.state_path(slot);
        let result = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|d| emu.core.load_state(&d));
        match result {
            Ok(()) => {
                self.set_status(format!("Loaded state from slot {slot}"));
                if let Some(a) = &self.audio {
                    a.clear();
                }
            }
            Err(e) => self.set_status(format!("Slot {slot}: {e}")),
        }
    }

    fn screenshot(&mut self) {
        let Some(emu) = &self.emu else { return };
        let stem = emu.rom_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "screenshot".into());
        let dir = emu.rom_path.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut n = 0;
        let path = loop {
            let p = dir.join(format!("{stem}-{n}.png"));
            if !p.exists() {
                break p;
            }
            n += 1;
        };
        let rgb: Vec<u8> = emu.core.framebuffer().iter().flat_map(|&p| [p as u8, (p >> 8) as u8, (p >> 16) as u8]).collect();
        let [w, h] = emu.core.screen_size();
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let file = std::fs::File::create(&path)?;
            let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header()?.write_image_data(&rgb)?;
            Ok(())
        })();
        match result {
            Ok(()) => self.set_status(format!("Screenshot saved to {}", path.display())),
            Err(e) => self.error = Some(format!("Couldn't save screenshot: {e}")),
        }
    }

    fn reset(&mut self) {
        if let Some(emu) = &mut self.emu {
            emu.core.reset();
            self.set_status("Reset");
        }
    }

    // ----- Emulation -----

    fn read_keys(&self, ctx: &egui::Context) -> u16 {
        if ctx.memory(|m| m.focused().is_some()) || self.rebinding.is_some() {
            return 0;
        }
        ctx.input(|i| {
            self.settings
                .keymap
                .iter()
                .filter(|(_, k)| i.key_down(*k))
                .fold(0u16, |acc, (b, _)| acc | b.bit())
        })
    }

    fn run_emulation(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let dt = self.last_tick.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
        self.last_tick = Some(now);

        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let running = self.emu.is_some() && !self.paused && !(self.settings.pause_when_unfocused && !focused);
        if !running {
            self.acc = 0.0;
            return;
        }

        let fast_forward = self.fast_forward_toggle || ctx.input(|i| i.key_down(Key::Tab));
        let keys = self.read_keys(ctx);
        let fps_target = self.emu.as_ref().map(|e| e.core.fps()).unwrap_or(60.0);

        self.acc += dt;
        if self.acc > 0.25 {
            // We fell far behind (window dragged, system stall); don't try to catch up.
            self.acc = 1.0 / fps_target;
        }
        let mut frames = 0u32;
        let frame_time = 1.0 / fps_target;
        let max_frames = if fast_forward { self.settings.fast_forward_multiplier.max(1) } else { 1 };
        while self.acc >= frame_time {
            self.acc -= frame_time;
            frames += 1;
        }
        if fast_forward {
            frames = frames.max(1) * max_frames;
            self.acc = 0.0;
        } else {
            frames = frames.min(3);
        }

        let emu = self.emu.as_mut().unwrap();
        emu.core.set_keys(keys);
        self.audio_scratch.clear();
        for _ in 0..frames {
            emu.core.run_frame();
            emu.core.drain_audio(&mut self.audio_scratch);
            if fast_forward {
                self.audio_scratch.clear();
            }
        }
        if emu.core.take_savedata_updated() {
            if let Err(e) = emu.write_save() {
                self.error = Some(format!("Couldn't write save file: {e}"));
            }
        }
        if self.show_log {
            for (level, msg) in emu.core.drain_log() {
                if !matches!(level, LogLevel::Debug) {
                    self.log_lines.push(format!("[{level:?}] {msg}"));
                }
            }
            let excess = self.log_lines.len().saturating_sub(500);
            self.log_lines.drain(..excess);
        }
        self.fps.tick(frames);

        if let Some(audio) = &self.audio {
            audio.set_volume(if self.settings.muted { 0.0 } else { self.settings.volume });
            if !self.audio_scratch.is_empty() {
                let target = audio.rate as f64 * 0.05;
                let fill = audio.buffered() as f64;
                let adjust = 1.0 + 0.005 * ((fill - target) / target).clamp(-1.0, 1.0);
                let in_rate = emu.core.audio_sample_rate() as f64;
                let out = self.resampler.process(&self.audio_scratch, in_rate, audio.rate as f64, adjust);
                audio.push(out);
            }
        }
    }

    fn update_texture(&mut self, ctx: &egui::Context) {
        let (size, pixels): ([usize; 2], Vec<Color32>) = match &self.emu {
            Some(emu) => (
                emu.core.screen_size(),
                emu.core.framebuffer().iter().map(|&p| Color32::from_rgb(p as u8, (p >> 8) as u8, (p >> 16) as u8)).collect(),
            ),
            None => ([GBA_W, GBA_H], vec![Color32::BLACK; GBA_W * GBA_H]),
        };
        let image = ColorImage::new(size, pixels);
        let options = if self.settings.smooth { TextureOptions::LINEAR } else { TextureOptions::NEAREST };
        match &mut self.texture {
            Some(t) => t.set(image, options),
            None => self.texture = Some(ctx.load_texture("screen", image, options)),
        }
    }

    // ----- Input / hotkeys -----

    fn handle_hotkeys(&mut self, ctx: &egui::Context) {
        if let Some(button) = self.rebinding {
            let pressed = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, .. } => Some(*key),
                    _ => None,
                })
            });
            if let Some(key) = pressed {
                if key != Key::Escape {
                    self.settings.set_key(button, key);
                }
                self.rebinding = None;
            }
            return;
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if let Some(path) = dropped.first() {
            self.load_rom(ctx, path);
        }

        let (ctrl, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        if ctrl && pressed(Key::O) {
            self.open_rom_dialog(ctx);
        }
        if ctrl && pressed(Key::R) {
            self.reset();
        }
        if ctrl && pressed(Key::P) || pressed(Key::Pause) {
            self.paused = !self.paused;
        }
        if pressed(Key::F12) {
            self.screenshot();
        }
        if pressed(Key::F11) {
            self.fullscreen = !self.fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }
        if pressed(Key::Escape) && self.fullscreen {
            self.fullscreen = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        const SLOT_KEYS: [Key; 9] = [Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6, Key::F7, Key::F8, Key::F9];
        for (i, k) in SLOT_KEYS.iter().enumerate() {
            if pressed(*k) {
                if shift {
                    self.save_state(i as u32 + 1);
                } else {
                    self.load_state(i as u32 + 1);
                }
            }
        }
    }

    // ----- UI -----

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.add(egui::Button::new("Open ROM…").shortcut_text("Ctrl+O")).clicked() {
                    ui.close();
                    self.open_rom_dialog(&ctx);
                }
                ui.menu_button("Recent", |ui| {
                    if self.settings.recent_roms.is_empty() {
                        ui.label("(none)");
                    }
                    let mut chosen = None;
                    for p in &self.settings.recent_roms {
                        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        if ui.button(name).on_hover_text(p.display().to_string()).clicked() {
                            chosen = Some(p.clone());
                        }
                    }
                    if let Some(p) = chosen {
                        ui.close();
                        self.load_rom(&ctx, &p);
                    }
                    if !self.settings.recent_roms.is_empty() && ui.button("Clear list").clicked() {
                        self.settings.recent_roms.clear();
                    }
                });
                if ui.add_enabled(self.emu.is_some(), egui::Button::new("Close ROM")).clicked() {
                    ui.close();
                    self.close_rom();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Title("mGBA-rs".into()));
                }
                ui.separator();
                let has = self.emu.is_some();
                ui.add_enabled_ui(has, |ui| {
                    ui.menu_button("Save state", |ui| {
                        for slot in 1..=9 {
                            if ui.add(egui::Button::new(format!("Slot {slot}")).shortcut_text(format!("Shift+F{slot}"))).clicked() {
                                ui.close();
                                self.save_state(slot);
                            }
                        }
                    });
                    ui.menu_button("Load state", |ui| {
                        for slot in 1..=9 {
                            let exists = self.emu.as_ref().map(|e| e.state_path(slot).exists()).unwrap_or(false);
                            if ui.add_enabled(exists, egui::Button::new(format!("Slot {slot}")).shortcut_text(format!("F{slot}"))).clicked() {
                                ui.close();
                                self.load_state(slot);
                            }
                        }
                    });
                    if ui.add(egui::Button::new("Take screenshot").shortcut_text("F12")).clicked() {
                        ui.close();
                        self.screenshot();
                    }
                });
                ui.separator();
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });

            ui.menu_button("Emulation", |ui| {
                let has = self.emu.is_some();
                if ui.add_enabled(has, egui::Button::new(if self.paused { "Resume" } else { "Pause" }).shortcut_text("Ctrl+P")).clicked() {
                    self.paused = !self.paused;
                    ui.close();
                }
                if ui.add_enabled(has, egui::Button::new("Reset").shortcut_text("Ctrl+R")).clicked() {
                    ui.close();
                    self.reset();
                }
                ui.checkbox(&mut self.fast_forward_toggle, "Fast forward (hold Tab)");
                ui.horizontal(|ui| {
                    ui.label("Fast-forward speed:");
                    ui.add(egui::Slider::new(&mut self.settings.fast_forward_multiplier, 2..=10).suffix("×"));
                });
                ui.checkbox(&mut self.settings.pause_when_unfocused, "Pause when window is inactive");
                ui.separator();
                ui.label("GBA BIOS");
                let bios = self.settings.bios_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "Built-in (HLE)".into());
                ui.label(egui::RichText::new(bios).small());
                if ui.button("Choose GBA BIOS file…").clicked() {
                    ui.close();
                    self.choose_bios_dialog();
                }
                if self.settings.bios_path.is_some() && ui.button("Use built-in BIOS").clicked() {
                    self.settings.bios_path = None;
                    self.set_status("Built-in BIOS will be used the next time a game is loaded");
                }
                ui.separator();
                ui.label("Game Boy boot ROMs (optional)");
                for (color, label) in [(false, "Game Boy"), (true, "Game Boy Color")] {
                    let current = if color { &self.settings.gbc_bios_path } else { &self.settings.gb_bios_path };
                    let shown = current.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "None (skip boot animation)".into());
                    ui.label(egui::RichText::new(format!("{label}: {shown}")).small());
                    ui.horizontal(|ui| {
                        if ui.button(format!("Choose {label} boot ROM…")).clicked() {
                            ui.close();
                            self.choose_gb_bios_dialog(color);
                        }
                        let has = if color { self.settings.gbc_bios_path.is_some() } else { self.settings.gb_bios_path.is_some() };
                        if has && ui.button("Clear").clicked() {
                            if color {
                                self.settings.gbc_bios_path = None;
                            } else {
                                self.settings.gb_bios_path = None;
                            }
                        }
                    });
                }
                if ui.checkbox(&mut self.settings.skip_bios, "Skip BIOS intro").changed() {
                    if let Some(emu) = &mut self.emu {
                        emu.core.set_skip_bios(self.settings.skip_bios);
                    }
                }
                ui.separator();
                ui.menu_button("Game Boy model", |ui| {
                    for (name, m) in GB_MODELS {
                        if ui.radio(self.settings.gb_model == m, name).clicked() {
                            self.settings.gb_model = m;
                            self.set_status("Model change takes effect the next time a Game Boy game is loaded");
                            ui.close();
                        }
                    }
                });
            });

            ui.menu_button("Video", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Window size:");
                    let [w, h] = self.emu.as_ref().map(|e| e.core.screen_size()).unwrap_or([GBA_W, GBA_H]);
                    for s in 1..=6 {
                        if ui.button(format!("{s}×")).clicked() {
                            self.settings.scale = s as f32;
                            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w as f32 * s as f32, h as f32 * s as f32 + 68.0)));
                        }
                    }
                });
                ui.checkbox(&mut self.settings.integer_scaling, "Integer scaling");
                ui.checkbox(&mut self.settings.smooth, "Bilinear filtering");
                if ui.add(egui::Button::new(if self.fullscreen { "Exit fullscreen" } else { "Fullscreen" }).shortcut_text("F11")).clicked() {
                    self.fullscreen = !self.fullscreen;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                    ui.close();
                }
                ui.checkbox(&mut self.settings.show_fps, "Show FPS");
                if let Some(emu) = &mut self.emu {
                    ui.separator();
                    ui.label("Layers");
                    match &mut emu.core {
                        Core::Gba(g) => {
                            let r = &mut g.renderer;
                            for i in 0..4 {
                                let mut on = !r.disable_bg[i];
                                if ui.checkbox(&mut on, format!("Background {i}")).changed() {
                                    r.disable_bg[i] = !on;
                                }
                            }
                            let mut on = !r.disable_obj;
                            if ui.checkbox(&mut on, "Sprites").changed() {
                                r.disable_obj = !on;
                            }
                            let mut on = !r.disable_win[0] && !r.disable_win[1];
                            if ui.checkbox(&mut on, "Windows").changed() {
                                r.disable_win = [!on, !on];
                                r.disable_objwin = !on;
                            }
                        }
                        Core::Gb(g) => {
                            let r = &mut g.renderer;
                            for (flag, name) in [(&mut r.disable_bg, "Background"), (&mut r.disable_win, "Window"), (&mut r.disable_obj, "Sprites")] {
                                let mut on = !*flag;
                                if ui.checkbox(&mut on, name).changed() {
                                    *flag = !on;
                                }
                            }
                        }
                    }
                }
            });

            ui.menu_button("Audio", |ui| {
                if self.audio.is_none() {
                    ui.label("No audio device available");
                }
                ui.checkbox(&mut self.settings.muted, "Mute");
                ui.add(egui::Slider::new(&mut self.settings.volume, 0.0..=1.5).text("Volume"));
                if let Some(emu) = &mut self.emu {
                    ui.separator();
                    ui.label("Channels");
                    let names = ["Square 1", "Square 2", "Wave", "Noise"];
                    match &mut emu.core {
                        Core::Gba(g) => {
                            let a = &mut g.audio;
                            for (i, name) in names.iter().enumerate() {
                                let mut on = !a.psg.force_disable_ch[i];
                                if ui.checkbox(&mut on, *name).changed() {
                                    a.psg.force_disable_ch[i] = !on;
                                }
                            }
                            let mut on = !a.force_disable_ch_a;
                            if ui.checkbox(&mut on, "Direct Sound A").changed() {
                                a.force_disable_ch_a = !on;
                            }
                            let mut on = !a.force_disable_ch_b;
                            if ui.checkbox(&mut on, "Direct Sound B").changed() {
                                a.force_disable_ch_b = !on;
                            }
                        }
                        Core::Gb(g) => {
                            for (i, name) in names.iter().enumerate() {
                                let mut on = !g.audio.force_disable_ch[i];
                                if ui.checkbox(&mut on, *name).changed() {
                                    g.audio.force_disable_ch[i] = !on;
                                }
                            }
                        }
                    }
                }
            });

            ui.menu_button("Settings", |ui| {
                if ui.button("Controls…").clicked() {
                    self.show_keymap = true;
                    ui.close();
                }
                if ui.checkbox(&mut self.show_log, "Show emulator log").changed() {
                    if let Some(emu) = &mut self.emu {
                        emu.core.set_log_enabled(self.show_log);
                    }
                }
            });

            ui.menu_button("Help", |ui| {
                if ui.button("About").clicked() {
                    self.show_about = true;
                    ui.close();
                }
            });
        });
    }

    fn screen(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_rect_before_wrap();
        ui.painter().rect_filled(available, 0.0, Color32::BLACK);
        let Some(tex) = &self.texture else { return };
        let [w, h] = self.emu.as_ref().map(|e| e.core.screen_size()).unwrap_or([GBA_W, GBA_H]);
        let mut scale = (available.width() / w as f32).min(available.height() / h as f32);
        if self.settings.integer_scaling && scale >= 1.0 {
            scale = scale.floor();
        }
        let size = egui::vec2(w as f32 * scale, h as f32 * scale);
        let rect = egui::Rect::from_center_size(available.center(), size);
        if self.emu.is_some() {
            ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            if self.paused {
                ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(110));
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "Paused", egui::FontId::proportional(28.0), Color32::WHITE);
            }
        } else {
            ui.painter().text(
                available.center(),
                egui::Align2::CENTER_CENTER,
                "Open a GB, GBC or GBA ROM (Ctrl+O) or drop one here",
                egui::FontId::proportional(18.0),
                Color32::from_gray(200),
            );
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(emu) = &self.emu {
                ui.label(&emu.title);
                ui.separator();
                if self.settings.show_fps {
                    ui.label(format!("{:.1} fps", self.fps.fps));
                    ui.separator();
                }
                ui.label(emu.core.cart_info());
            }
            if let Some((msg, at)) = &self.status {
                if at.elapsed() < Duration::from_secs(4) {
                    ui.separator();
                    ui.label(msg);
                }
            }
        });
    }

    fn windows(&mut self, ctx: &egui::Context) {
        if self.show_keymap {
            let mut open = true;
            egui::Window::new("Controls").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
                egui::Grid::new("keymap").striped(true).show(ui, |ui| {
                    for b in Button::ALL {
                        ui.label(b.name());
                        let label = if self.rebinding == Some(b) {
                            "Press a key… (Esc cancels)".to_string()
                        } else {
                            self.settings.key_for(b).map(|k| k.name().to_string()).unwrap_or_else(|| "—".into())
                        };
                        if ui.button(label).clicked() {
                            self.rebinding = Some(b);
                        }
                        ui.end_row();
                    }
                });
                ui.separator();
                ui.label("Hold Tab to fast-forward. F1–F9 load and Shift+F1–F9 save states.");
                if ui.button("Restore defaults").clicked() {
                    self.settings.keymap = Settings::default().keymap;
                }
            });
            if !open {
                self.show_keymap = false;
                self.rebinding = None;
            }
        }
        if self.show_about {
            egui::Window::new("About mGBA-rs").open(&mut self.show_about).resizable(false).collapsible(false).show(ctx, |ui| {
                ui.label("A Rust port of mGBA's Game Boy Advance and Game Boy / Game Boy Color emulation cores.");
                ui.label("mGBA is © 2013–2026 Jeffrey Pfau and contributors.");
                ui.label("Licensed under the Mozilla Public License 2.0.");
            });
        }
        if self.show_log {
            let mut open = true;
            egui::Window::new("Emulator log").open(&mut open).default_size([520.0, 260.0]).show(ctx, |ui| {
                if ui.button("Clear").clicked() {
                    self.log_lines.clear();
                }
                egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                    for l in &self.log_lines {
                        ui.monospace(l);
                    }
                });
            });
            if !open {
                self.show_log = false;
                if let Some(emu) = &mut self.emu {
                    emu.core.set_log_enabled(false);
                }
            }
        }
        if let Some(err) = self.error.clone() {
            egui::Window::new("Error").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(err);
                if ui.button("OK").clicked() {
                    self.error = None;
                }
            });
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_hotkeys(&ctx);
        self.run_emulation(&ctx);
        self.update_texture(&ctx);

        if !self.fullscreen {
            egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
            egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::BLACK)).show(ui, |ui| self.screen(ui));
        self.windows(&ctx);

        if self.emu.is_some() && !self.paused {
            ctx.request_repaint();
        } else if self.status.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
    }

    fn on_exit(&mut self) {
        self.close_rom();
    }
}
