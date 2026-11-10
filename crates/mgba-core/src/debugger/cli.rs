// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's command-line debugger (src/debugger/cli-debugger.c,
// src/debugger/cli-el-backend.c's role, src/arm/debugger/cli-debugger.c,
// src/sm83/debugger/cli-debugger.c, src/gba/debugger/cli.c, src/gb/debugger/cli.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io::Write;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use super::parser::{lex_expression, ParseTree};
use super::{
    Breakpoint, Context, CoreEnv, EntryInfo, EntryReason, Module, Watchpoint, WATCHPOINT_READ, WATCHPOINT_RW,
    WATCHPOINT_WRITE, WATCHPOINT_WRITE_CHANGE,
};

const ERROR_MISSING_ARGS: &str = "Arguments missing";
const ERROR_OVERFLOW: &str = "Arguments overflow";
const ERROR_INVALID_ARGS: &str = "Invalid arguments";

/// `struct CLIDebuggerBackend`
pub trait CliBackend {
    fn print(&mut self, text: &str);
    /// Wait up to `timeout_ms` for a line: 1 when one is ready, 0 on timeout,
    /// negative when input is closed.
    fn poll(&mut self, timeout_ms: i32) -> i32;
    fn readline(&mut self) -> Option<String>;
    fn history_last(&self) -> Option<String>;
    fn history_append(&mut self, line: &str);
}

/// A backend reading lines from a channel (fed from stdin by the frontend)
/// and printing to stdout.
pub struct ChannelBackend {
    rx: Receiver<String>,
    ready: Option<String>,
    history: Vec<String>,
}

impl ChannelBackend {
    pub fn new(rx: Receiver<String>) -> ChannelBackend {
        ChannelBackend { rx, ready: None, history: Vec::new() }
    }

