// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's cheat engine (src/core/cheats.c) with the Game Boy
// (src/gb/cheats.c) and GBA (src/gba/cheats.c, src/gba/cheats/*.c) code formats.
//
// Codes are parsed into a list of simple operations (`Cheat`) and ROM patches
// (`CheatPatch`), then applied once per frame by `CheatDevice::refresh`, as
// `mCheatRefresh` does from `GBFrameEnded`/`GBAFrameEnded`.
//
// One difference: GBA codes with a "hook" (an address the cheat engine should
// run at) are refreshed at the end of each frame like other codes, because
// this port has no ROM breakpoints to trap the hook address.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub mod gb;
pub mod gba;
mod tables;

use std::collections::HashMap;

/// `enum mCheatType`
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CheatType {
    #[default]
    Assign,
    AssignIndirect,
    And,
    Add,
    Sub,
    Or,
    IfEq,
    IfNe,
    IfLt,
    IfGt,
    IfLe,
    IfGe,
    IfUlt,
    IfUgt,
    IfAnd,
    IfLand,
    IfNand,
    IfButton,
    Never,
}

/// `struct mCheat`
#[derive(Clone, Copy, Debug, Default)]
pub struct Cheat {
    pub ty: CheatType,
    pub width: i32,
    pub address: u32,
    pub operand: u32,
    pub repeat: u32,
    pub negative_repeat: u32,
    pub address_offset: i32,
    pub operand_offset: i32,
}

/// `struct mCheatPatch`
#[derive(Clone, Copy, Debug, Default)]
pub struct CheatPatch {
    pub address: u32,
    pub segment: i32,
    pub value: u32,
    pub width: i32,
    pub applied: bool,
    pub check_value: u32,
    pub check: bool,
}

/// Which system a set's codes are for, with its parser state.
#[derive(Clone, Debug)]
pub enum SetKind {
    Gb,
    Gba(gba::GbaSetState),
}

/// `struct mCheatSet`
#[derive(Clone, Debug)]
pub struct CheatSet {
    pub name: String,
    pub enabled: bool,
    pub list: Vec<Cheat>,
    pub rom_patches: Vec<CheatPatch>,
    /// The code lines as entered (what the cheats file stores).
    pub lines: Vec<String>,
    pub kind: SetKind,
}

impl CheatSet {
    pub fn new_gb(name: &str) -> CheatSet {
        CheatSet { name: name.into(), enabled: true, list: vec![], rom_patches: vec![], lines: vec![], kind: SetKind::Gb }
    }

    pub fn new_gba(name: &str) -> CheatSet {
        CheatSet {
            name: name.into(),
            enabled: true,
            list: vec![],
            rom_patches: vec![],
            lines: vec![],
            kind: SetKind::Gba(gba::GbaSetState::new()),
        }
    }

    /// `mCheatAddLine` with autodetected code type.
    pub fn add_line(&mut self, line: &str) -> bool {
        let line = line.trim();
        if line.is_empty() {
            return true;
        }
        let ok = match self.kind {
            SetKind::Gb => gb::add_line(self, line),
            SetKind::Gba(_) => gba::add_line(self, line),
        };
        if ok {
            self.lines.push(line.to_string());
        }
        ok
    }

    /// `copyProperties`: a new set in a file inherits the previous set's
    /// code-format state (GBA encryption seeds).
    fn copy_properties(&mut self, old: &CheatSet) {
        if let (SetKind::Gba(new), SetKind::Gba(old)) = (&mut self.kind, &old.kind) {
            new.copy_properties(old);
        }
    }

    fn parse_directives(&mut self, directives: &[String]) {
        if let SetKind::Gba(state) = &mut self.kind {
            state.parse_directives(directives);
        }
    }

    fn dump_directives(&self) -> Vec<String> {
        match &self.kind {
            SetKind::Gba(state) => state.dump_directives(),
            SetKind::Gb => vec![],
        }
    }
}

/// Memory access the cheat engine needs from a core (`busRead`/`busWrite`
/// and `rawRead`/`rawWrite` in `struct mCore`).
pub trait CheatBus {
    fn bus_read(&mut self, address: u32, width: i32) -> u32;
    fn bus_write(&mut self, address: u32, width: i32, value: u32);
    fn raw_read(&mut self, address: u32, segment: i32, width: i32) -> u32;
    fn raw_write(&mut self, address: u32, segment: i32, width: i32, value: u32);
    /// `maxSegment` of the memory block containing `address`, if any.
    fn max_segment(&self, address: u32) -> Option<i32>;
}

/// `struct mCheatPatchedMem`
#[derive(Clone, Copy, Debug)]
struct PatchedMem {
    original_value: u32,
    refs: i32,
    dirty: bool,
}

/// `struct mCheatDevice`
#[derive(Clone, Debug, Default)]
pub struct CheatDevice {
    pub sets: Vec<CheatSet>,
    unpatched: HashMap<u32, PatchedMem>,
    pub button_down: bool,
}

