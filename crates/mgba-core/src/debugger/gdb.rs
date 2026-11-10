// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's GDB remote stub (src/debugger/gdb-stub.c). Like mGBA's,
// it speaks for the GBA's ARM7TDMI.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use super::{
    Breakpoint, Context, EntryInfo, EntryReason, Module, Watchpoint, BreakpointType, WATCHPOINT_READ, WATCHPOINT_RW,
    WATCHPOINT_WRITE, WATCHPOINT_WRITE_CHANGE,
};
use crate::core::{MEMORY_MAPPED, MEMORY_WORM, MEMORY_WRITE};

const GDB_STUB_MAX_LINE: usize = 1400;
const GDB_STUB_INTERVAL: i32 = 32;
const SIGINT: u32 = 2;
const SIGILL: u32 = 4;
const SIGTRAP: u32 = 5;
const GDB_BAD_ARGUMENTS: u32 = 1;
const GDB_UNSUPPORTED_COMMAND: u32 = 2;
const GBA_SIZE_BIOS: u32 = 0x4000;
const MACH_O_ARM: u32 = 12;
const MACH_O_ARM_V4T: u32 = 5;

const TARGET_XML: &str = concat!(
    "<target version=\"1.0\">",
    "<architecture>armv4t</architecture>",
    "<osabi>none</osabi>",
    "<feature name=\"org.gnu.gdb.arm.core\">",
    "<reg name=\"r0\" bitsize=\"32\" type=\"uint32\"/>",
    "<reg name=\"r1\" bitsize=\"32\" type=\"uint32\"/>",
    "<reg name=\"r2\" bitsize=\"32\" type=\"uint32\"/>",
    "<reg name=\"r3\" bitsize=\"32\" type=\"uint32\"/>",
    "<reg name=\"r4\" bitsize=\"32\" type=\"uint32\"/>",
    "<reg name=\"r5\" bitsize=\"32\" type=\"uint32\"/>",
    "<reg name=\"r6\" bitsize=\"32\" type=\"uint32\"/>",
    "<reg name=\"r7\" bitsize=\"32\" type=\"uint32\"/>",
    "<reg name=\"r8\" bitsize=\"32\" type=\"uint32\"/>",
    "<reg name=\"r9\" bitsize=\"32\" type=\"uint32\"/>",
    "<reg name=\"r10\" bitsize=\"32\" type=\"uint32\"/>",
    "<reg name=\"r11\" bitsize=\"32\" type=\"uint32\"/>",
    "<reg name=\"r12\" bitsize=\"32\" type=\"uint32\"/>",
    "<reg name=\"sp\" bitsize=\"32\" type=\"data_ptr\"/>",
    "<reg name=\"lr\" bitsize=\"32\"/>",
    "<reg name=\"pc\" bitsize=\"32\" type=\"code_ptr\"/>",
    "<flags id=\"cpsr_flags\" size=\"4\">",
    "<field name=\"N\" start=\"31\" end=\"31\"/>",
    "<field name=\"Z\" start=\"30\" end=\"30\"/>",
    "<field name=\"C\" start=\"29\" end=\"29\"/>",
    "<field name=\"V\" start=\"28\" end=\"28\"/>",
    "<field name=\"I\" start=\"7\" end=\"7\"/>",
    "<field name=\"F\" start=\"6\" end=\"6\"/>",
    "<field name=\"T\" start=\"5\" end=\"5\"/>",
    "<field name=\"M\" start=\"0\" end=\"4\"/>",
    "</flags>",
    "<reg name=\"cpsr\" bitsize=\"32\" regnum=\"25\" type=\"cpsr_flags\"/>",
    "</feature>",
    "</target>"
);

/// `enum GDBStubAckState`
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ack {
    Pending,
    Received,
    NakReceived,
    Off,
}

/// `enum GDBWatchpointsBehvaior`
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WatchpointsBehavior {
    StandardLogic,
    OverrideLogic,
    OverrideLogicAnyWrite,
}

