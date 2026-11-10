// Copyright (c) 2013-2017 Jeffrey Pfau
// Rust port of mGBA's debugger symbol table (src/debugger/symbols.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

/// `struct mDebuggerSymbols`
#[derive(Clone, Debug, Default)]
pub struct Symbols {
    names: HashMap<String, (i32, i32)>,
    reverse: HashMap<(i32, i32), String>,
}

impl Symbols {
    pub fn new() -> Symbols {
        Symbols::default()
    }

    /// `mDebuggerSymbolLookup`: `(value, segment)` for a name.
    pub fn lookup(&self, name: &str) -> Option<(i32, i32)> {
        self.names.get(name).copied()
    }

    /// `mDebuggerSymbolReverseLookup`
    pub fn reverse_lookup(&self, value: i32, segment: i32) -> Option<&str> {
        self.reverse.get(&(value, segment)).map(String::as_str)
    }

    /// `mDebuggerSymbolAdd`
    pub fn add(&mut self, name: &str, value: i32, segment: i32) {
        self.names.insert(name.to_string(), (value, segment));
        self.reverse.insert((value, segment), name.to_string());
    }

    /// `mDebuggerSymbolRemove`
    pub fn remove(&mut self, name: &str) {
        if let Some(sym) = self.names.remove(name) {
            self.reverse.remove(&sym);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// `mDebuggerLoadARMIPSSymbols`: lines of `ADDRESS name[,size]`.
    pub fn load_armips(&mut self, text: &str) {
        for line in text.lines() {
            let Some(hex) = line.get(..8) else { continue };
            let Ok(address) = u32::from_str_radix(hex, 16) else { continue };
            let rest = line[8..].trim_start();
            if rest.is_empty() || rest.starts_with('.') {
                // Directives are not handled yet
                continue;
            }
            // Commas separate names from function sizes
            let name = rest.split(',').next().unwrap_or(rest);
            self.add(name, address as i32, -1);
        }
    }
}
