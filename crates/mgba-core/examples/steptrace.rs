// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Debug helper: single-step until the first logged error and print recent instructions.
use mgba_core::{arm, Gba};
use std::collections::VecDeque;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom = std::fs::read(&args[1]).unwrap();
    let mut gba = Gba::new();
    gba.log_enabled = true;
    gba.load_rom(rom);
    gba.reset();
    let mut ring: VecDeque<String> = VecDeque::new();
    for _ in 0..50_000_000u64 {
        let c = &gba.cpu;
        let thumb = c.cpsr.t();
        let pc = c.gprs[15].wrapping_sub(if thumb { 4 } else { 8 });
        ring.push_back(format!("{} {:08X}: {:08X}  r0={:08X} r1={:08X} r2={:08X} r3={:08X} r4={:08X} sp={:08X} lr={:08X} cpsr={:08X}",
            if thumb {"T"} else {"A"}, pc, c.prefetch[0], c.gprs[0], c.gprs[1], c.gprs[2], c.gprs[3], c.gprs[4], c.gprs[13], c.gprs[14], c.cpsr.0));
        if ring.len() > 60 { ring.pop_front(); }
        arm::run(&mut gba);
        let log = gba.drain_log();
        if log.iter().any(|(l, _)| !matches!(l, mgba_core::gba::LogLevel::Debug | mgba_core::gba::LogLevel::Info)) {
            for l in &ring { println!("{l}"); }
            for l in log { println!("{:?}", l); }
            return;
        }
    }
}