/// `struct GDBStub`
pub struct GdbStub {
    listener: Option<TcpListener>,
    connection: Option<TcpStream>,
    outgoing: String,
    memory_map_xml: String,
    line_ack: Ack,
    until_poll: i32,
    supports_swbreak: bool,
    supports_hwbreak: bool,
    watchpoints_behavior: WatchpointsBehavior,
    is_paused: bool,
    needs_callback: bool,
}

/// `_hex2int`
fn hex2int(hex: &[u8], max_digits: usize) -> u32 {
    let mut value = 0u32;
    for &c in hex.iter().take(max_digits) {
        let digit = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => break,
        };
        value = value.wrapping_mul(16).wrapping_add(digit as u32);
    }
    value
}

/// `_int2hex32`: little-endian byte order, as GDB expects for registers.
fn int2hex32(value: u32) -> String {
    value.to_le_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// `_readHex`: up to 8 digits before a `,`, `:` or `=`; advances `pos`.
fn read_hex(input: &[u8], pos: &mut usize) -> u32 {
    let mut i = 0;
    while i < 8 && *pos + i < input.len() && !matches!(input[*pos + i], b',' | b':' | b'=') {
        i += 1;
    }
    let v = hex2int(&input[*pos..], i);
    *pos += i;
    v
}

impl GdbStub {
    /// `GDBStubCreate` + `GDBStubListen`
    pub fn listen(address: SocketAddr, behavior: WatchpointsBehavior) -> std::io::Result<GdbStub> {
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        Ok(GdbStub {
            listener: Some(listener),
            connection: None,
            outgoing: String::new(),
            memory_map_xml: String::new(),
            line_ack: Ack::Pending,
            until_poll: GDB_STUB_INTERVAL,
            supports_swbreak: false,
            supports_hwbreak: false,
            watchpoints_behavior: behavior,
            is_paused: false,
            needs_callback: false,
        })
    }

    /// The address the stub is listening on.
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.listener.as_ref().and_then(|l| l.local_addr().ok())
    }

    fn send_raw(&mut self, data: &[u8]) {
        if let Some(c) = self.connection.as_mut() {
            let _ = c.write_all(data);
        }
    }

    /// `_sendMessage`
    fn send_message(&mut self) {
        if self.line_ack != Ack::Off {
            self.line_ack = Ack::Pending;
        }
        let mut body = std::mem::take(&mut self.outgoing);
        body.truncate(GDB_STUB_MAX_LINE - 5);
        let checksum = body.bytes().fold(0u8, |a, b| a.wrapping_add(b));
        let packet = format!("${body}#{checksum:02x}");
        self.send_raw(packet.as_bytes());
    }

    fn reply(&mut self, text: &str) {
        self.outgoing = text.to_string();
        self.send_message();
    }

    /// `_error`
    fn error(&mut self, error: u32) {
        self.reply(&format!("E{error:02x}"));
    }

    /// `_continue`
    fn cont(&mut self) {
        self.until_poll = GDB_STUB_INTERVAL;
        self.is_paused = false;
        self.needs_callback = true;
    }

    /// `_step`
    fn step(&mut self, ctx: &mut Context) {
        let pc = ctx.core.read_register("pc").unwrap_or(0) as u32;
        ctx.core.step();
        let new_pc = ctx.core.read_register("pc").unwrap_or(0) as u32;
        if pc >= GBA_SIZE_BIOS && new_pc < GBA_SIZE_BIOS {
            // GDB cannot cope with jumps into BIOS: skip over them with a
            // temporary breakpoint after the jump.
            let mut bp = Breakpoint::new(pc, -1);
            bp.is_temporary = true;
            if let Some(p) = ctx.core.debug_platform().as_mut() {
                p.set_breakpoint(bp, true);
            }
            self.cont();
            return;
        }
        self.reply(&format!("S{SIGTRAP:02x}"));
    }

    fn read_pc(ctx: &mut Context) -> u32 {
        ctx.core.instruction_address()
    }

    /// `_readMemory`
    fn read_memory(&mut self, ctx: &mut Context, message: &[u8]) {
        let mut i = 0;
        let address = read_hex(message, &mut i);
        i += 1;
        let size = read_hex(message, &mut i);
        if size > 512 {
            self.error(GDB_BAD_ARGUMENTS);
            return;
        }
        let mut out = String::new();
        for n in 0..size {
            let byte = ctx.core.bus_read8(address.wrapping_add(n));
            out.push_str(&format!("{:02x}", byte as u8));
        }
        self.reply(&out);
    }

    /// `_writeMemory` / `_writeMemoryBinary`
    fn write_memory(&mut self, ctx: &mut Context, message: &[u8], binary: bool) {
        let mut i = 0;
        let address = read_hex(message, &mut i);
        i += 1;
        let size = read_hex(message, &mut i);
        i += 1;
        if size > 512 {
            self.error(GDB_BAD_ARGUMENTS);
            return;
        }
        let data = &message[i.min(message.len())..];
        let mut pos = 0;
        for n in 0..size {
            let byte = if binary {
                let mut b = *data.get(pos).unwrap_or(&0);
                pos += 1;
                // Parse escape char
                if b == 0x7D {
                    b = data.get(pos).unwrap_or(&0) ^ 0x20;
                    pos += 1;
                }
                b
            } else {
                let b = hex2int(&data[pos.min(data.len())..], 2) as u8;
                pos += 2;
                b
            };
            ctx.core.raw_write8(address.wrapping_add(n), -1, byte as u32);
        }
        self.reply("OK");
    }

    /// `_readGPRs`
    fn read_gprs(&mut self, ctx: &mut Context) {
        let mut out = String::new();
        for r in 0..15 {
            out.push_str(&int2hex32(ctx.core.read_register(&format!("r{r}")).unwrap_or(0) as u32));
        }
        out.push_str(&int2hex32(Self::read_pc(ctx)));
        out.push_str(&int2hex32(ctx.core.read_register("cpsr").unwrap_or(0) as u32));
        self.reply(&out);
    }

    /// `_writeGPRs`
    fn write_gprs(&mut self, ctx: &mut Context, message: &[u8]) {
        for r in 0..=15 {
            let v = hex2int(&message[(r * 8).min(message.len())..], 8);
            ctx.core.write_register(&format!("r{r}"), v as i32);
        }
        self.reply("OK");
    }

    /// `_readRegister`
    fn read_register(&mut self, ctx: &mut Context, message: &[u8]) {
        let mut i = 0;
        let reg = read_hex(message, &mut i);
        let value = match reg {
            0..=14 => ctx.core.read_register(&format!("r{reg}")).unwrap_or(0) as u32,
            15 => Self::read_pc(ctx),
            0x19 => ctx.core.read_register("cpsr").unwrap_or(0) as u32,
            _ => {
                self.reply("");
                return;
            }
        };
        self.reply(&int2hex32(value));
    }

    /// `_writeRegister`
    fn write_register(&mut self, ctx: &mut Context, message: &[u8]) {
        let mut i = 0;
        let reg = read_hex(message, &mut i);
        i += 1;
        let value = read_hex(message, &mut i).swap_bytes();
        let ok = match reg {
            0..=15 => ctx.core.write_register(&format!("r{reg}"), value as i32),
            0x19 => ctx.core.write_register("cpsr", value as i32),
            _ => false,
        };
        self.reply(if ok { "OK" } else { "" });
    }

    /// `_processQSupportedCommand`
    fn q_supported(&mut self, message: &str) {
        self.supports_swbreak = false;
        self.supports_hwbreak = false;
        let body = message.split('#').next().unwrap_or("");
        for feature in body.split(';') {
            match feature {
                "swbreak+" => self.supports_swbreak = true,
                "hwbreak+" => self.supports_hwbreak = true,
                "swbreak-" => self.supports_swbreak = false,
                "hwbreak-" => self.supports_hwbreak = false,
                _ => {}
            }
        }
        self.outgoing =
            "swbreak+;hwbreak+;qXfer:features:read+;qXfer:memory-map:read+;QStartNoAckMode+".to_string();
    }

    /// `_processQXferCommand`
    fn q_xfer(&mut self, params: &str, data: &str) -> bool {
        let params = params.split('#').next().unwrap_or("");
        let Some((off, len)) = params.split_once(',') else {
            self.error(GDB_BAD_ARGUMENTS);
            return false;
        };
        let offset = u32::from_str_radix(off, 16).unwrap_or(0) as usize;
        let mut length = u32::from_str_radix(len, 16).unwrap_or(0) as usize;
        if length + 4 > GDB_STUB_MAX_LINE {
            length = GDB_STUB_MAX_LINE - 4;
        }
        if data.len() < offset {
            self.error(GDB_BAD_ARGUMENTS);
            return false;
        }
        let end = (offset + length).min(data.len());
        let prefix = if data.len() <= offset + length { 'l' } else { 'm' };
        self.outgoing = format!("{prefix}{}", &data[offset..end]);
        true
    }

    /// `_generateMemoryMapXml`
    fn memory_map(ctx: &mut Context) -> String {
        let mut xml = String::from("<memory-map version=\"1.0\">");
        for b in ctx.core.list_memory_blocks() {
            if b.flags & MEMORY_MAPPED == 0 {
                continue;
            }
            let ty = if b.flags & (MEMORY_WRITE | MEMORY_WORM) != 0 { "ram" } else { "rom" };
            xml.push_str(&format!("<memory type=\"{ty}\" start=\"0x{:08x}\" length=\"0x{:08x}\"/>", b.start, b.size));
        }
        xml.push_str("</memory-map>");
        xml
    }

    /// `_processQReadCommand`
    fn q_read(&mut self, ctx: &mut Context, message: &str) {
        self.outgoing.clear();
        if message.starts_with("HostInfo#") {
            let info =
                format!("cputype:{MACH_O_ARM};cpusubtype:{MACH_O_ARM_V4T}:ostype:none;vendor:none;endian:little;ptrsize:4;");
            self.reply(&info);
            return;
        }
        if message.starts_with("Attached#") {
            self.outgoing = "1".into();
        } else if message.starts_with("VAttachOrWaitSupported#") {
            self.outgoing = "OK".into();
        } else if message.starts_with("C#") {
            self.outgoing = "QC1".into();
        } else if message.starts_with("fThreadInfo#") {
            self.outgoing = "m1".into();
        } else if message.starts_with("sThreadInfo#") {
            self.outgoing = "l".into();
        } else if let Some(rest) = message.strip_prefix("Xfer:features:read:target.xml:") {
            if !self.q_xfer(rest, TARGET_XML) {
                return;
            }
        } else if let Some(rest) = message.strip_prefix("Xfer:memory-map:read::") {
            if self.memory_map_xml.is_empty() {
                self.memory_map_xml = Self::memory_map(ctx);
            }
            let xml = self.memory_map_xml.clone();
            if !self.q_xfer(rest, &xml) {
                return;
            }
        } else if let Some(rest) = message.strip_prefix("Supported:") {
            self.q_supported(rest);
        }
        self.send_message();
    }

    /// `_setBreakpoint`
    fn set_breakpoint(&mut self, ctx: &mut Context, message: &[u8]) {
        let mut i = 2;
        let address = read_hex(message, &mut i);
        let behavior = self.watchpoints_behavior;
        let Some(p) = ctx.core.debug_platform().as_mut() else {
            self.reply("");
            return;
        };
        let wp = |ty| Watchpoint::new(address, address.wrapping_add(1), -1, ty);
        match message.first() {
            Some(b'0' | b'1') => {
                p.set_breakpoint(Breakpoint::new(address, -1), true);
            }
            Some(b'2') => {
                let ty = if behavior == WatchpointsBehavior::OverrideLogicAnyWrite {
                    WATCHPOINT_WRITE
                } else {
                    WATCHPOINT_WRITE_CHANGE
                };
                p.set_watchpoint(wp(ty));
            }
            Some(b'3') => {
                p.set_watchpoint(wp(WATCHPOINT_READ));
            }
            Some(b'4') => {
                p.set_watchpoint(wp(WATCHPOINT_RW));
            }
            _ => {
                self.reply("");
                return;
            }
        }
        self.reply("OK");
    }

    /// `_clearBreakpoint`
    fn clear_breakpoint(&mut self, ctx: &mut Context, message: &[u8]) {
        let mut i = 2;
        let address = read_hex(message, &mut i);
        let ids: Vec<isize> = match (message.first(), ctx.core.debug_platform().as_ref()) {
            (Some(b'0' | b'1'), Some(p)) => {
                p.list_breakpoints().iter().filter(|b| b.address == address).map(|b| b.id).collect()
            }
            (Some(b'2' | b'3' | b'4'), Some(p)) => p
                .watchpoints
                .iter()
                .filter(|w| address >= w.min_address && address < w.max_address)
                .map(|w| w.id)
                .collect(),
            _ => Vec::new(),
        };
        for id in ids {
            ctx.core.clear_breakpoint(id);
        }
        self.reply("OK");
    }

    /// `_parseGDBMessage`: returns how many bytes of `line` were consumed.
    fn parse_message(&mut self, ctx: &mut Context, line: &[u8]) -> (usize, Option<EntryReason>) {
        let mut parsed = 1;
        match line[0] {
            b'+' => {
                self.line_ack = Ack::Received;
                return (parsed, None);
            }
            b'-' => {
                self.line_ack = Ack::NakReceived;
                return (parsed, None);
            }
            b'$' => {}
            0x03 => {
                self.is_paused = true;
                return (parsed, Some(EntryReason::Manual));
            }
            _ => {
                self.send_raw(b"-");
                return (parsed, None);
            }
        }
        let message = &line[1..];
        let Some(hash) = message.iter().position(|&b| b == b'#') else {
            self.send_raw(b"-");
            return (line.len(), None);
        };
        parsed += hash + 1;
        if message.len() < hash + 3 {
            self.send_raw(b"-");
            return (line.len(), None);
        }
        parsed += 2;
        let checksum = message[..hash].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        let network_checksum = hex2int(&message[hash + 1..], 2) as u8;
        if network_checksum != checksum {
            self.send_raw(b"-");
            return (parsed, None);
        }
        if self.line_ack != Ack::Off {
            self.send_raw(b"+");
        }
        let ty = message[0];
        // The handlers see the rest of the packet including '#' and the checksum.
        let body = &message[1..hash + 3];
        let body_str = String::from_utf8_lossy(body).into_owned();
        let mut entry = None;
        match ty {
            b'?' => self.reply(&format!("S{SIGINT:02x}")),
            b'c' => self.cont(),
            b'G' => self.write_gprs(ctx, body),
            b'g' => self.read_gprs(ctx),
            b'H' | b'T' => self.reply("OK"),
            b'M' => self.write_memory(ctx, body, false),
            b'm' => self.read_memory(ctx, body),
            b'P' => self.write_register(ctx, body),
            b'p' => self.read_register(ctx, body),
            b'Q' => {
                if body_str.starts_with("StartNoAckMode#") {
                    self.line_ack = Ack::Off;
                    self.reply("OK");
                } else {
                    self.reply("");
                }
            }
            b'q' => self.q_read(ctx, &body_str),
            b's' => self.step(ctx),
            b'V' => self.reply(""),
            b'v' => {
                if body_str.starts_with("Attach") {
                    self.is_paused = true;
                    entry = Some(EntryReason::Manual);
                    self.reply("1");
                } else {
                    self.reply("");
                }
            }
            b'X' => self.write_memory(ctx, body, true),
            b'Z' => self.set_breakpoint(ctx, body),
            b'z' => self.clear_breakpoint(ctx, body),
            _ => self.error(GDB_UNSUPPORTED_COMMAND),
        }
        (parsed, entry)
    }

    /// `GDBStubHangup`
    fn hangup(&mut self) {
        self.reply("W00");
        self.connection = None;
        self.needs_callback = false;
        self.is_paused = false;
    }

    /// `GDBStubUpdate`. Returns an entry the stub wants the debugger to make.
    fn update_stub(&mut self, ctx: &mut Context, timeout_ms: i32) -> Option<EntryReason> {
        let Some(listener) = self.listener.as_ref() else {
            self.needs_callback = false;
            self.is_paused = false;
            return None;
        };
        if self.connection.is_none() {
            let deadline = Instant::now() + Duration::from_millis(timeout_ms.max(0) as u64);
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(true);
                        let _ = stream.set_nodelay(true);
                        self.connection = Some(stream);
                        return Some(EntryReason::Attached);
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            return None;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => return None,
                }
            }
        }
        let mut buf = vec![0u8; GDB_STUB_MAX_LINE - 1];
        let deadline = Instant::now() + Duration::from_millis(timeout_ms.max(0) as u64);
        let n = loop {
            let conn = self.connection.as_mut().unwrap();
            match conn.read(&mut buf) {
                Ok(0) => {
                    self.hangup();
                    return None;
                }
                Ok(n) => break n,
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(_) => {
                    self.hangup();
                    return None;
                }
            }
        };
        let line = &buf[..n];
        let mut position = 0;
        let mut entry = None;
        while position < n {
            let (used, e) = self.parse_message(ctx, &line[position..]);
            position += used.max(1);
            if e.is_some() {
                entry = e;
            }
        }
        entry
    }

    /// The `mDebuggerEnter` calls inside `GDBStubUpdate` and `_parseGDBMessage`:
    /// every entry pauses the module; a manual one also reports SIGINT.
    fn handle_entry(&mut self, ctx: &mut Context, entry: Option<EntryReason>) {
        match entry {
            Some(EntryReason::Manual) => {
                self.is_paused = true;
                self.entered(ctx, EntryReason::Manual, None);
            }
            Some(_) => self.is_paused = true,
            None => {}
        }
    }
}

