// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Developer tools and the smaller GBA features ported alongside them:
//! idle-loop detection, AGBPrint, cheat hooks, memory search, the CLI
//! debugger (GBA and GB) and the GDB stub. ROMs are assembled here.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mgba_core::cheats::{Cheat, CheatSet, CheatType, SetKind};
use mgba_core::core::{MCore, MEMORY_RW};
use mgba_core::debugger::cli::{ChannelBackend, CliBackend, CliDebugger};
use mgba_core::debugger::gdb::{GdbStub, WatchpointsBehavior};
use mgba_core::debugger::{Debugger, EntryReason, State};
use mgba_core::gba::IdleLoopOptimization;
use mgba_core::mem_search::{search, search_repeat, SearchOp, SearchParams, SearchType};
use mgba_core::{Gb, Gba};

// ----- tiny ARM assembler -----

fn imm(v: u32) -> u32 {
    for rot in 0..16 {
        let x = v.rotate_left(rot * 2);
        if x < 0x100 {
            return (rot << 8) | x;
        }
    }
    panic!("{v:#x} is not an ARM immediate");
}

/// Load a 32-bit constant into `rd` with mov/orr.
fn li(code: &mut Vec<u32>, rd: u32, value: u32) {
    code.push(0xE3A0_0000 | (rd << 12) | imm(value & 0xFF));
    for shift in [8, 16, 24] {
        let byte = (value >> shift) & 0xFF;
        if byte != 0 {
            code.push(0xE380_0000 | (rd << 16) | (rd << 12) | imm(byte << shift));
        }
    }
}

/// `strh rd, [rn]`
fn strh(code: &mut Vec<u32>, rd: u32, rn: u32) {
    code.push(0xE1C0_00B0 | (rn << 16) | (rd << 12));
}

/// ARM code at 0xC0, then `thumb` halfwords at `thumb_at` (an offset in the ROM).
fn gba_rom(arm: &[u32], thumb: &[(usize, u16)]) -> Vec<u8> {
    let mut rom = vec![0u8; 0x10000];
    rom[0..4].copy_from_slice(&0xEA00_002Eu32.to_le_bytes()); // b 0xC0
    rom[0xA0..0xAC].copy_from_slice(b"DEVTOOLS\0\0\0\0");
    rom[0xAC..0xB0].copy_from_slice(b"DEVT");
    rom[0xB2] = 0x96;
    for (i, w) in arm.iter().enumerate() {
        rom[0xC0 + i * 4..0xC4 + i * 4].copy_from_slice(&w.to_le_bytes());
    }
    for &(at, h) in thumb {
        rom[at..at + 2].copy_from_slice(&h.to_le_bytes());
    }
    rom
}

fn gba_with(rom: Vec<u8>) -> Box<Gba> {
    let mut g = Box::new(Gba::new());
    assert!(g.load_rom(rom));
    g.skip_bios_on_reset = true;
    g.reset();
    g
}

/// Enter Thumb at 0x080000C8: `add r0, pc, #1; bx r0`, then Thumb code.
fn thumb_entry() -> Vec<u32> {
    vec![0xE28F_0001, 0xE12F_FF10]
}

// ----- idle loops, AGBPrint, cheat hooks -----

#[test]
fn detects_thumb_idle_loop() {
    // 0x080000C8: b .
    let mut gba = gba_with(gba_rom(&thumb_entry(), &[(0xC8, 0xE7FE)]));
    gba.set_idle_optimization(IdleLoopOptimization::Detect);
    assert_eq!(gba.idle_optimization, IdleLoopOptimization::Detect);
    for _ in 0..5 {
        gba.run_frame();
    }
    assert_eq!(gba.idle_loop, 0x0800_00C8, "idle loop not found");
    assert_eq!(gba.idle_optimization, IdleLoopOptimization::Remove);
}

#[test]
fn agbprint_reaches_the_log() {
    let mut code = vec![];
    // Unlock AGBPrint: protect = 0x20
    li(&mut code, 0, 0x09FE_2FFE);
    li(&mut code, 1, 0x20);
    strh(&mut code, 1, 0);
    // "Hi" into the buffer
    li(&mut code, 0, 0x09FD_0000);
    li(&mut code, 1, 0x6948);
    strh(&mut code, 1, 0);
    // get = 0, put = 2
    li(&mut code, 0, 0x09FE_20FC);
    li(&mut code, 1, 0);
    strh(&mut code, 1, 0);
    li(&mut code, 0, 0x09FE_20FE);
    li(&mut code, 1, 2);
    strh(&mut code, 1, 0);
    // Call the flush stub (swi 0xFA; bx lr) in Thumb mode
    li(&mut code, 2, 0x09FE_209D);
    code.push(0xE1A0_E00F); // mov lr, pc
    code.push(0xE12F_FF12); // bx r2
    code.push(0xEAFF_FFFE); // b .
    let mut gba = gba_with(gba_rom(&code, &[]));
    gba.run_frame();
    let log: Vec<String> = gba.drain_log().into_iter().map(|(_, m)| m).collect();
    assert!(log.iter().any(|m| m == "[GBA Debug] Hi"), "AGBPrint output missing: {log:?}");
}