fn patch_key(patch: &CheatPatch) -> u32 {
    // NB: This assumes patches have only one valid size per platform
    let mut key = patch.address;
    match patch.width {
        2 => key >>= 1,
        4 => key >>= 2,
        _ => {}
    }
    // TODO: More than one segment
    if patch.segment > 0 {
        key |= (patch.segment as u32) << 16;
    }
    key
}

impl CheatDevice {
    pub fn new() -> CheatDevice {
        CheatDevice::default()
    }

    /// `_patchROM`
    fn patch_rom<B: CheatBus>(&mut self, bus: &mut B, set_index: usize) {
        let set = &mut self.sets[set_index];
        for patch in set.rom_patches.iter_mut() {
            let mut segment = -1;
            if patch.check && patch.segment < 0 {
                let Some(max) = bus.max_segment(patch.address) else { continue };
                segment = 0;
                while segment < max {
                    if bus.raw_read(patch.address, segment, patch.width) == patch.check_value {
                        break;
                    }
                    segment += 1;
                }
                if segment == max {
                    continue;
                }
            }
            patch.segment = segment;

            let key = patch_key(patch);
            match self.unpatched.get_mut(&key) {
                None => {
                    let original_value = bus.raw_read(patch.address, segment, patch.width);
                    self.unpatched.insert(key, PatchedMem { original_value, refs: 1, dirty: false });
                }
                Some(data) => {
                    if !patch.applied {
                        data.refs += 1;
                        data.dirty = true;
                    } else if !data.dirty {
                        continue;
                    }
                }
            }
            bus.raw_write(patch.address, segment, patch.width, patch.value);
            patch.applied = true;
        }
    }

    /// `_unpatchROM`
    fn unpatch_rom<B: CheatBus>(&mut self, bus: &mut B, set_index: usize) {
        let set = &mut self.sets[set_index];
        for patch in set.rom_patches.iter_mut() {
            if !patch.applied {
                continue;
            }
            let key = patch_key(patch);
            if let Some(data) = self.unpatched.get_mut(&key) {
                data.refs -= 1;
                data.dirty = true;
                if data.refs <= 0 {
                    let original = data.original_value;
                    bus.raw_write(patch.address, patch.segment, patch.width, original);
                    self.unpatched.remove(&key);
                }
            }
            patch.applied = false;
        }
    }

    /// Remove a set, undoing its ROM patches (`mCheatRemoveSet` + unpatch).
    pub fn remove_set<B: CheatBus>(&mut self, bus: &mut B, index: usize) {
        if index >= self.sets.len() {
            return;
        }
        self.unpatch_rom(bus, index);
        self.sets.remove(index);
    }

    /// Undo every ROM patch (before the cheats are thrown away).
    pub fn unpatch_all<B: CheatBus>(&mut self, bus: &mut B) {
        for i in 0..self.sets.len() {
            self.unpatch_rom(bus, i);
        }
    }

    /// `mCheatRefresh` for every set.
    pub fn refresh_all<B: CheatBus>(&mut self, bus: &mut B) {
        for i in 0..self.sets.len() {
            self.refresh(bus, i);
        }
    }

