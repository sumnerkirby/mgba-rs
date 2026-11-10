// Copyright (c) 2013-2022 Jeffrey Pfau
// Lua scripting with mGBA's script API: the `emu`, `console`, `callbacks`,
// `C` and `util` globals that mGBA's scripting bridge defines
// (src/core/scripting.c, src/script/stdlib.c, src/script/console.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! mGBA reaches Lua through its own reflection layer (`src/script/types.c`,
//! `src/script/engines/lua.c`); this crate binds the same names directly
//! with `mlua`, so scripts written for mGBA's API run unchanged as long as
//! they stay within what is listed in the README.
//!
//! Scripts only touch the emulator while the frontend lends it to them
//! ([`ScriptHost::with_core`]): loading a script and triggering callbacks
//! both take the core, and `emu` methods fail outside those calls.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use mgba_core::core::{MCore, MemoryBlock, System};
use mlua::{Function, Lua, Table, UserData, UserDataFields, UserDataMethods, Value};

/// How a script message was logged (`console:log/warn/error`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogKind {
    Log,
    Warn,
    Error,
}

/// `struct mScriptTextBuffer`: a named text panel a script writes into.
#[derive(Clone, Debug, Default)]
pub struct TextBuffer {
    pub name: String,
    pub cols: u32,
    pub rows: u32,
    pub lines: Vec<String>,
    pub x: u32,
    pub y: u32,
}

impl TextBuffer {
    fn new(name: &str) -> TextBuffer {
        TextBuffer { name: name.to_string(), cols: 80, rows: 24, lines: vec![String::new()], x: 0, y: 0 }
    }

    /// `mScriptTextBufferPrint`: write at the cursor, wrapping and scrolling.
    fn print(&mut self, text: &str) {
        for ch in text.chars() {
            if ch == '\n' {
                self.y += 1;
                self.x = 0;
            } else {
                while self.lines.len() <= self.y as usize {
                    self.lines.push(String::new());
                }
                let line = &mut self.lines[self.y as usize];
                let mut chars: Vec<char> = line.chars().collect();
                while chars.len() < self.x as usize {
                    chars.push(' ');
                }
                if (self.x as usize) < chars.len() {
                    chars[self.x as usize] = ch;
                } else {
                    chars.push(ch);
                }
                *line = chars.into_iter().collect();
                self.x += 1;
                if self.x >= self.cols {
                    self.x = 0;
                    self.y += 1;
                }
            }
            while self.y >= self.rows {
                self.lines.remove(0);
                self.y -= 1;
            }
            while self.lines.len() <= self.y as usize {
                self.lines.push(String::new());
            }
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }
}

struct Shared {
    /// The core while the frontend lends it (`with_core`).
    core: Cell<Option<*mut (dyn MCore + 'static)>>,
    /// Keys the script holds down (`setKeys`/`addKey`/...), ORed into input.
    keys: Cell<u32>,
    output: RefCell<Vec<(LogKind, String)>>,
    buffers: RefCell<Vec<TextBuffer>>,
    next_callback: Cell<i64>,
}

impl Shared {
    fn core(&self) -> mlua::Result<&mut dyn MCore> {
        match self.core.get() {
            // SAFETY: the pointer is only set inside `ScriptHost::with_core`,
            // which holds the core's unique borrow for the whole call, and
            // scripts run on that thread only.
            Some(p) => Ok(unsafe { &mut *p }),
            None => Err(mlua::Error::RuntimeError("no game is running".into())),
        }
    }

