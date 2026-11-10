//! The desktop frontend: window, menus, input, timing, audio and file handling.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Color32, ColorImage, Key, TextureHandle, TextureOptions};
use mgba_core::gb::{self, model};
use mgba_core::gba::{keys, LogLevel};
use mgba_core::{Gb, Gba};
use serde::{Deserialize, Serialize};

use crate::audio::{AudioOut, Resampler};
use crate::core::{detect_platform, run_linked_frame, Core, LinkCable, Platform};
use crate::gamepad::{self, Gamepads};
use crate::rewind::{Rewind, Snapshot};

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

    pub fn bit(self) -> u16 {
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
    /// Super Game Boy borders (mGBA's `sgb.borders`)
    pub sgb_borders: bool,
    pub skip_bios: bool,
    pub keymap: Vec<(Button, Key)>,
    /// Player 2's keyboard layout, used while a second player is linked.
    pub keymap_p2: Vec<(Button, Key)>,
    /// Controller bindings: button -> Linux input event code.
    pub pad_map: Vec<(Button, u16)>,
    pub recent_roms: Vec<PathBuf>,
    pub fast_forward_multiplier: u32,
    pub show_fps: bool,
    pub rewind_enabled: bool,
    /// Seconds of history kept for rewinding.
    pub rewind_seconds: u32,
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
            sgb_borders: true,
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
            keymap_p2: vec![
                (Button::Up, Key::I),
                (Button::Down, Key::K),
                (Button::Left, Key::J),
                (Button::Right, Key::L),
                (Button::A, Key::M),
                (Button::B, Key::N),
                (Button::L, Key::U),
                (Button::R, Key::O),
                (Button::Start, Key::P),
                (Button::Select, Key::Y),
            ],
            pad_map: gamepad::default_mapping(),
            recent_roms: Vec::new(),
            fast_forward_multiplier: 4,
            show_fps: true,
            rewind_enabled: true,
            rewind_seconds: 30,
            pause_when_unfocused: false,
        }
    }
}

impl Settings {
    fn keymap_mut(&mut self, player: usize) -> &mut Vec<(Button, Key)> {
        if player == 0 {
            &mut self.keymap
        } else {
            &mut self.keymap_p2
        }
    }

    fn key_for(&self, player: usize, b: Button) -> Option<Key> {
        let map = if player == 0 { &self.keymap } else { &self.keymap_p2 };
        map.iter().find(|(btn, _)| *btn == b).map(|(_, k)| *k)
    }

    fn set_key(&mut self, player: usize, b: Button, key: Key) {
        let map = self.keymap_mut(player);
        map.retain(|(btn, _)| *btn != b);
        map.push((b, key));
    }
}

/// A second player on a link cable (mGBA's `MultiplayerController` with two
/// games attached), shown beside player 1.
struct Link {
    cable: LinkCable,
    p2: Emulator,
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

    /// mGBA's cheats file for this game (`<rom>.cheats`).
    fn cheats_path(&self) -> PathBuf {
        self.rom_path.with_extension("cheats")
    }