    /// `mCheatRefresh`
    pub fn refresh<B: CheatBus>(&mut self, bus: &mut B, set_index: usize) {
        if self.sets[set_index].enabled {
            self.patch_rom(bus, set_index);
        }
        if !self.sets[set_index].enabled {
            self.unpatch_rom(bus, set_index);
            return;
        }

        let button_down = self.button_down;
        let cheats = &self.sets[set_index].list;
        let mut else_loc = 0usize;
        let mut end_loc = 0usize;
        let n = cheats.len();
        let mut i = 0usize;
        while i < n {
            let cheat = cheats[i];
            let mut value: u32 = 0;
            let mut operand = cheat.operand;
            let mut operations_remaining = cheat.repeat;
            let mut address = cheat.address;
            let mut perform_assignment = false;
            let mut condition = true;
            let mut condition_remaining: u32 = 0;
            let mut negative_condition_remaining: u32 = 0;

            while operations_remaining > 0 {
                let read = |bus: &mut B, address: u32| bus.bus_read(address, cheat.width);
                let mut cond = |c: bool, ops: &mut u32| {
                    condition = c;
                    condition_remaining = cheat.repeat;
                    negative_condition_remaining = cheat.negative_repeat;
                    *ops = 1;
                };
                match cheat.ty {
                    CheatType::Assign => {
                        value = operand;
                        perform_assignment = true;
                    }
                    CheatType::AssignIndirect => {
                        value = operand;
                        address = bus.bus_read(address, 4).wrapping_add(cheat.address_offset as u32);
                        perform_assignment = true;
                    }
                    CheatType::And => {
                        value = read(bus, address) & operand;
                        perform_assignment = true;
                    }
                    CheatType::Add => {
                        value = read(bus, address).wrapping_add(operand);
                        perform_assignment = true;
                    }
                    CheatType::Sub => {
                        value = read(bus, address).wrapping_sub(operand);
                        perform_assignment = true;
                    }
                    CheatType::Or => {
                        value = read(bus, address) | operand;
                        perform_assignment = true;
                    }
                    CheatType::IfEq => cond(read(bus, address) == operand, &mut operations_remaining),
                    CheatType::IfNe => cond(read(bus, address) != operand, &mut operations_remaining),
                    CheatType::IfLt => cond((read(bus, address) as i32) < operand as i32, &mut operations_remaining),
                    CheatType::IfGt => cond((read(bus, address) as i32) > operand as i32, &mut operations_remaining),
                    CheatType::IfLe => cond((read(bus, address) as i32) <= operand as i32, &mut operations_remaining),
                    CheatType::IfGe => cond((read(bus, address) as i32) >= operand as i32, &mut operations_remaining),
                    CheatType::IfUlt => cond(read(bus, address) < operand, &mut operations_remaining),
                    CheatType::IfUgt => cond(read(bus, address) > operand, &mut operations_remaining),
                    CheatType::IfAnd => cond(read(bus, address) & operand != 0, &mut operations_remaining),
                    CheatType::IfLand => cond(read(bus, address) != 0 && operand != 0, &mut operations_remaining),
                    CheatType::IfNand => cond(read(bus, address) & operand == 0, &mut operations_remaining),
                    CheatType::IfButton => cond(button_down, &mut operations_remaining),
                    CheatType::Never => cond(false, &mut operations_remaining),
                }

                if perform_assignment {
                    bus.bus_write(address, cheat.width, value);
                }

                address = address.wrapping_add(cheat.address_offset as u32);
                operand = operand.wrapping_add(cheat.operand_offset as u32);
                operations_remaining -= 1;
            }

            if else_loc != 0 && i == else_loc {
                i = end_loc;
                end_loc = 0;
            }
            if condition_remaining > 0 && !condition {
                i += condition_remaining as usize;
            } else if negative_condition_remaining > 0 {
                else_loc = i + condition_remaining as usize;
                end_loc = else_loc + negative_condition_remaining as usize;
            }
            i += 1;
        }
    }

    /// `mCheatParseFile` (mGBA's `.cheats` format: `# name` starts a set,
    /// `!disabled` and other `!` directives apply to the next set).
    /// `new_set` makes an empty set of the right kind.
    pub fn parse_file(&mut self, text: &str, new_set: fn(&str) -> CheatSet) -> bool {
        let mut set: Option<CheatSet> = None;
        let mut next_disabled = false;
        let mut directives: Vec<String> = vec![];
        for raw in text.lines() {
            let line = raw.trim_end();
            let body = line.trim_start();
            if let Some(name) = body.strip_prefix('#') {
                let mut new = new_set(name.trim_start());
                new.enabled = !next_disabled;
                next_disabled = false;
                if let Some(old) = set.take() {
                    new.copy_properties(&old);
                    self.sets.push(old);
                }
                new.parse_directives(&directives);
                set = Some(new);
            } else if let Some(d) = body.strip_prefix('!') {
                let d = d.trim_start();
                if d.eq_ignore_ascii_case("disabled") {
                    next_disabled = true;
                } else if d.eq_ignore_ascii_case("reset") {
                    directives.clear();
                } else {
                    directives.push(d.to_string());
                }
            } else {
                if body.is_empty() {
                    continue;
                }
                let set = set.get_or_insert_with(|| {
                    let mut s = new_set("");
                    s.enabled = !next_disabled;
                    s
                });
                next_disabled = false;
                set.add_line(line);
            }
        }
        if let Some(s) = set {
            self.sets.push(s);
        }
        true
    }

    /// `mCheatSaveFile`
    pub fn save_file(&self) -> String {
        let mut out = String::new();
        for set in &self.sets {
            if !set.enabled {
                out.push_str("!disabled\n");
            }
            for d in set.dump_directives() {
                out.push('!');
                out.push_str(&d);
                out.push('\n');
            }
            out.push_str("# ");
            out.push_str(&set.name);
            out.push('\n');
            for l in &set.lines {
                out.push_str(l);
                out.push('\n');
            }
        }
        out
    }
}

/// `hex32`, `hex16`, `hex12`, `hex8`: parse exactly `digits` hex digits
/// from the start of `s`, returning the value and the rest of the string.
pub(crate) fn hex(s: &str, digits: usize) -> Option<(u32, &str)> {
    if s.len() < digits || !s.is_char_boundary(digits) {
        return None;
    }
    let (h, rest) = s.split_at(digits);
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((u32::from_str_radix(h, 16).ok()?, rest))
}