#[test]
fn cheat_hook_runs_at_its_address() {
    // 0x080000C8: adds r3, #1; b 0x080000C8
    let mut gba = gba_with(gba_rom(&thumb_entry(), &[(0xC8, 0x3301), (0xCA, 0xE7FD)]));
    let mut set = CheatSet::new_gba("hooked");
    if let SetKind::Gba(st) = &mut set.kind {
        st.hook = Some(0x0800_00C8);
    }
    set.list.push(Cheat { ty: CheatType::Add, width: 1, address: 0x0200_0010, operand: 1, repeat: 1, ..Default::default() });
    gba.cheats.sets.push(set);
    // The first frame end plants the hook breakpoint in ROM.
    gba.run_frame();
    assert_eq!(&gba.memory.rom[0xC8..0xCA], &[0x01, 0xBE]);
    let (r3_before, hits_before) = (gba.cpu.gprs[3], gba.memory.wram[0x10]);
    for _ in 0..2 {
        gba.run_frame();
    }
    let r3 = gba.cpu.gprs[3];
    assert!(r3 > r3_before + 1000, "the hooked instruction didn't keep running: r3 = {r3}");
    // The cheat ran once per pass through the hook, not once per frame.
    let loops = (r3 - r3_before) & 0xFF;
    let hits = gba.memory.wram[0x10].wrapping_sub(hits_before) as u32;
    // A frame can end between the hook and the instruction it runs.
    assert!(hits.abs_diff(loops) <= 1, "hook ran {hits} times for {loops} loop iterations (mod 256)");
    // Removing the set restores the instruction.
    let mut cheats = std::mem::take(&mut gba.cheats);
    cheats.remove_set(gba.as_mut(), 0);
    gba.cheats = cheats;
    assert_eq!(&gba.memory.rom[0xC8..0xCA], &[0x01, 0x33]);
}

// ----- memory search -----

#[test]
fn memory_search_finds_and_narrows() {
    let mut gba = gba_with(gba_rom(&[0xEAFF_FFFE], &[]));
    gba.bus_write16(0x0200_0100, 1234);
    gba.bus_write16(0x0300_0200, 1234);
    let params = SearchParams {
        memory_flags: MEMORY_RW,
        ty: SearchType::Int,
        op: SearchOp::Equal,
        align: 2,
        width: 2,
        value_int: 1234,
        value_str: String::new(),
    };
    let mut results = search(gba.as_ref(), &params, 0);
    let addresses: Vec<u32> = results.iter().map(|r| r.address).collect();
    assert!(addresses.contains(&0x0200_0100) && addresses.contains(&0x0300_0200), "{addresses:x?}");
    // One value goes up by 5: a delta search keeps only it.
    gba.bus_write16(0x0200_0100, 1239);
    let delta = SearchParams { op: SearchOp::Delta, value_int: 5, ..params.clone() };
    search_repeat(gba.as_mut(), &delta, &mut results);
    assert_eq!(results.iter().map(|r| r.address).collect::<Vec<_>>(), vec![0x0200_0100]);
    // Guessing finds a value stored divided by 10.
    gba.bus_write16(0x0200_0300, 37);
    let guess = SearchParams { ty: SearchType::Guess, value_str: "370".into(), width: -1, align: -1, ..params };
    let found = search(gba.as_ref(), &guess, 0);
    assert!(found.iter().any(|r| r.address == 0x0200_0300 && r.guess_divisor == 10), "guess failed");
}

// ----- CLI debugger -----

/// A CLI backend fed from a list of lines, capturing what's printed.
struct Scripted {
    inner: ChannelBackend,
    out: Arc<Mutex<String>>,
}

impl CliBackend for Scripted {
    fn print(&mut self, text: &str) {
        self.out.lock().unwrap().push_str(text);
    }
    fn poll(&mut self, timeout_ms: i32) -> i32 {
        self.inner.poll(timeout_ms)
    }
    fn readline(&mut self) -> Option<String> {
        self.inner.readline()
    }
    fn history_last(&self) -> Option<String> {
        self.inner.history_last()
    }
    fn history_append(&mut self, line: &str) {
        self.inner.history_append(line)
    }
}