    fn log(&self, kind: LogKind, text: String) {
        self.output.borrow_mut().push((kind, text));
    }
}

/// A loaded set of scripts (`struct mScriptContext` with the bridge).
pub struct ScriptHost {
    lua: Lua,
    shared: Rc<Shared>,
}

/// `emu`: the core (`struct mScriptCoreAdapter` over `struct mCore`).
struct Emu(Rc<Shared>);

/// `emu.memory.<name>` (`struct mScriptMemoryDomain`).
struct MemoryDomain {
    shared: Rc<Shared>,
    block: MemoryBlock,
}

/// `console:createBuffer` result.
struct Buffer {
    shared: Rc<Shared>,
    index: usize,
}

/// `console`
struct Console(Rc<Shared>);

/// `callbacks` (`struct mScriptCallbackManager`)
struct Callbacks(Rc<Shared>);

impl MemoryDomain {
    /// `CALCULATE_SEGMENT_INFO` + `CALCULATE_SEGMENT_ADDRESS`: an offset into
    /// the domain as an address and bank.
    fn locate(&self, address: u32) -> (u32, i32) {
        let b = &self.block;
        let mut segment_size = b.end.wrapping_sub(b.start);
        let segment_start = b.segment_start.wrapping_sub(b.start);
        if b.segment_start != 0 {
            segment_size = segment_size.wrapping_sub(segment_start);
        }
        if segment_size == 0 {
            return (b.start.wrapping_add(address), -1);
        }
        let mut segment_address = address % segment_size;
        let segment = (address / segment_size) as i32;
        segment_address = segment_address.wrapping_add(b.start);
        if b.segment_start != 0 && segment != 0 {
            segment_address = segment_address.wrapping_add(segment_start);
        }
        let segment = if b.max_segment == 0 { -1 } else { segment };
        (segment_address, segment)
    }
}

impl UserData for MemoryDomain {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("read8", |_, this, address: u32| {
            let (a, s) = this.locate(address);
            Ok(this.shared.core()?.raw_read8(a, s))
        });
        methods.add_method("read16", |_, this, address: u32| {
            let (a, s) = this.locate(address);
            Ok(this.shared.core()?.raw_read16(a, s))
        });
        methods.add_method("read32", |_, this, address: u32| {
            let (a, s) = this.locate(address);
            Ok(this.shared.core()?.raw_read32(a, s))
        });
        methods.add_method("write8", |_, this, (address, value): (u32, u32)| {
            let (a, s) = this.locate(address);
            this.shared.core()?.raw_write8(a, s, value);
            Ok(())
        });
        methods.add_method("write16", |_, this, (address, value): (u32, u32)| {
            let (a, s) = this.locate(address);
            this.shared.core()?.raw_write16(a, s, value);
            Ok(())
        });
        methods.add_method("write32", |_, this, (address, value): (u32, u32)| {
            let (a, s) = this.locate(address);
            this.shared.core()?.raw_write32(a, s, value);
            Ok(())
        });
        methods.add_method("readRange", |lua, this, (address, length): (u32, u32)| {
            let core = this.shared.core()?;
            let bytes: Vec<u8> = (0..length)
                .map(|i| {
                    let (a, s) = this.locate(address.wrapping_add(i));
                    core.raw_read8(a, s) as u8
                })
                .collect();
            lua.create_string(&bytes)
        });
        methods.add_method("base", |_, this, ()| Ok(this.block.start));
        methods.add_method("bound", |_, this, ()| Ok(this.block.end));
        methods.add_method("size", |_, this, ()| Ok(this.block.size));
        methods.add_method("name", |_, this, ()| Ok(this.block.long_name.to_string()));
    }
}

impl UserData for Emu {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        // `emu.memory`: one domain per mapped block, by internal name.
        fields.add_field_method_get("memory", |lua, this| {
            let table = lua.create_table()?;
            for block in this.0.core()?.list_memory_blocks() {
                if block.id < 0 {
                    continue;
                }
                table.set(block.internal_name, MemoryDomain { shared: this.0.clone(), block })?;
            }
            Ok(table)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("read8", |_, this, a: u32| Ok(this.0.core()?.bus_read8(a)));
        methods.add_method("read16", |_, this, a: u32| Ok(this.0.core()?.bus_read16(a)));
        methods.add_method("read32", |_, this, a: u32| Ok(this.0.core()?.bus_read32(a)));
        methods.add_method("write8", |_, this, (a, v): (u32, u32)| {
            this.0.core()?.bus_write8(a, v);
            Ok(())
        });
        methods.add_method("write16", |_, this, (a, v): (u32, u32)| {
            this.0.core()?.bus_write16(a, v);
            Ok(())
        });
        methods.add_method("write32", |_, this, (a, v): (u32, u32)| {
            this.0.core()?.bus_write32(a, v);
            Ok(())
        });
        methods.add_method("readRange", |lua, this, (address, length): (u32, u32)| {
            let core = this.0.core()?;
            let bytes: Vec<u8> = (0..length).map(|i| core.bus_read8(address.wrapping_add(i)) as u8).collect();
            lua.create_string(&bytes)
        });
        methods.add_method("readRegister", |_, this, name: String| Ok(this.0.core()?.read_register(&name)));
        methods.add_method("writeRegister", |_, this, (name, value): (String, i32)| {
            Ok(this.0.core()?.write_register(&name, value))
        });
        methods.add_method("platform", |_, this, ()| {
            Ok(match this.0.core()?.system() {
                System::Gba => 0,
                System::Gb => 1,
            })
        });
        methods.add_method("frameCycles", |_, this, ()| Ok(this.0.core()?.frame_cycles()));
        methods.add_method("frequency", |_, this, ()| Ok(this.0.core()?.frequency()));
        methods.add_method("romSize", |_, this, ()| Ok(this.0.core()?.rom_size()));
        methods.add_method("checksum", |lua, this, _ty: Option<i32>| {
            lua.create_string(this.0.core()?.crc32().to_be_bytes())
        });
        methods.add_method("getGameTitle", |_, this, ()| Ok(this.0.core()?.game_title()));
        methods.add_method("getGameCode", |_, this, ()| Ok(this.0.core()?.game_code()));
        methods.add_method("currentFrame", |_, this, ()| Ok(this.0.core()?.frame_counter()));
        methods.add_method("currentCycle", |_, this, ()| Ok(this.0.core()?.global_time()));
        methods.add_method("reset", |_, this, ()| {
            this.0.core()?.reset();
            Ok(())
        });
        methods.add_method("runFrame", |_, this, ()| {
            this.0.core()?.run_frame();
            Ok(())
        });
        methods.add_method("step", |_, this, ()| {
            this.0.core()?.step();
            Ok(())
        });
        methods.add_method("setKeys", |_, this, keys: u32| {
            this.0.keys.set(keys);
            Ok(())
        });
        methods.add_method("addKey", |_, this, key: u32| {
            this.0.keys.set(this.0.keys.get() | (1 << key));
            Ok(())
        });
        methods.add_method("addKeys", |_, this, keys: u32| {
            this.0.keys.set(this.0.keys.get() | keys);
            Ok(())
        });
        methods.add_method("clearKey", |_, this, key: u32| {
            this.0.keys.set(this.0.keys.get() & !(1 << key));
            Ok(())
        });
        methods.add_method("clearKeys", |_, this, keys: u32| {
            this.0.keys.set(this.0.keys.get() & !keys);
            Ok(())
        });
        methods.add_method("getKey", |_, this, key: u32| Ok((this.0.keys.get() >> key) & 1));
        methods.add_method("getKeys", |_, this, ()| Ok(this.0.keys.get()));
        methods.add_method("saveStateBuffer", |lua, this, _flags: Option<i32>| {
            lua.create_string(this.0.core()?.save_state_bytes())
        });
        methods.add_method("loadStateBuffer", |_, this, (data, _flags): (mlua::String, Option<i32>)| {
            Ok(this.0.core()?.load_state_bytes(&data.as_bytes()))
        });
    }
}

impl UserData for Buffer {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        fn with<R>(this: &Buffer, f: impl FnOnce(&mut TextBuffer) -> R) -> R {
            f(&mut this.shared.buffers.borrow_mut()[this.index])
        }
        methods.add_method("print", |_, this, text: String| {
            with(this, |b| b.print(&text));
            Ok(())
        });
        methods.add_method("clear", |_, this, ()| {
            with(this, |b| {
                b.lines = vec![String::new()];
                b.x = 0;
                b.y = 0;
            });
            Ok(())
        });
        methods.add_method("setSize", |_, this, (cols, rows): (u32, u32)| {
            with(this, |b| {
                b.cols = cols.max(1);
                b.rows = rows.max(1);
            });
            Ok(())
        });
        methods.add_method("moveCursor", |_, this, (x, y): (u32, u32)| {
            with(this, |b| {
                b.x = x.min(b.cols.saturating_sub(1));
                b.y = y.min(b.rows.saturating_sub(1));
            });
            Ok(())
        });
        methods.add_method("advance", |_, this, n: i32| {
            with(this, |b| {
                let pos = (b.y * b.cols + b.x) as i64 + n as i64;
                let pos = pos.max(0) as u32;
                b.x = pos % b.cols;
                b.y = (pos / b.cols).min(b.rows - 1);
            });
            Ok(())
        });
        methods.add_method("getX", |_, this, ()| Ok(with(this, |b| b.x)));
        methods.add_method("getY", |_, this, ()| Ok(with(this, |b| b.y)));
        methods.add_method("cols", |_, this, ()| Ok(with(this, |b| b.cols)));
        methods.add_method("rows", |_, this, ()| Ok(with(this, |b| b.rows)));
        methods.add_method("setName", |_, this, name: String| {
            with(this, |b| b.name = name);
            Ok(())
        });
    }
}

fn stringify(values: mlua::MultiValue) -> mlua::Result<String> {
    let mut parts = Vec::new();
    for v in values {
        parts.push(match v {
            Value::String(s) => s.to_str()?.to_string(),
            other => other.to_string()?,
        });
    }
    Ok(parts.join(" "))
}

impl UserData for Console {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("log", |_, this, msg: mlua::MultiValue| {
            this.0.log(LogKind::Log, stringify(msg)?);
            Ok(())
        });
        methods.add_method("warn", |_, this, msg: mlua::MultiValue| {
            this.0.log(LogKind::Warn, stringify(msg)?);
            Ok(())
        });
        methods.add_method("error", |_, this, msg: mlua::MultiValue| {
            this.0.log(LogKind::Error, stringify(msg)?);
            Ok(())
        });
        methods.add_method("createBuffer", |_, this, name: Option<String>| {
            let mut buffers = this.0.buffers.borrow_mut();
            buffers.push(TextBuffer::new(name.as_deref().unwrap_or("")));
            Ok(Buffer { shared: this.0.clone(), index: buffers.len() - 1 })
        });
    }
}

