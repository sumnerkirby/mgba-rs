// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Cheat engine checks for both cores, using small synthesized Game Boy ROMs
//! and one of the GBA cinema test ROMs as a host program.

use std::path::Path;

use mgba_core::cheats::gba::{encrypt_gameshark, GAMESHARK_SEEDS};
use mgba_core::cheats::{CheatDevice, CheatSet};
use mgba_core::{Gb, Gba};

fn gb_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];
    rom[0x100..0x104].copy_from_slice(&[0x00, 0xC3, 0x50, 0x01]);
    rom[0x150..0x152].copy_from_slice(&[0x18, 0xFE]); // jr -2
    rom[0x200] = 0x11;
    rom
}

#[test]
fn tea_round_trips() {
    let (mut a, mut b) = (0x12345678u32, 0x9ABCDEF0u32);
    encrypt_gameshark(&mut a, &mut b, &GAMESHARK_SEEDS);
    assert_ne!((a, b), (0x12345678, 0x9ABCDEF0));
    mgba_core::cheats::gba::decrypt_gameshark(&mut a, &mut b, &GAMESHARK_SEEDS);
    assert_eq!((a, b), (0x12345678, 0x9ABCDEF0));
}

#[test]
fn gb_gameshark_and_game_genie() {
    let mut gb = Gb::new();
    gb.load_rom(gb_rom());
    gb.reset();
    let mut set = CheatSet::new_gb("test");
    assert!(set.add_line("014200C0")); // GameShark: $C000 = $42 every frame
    assert!(set.add_line("992-00F")); // Game Genie: ROM $0200 = $99
    assert!(!set.add_line("not a code"));
    gb.cheats.sets.push(set);
    gb.run_frame();
    gb.run_frame();
    assert_eq!(gb.memory.wram[0], 0x42);
    assert_eq!(gb.view8(0x200), 0x99);
    // Disabling the set restores the ROM byte.
    gb.cheats.sets[0].enabled = false;
    gb.run_frame();
    gb.run_frame();
    assert_eq!(gb.view8(0x200), 0x11);
}

fn gba_host() -> Option<Gba> {
    let rom = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mgba-master/cinema/gba/irq/keyirq/test.gba")).ok()?;
    let mut gba = Gba::new();
    gba.load_rom(rom);
    gba.reset();
    Some(gba)
}

#[test]
fn gba_vba_codebreaker_and_encrypted_gameshark() {
    let Some(mut gba) = gba_host() else { return };
    // GameShark v1: assign 4 bytes at $02000008, encrypted with the default seeds
    let (mut a, mut b) = (0x2200_0008u32, 0xDEAD_BEEFu32);
    encrypt_gameshark(&mut a, &mut b, &GAMESHARK_SEEDS);
    let mut gs = CheatSet::new_gba("gs");
    assert!(gs.add_line(&format!("{a:08X} {b:08X}")));
    let mut other = CheatSet::new_gba("vba+cb");
    assert!(other.add_line("02000000:BEEF")); // VBA: 16-bit write
    assert!(other.add_line("32000004 0055")); // CodeBreaker: 8-bit write
    gba.cheats.sets.push(gs);
    gba.cheats.sets.push(other);
    for _ in 0..3 {
        gba.run_frame();
    }
    let w = &gba.memory.wram;
    assert_eq!(u16::from_le_bytes([w[0], w[1]]), 0xBEEF);
    assert_eq!(w[4], 0x55);
    assert_eq!(u32::from_le_bytes([w[8], w[9], w[10], w[11]]), 0xDEADBEEF);
}

#[test]
fn cheats_file_round_trips() {
    let text = "# Infinite health\n014200C0\n!disabled\n# Off for now\n992-00F\n";
    let mut device = CheatDevice::new();
    device.parse_file(text, CheatSet::new_gb);
    assert_eq!(device.sets.len(), 2);
    assert_eq!(device.sets[0].name, "Infinite health");
    assert!(device.sets[0].enabled);
    assert!(!device.sets[1].enabled);
    assert_eq!(device.save_file(), text);
}