    fn write_cheats(&self) -> std::io::Result<()> {
        let path = self.cheats_path();
        let cheats = self.core.cheats();
        if cheats.sets.is_empty() {
            if path.exists() {
                std::fs::remove_file(path)?;
            }
            return Ok(());
        }
        std::fs::write(path, cheats.save_file())
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
    /// Keyboard key being rebound: (player, button).
    rebinding: Option<(usize, Button)>,
    rebinding_pad: Option<Button>,
    gamepads: Gamepads,
    link: Option<Link>,
    texture_p2: Option<TextureHandle>,
    rewind: Rewind,
    rewind_tick: u32,
    rewinding: bool,
    /// Frame to show instead of the core's (the frame restored by rewind).
    shown_frame: Option<Vec<u32>>,
    pad_pressed: Vec<u16>,
    show_about: bool,
    show_cheats: bool,
    cheat_name: String,
    cheat_codes: String,
    cheat_error: Option<String>,
    show_log: bool,
    log_lines: Vec<String>,
    fullscreen: bool,
}

impl App {
    /// `rom` is loaded for player 1; `link` (`Some(None)` for the same game)
    /// starts a linked player 2.
    pub fn new(cc: &eframe::CreationContext<'_>, rom: Option<PathBuf>, link: Option<Option<PathBuf>>) -> App {
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
            rebinding_pad: None,
            gamepads: Gamepads::new(),
            link: None,
            texture_p2: None,
            rewind: Rewind::new(1),
            rewind_tick: 0,
            rewinding: false,
            shown_frame: None,
            pad_pressed: Vec::new(),
            show_about: false,
            show_cheats: false,
            cheat_name: String::new(),
            cheat_codes: String::new(),
            cheat_error: None,
            show_log: false,
            log_lines: Vec::new(),
            fullscreen: false,
        };
        if app.audio.is_none() {
            app.set_status("No audio device available; running without sound");
        }
        if let Some(path) = rom {
            app.load_rom(&cc.egui_ctx, &path);
            if let Some(p2) = link {
                app.start_player2(&cc.egui_ctx, p2);
            }
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
        let mut emu = Emulator { core, rom_path: path.to_path_buf(), save_path, title: title.clone() };
        if let Ok(text) = std::fs::read_to_string(emu.cheats_path()) {
            emu.core.load_cheats(&text);
        }
        self.emu = Some(emu);
        self.rewind.clear();
        self.shown_frame = None;
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
        gb.video.sgb_borders = self.settings.sgb_borders;
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
        self.stop_player2();
        if let Some(mut emu) = self.emu.take() {
            if let Err(e) = emu.write_save() {
                self.error = Some(format!("Couldn't write save file {}: {e}", emu.save_path.display()));
            }
        }
    }

    // ----- Link cable -----