const CALLBACKS_KEY: &str = "mgba_rs_callbacks";

impl UserData for Callbacks {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        fn add(lua: &Lua, shared: &Shared, name: String, f: Function, oneshot: bool) -> mlua::Result<i64> {
            let table: Table = lua.named_registry_value(CALLBACKS_KEY)?;
            let id = shared.next_callback.get() + 1;
            shared.next_callback.set(id);
            let entry = lua.create_table()?;
            entry.set("name", name)?;
            entry.set("fn", f)?;
            entry.set("oneshot", oneshot)?;
            table.set(id, entry)?;
            Ok(id)
        }
        methods.add_method("add", |lua, this, (name, f): (String, Function)| add(lua, &this.0, name, f, false));
        methods.add_method("oneshot", |lua, this, (name, f): (String, Function)| add(lua, &this.0, name, f, true));
        methods.add_method("remove", |lua, _this, id: i64| {
            let table: Table = lua.named_registry_value(CALLBACKS_KEY)?;
            table.set(id, Value::Nil)?;
            Ok(())
        });
    }
}

/// `C` constants from mGBA's script stdlib (`src/script/stdlib.c`).
fn constants(lua: &Lua) -> mlua::Result<Table> {
    let c = lua.create_table()?;
    let platform = lua.create_table()?;
    platform.set("NONE", -1)?;
    platform.set("GBA", 0)?;
    platform.set("GB", 1)?;
    c.set("PLATFORM", platform)?;
    let checksum = lua.create_table()?;
    checksum.set("CRC32", 0)?;
    c.set("CHECKSUM", checksum)?;
    let savestate = lua.create_table()?;
    for (k, v) in [("SCREENSHOT", 1), ("SAVEDATA", 2), ("CHEATS", 4), ("RTC", 8), ("METADATA", 16), ("ALL", 31)] {
        savestate.set(k, v)?;
    }
    c.set("SAVESTATE", savestate)?;
    let keys = ["A", "B", "SELECT", "START", "RIGHT", "LEFT", "UP", "DOWN", "R", "L"];
    let gba = lua.create_table()?;
    for (i, k) in keys.iter().enumerate() {
        gba.set(*k, i)?;
    }
    c.set("GBA_KEY", gba)?;
    let gb = lua.create_table()?;
    for (i, k) in keys[..8].iter().enumerate() {
        gb.set(*k, i)?;
    }
    c.set("GB_KEY", gb)?;
    Ok(c)
}