impl Module for GdbStub {
    fn is_paused(&self) -> bool {
        self.is_paused
    }

    fn set_paused(&mut self, paused: bool) {
        self.is_paused = paused;
    }

    fn needs_callback(&self) -> bool {
        self.needs_callback
    }

    /// `_gdbStubDeinit` → `GDBStubShutdown`
    fn deinit(&mut self, _ctx: &mut Context) {
        if self.connection.is_some() {
            self.hangup();
        }
        self.listener = None;
    }

    /// `_gdbStubWait`
    fn paused(&mut self, ctx: &mut Context, timeout_ms: i32) {
        let entry = self.update_stub(ctx, timeout_ms);
        self.handle_entry(ctx, entry);
    }

    /// `_gdbStubUpdate`
    fn update(&mut self, ctx: &mut Context) {
        let entry = self.update_stub(ctx, 0);
        self.handle_entry(ctx, entry);
    }

    /// `_gdbStubPoll`
    fn custom(&mut self, ctx: &mut Context) {
        self.until_poll -= 1;
        if self.until_poll > 0 {
            return;
        }
        self.until_poll = GDB_STUB_INTERVAL;
        let entry = self.update_stub(ctx, 0);
        self.handle_entry(ctx, entry);
    }

    /// `_gdbStubEntered`
    fn entered(&mut self, _ctx: &mut Context, reason: EntryReason, info: Option<&EntryInfo>) {
        let text = match reason {
            EntryReason::Manual => format!("S{SIGINT:02x}"),
            EntryReason::Breakpoint => match info {
                Some(i) if self.supports_hwbreak && self.supports_swbreak => {
                    let k = if i.break_type == BreakpointType::Software { 's' } else { 'h' };
                    format!("T{SIGTRAP:02x}{k}wbreak:;")
                }
                _ => format!("S{SIGTRAP:02x}k"),
            },
            EntryReason::Watchpoint => match info {
                Some(i) => {
                    if self.watchpoints_behavior != WatchpointsBehavior::StandardLogic
                        && i.watch_type & WATCHPOINT_WRITE != 0
                    {
                        format!("S{SIGTRAP:02x}")
                    } else {
                        let ty = match i.watch_type {
                            WATCHPOINT_WRITE | WATCHPOINT_WRITE_CHANGE => "watch",
                            WATCHPOINT_READ => "rwatch",
                            WATCHPOINT_RW => "awatch",
                            _ => "",
                        };
                        format!("T{SIGTRAP:02x}{ty}:{:08x};", i.address)
                    }
                }
                None => format!("S{SIGTRAP:02x}"),
            },
            EntryReason::IllegalOp => format!("S{SIGILL:02x}"),
            EntryReason::Attached | EntryReason::Stack => return,
        };
        self.reply(&text);
    }
}