fn cli(lines: &[&str]) -> (Box<CliDebugger>, Arc<Mutex<String>>) {
    let (tx, rx) = channel();
    for l in lines {
        tx.send(format!("{l}\n")).unwrap();
    }
    drop(tx);
    let out = Arc::new(Mutex::new(String::new()));
    let backend = Scripted { inner: ChannelBackend::new(rx), out: out.clone() };
    (Box::new(CliDebugger::new(Box::new(backend))), out)
}

fn run_until_shutdown(debugger: &mut Debugger, core: &mut dyn mgba_core::debugger::DebugCore) {
    for _ in 0..2_000_000 {
        if debugger.is_shutdown() {
            return;
        }
        debugger.run_timeout(core, 10);
    }
    panic!("debugger never shut down (state {:?})", debugger.state);
}

#[test]
fn cli_debugger_breakpoints_watchpoints_and_memory() {
    // 0xC0: mov r0, #0x02000000; mov r1, #0x42; strb r1, [r0]; mov r2, #7; b .
    let code = vec![0xE3A0_0402, 0xE3A0_1042, 0xE5C0_1000, 0xE3A0_2007, 0xEAFF_FFFE];
    let mut gba = gba_with(gba_rom(&code, &[]));
    let (module, out) = cli(&[
        "break 0x080000CC",
        "watch/w 0x02000000",
        "listb",
        // The watchpoint (during the strb) and the breakpoint on the next
        // instruction are both reported after the same step, as in mGBA.
        "c",
        "p/x r1",
        "x/1 0x02000000 2",
        "dis 0x080000C0 2",
        "w/r r5 0x1234",
        "p r5 + 1",
        "delete 1",
        "q",
    ]);
    let mut debugger = Debugger::new();
    debugger.attach_module(module);
    debugger.attach(gba.as_mut());
    debugger.enter(gba.as_mut(), EntryReason::Attached, None);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_until_shutdown(&mut debugger, gba.as_mut())));
    let out = out.lock().unwrap().clone();
    assert!(result.is_ok(), "output so far:\n{out}");
    assert!(out.contains("Added breakpoint #1"), "{out}");
    assert!(out.contains("Added watchpoint #2"), "{out}");
    assert!(out.contains("E 1: 0x80000CC"), "{out}");
    assert!(out.contains("Hit watchpoint 2 at 0x02000000: (new value = 0x00000042, old value = 0x00000000)"), "{out}");
    assert!(out.contains("Hit breakpoint 1 at 0x080000CC"), "{out}");
    assert!(out.contains(" 0x00000042"), "{out}");
    assert!(out.contains("0x02000000: 42 00"), "{out}");
    assert!(out.contains("080000C0:  E3A00402\tmov r0, #33554432"), "{out}");
    assert!(out.contains(" 4661"), "{out}");
    assert!(out.contains(" r0: 02000000"), "status should show registers: {out}");
    debugger.detach(gba.as_mut());
    assert!(gba.debugger.is_none());
}

#[test]
fn cli_debugger_software_breakpoint_and_trace() {
    // 0xC0: mov r0, #1; mov r1, #2; add r2, r0, r1; b .
    let code = vec![0xE3A0_0001, 0xE3A0_1002, 0xE080_2001, 0xEAFF_FFFE];
    let mut gba = gba_with(gba_rom(&code, &[]));
    let (module, out) = cli(&["break/a 0x080000C8", "c", "trace 2", "q"]);
    let mut debugger = Debugger::new();
    debugger.attach_module(module);
    debugger.attach(gba.as_mut());
    // The BKPT is in ROM while the breakpoint is set.
    debugger.enter(gba.as_mut(), EntryReason::Attached, None);
    run_until_shutdown(&mut debugger, gba.as_mut());
    let out = out.lock().unwrap().clone();
    assert!(out.contains("Hit breakpoint 1 at 0x080000C8"), "{out}");
    assert_eq!(gba.cpu.gprs[2], 3, "the replaced instruction must still run");
    assert!(out.contains("| E0802001: add r2, r0, r1") || out.contains("| EAFFFFFE: b 0x080000CC"), "{out}");
    debugger.detach(gba.as_mut());
    assert_eq!(&gba.memory.rom[0xC8..0xCC], &0xE080_2001u32.to_le_bytes(), "detaching restores the ROM");
}