/// `util` from mGBA's script stdlib.
fn util(lua: &Lua) -> mlua::Result<Table> {
    let util = lua.create_table()?;
    util.set(
        "makeBitmask",
        lua.create_function(|_, bits: Vec<u32>| Ok(bits.iter().fold(0u32, |m, b| m | (1u32 << (b & 31)))))?,
    )?;
    util.set(
        "expandBitmask",
        lua.create_function(|_, mask: u32| Ok((0..32).filter(|b| mask & (1 << b) != 0).collect::<Vec<u32>>()))?,
    )?;
    Ok(util)
}

impl ScriptHost {
    /// `mScriptContextInit` with the bridge's globals.
    pub fn new() -> mlua::Result<ScriptHost> {
        let lua = Lua::new();
        let shared = Rc::new(Shared {
            core: Cell::new(None),
            keys: Cell::new(0),
            output: RefCell::new(Vec::new()),
            buffers: RefCell::new(Vec::new()),
            next_callback: Cell::new(0),
        });
        lua.set_named_registry_value(CALLBACKS_KEY, lua.create_table()?)?;
        let g = lua.globals();
        g.set("emu", Emu(shared.clone()))?;
        g.set("console", Console(shared.clone()))?;
        g.set("callbacks", Callbacks(shared.clone()))?;
        g.set("C", constants(&lua)?)?;
        g.set("util", util(&lua)?)?;
        // `print` goes to the console like mGBA's.
        let s = shared.clone();
        g.set(
            "print",
            lua.create_function(move |_, msg: mlua::MultiValue| {
                s.log(LogKind::Log, stringify(msg)?);
                Ok(())
            })?,
        )?;
        Ok(ScriptHost { lua, shared })
    }