    /// Start a second player linked to the first, running `path` (or player
    /// 1's game). Like mGBA, a second copy of the same game gets its own save
    /// file (`.sa2`).
    fn start_player2(&mut self, ctx: &egui::Context, path: Option<PathBuf>) {
        let Some(p1) = &self.emu else { return };
        if self.link.is_some() {
            return;
        }
        let path = path.unwrap_or_else(|| p1.rom_path.clone());
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                self.error = Some(format!("Couldn't read {}: {e}", path.display()));
                return;
            }
        };
        let platform = p1.core.platform();
        if detect_platform(&path, &data) != Some(platform) {
            let what = match platform {
                Platform::Gba => "Game Boy Advance",
                Platform::Gb => "Game Boy",
            };
            self.error = Some(format!("Player 2 needs a {what} game to link with player 1"));
            return;
        }
        let save_path = if path == p1.rom_path { path.with_extension("sa2") } else { path.with_extension("sav") };
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
        let mut p2 = Emulator { core, rom_path: path, save_path, title };
        let cable = LinkCable::new(platform);
        let p1 = self.emu.as_mut().unwrap();
        if !p1.core.attach_link(&cable, 0) || !p2.core.attach_link(&cable, 1) {
            p1.core.detach_link();
            self.error = Some("Couldn't connect the link cable".into());
            return;
        }
        // States and rewind would desynchronize the two sides.
        self.rewind.clear();
        self.shown_frame = None;
        let title = p2.title.clone();
        self.link = Some(Link { cable, p2 });
        let [w, h] = self.emu.as_ref().unwrap().core.screen_size();
        let s = self.settings.scale;
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w as f32 * s * 2.0, h as f32 * s + 68.0)));
        self.set_status(format!("Player 2 connected: {title}"));
    }

    /// Disconnect player 2 and write its save.
    fn stop_player2(&mut self) {
        let Some(mut link) = self.link.take() else { return };
        if let Err(e) = link.p2.write_save() {
            self.error = Some(format!("Couldn't write save file {}: {e}", link.p2.save_path.display()));
        }
        link.p2.core.detach_link();
        if let Some(emu) = &mut self.emu {
            emu.core.detach_link();
        }
        drop(link.cable);
        self.texture_p2 = None;
        self.set_status("Player 2 disconnected");
    }

    fn open_player2_dialog(&mut self, ctx: &egui::Context) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Open ROM for player 2")
            .add_filter("Game Boy / Game Boy Advance ROM", &["gba", "agb", "bin", "gb", "gbc", "sgb", "cgb"]);
        if let Some(dir) = self.emu.as_ref().and_then(|e| e.rom_path.parent()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.start_player2(ctx, Some(path));
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
        if self.link.is_some() {
            self.set_status("Save states are off while player 2 is linked");
            return;
        }
        let Some(emu) = &self.emu else { return };
        let path = emu.state_path(slot);
        match std::fs::write(&path, emu.core.save_state()) {
            Ok(()) => self.set_status(format!("Saved state to slot {slot}")),
            Err(e) => self.error = Some(format!("Couldn't write {}: {e}", path.display())),
        }
    }

    fn load_state(&mut self, slot: u32) {
        if self.link.is_some() {
            self.set_status("Save states are off while player 2 is linked");
            return;
        }
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
            if let Some(link) = &mut self.link {
                link.p2.core.reset();
            }
            self.shown_frame = None;
            self.set_status("Reset");
        }
    }

    // ----- Emulation -----

    /// Keys held for player 1 and player 2.
    fn read_keys(&mut self, ctx: &egui::Context) -> (u16, u16) {
        let (pressed, newly) = self.gamepads.poll();
        self.pad_pressed = pressed;
        if let (Some(button), Some(c)) = (self.rebinding_pad, newly) {
            self.settings.pad_map.retain(|(b, _)| *b != button);
            self.settings.pad_map.push((button, c));
            self.rebinding_pad = None;
            return (0, 0);
        }
        if self.rebinding.is_some() || self.rebinding_pad.is_some() {
            return (0, 0);
        }
        // With a second player and two controllers, the second one is theirs.
        let per_pad = self.gamepads.pressed_per_pad();
        let (pad1, pad2) = if self.link.is_some() && per_pad.len() >= 2 {
            let map = &self.settings.pad_map;
            let rest: Vec<u16> = per_pad.iter().enumerate().filter(|(i, _)| *i != 1).flat_map(|(_, p)| p.iter().copied()).collect();
            (Gamepads::keys(&rest, map), Gamepads::keys(&per_pad[1], map))
        } else {
            (Gamepads::keys(&self.pad_pressed, &self.settings.pad_map), 0)
        };
        if ctx.memory(|m| m.focused().is_some()) {
            return (pad1, pad2);
        }
        let held = |map: &[(Button, Key)]| {
            ctx.input(|i| map.iter().filter(|(_, k)| i.key_down(*k)).fold(0u16, |acc, (b, _)| acc | b.bit()))
        };
        let p2 = if self.link.is_some() { held(&self.settings.keymap_p2) } else { 0 };
        (pad1 | held(&self.settings.keymap), pad2 | p2)
    }

    fn run_emulation(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let dt = self.last_tick.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
        self.last_tick = Some(now);

        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let running = self.emu.is_some() && !self.paused && !(self.settings.pause_when_unfocused && !focused);
        // Poll input even while stopped so controller rebinding works.
        let (keys, keys_p2) = self.read_keys(ctx);
        if !running {
            self.acc = 0.0;
            if self.rebinding_pad.is_some() {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
            return;
        }

        let rewind_held = self.settings.rewind_enabled
            && self.link.is_none()
            && self.rebinding.is_none()
            && ctx.memory(|m| m.focused().is_none())
            && ctx.input(|i| i.key_down(Key::Backtick));
        if rewind_held {
            self.rewinding = true;
            self.acc = 0.0;
            let emu = self.emu.as_mut().unwrap();
            // Step back one snapshot per displayed frame.
            if let Some(snap) = self.rewind.pop() {
                if emu.core.load_state(&snap.state).is_ok() {
                    self.shown_frame = Some(snap.frame);
                }
                if self.rewind.len() == 0 {
                    // Keep the oldest moment so rewinding again stays there.
                    self.rewind.push(Snapshot { state: emu.core.save_state(), frame: self.shown_frame.clone().unwrap_or_default() });
                }
            }
            if let Some(a) = &self.audio {
                a.clear();
            }
            return;
        }
        if self.rewinding {
            self.rewinding = false;
            self.last_tick = Some(Instant::now());
        }

        let fast_forward = self.fast_forward_toggle || ctx.input(|i| i.key_down(Key::Tab));
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
        // One snapshot every other frame, like mGBA's rewind interval setting.
        const REWIND_INTERVAL: u32 = 2;
        let capacity = (self.settings.rewind_seconds as f64 * fps_target / REWIND_INTERVAL as f64) as usize;
        self.rewind.capacity = capacity.max(1);
        if frames > 0 {
            self.shown_frame = None;
        }
        if let Some(link) = &mut self.link {
            link.p2.core.set_keys(keys_p2);
        }
        for _ in 0..frames {
            if let Some(link) = &mut self.link {
                run_linked_frame(&mut [&mut emu.core, &mut link.p2.core]);
                // Only player 1 is heard.
                let mut discard = Vec::new();
                link.p2.core.drain_audio(&mut discard);
            } else {
                emu.core.run_frame();
            }
            if self.settings.rewind_enabled && self.link.is_none() {
                self.rewind_tick += 1;
                if self.rewind_tick >= REWIND_INTERVAL {
                    self.rewind_tick = 0;
                    self.rewind.push(Snapshot { state: emu.core.save_state(), frame: emu.core.framebuffer().to_vec() });
                }
            }
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
        if let Some(link) = &mut self.link {
            if link.p2.core.take_savedata_updated() {
                if let Err(e) = link.p2.write_save() {
                    self.error = Some(format!("Couldn't write player 2's save file: {e}"));
                }
            }
            for (level, msg) in link.p2.core.drain_log() {
                if self.show_log && !matches!(level, LogLevel::Debug) {
                    self.log_lines.push(format!("[P2] [{level:?}] {msg}"));
                }
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
            Some(emu) => {
                let size = emu.core.screen_size();
                let src: &[u32] = match &self.shown_frame {
                    Some(f) if f.len() == size[0] * size[1] => f,
                    _ => emu.core.framebuffer(),
                };
                (size, src.iter().map(|&p| Color32::from_rgb(p as u8, (p >> 8) as u8, (p >> 16) as u8)).collect())
            }
            None => ([GBA_W, GBA_H], vec![Color32::BLACK; GBA_W * GBA_H]),
        };
        let image = ColorImage::new(size, pixels);
        let options = if self.settings.smooth { TextureOptions::LINEAR } else { TextureOptions::NEAREST };
        match &mut self.texture {
            Some(t) => t.set(image, options),
            None => self.texture = Some(ctx.load_texture("screen", image, options)),
        }
        if let Some(link) = &self.link {
            let size = link.p2.core.screen_size();
            let pixels = link.p2.core.framebuffer().iter().map(|&p| Color32::from_rgb(p as u8, (p >> 8) as u8, (p >> 16) as u8)).collect();
            let image = ColorImage::new(size, pixels);
            match &mut self.texture_p2 {
                Some(t) => t.set(image, options),
                None => self.texture_p2 = Some(ctx.load_texture("screen-p2", image, options)),
            }
        }
    }

    // ----- Input / hotkeys -----

    fn handle_hotkeys(&mut self, ctx: &egui::Context) {
        if let Some((player, button)) = self.rebinding {
            let pressed = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, .. } => Some(*key),
                    _ => None,
                })
            });
            if let Some(key) = pressed {
                if key != Key::Escape {
                    self.settings.set_key(player, button, key);
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
                if ui.checkbox(&mut self.settings.rewind_enabled, "Rewind (hold `)").changed() && !self.settings.rewind_enabled {
                    self.rewind.clear();
                }
                ui.add_enabled_ui(self.settings.rewind_enabled, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Rewind history:");
                        ui.add(egui::Slider::new(&mut self.settings.rewind_seconds, 5..=120).suffix(" s"));
                    });
                    if self.emu.is_some() {
                        ui.label(egui::RichText::new(format!("{:.1} MB in use", self.rewind.memory() as f64 / 1e6)).small());
                    }
                });
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

            ui.menu_button("Link", |ui| {
                let has = self.emu.is_some();
                let linked = self.link.is_some();
                if ui.add_enabled(has && !linked, egui::Button::new("Start player 2 with this game")).clicked() {
                    ui.close();
                    self.start_player2(&ctx, None);
                }
                if ui.add_enabled(has && !linked, egui::Button::new("Start player 2 with another game…")).clicked() {
                    ui.close();
                    self.open_player2_dialog(&ctx);
                }
                if ui.add_enabled(linked, egui::Button::new("Disconnect player 2")).clicked() {
                    ui.close();
                    self.stop_player2();
                }
                ui.separator();
                ui.label(egui::RichText::new("Player 2 runs beside player 1 on a link cable.\nKeys: Settings → Controls.").small());
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
                if ui.checkbox(&mut self.settings.sgb_borders, "Super Game Boy borders").changed() {
                    if let Some(Emulator { core: Core::Gb(g), .. }) = &mut self.emu {
                        g.set_sgb_borders(self.settings.sgb_borders);
                    }
                }
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
                if ui.add_enabled(self.emu.is_some(), egui::Button::new("Cheats…")).clicked() {
                    self.show_cheats = true;
                    ui.close();
                }
                if ui.checkbox(&mut self.show_log, "Show emulator log").changed() {
                    if let Some(emu) = &mut self.emu {
                        emu.core.set_log_enabled(self.show_log);
                    }
                    if let Some(link) = &mut self.link {
                        link.p2.core.set_log_enabled(self.show_log);
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

    /// Fit a `w`×`h` screen in `area`, centred.
    fn fit(&self, area: egui::Rect, w: usize, h: usize) -> egui::Rect {
        let mut scale = (area.width() / w as f32).min(area.height() / h as f32);
        if self.settings.integer_scaling && scale >= 1.0 {
            scale = scale.floor();
        }
        egui::Rect::from_center_size(area.center(), egui::vec2(w as f32 * scale, h as f32 * scale))
    }

    fn screen(&mut self, ui: &mut egui::Ui) {
        let mut available = ui.available_rect_before_wrap();
        ui.painter().rect_filled(available, 0.0, Color32::BLACK);
        let Some(tex) = &self.texture else { return };
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        if let (Some(link), Some(tex2)) = (&self.link, &self.texture_p2) {
            // Player 2 gets the right half of the window.
            let (left, right) = available.split_left_right_at_fraction(0.5);
            available = left;
            let [w, h] = link.p2.core.screen_size();
            let rect = self.fit(right, w, h);
            ui.painter().image(tex2.id(), rect, uv, Color32::WHITE);
            let label = egui::FontId::proportional(13.0);
            let faint = Color32::from_rgba_unmultiplied(255, 255, 255, 160);
            ui.painter().text(right.left_top() + egui::vec2(6.0, 4.0), egui::Align2::LEFT_TOP, "P2", label.clone(), faint);
            ui.painter().text(left.left_top() + egui::vec2(6.0, 4.0), egui::Align2::LEFT_TOP, "P1", label, faint);
        }
        let [w, h] = self.emu.as_ref().map(|e| e.core.screen_size()).unwrap_or([GBA_W, GBA_H]);
        let rect = self.fit(available, w, h);
        if self.emu.is_some() {
            ui.painter().image(tex.id(), rect, uv, Color32::WHITE);
            if self.rewinding {
                ui.painter().text(
                    rect.left_top() + egui::vec2(8.0, 6.0),
                    egui::Align2::LEFT_TOP,
                    "◀◀ Rewinding",
                    egui::FontId::proportional(16.0),
                    Color32::from_rgba_unmultiplied(255, 255, 255, 220),
                );
            }
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
                if let Some(link) = &self.link {
                    ui.separator();
                    ui.label(format!("Linked with player 2: {}", link.p2.title));
                }
            }
            if let Some((msg, at)) = &self.status {
                if at.elapsed() < Duration::from_secs(4) {
                    ui.separator();
                    ui.label(msg);
                }
            }
        });
    }

    fn cheats_window(&mut self, ctx: &egui::Context) {
        if !self.show_cheats {
            return;
        }
        let Some(emu) = &mut self.emu else {
            self.show_cheats = false;
            return;
        };
        let mut open = true;
        let mut changed = false;
        let mut remove = None;
        egui::Window::new("Cheats").open(&mut open).default_size([420.0, 360.0]).show(ctx, |ui| {
            let sets = &mut emu.core.cheats_mut().sets;
            if sets.is_empty() {
                ui.label("No cheats for this game yet.");
            }
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                for (i, set) in sets.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let name = if set.name.is_empty() { "(unnamed)".to_string() } else { set.name.clone() };
                        if ui.checkbox(&mut set.enabled, name).changed() {
                            changed = true;
                        }
                        ui.label(egui::RichText::new(format!("{} line(s)", set.lines.len())).small().weak());
                        if ui.small_button("Remove").clicked() {
                            remove = Some(i);
                        }
                    })
                    .response
                    .on_hover_text(set.lines.join("\n"));
                }
            });
            ui.separator();
            ui.label("Add cheat");
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut self.cheat_name);
            });
            ui.add(
                egui::TextEdit::multiline(&mut self.cheat_codes)
                    .hint_text("One code per line: GameShark, Action Replay, CodeBreaker, Game Genie or VBA format")
                    .desired_rows(5)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
            if ui.button("Add").clicked() && !self.cheat_codes.trim().is_empty() {
                let mut set = emu.core.new_cheat_set(self.cheat_name.trim());
                let bad: Vec<String> = self
                    .cheat_codes
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .filter(|l| !set.add_line(l))
                    .map(str::to_string)
                    .collect();
                if set.lines.is_empty() {
                    self.cheat_error = Some("None of those lines are codes this system understands.".into());
                } else {
                    self.cheat_error = (!bad.is_empty()).then(|| format!("Skipped unrecognized line(s): {}", bad.join(", ")));
                    emu.core.cheats_mut().sets.push(set);
                    self.cheat_name.clear();
                    self.cheat_codes.clear();
                    changed = true;
                }
            }
            if let Some(err) = &self.cheat_error {
                ui.colored_label(Color32::from_rgb(230, 120, 80), err);
            }
            ui.label(egui::RichText::new(format!("Saved to {}", emu.cheats_path().display())).small().weak());
        });
        if let Some(i) = remove {
            emu.core.remove_cheat_set(i);
            changed = true;
        }
        if changed {
            if let Err(e) = emu.write_cheats() {
                self.error = Some(format!("Couldn't write cheats file: {e}"));
            }
        }
        if !open {
            self.show_cheats = false;
            self.cheat_error = None;
        }
    }

    fn windows(&mut self, ctx: &egui::Context) {
        self.cheats_window(ctx);
        if self.show_keymap {
            let mut open = true;
            egui::Window::new("Controls").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
                egui::Grid::new("keymap").striped(true).show(ui, |ui| {
                    ui.strong("Button");
                    ui.strong("Keyboard");
                    ui.strong("Controller");
                    ui.strong("Player 2 keyboard");
                    ui.end_row();
                    for b in Button::ALL {
                        ui.label(b.name());
                        let label = if self.rebinding == Some((0, b)) {
                            "Press a key… (Esc cancels)".to_string()
                        } else {
                            self.settings.key_for(0, b).map(|k| k.name().to_string()).unwrap_or_else(|| "—".into())
                        };
                        if ui.button(label).clicked() {
                            self.rebinding = Some((0, b));
                            self.rebinding_pad = None;
                        }
                        let pad_label = if self.rebinding_pad == Some(b) {
                            "Press a controller button…".to_string()
                        } else {
                            self.settings
                                .pad_map
                                .iter()
                                .find(|(btn, _)| *btn == b)
                                .map(|(_, c)| gamepad::button_name(*c))
                                .unwrap_or_else(|| "—".into())
                        };
                        if ui.button(pad_label).clicked() {
                            self.rebinding_pad = Some(b);
                            self.rebinding = None;
                        }
                        let label = if self.rebinding == Some((1, b)) {
                            "Press a key… (Esc cancels)".to_string()
                        } else {
                            self.settings.key_for(1, b).map(|k| k.name().to_string()).unwrap_or_else(|| "—".into())
                        };
                        if ui.button(label).clicked() {
                            self.rebinding = Some((1, b));
                            self.rebinding_pad = None;
                        }
                        ui.end_row();
                    }
                });
                let pads = self.gamepads.connected();
                if pads.is_empty() {
                    ui.label("No controller detected (plug one in; the stick and d-pad both steer).");
                } else {
                    ui.label(format!("Controllers: {}", pads.join(", ")));
                }
                ui.label("While player 2 is linked, a second controller plays as player 2.");
                ui.separator();
                ui.label("Hold Tab to fast-forward and ` (backtick) to rewind. F1–F9 load and Shift+F1–F9 save states.");
                if ui.button("Restore defaults").clicked() {
                    self.settings.keymap = Settings::default().keymap;
                    self.settings.keymap_p2 = Settings::default().keymap_p2;
                    self.settings.pad_map = gamepad::default_mapping();
                }
            });
            if !open {
                self.show_keymap = false;
                self.rebinding = None;
                self.rebinding_pad = None;
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