    /// Start a thread that forwards stdin lines.
    pub fn stdin() -> ChannelBackend {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            let mut line = String::new();
            loop {
                line.clear();
                match stdin.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if tx.send(line.clone()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        ChannelBackend::new(rx)
    }
}

impl CliBackend for ChannelBackend {
    fn print(&mut self, text: &str) {
        let mut out = std::io::stdout();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }

    fn poll(&mut self, timeout_ms: i32) -> i32 {
        if self.ready.is_some() {
            return 1;
        }
        let result = if timeout_ms < 0 {
            self.rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            self.rx.recv_timeout(Duration::from_millis(timeout_ms as u64))
        };
        match result {
            Ok(line) => {
                self.ready = Some(line);
                1
            }
            Err(RecvTimeoutError::Timeout) => 0,
            Err(RecvTimeoutError::Disconnected) => -1,
        }
    }

    fn readline(&mut self) -> Option<String> {
        self.ready.take()
    }

    fn history_last(&self) -> Option<String> {
        self.history.last().cloned()
    }

    fn history_append(&mut self, line: &str) {
        self.history.push(line.trim_end().to_string());
    }
}

/// `struct CLIDebugVector`
#[derive(Clone, Debug)]
enum Dv {
    Int { value: i32, segment: i32 },
    Char(String),
    Error,
}

/// `struct CLIDebugger`
pub struct CliDebugger {
    backend: Box<dyn CliBackend>,
    is_paused: bool,
    needs_callback: bool,
    trace_remaining: i32,
    trace_file: Option<std::fs::File>,
    skip_status: bool,
    /// `GBACLIDebugger::frameAdvance`: frame counter to stop after.
    frame_advance: Option<u32>,
}

type Command = fn(&mut CliDebugger, &mut Context, &[Dv]);

struct CommandSummary {
    name: &'static str,
    command: Command,
    format: &'static str,
    summary: &'static str,
}

macro_rules! cmd {
    ($name:expr, $f:expr, $fmt:expr, $sum:expr) => {
        CommandSummary { name: $name, command: $f, format: $fmt, summary: $sum }
    };
}

/// `_debuggerCommands`
const COMMANDS: &[CommandSummary] = &[
    cmd!("break", CliDebugger::set_breakpoint, "Is", "Set a breakpoint"),
    cmd!("continue", CliDebugger::cont, "", "Continue execution"),
    cmd!("enable", CliDebugger::enable_breakpoint, "I+", "Enable a breakpoint or watchpoint"),
    cmd!("disable", CliDebugger::disable_breakpoint, "I+", "Disable a breakpoint or watchpoint"),
    cmd!("delete", CliDebugger::clear_breakpoint, "I", "Delete a breakpoint or watchpoint"),
    cmd!("disassemble", CliDebugger::disassemble, "Ii", "Disassemble instructions"),
    cmd!("events", CliDebugger::events, "", "Print list of scheduled events"),
    cmd!("help", CliDebugger::print_help, "S", "Print help"),
    cmd!("listb", CliDebugger::list_breakpoints, "", "List breakpoints"),
    cmd!("listw", CliDebugger::list_watchpoints, "", "List watchpoints"),
    cmd!("load-symbols", CliDebugger::load_symbols, "S", "Load symbols from an external file"),
    cmd!("next", CliDebugger::next, "", "Execute next instruction"),
    cmd!("print", CliDebugger::print, "S+", "Print a value"),
    cmd!("print/t", CliDebugger::print_bin, "S+", "Print a value as binary"),
    cmd!("print/x", CliDebugger::print_hex, "S+", "Print a value as hexadecimal"),
    cmd!("quit", CliDebugger::quit, "", "Quit the emulator"),
    cmd!("reset", CliDebugger::reset, "", "Reset the emulation"),
    cmd!("r/1", CliDebugger::read_byte, "I", "Read a byte from a specified offset"),
    cmd!("r/2", CliDebugger::read_halfword, "I", "Read a halfword from a specified offset"),
    cmd!("r/4", CliDebugger::read_word, "I", "Read a word from a specified offset"),
    cmd!("set", CliDebugger::set_symbol, "SI", "Assign a symbol to an address"),
    cmd!("status", CliDebugger::print_status_cmd, "", "Print the current status"),
    cmd!("symbol", CliDebugger::find_symbol, "I", "Find the symbol name for an address"),
    cmd!("trace", CliDebugger::trace, "Is", "Trace a number of instructions"),
    cmd!("w/1", CliDebugger::write_byte, "II", "Write a byte at a specified offset"),
    cmd!("w/2", CliDebugger::write_halfword, "II", "Write a halfword at a specified offset"),
    cmd!("w/r", CliDebugger::write_register, "SI", "Write a register"),
    cmd!("w/4", CliDebugger::write_word, "II", "Write a word at a specified offset"),
    cmd!("watch", CliDebugger::set_rw_watchpoint, "Is", "Set a watchpoint"),
    cmd!("watch/c", CliDebugger::set_write_changed_watchpoint, "Is", "Set a change watchpoint"),
    cmd!("watch/r", CliDebugger::set_read_watchpoint, "Is", "Set a read watchpoint"),
    cmd!("watch/w", CliDebugger::set_write_watchpoint, "Is", "Set a write watchpoint"),
    cmd!("watch-range", CliDebugger::set_rw_range_watchpoint, "IIs", "Set a range watchpoint"),
    cmd!("watch-range/c", CliDebugger::set_write_changed_range_watchpoint, "IIs", "Set a change range watchpoint"),
    cmd!("watch-range/r", CliDebugger::set_read_range_watchpoint, "IIs", "Set a read range watchpoint"),
    cmd!("watch-range/w", CliDebugger::set_write_range_watchpoint, "IIs", "Set a write range watchpoint"),
    cmd!("x/1", CliDebugger::dump_byte, "Ii", "Examine bytes at a specified offset"),
    cmd!("x/2", CliDebugger::dump_halfword, "Ii", "Examine halfwords at a specified offset"),
    cmd!("x/4", CliDebugger::dump_word, "Ii", "Examine words at a specified offset"),
];

/// `_debuggerCommandAliases`
const ALIASES: &[(&str, &str)] = &[
    ("b", "break"),
    ("c", "continue"),
    ("eb", "enable"),
    ("db", "disable"),
    ("d", "delete"),
    ("dis", "disassemble"),
    ("disasm", "disassemble"),
    ("h", "help"),
    ("i", "status"),
    ("info", "status"),
    ("loadsyms", "load-symbols"),
    ("lb", "listb"),
    ("lw", "listw"),
    ("n", "next"),
    ("p", "print"),
    ("p/t", "print/t"),
    ("p/x", "print/x"),
    ("q", "quit"),
    ("w", "watch"),
    ("watchr", "watch-range"),
    ("wr", "watch-range"),
    ("watchr/c", "watch-range/c"),
    ("wr/c", "watch-range/c"),
    ("watchr/r", "watch-range/r"),
    ("wr/r", "watch-range/r"),
    ("watchr/w", "watch-range/w"),
    ("wr/w", "watch-range/w"),
];

/// `_armCommands`
const ARM_COMMANDS: &[CommandSummary] = &[
    cmd!("break/a", CliDebugger::set_breakpoint_arm, "I", "Set a software breakpoint as ARM"),
    cmd!("break/t", CliDebugger::set_breakpoint_thumb, "I", "Set a software breakpoint as Thumb"),
    cmd!("disassemble/a", CliDebugger::disassemble_arm, "Ii", "Disassemble instructions as ARM"),
    cmd!("disassemble/t", CliDebugger::disassemble_thumb, "Ii", "Disassemble instructions as Thumb"),
];

/// `_armCommandAliases`
const ARM_ALIASES: &[(&str, &str)] = &[
    ("b/a", "break/a"),
    ("b/t", "break/t"),
    ("dis/a", "disassemble/a"),
    ("dis/t", "disassemble/t"),
    ("disasm/a", "disassemble/a"),
    ("disasm/t", "disassemble/t"),
];

/// `_GBACLIDebuggerCommands` / `_GBCLIDebuggerCommands`
const SYSTEM_COMMANDS: &[CommandSummary] = &[cmd!("frame", CliDebugger::frame, "", "Frame advance")];

impl CliDebugger {
    /// `CLIDebuggerCreate`
    pub fn new(backend: Box<dyn CliBackend>) -> CliDebugger {
        CliDebugger {
            backend,
            is_paused: false,
            needs_callback: false,
            trace_remaining: 0,
            trace_file: None,
            skip_status: false,
            frame_advance: None,
        }
    }

    fn out(&mut self, text: &str) {
        self.backend.print(text);
    }

    fn platform_commands(ctx: &Context) -> &'static [CommandSummary] {
        if ctx.core.platform_name() == "ARM" {
            ARM_COMMANDS
        } else {
            &[]
        }
    }

    fn platform_aliases(ctx: &Context) -> &'static [(&'static str, &'static str)] {
        if ctx.core.platform_name() == "ARM" {
            ARM_ALIASES
        } else {
            &[]
        }
    }