    /// Lend `core` to scripts for the duration of `f`.
    pub fn with_core<R>(&mut self, core: &mut dyn MCore, f: impl FnOnce(&mut ScriptHost) -> R) -> R {
        // SAFETY: lifetime erasure only; the pointer is cleared before `core`'s
        // borrow ends, and nothing keeps it past this call.
        let ptr: *mut (dyn MCore + 'static) = unsafe { std::mem::transmute(core as *mut dyn MCore) };
        self.shared.core.set(Some(ptr));
        let result = f(self);
        self.shared.core.set(None);
        result
    }

    /// Run a script's code (`mScriptContextLoadFile`), lending it `core`.
    pub fn load(&mut self, core: &mut dyn MCore, name: &str, code: &str) -> Result<(), String> {
        self.with_core(core, |host| host.lua.load(code).set_name(name).exec().map_err(|e| e.to_string()))
            .inspect_err(|e| self.shared.log(LogKind::Error, e.clone()))
    }

    /// `mScriptContextTriggerCallback`: run every callback registered for `name`.
    pub fn trigger(&mut self, core: &mut dyn MCore, name: &str) {
        self.with_core(core, |host| {
            let Ok(table) = host.lua.named_registry_value::<Table>(CALLBACKS_KEY) else { return };
            let mut due: Vec<(i64, Function, bool)> = Vec::new();
            for (id, entry) in table.clone().pairs::<i64, Table>().flatten() {
                if entry.get::<String>("name").ok().as_deref() == Some(name) {
                    if let Ok(f) = entry.get::<Function>("fn") {
                        due.push((id, f, entry.get::<bool>("oneshot").unwrap_or(false)));
                    }
                }
            }
            due.sort_by_key(|(id, _, _)| *id);
            for (id, f, oneshot) in due {
                if oneshot {
                    let _ = table.set(id, Value::Nil);
                }
                if let Err(e) = f.call::<()>(()) {
                    host.shared.log(LogKind::Error, e.to_string());
                }
            }
        })
    }

    /// Whether any callback is registered for `name`.
    pub fn has_callbacks(&self, name: &str) -> bool {
        let Ok(table) = self.lua.named_registry_value::<Table>(CALLBACKS_KEY) else { return false };
        table.pairs::<i64, Table>().flatten().any(|(_, e)| e.get::<String>("name").ok().as_deref() == Some(name))
    }

    /// Messages the scripts logged since the last call.
    pub fn drain_output(&mut self) -> Vec<(LogKind, String)> {
        std::mem::take(&mut *self.shared.output.borrow_mut())
    }

    /// Text buffers the scripts created.
    pub fn buffers(&self) -> Vec<TextBuffer> {
        self.shared.buffers.borrow().clone()
    }

    /// Keys the scripts are holding (GBA KEYINPUT bit order).
    pub fn keys(&self) -> u32 {
        self.shared.keys.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mgba_core::Gb;

    fn gb() -> Box<Gb> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100..0x104].copy_from_slice(&[0x00, 0xC3, 0x50, 0x01]);
        rom[0x134..0x13C].copy_from_slice(b"SCRIPTED");
        rom[0x150..0x152].copy_from_slice(&[0x18, 0xFE]);
        let mut g = Box::new(Gb::new());
        g.load_rom(rom);
        g.reset();
        g
    }