#[test]
fn cli_debugger_on_game_boy() {
    let mut rom = vec![0u8; 0x8000];
    rom[0x100..0x104].copy_from_slice(&[0x00, 0xC3, 0x50, 0x01]);
    rom[0x134..0x13C].copy_from_slice(b"DEVTOOLS");
    // 0x150: ld a, $42; ld ($C000), a; jr -2
    rom[0x150..0x157].copy_from_slice(&[0x3E, 0x42, 0xEA, 0x00, 0xC0, 0x18, 0xFE]);
    let mut gb = Box::new(Gb::new());
    gb.load_rom(rom);
    gb.reset();
    let (module, out) = cli(&["break 0x152", "watch/w 0xC000", "c", "c", "p/x a", "dis 0x150 3", "x/1 $C000 1", "q"]);
    let mut debugger = Debugger::new();
    debugger.attach_module(module);
    debugger.attach(gb.as_mut());
    debugger.enter(gb.as_mut(), EntryReason::Attached, None);
    run_until_shutdown(&mut debugger, gb.as_mut());
    let out = out.lock().unwrap().clone();
    assert!(out.contains("Hit breakpoint 1 at 0x00000152"), "{out}");
    assert!(out.contains("Hit watchpoint 2 at 0x0000C000: (new value = 0x00000042, old value = 0x00000000)"), "{out}");
    assert!(out.contains(" 0x00000042"), "{out}");
    assert!(out.contains("0150:  3E42\tld a, $42"), "{out}");
    assert!(out.contains("0152:  EA00C0\tld [$C000], a"), "{out}");
    assert!(out.contains("A: 42"), "{out}");
}

// ----- GDB stub -----

fn packet(body: &str) -> String {
    let sum = body.bytes().fold(0u8, |a, b| a.wrapping_add(b));
    format!("${body}#{sum:02x}")
}

fn read_reply(s: &mut TcpStream) -> String {
    let mut got = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        let n = s.read(&mut buf).expect("gdb read");
        assert!(n > 0, "connection closed");
        got.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&got);
        if let Some(start) = text.find('$') {
            if let Some(end) = text[start..].find('#') {
                if text.len() >= start + end + 3 {
                    return text[start + 1..start + end].to_string();
                }
            }
        }
    }
}

#[test]
fn gdb_stub_reads_registers_memory_and_breaks() {
    let code = vec![0xE3A0_0001, 0xE3A0_1002, 0xE080_2001, 0xEAFF_FFFE];
    let mut gba = gba_with(gba_rom(&code, &[]));
    let stub = GdbStub::listen("127.0.0.1:0".parse().unwrap(), WatchpointsBehavior::StandardLogic).unwrap();
    let addr = stub.local_addr().unwrap();
    let mut debugger = Debugger::new();
    debugger.attach_module(Box::new(stub));
    debugger.attach(gba.as_mut());
    debugger.enter(gba.as_mut(), EntryReason::Attached, None);

    let client = std::thread::spawn(move || {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let ask = |s: &mut TcpStream, body: &str| {
            s.write_all(packet(body).as_bytes()).unwrap();
            read_reply(s)
        };
        let mut replies = vec![];
        replies.push(ask(&mut s, "?"));
        replies.push(ask(&mut s, "qSupported:swbreak+;hwbreak+"));
        replies.push(ask(&mut s, "m80000c0,4"));
        replies.push(ask(&mut s, "Z0,80000c8,4"));
        replies.push(ask(&mut s, "c"));
        replies.push(ask(&mut s, "p2"));
        replies.push(ask(&mut s, "g"));
        replies.push(ask(&mut s, "qXfer:features:read:target.xml:0,40"));
        s.write_all(packet("D").as_bytes()).unwrap();
        replies.push(read_reply(&mut s));
        drop(s);
        replies
    });

    for _ in 0..2_000_000 {
        if client.is_finished() {
            break;
        }
        debugger.run_timeout(gba.as_mut(), 5);
    }
    let replies = client.join().unwrap();
    assert_eq!(replies[0], "S02");
    assert!(replies[1].contains("swbreak+;hwbreak+"));
    assert_eq!(replies[2], "0100a0e3", "memory is little-endian bytes of mov r0, #1");
    assert_eq!(replies[3], "OK");
    assert_eq!(replies[4], "T05hwbreak:;");
    assert_eq!(replies[5], "00000000", "r2 before the add runs");
    assert!(replies[6].starts_with("0100000002000000"), "{}", replies[6]);
    assert!(replies[6][15 * 8..16 * 8].eq("c8000008"), "pc at the breakpoint: {}", replies[6]);
    assert!(replies[7].starts_with("m<target version"), "{}", replies[7]);
    assert_eq!(replies[8], "E02");
    assert_ne!(debugger.state, State::Shutdown);
}