    /// `CLIDVParse`
    fn dv_parse(ctx: &mut Context, s: &str) -> Dv {
        let mut lv = Vec::new();
        let adjusted = lex_expression(&mut lv, s, Some(" "));
        if adjusted > s.len() {
            return Dv::Error;
        }
        let Some(tree) = ParseTree::parse(&lv) else { return Dv::Error };
        let symbols = ctx.core.debug_platform().as_ref().and_then(|p| p.symbols.clone());
        let mut env = CoreEnv { core: ctx.core.as_mcore(), symbols };
        match tree.evaluate(&mut env) {
            Some((value, segment)) => Dv::Int { value, segment },
            None => Dv::Error,
        }
    }

    /// `_parseArg`
    fn parse_arg(ctx: &mut Context, s: &str, ty: char) -> Option<Dv> {
        match ty {
            'I' | 'i' => Some(Self::dv_parse(ctx, s)),
            'S' | 's' => Some(Dv::Char(s.to_string())),
            '*' => {
                let dv = Self::dv_parse(ctx, s);
                if matches!(dv, Dv::Error) {
                    Some(Dv::Char(s.to_string()))
                } else {
                    Some(dv)
                }
            }
            _ => None,
        }
    }

    /// `_tryCommands`: 1 if run, 0 on an argument error, -1 if not found.
    fn try_commands(
        &mut self,
        ctx: &mut Context,
        commands: &[CommandSummary],
        aliases: &[(&str, &str)],
        command: &str,
        args: Option<&str>,
    ) -> i32 {
        let mut command = command;
        for (alias, original) in aliases {
            if alias.eq_ignore_ascii_case(command) {
                command = original;
            }
        }
        for c in commands {
            if !c.name.eq_ignore_ascii_case(command) {
                continue;
            }
            let mut dv = Vec::new();
            let mut args = args;
            if let Some(mut a) = args {
                let format: Vec<char> = c.format.chars().collect();
                let mut arg = 0usize;
                let mut last_arg = '\0';
                while arg < format.len() && !a.is_empty() {
                    a = a.trim_start();
                    if a.is_empty() {
                        self.out("Wrong number of arguments\n");
                        return 0;
                    }
                    let end = a.find(' ').unwrap_or(a.len());
                    let token = &a[..end];
                    let mandatory;
                    let next = if format[arg] == '+' {
                        mandatory = false;
                        let d = Self::parse_arg(ctx, token, last_arg);
                        arg = arg.wrapping_sub(1);
                        d
                    } else {
                        mandatory = format[arg].is_ascii_uppercase() || format[arg] == '*';
                        last_arg = format[arg];
                        Self::parse_arg(ctx, token, format[arg])
                    };
                    a = &a[end..];
                    arg = arg.wrapping_add(1);
                    let Some(next) = next else {
                        if !mandatory {
                            args = None;
                        }
                        break;
                    };
                    if matches!(next, Dv::Error) {
                        self.out("Parse error\n");
                        return 0;
                    }
                    dv.push(next);
                }
                if args.is_some() {
                    args = Some(a);
                }
            }
            if let Some(a) = args {
                if !a.trim_start().is_empty() {
                    self.out("Wrong number of arguments\n");
                    return 0;
                }
            }
            (c.command)(self, ctx, &dv);
            return 1;
        }
        -1
    }