    #[test]
    fn mgba_style_script_runs() {
        let mut core = gb();
        let mut host = ScriptHost::new().unwrap();
        let script = r#"
            console:log("title", emu:getGameTitle(), emu:platform() == C.PLATFORM.GB)
            emu:write8(0xC000, 0x5A)
            emu.memory.wram:write8(1, 0x11)
            frames = 0
            callbacks:add("frame", function()
                frames = frames + 1
                emu.memory.wram:write8(2, frames)
            end)
            local id = callbacks:oneshot("frame", function() console:warn("once") end)
            emu:addKey(C.GB_KEY.START)
            buf = console:createBuffer("status")
            buf:print("hello\nworld")
            console:log(util.makeBitmask({0, 3}), #util.expandBitmask(9))
        "#;
        host.load(core.as_mut(), "test.lua", script).unwrap();
        for _ in 0..3 {
            core.run_frame();
            host.trigger(core.as_mut(), "frame");
        }
        let out = host.drain_output();
        assert_eq!(out[0], (LogKind::Log, "title SCRIPTED true".into()));
        assert_eq!(out[1], (LogKind::Log, "9 2".into()));
        assert_eq!(out.iter().filter(|(k, m)| *k == LogKind::Warn && m == "once").count(), 1);
        assert_eq!(core.view8(0xC000), 0x5A);
        assert_eq!(core.view8(0xC001), 0x11);
        assert_eq!(core.view8(0xC002), 3);
        assert_eq!(host.keys(), 1 << 3);
        assert_eq!(host.buffers()[0].text(), "hello\nworld");
        assert!(host.has_callbacks("frame"));
    }

    #[test]
    fn errors_are_reported() {
        let mut core = gb();
        let mut host = ScriptHost::new().unwrap();
        assert!(host.load(core.as_mut(), "bad.lua", "this is not lua").is_err());
        host.load(core.as_mut(), "cb.lua", r#"callbacks:add("frame", function() error("boom") end)"#).unwrap();
        host.trigger(core.as_mut(), "frame");
        let out = host.drain_output();
        assert!(out.iter().any(|(k, m)| *k == LogKind::Error && m.contains("boom")));
    }
}
