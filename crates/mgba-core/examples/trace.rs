// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Debug helper: run a ROM for N frames, print CPU state and core log.
use mgba_core::Gba;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom = std::fs::read(&args[1]).unwrap();
    let frames: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(10);
    let skip: usize = args.get(3).and_then(|a| a.parse().ok()).unwrap_or(0);
    let keys: u16 = args.get(4).map(|a| u16::from_str_radix(a, 16).unwrap()).unwrap_or(0);
    let mut gba = Gba::new();
    gba.log_enabled = true;
    gba.load_rom(rom);
    gba.reset();
    for f in 0..frames {
        if f == skip { gba.set_keys(keys); }
        gba.run_frame();
        let log = gba.drain_log();
        for (lvl, msg) in log.iter().filter(|(l, _)| !matches!(l, mgba_core::gba::LogLevel::Debug)).take(10) {
            println!("[f{f}] {:?}: {}", lvl, msg);
        }
        let c = &gba.cpu;
        println!("  root={:?} reroot={:?} master={} cycles={} next={}", gba.timing.root, gba.timing.reroot, gba.timing.master_cycles, c.cycles, c.next_event);
        println!("f{f} pc={:08X} cpsr={:08X} halted={} dispcnt={:04X} ie={:04X} if={:04X} ime={} keycnt={:04X} keys_last={:04X}", c.gprs[15], c.cpsr.0, c.halted,
            gba.memory.io[0], gba.memory.io[0x100], gba.memory.io[0x101], gba.memory.io[0x104], gba.memory.io[0x132/2], gba.keys_last);
    }
}