    /// `CLIDebuggerRunCommand`
    pub fn run_command(&mut self, ctx: &mut Context, line: &str) -> bool {
        let line = line.trim_end_matches(['\n', '\r']);
        let (command, args) = match line.find(' ') {
            Some(i) => (&line[..i], Some(&line[i + 1..])),
            None => (line, None),
        };
        let mut result = self.try_commands(ctx, COMMANDS, ALIASES, command, args);
        if result < 0 {
            result = self.try_commands(ctx, SYSTEM_COMMANDS, &[], command, args);
        }
        if result < 0 {
            let (cmds, aliases) = (Self::platform_commands(ctx), Self::platform_aliases(ctx));
            result = self.try_commands(ctx, cmds, aliases, command, args);
        }
        if result < 0 {
            self.out("Command not found\n");
        }
        result > 0
    }

    // ----- commands -----

    fn cont(&mut self, _ctx: &mut Context, _dv: &[Dv]) {
        self.needs_callback = self.trace_remaining != 0;
        self.is_paused = false;
    }

    fn next(&mut self, ctx: &mut Context, _dv: &[Dv]) {
        ctx.core.step();
        self.print_status_cmd(ctx, &[]);
    }

    fn disassemble_mode(&mut self, ctx: &mut Context, dv: &[Dv], mode: Option<bool>) {
        let (address, segment) = match dv.first() {
            Some(Dv::Int { value, segment }) => (Some(*value as u32), *segment),
            _ => (None, -1),
        };
        let count = match (address, dv.get(if address.is_some() { 1 } else { 0 })) {
            (_, Some(Dv::Int { value, .. })) => *value as u32,
            _ => 1,
        };
        for line in ctx.core.disassemble(address, segment, count, mode) {
            self.out(&format!("{line}\n"));
        }
    }

    fn disassemble(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.disassemble_mode(ctx, dv, None);
    }

    fn disassemble_arm(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.disassemble_mode(ctx, dv, Some(false));
    }

    fn disassemble_thumb(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.disassemble_mode(ctx, dv, Some(true));
    }

    /// `_parseExpression`
    fn parse_expression(ctx: &mut Context, dv: &[Dv]) -> Option<(i32, i32)> {
        let strings: Vec<&str> = dv
            .iter()
            .filter_map(|d| match d {
                Dv::Char(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        let tree = ParseTree::from_strings(&strings)?;
        let symbols = ctx.core.debug_platform().as_ref().and_then(|p| p.symbols.clone());
        let mut env = CoreEnv { core: ctx.core.as_mcore(), symbols };
        tree.evaluate(&mut env)
    }

    fn print(&mut self, ctx: &mut Context, dv: &[Dv]) {
        match Self::parse_expression(ctx, dv) {
            None => self.out("Parse error\n"),
            Some((v, s)) if s >= 0 => self.out(&format!(" ${s:02X}:{v:04X}\n")),
            Some((v, _)) => self.out(&format!(" {}\n", v as u32)),
        }
    }

    fn print_bin(&mut self, ctx: &mut Context, dv: &[Dv]) {
        match Self::parse_expression(ctx, dv) {
            None => self.out("Parse error\n"),
            Some((v, _)) => self.out(&format!(" 0b{:032b}\n", v as u32)),
        }
    }

    fn print_hex(&mut self, ctx: &mut Context, dv: &[Dv]) {
        match Self::parse_expression(ctx, dv) {
            None => self.out("Parse error\n"),
            Some((v, _)) => self.out(&format!(" 0x{:08X}\n", v as u32)),
        }
    }

    /// `_printCommands`
    fn print_commands(&mut self, commands: &[CommandSummary], aliases: &[(&str, &str)]) {
        for c in commands {
            self.out(&format!("{:<15}  {}\n", c.name, c.summary));
            let names: Vec<&str> = aliases.iter().filter(|(_, o)| *o == c.name).map(|(a, _)| *a).collect();
            if !names.is_empty() {
                self.out(&format!("                 Aliases: {}\n", names.join(" ")));
            }
        }
    }

    fn print_help(&mut self, ctx: &mut Context, dv: &[Dv]) {
        let platform = ctx.core.platform_name();
        let system = ctx.core.system_name();
        let pc = Self::platform_commands(ctx);
        let pa = Self::platform_aliases(ctx);
        match dv.first() {
            Some(Dv::Char(name)) => {
                for (commands, aliases) in [(COMMANDS, ALIASES), (pc, pa), (SYSTEM_COMMANDS, &[][..])] {
                    if let Some(c) = commands.iter().find(|c| c.name == name) {
                        self.out(&format!(" {}\n", c.summary));
                        let names: Vec<&str> =
                            aliases.iter().filter(|(_, o)| *o == c.name).map(|(a, _)| *a).collect();
                        if !names.is_empty() {
                            self.out(&format!(" Aliases: {}\n", names.join(" ")));
                        }
                    }
                }
            }
            _ => {
                self.out("Generic commands:\n");
                self.print_commands(COMMANDS, ALIASES);
                if !pc.is_empty() {
                    self.out(&format!("\n{platform} commands:\n"));
                    self.print_commands(pc, pa);
                }
                self.out(&format!("\n{system} commands:\n"));
                self.print_commands(SYSTEM_COMMANDS, &[]);
            }
        }
    }

    fn quit(&mut self, ctx: &mut Context, _dv: &[Dv]) {
        ctx.shutdown = true;
    }

    fn int_arg(&mut self, dv: &[Dv], index: usize) -> Option<(u32, i32)> {
        match dv.get(index) {
            Some(Dv::Int { value, segment }) => Some((*value as u32, *segment)),
            _ => None,
        }
    }

    fn read(&mut self, ctx: &mut Context, dv: &[Dv], width: u32) {
        let Some((address, segment)) = self.int_arg(dv, 0) else {
            self.out(&format!("{ERROR_MISSING_ARGS}\n"));
            return;
        };
        let core = ctx.core.as_mcore();
        let value = match (width, segment >= 0) {
            (1, true) => core.raw_read8(address, segment),
            (1, false) => core.bus_read8(address),
            (2, true) => core.raw_read16(address, segment),
            (2, false) => core.bus_read16(address & !1),
            (_, true) => core.raw_read32(address, segment),
            (_, false) => core.bus_read32(address & !3),
        };
        let w = (width * 2) as usize;
        self.out(&format!(" 0x{value:0w$X}\n"));
    }

    fn read_byte(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.read(ctx, dv, 1);
    }
    fn read_halfword(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.read(ctx, dv, 2);
    }
    fn read_word(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.read(ctx, dv, 4);
    }

    fn reset(&mut self, ctx: &mut Context, _dv: &[Dv]) {
        ctx.core.reset();
        self.print_status_cmd(ctx, &[]);
    }

    fn write(&mut self, ctx: &mut Context, dv: &[Dv], width: u32) {
        if dv.len() < 2 {
            self.out(&format!("{ERROR_MISSING_ARGS}\n"));
            return;
        }
        let (Some((address, segment)), Some((value, _))) = (self.int_arg(dv, 0), self.int_arg(dv, 1)) else {
            self.out(&format!("{ERROR_INVALID_ARGS}\n"));
            return;
        };
        if (width == 1 && value > 0xFF) || (width == 2 && value > 0xFFFF) {
            self.out(&format!("{ERROR_OVERFLOW}\n"));
            return;
        }
        let core = ctx.core.as_mcore();
        match (width, segment >= 0) {
            (1, true) => core.raw_write8(address, segment, value),
            (1, false) => core.bus_write8(address, value),
            (2, true) => core.raw_write16(address, segment, value),
            (2, false) => core.bus_write16(address, value),
            (_, true) => core.raw_write32(address, segment, value),
            (_, false) => core.bus_write32(address, value),
        }
    }

    fn write_byte(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.write(ctx, dv, 1);
    }
    fn write_halfword(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.write(ctx, dv, 2);
    }
    fn write_word(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.write(ctx, dv, 4);
    }

    fn write_register(&mut self, ctx: &mut Context, dv: &[Dv]) {
        if dv.len() < 2 {
            self.out(&format!("{ERROR_MISSING_ARGS}\n"));
            return;
        }
        let (Dv::Char(name), Dv::Int { value, .. }) = (&dv[0], &dv[1]) else {
            self.out(&format!("{ERROR_INVALID_ARGS}\n"));
            return;
        };
        if !ctx.core.write_register(name, *value) {
            self.out(&format!("{ERROR_INVALID_ARGS}\n"));
        }
    }

    fn dump(&mut self, ctx: &mut Context, dv: &[Dv], width: u32) {
        let Some((mut address, segment)) = self.int_arg(dv, 0) else {
            self.out(&format!("{ERROR_MISSING_ARGS}\n"));
            return;
        };
        let per_line = 16 / width;
        let mut words = match self.int_arg(dv, 1) {
            Some((n, _)) => n,
            None => per_line,
        };
        let core = ctx.core.as_mcore();
        let mut text = String::new();
        while words > 0 {
            let line = per_line.min(words);
            text.push_str(&format!("0x{address:08X}:"));
            for _ in 0..line {
                let value = match (width, segment >= 0) {
                    (1, true) => core.raw_read8(address, segment),
                    (1, false) => core.bus_read8(address),
                    (2, true) => core.raw_read16(address, segment),
                    (2, false) => core.bus_read16(address),
                    (_, true) => core.raw_read32(address, segment),
                    (_, false) => core.bus_read32(address),
                };
                let w = (width * 2) as usize;
                text.push_str(&format!(" {value:0w$X}"));
                address = address.wrapping_add(width);
                words -= 1;
            }
            text.push('\n');
        }
        self.out(&text);
    }

    fn dump_byte(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.dump(ctx, dv, 1);
    }
    fn dump_halfword(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.dump(ctx, dv, 2);
    }
    fn dump_word(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.dump(ctx, dv, 4);
    }

    fn condition(&mut self, dv: Option<&Dv>) -> Result<Option<ParseTree>, ()> {
        match dv {
            Some(Dv::Char(s)) => match ParseTree::from_strings(&[s.as_str()]) {
                Some(t) => Ok(Some(t)),
                None => {
                    self.out(&format!("{ERROR_INVALID_ARGS}\n"));
                    Err(())
                }
            },
            _ => Ok(None),
        }
    }

    fn set_breakpoint(&mut self, ctx: &mut Context, dv: &[Dv]) {
        let Some((address, segment)) = self.int_arg(dv, 0) else {
            self.out(&format!("{ERROR_MISSING_ARGS}\n"));
            return;
        };
        let mut bp = Breakpoint::new(address, segment);
        match self.condition(dv.get(1)) {
            Ok(c) => bp.condition = c,
            Err(()) => return,
        }
        let arm = ctx.core.platform_name() == "ARM";
        let id = ctx.core.debug_platform().as_mut().map_or(-1, |p| p.set_breakpoint(bp, arm));
        if id > 0 {
            self.out(&format!("Added breakpoint #{id}\n"));
        }
    }

    fn set_software_breakpoint(&mut self, ctx: &mut Context, dv: &[Dv], thumb: bool) {
        let Some((address, _)) = self.int_arg(dv, 0) else {
            self.out(ERROR_MISSING_ARGS);
            return;
        };
        let id = ctx.core.set_software_breakpoint(address, thumb);
        if id > 0 {
            self.out(&format!("Added breakpoint #{id}\n"));
        }
    }

    fn set_breakpoint_arm(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.set_software_breakpoint(ctx, dv, false);
    }
    fn set_breakpoint_thumb(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.set_software_breakpoint(ctx, dv, true);
    }

    fn set_watchpoint(&mut self, ctx: &mut Context, dv: &[Dv], ty: u32) {
        let Some((address, segment)) = self.int_arg(dv, 0) else {
            self.out(&format!("{ERROR_MISSING_ARGS}\n"));
            return;
        };
        let mut wp = Watchpoint::new(address, address.wrapping_add(1), segment, ty);
        match self.condition(dv.get(1)) {
            Ok(c) => wp.condition = c,
            Err(()) => return,
        }
        let id = ctx.core.debug_platform().as_mut().map_or(-1, |p| p.set_watchpoint(wp));
        if id > 0 {
            self.out(&format!("Added watchpoint #{id}\n"));
        }
    }

    fn set_range_watchpoint(&mut self, ctx: &mut Context, dv: &[Dv], ty: u32) {
        let (Some((min, seg0)), Some((max, seg1))) = (self.int_arg(dv, 0), self.int_arg(dv, 1)) else {
            self.out(&format!("{ERROR_MISSING_ARGS}\n"));
            return;
        };
        if min >= max {
            self.out("Range watchpoint end is before start. Note that the end of the range is not included.\n");
            return;
        }
        if seg0 != seg1 {
            self.out("Range watchpoint does not start and end in the same segment.\n");
            return;
        }
        let mut wp = Watchpoint::new(min, max, seg0, ty);
        match self.condition(dv.get(2)) {
            Ok(c) => wp.condition = c,
            Err(()) => return,
        }
        let id = ctx.core.debug_platform().as_mut().map_or(-1, |p| p.set_watchpoint(wp));
        if id > 0 {
            self.out(&format!("Added watchpoint #{id}\n"));
        }
    }

    fn set_rw_watchpoint(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.set_watchpoint(ctx, dv, WATCHPOINT_RW);
    }
    fn set_read_watchpoint(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.set_watchpoint(ctx, dv, WATCHPOINT_READ);
    }
    fn set_write_watchpoint(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.set_watchpoint(ctx, dv, WATCHPOINT_WRITE);
    }
    fn set_write_changed_watchpoint(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.set_watchpoint(ctx, dv, WATCHPOINT_WRITE_CHANGE);
    }
    fn set_rw_range_watchpoint(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.set_range_watchpoint(ctx, dv, WATCHPOINT_RW);
    }
    fn set_read_range_watchpoint(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.set_range_watchpoint(ctx, dv, WATCHPOINT_READ);
    }
    fn set_write_range_watchpoint(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.set_range_watchpoint(ctx, dv, WATCHPOINT_WRITE);
    }
    fn set_write_changed_range_watchpoint(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.set_range_watchpoint(ctx, dv, WATCHPOINT_WRITE_CHANGE);
    }

    fn toggle(&mut self, ctx: &mut Context, dv: &[Dv], status: bool) {
        if !matches!(dv.first(), Some(Dv::Int { .. })) {
            self.out(&format!("{ERROR_MISSING_ARGS}\n"));
            return;
        }
        for d in dv {
            if let Dv::Int { value, .. } = d {
                if let Some(p) = ctx.core.debug_platform().as_mut() {
                    p.toggle_breakpoint(*value as isize, status);
                }
            }
        }
    }

    fn enable_breakpoint(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.toggle(ctx, dv, true);
    }
    fn disable_breakpoint(&mut self, ctx: &mut Context, dv: &[Dv]) {
        self.toggle(ctx, dv, false);
    }

    fn clear_breakpoint(&mut self, ctx: &mut Context, dv: &[Dv]) {
        let Some((id, _)) = self.int_arg(dv, 0) else {
            self.out(&format!("{ERROR_MISSING_ARGS}\n"));
            return;
        };
        ctx.core.clear_breakpoint(id as isize);
    }

    fn list_breakpoints(&mut self, ctx: &mut Context, _dv: &[Dv]) {
        let list = ctx.core.debug_platform().as_ref().map(|p| p.list_breakpoints()).unwrap_or_default();
        for bp in list {
            let status = if bp.disabled { 'D' } else { 'E' };
            if bp.segment >= 0 {
                self.out(&format!("{status} {}: {:02X}:{:X}\n", bp.id, bp.segment, bp.address));
            } else {
                self.out(&format!("{status} {}: 0x{:X}\n", bp.id, bp.address));
            }
        }
    }

    fn list_watchpoints(&mut self, ctx: &mut Context, _dv: &[Dv]) {
        let list = ctx.core.debug_platform().as_ref().map(|p| p.watchpoints.clone()).unwrap_or_default();
        for wp in list {
            let status = if wp.disabled { 'D' } else { 'E' };
            let single = wp.max_address == wp.min_address.wrapping_add(1);
            let text = match (wp.segment >= 0, single) {
                (true, true) => format!("{status} {}: {:02X}:{:X}\n", wp.id, wp.segment, wp.min_address),
                (true, false) => {
                    format!("{status} {}: {:02X}:{:X}-{:X}\n", wp.id, wp.segment, wp.min_address, wp.max_address)
                }
                (false, true) => format!("{status} {}: 0x{:X}\n", wp.id, wp.min_address),
                (false, false) => format!("{status} {}: 0x{:X}-0x{:X}\n", wp.id, wp.min_address, wp.max_address),
            };
            self.out(&text);
        }
    }

    fn trace(&mut self, ctx: &mut Context, dv: &[Dv]) {
        let Some(first) = dv.first() else {
            self.out(&format!("{ERROR_MISSING_ARGS}\n"));
            return;
        };
        let Dv::Int { value, .. } = first else {
            self.out(&format!("{ERROR_INVALID_ARGS}\n"));
            return;
        };
        if *value < 0 {
            self.out(&format!("{ERROR_INVALID_ARGS}\n"));
            return;
        }
        self.trace_remaining = *value;
        self.trace_file = None;
        self.needs_callback = self.trace_remaining != 0;
        if self.trace_remaining == 0 {
            return;
        }
        if let Some(Dv::Char(path)) = dv.get(1) {
            self.trace_file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok();
        }
        if self.do_trace(ctx) {
            self.is_paused = false;
        } else {
            let status = ctx.core.print_status();
            self.out(&status);
        }
    }

    /// `_doTrace`
    fn do_trace(&mut self, ctx: &mut Context) -> bool {
        let mut line = ctx.core.trace();
        line.push('\n');
        match self.trace_file.as_mut() {
            Some(f) => {
                let _ = f.write_all(line.as_bytes());
            }
            None => self.out(&line),
        }
        if self.trace_remaining > 0 {
            self.trace_remaining -= 1;
        }
        if self.trace_remaining == 0 {
            self.trace_file = None;
            self.needs_callback = false;
            return false;
        }
        true
    }

    fn print_status_cmd(&mut self, ctx: &mut Context, _dv: &[Dv]) {
        let status = ctx.core.print_status();
        self.out(&status);
    }

    fn events(&mut self, ctx: &mut Context, _dv: &[Dv]) {
        for (name, until) in ctx.core.events() {
            self.out(&format!("{name} in {until} cycles\n"));
        }
    }

    fn set_symbol(&mut self, ctx: &mut Context, dv: &[Dv]) {
        let (Some(Dv::Char(name)), Some(Dv::Int { value, segment })) = (dv.first(), dv.get(1)) else {
            self.out(&format!("{ERROR_MISSING_ARGS}\n"));
            return;
        };
        if let Some(p) = ctx.core.debug_platform().as_mut() {
            let mut symbols = p.symbols.as_deref().cloned().unwrap_or_default();
            symbols.add(name, *value, *segment);
            p.symbols = Some(std::sync::Arc::new(symbols));
        }
    }

    fn find_symbol(&mut self, ctx: &mut Context, dv: &[Dv]) {
        let Some(Dv::Int { value, segment }) = dv.first() else {
            self.out(&format!("{ERROR_MISSING_ARGS}\n"));
            return;
        };
        let symbols = ctx.core.debug_platform().as_ref().and_then(|p| p.symbols.clone());
        let name = symbols.as_ref().and_then(|s| s.reverse_lookup(*value, *segment).map(str::to_string));
        match name {
            Some(n) => self.out(&format!(" 0x{:08X}: {n}\n", *value as u32)),
            None => self.out("Not found.\n"),
        }
    }

    fn load_symbols(&mut self, ctx: &mut Context, dv: &[Dv]) {
        let Some(Dv::Char(path)) = dv.first() else {
            self.out("Needs a filename\n");
            return;
        };
        match std::fs::read_to_string(path) {
            Ok(text) => {
                if let Some(p) = ctx.core.debug_platform().as_mut() {
                    let mut symbols = p.symbols.as_deref().cloned().unwrap_or_default();
                    symbols.load_armips(&text);
                    p.symbols = Some(std::sync::Arc::new(symbols));
                }
            }
            Err(_) => self.out(&format!("Failed to open file '{path}'\n")),
        }
    }

    fn frame(&mut self, ctx: &mut Context, _dv: &[Dv]) {
        self.needs_callback = true;
        self.frame_advance = Some(ctx.core.frame_counter());
        self.is_paused = false;
    }
}

impl Module for CliDebugger {
    fn is_paused(&self) -> bool {
        self.is_paused
    }

    fn set_paused(&mut self, paused: bool) {
        self.is_paused = paused;
    }

    fn needs_callback(&self) -> bool {
        self.needs_callback
    }

    /// `_cliDebuggerInit`
    fn init(&mut self, _ctx: &mut Context) {
        self.trace_remaining = 0;
        self.trace_file = None;
    }

    /// `_commandLine`
    fn paused(&mut self, ctx: &mut Context, timeout_ms: i32) {
        if self.skip_status {
            self.skip_status = false;
        } else {
            self.print_status_cmd(ctx, &[]);
        }
        while self.is_paused && !ctx.shutdown {
            let poll = self.backend.poll(timeout_ms);
            if poll <= 0 {
                if poll < 0 {
                    ctx.shutdown = true;
                } else {
                    self.skip_status = true;
                }
                return;
            }
            let Some(line) = self.backend.readline() else {
                ctx.shutdown = true;
                return;
            };
            if line.is_empty() {
                ctx.shutdown = true;
                return;
            }
            if line.starts_with('\n') {
                if let Some(last) = self.backend.history_last() {
                    if !last.is_empty() {
                        self.run_command(ctx, &last);
                    }
                }
            } else {
                if line.starts_with('#') {
                    self.skip_status = true;
                } else {
                    self.run_command(ctx, &line);
                }
                self.backend.history_append(&line);
            }
        }
    }

    /// `_reportEntry`
    fn entered(&mut self, _ctx: &mut Context, reason: EntryReason, info: Option<&EntryInfo>) {
        if self.trace_remaining > 0 {
            self.trace_remaining = 0;
        }
        self.skip_status = false;
        let text = match (reason, info) {
            (EntryReason::Manual | EntryReason::Attached, _) => return,
            (EntryReason::Breakpoint, Some(i)) if i.point_id > 0 => {
                format!("Hit breakpoint {} at 0x{:08X}\n", i.point_id, i.address)
            }
            (EntryReason::Breakpoint, Some(i)) => format!("Hit unknown breakpoint at 0x{:08X}\n", i.address),
            (EntryReason::Breakpoint, None) => "Hit breakpoint\n".to_string(),
            (EntryReason::Watchpoint, Some(i)) if i.access_type & WATCHPOINT_WRITE != 0 => format!(
                "Hit watchpoint {} at 0x{:08X}: (new value = 0x{:08X}, old value = 0x{:08X})\n",
                i.point_id, i.address, i.new_value, i.old_value
            ),
            (EntryReason::Watchpoint, Some(i)) => {
                format!("Hit watchpoint {} at 0x{:08X}: (value = 0x{:08X})\n", i.point_id, i.address, i.old_value)
            }
            (EntryReason::Watchpoint, None) => "Hit watchpoint\n".to_string(),
            (EntryReason::IllegalOp, Some(i)) => {
                format!("Hit illegal opcode at 0x{:08X}: 0x{:08X}\n", i.address, i.opcode)
            }
            (EntryReason::IllegalOp, None) => "Hit illegal opcode\n".to_string(),
            (EntryReason::Stack, _) => "Hit function call or return\n".to_string(),
        };
        self.out(&text);
    }

    /// `_GBACLIDebuggerCustom` (frame advance) and trace stepping.
    fn custom(&mut self, ctx: &mut Context) {
        if let Some(frame) = self.frame_advance {
            if ctx.core.frame_counter() != frame {
                self.frame_advance = None;
                self.needs_callback = self.trace_remaining != 0;
                self.is_paused = true;
                return;
            }
        }
        if self.trace_remaining != 0 && !self.do_trace(ctx) {
            self.is_paused = true;
        }
    }
}
