//! Link cable: two (or more) cores in one process exchanging data through the
//! lockstep drivers ported from mGBA (src/gb/sio/lockstep.c,
//! src/gba/sio/lockstep.c). The ROMs are assembled here from a few
//! hand-written instructions.

use mgba_core::gb::GbSioLockstep;
use mgba_core::gba::{GbaSioLockstepCoordinator, LogLevel};
use mgba_core::lockstep::run_linked_frame;
use mgba_core::{Gb, Gba};

// ----- Game Boy -----

/// Build a 32 KiB ROM: entry point jumps to `code` placed at 0x150.
fn gb_rom(code: &[u8]) -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];
    rom[0x100..0x104].copy_from_slice(&[0x00, 0xC3, 0x50, 0x01]); // nop; jp $0150
    rom[0x134..0x13C].copy_from_slice(b"LINKTEST");
    rom[0x150..0x150 + code.len()].copy_from_slice(code);
    rom
}

/// Put `sb` in SB, write `sc` to SC, wait for the transfer to end and copy SB
/// to $C000. With `delay`, idle a while first so the partner is listening.
fn gb_serial_program(sb: u8, sc: u8, delay: bool) -> Vec<u8> {
    let mut code = vec![];
    if delay {
        // ld c, $20; outer: ld b, 0; inner: dec b; jr nz, inner; dec c; jr nz, outer
        code.extend_from_slice(&[0x0E, 0x20, 0x06, 0x00, 0x05, 0x20, 0xFD, 0x0D, 0x20, 0xF8]);
    }
    code.extend_from_slice(&[0x3E, sb, 0xE0, 0x01]); // ld a, sb; ldh (SB), a
    code.extend_from_slice(&[0x3E, sc, 0xE0, 0x02]); // ld a, sc; ldh (SC), a
    code.extend_from_slice(&[0xF0, 0x02, 0xCB, 0x7F, 0x20, 0xFA]); // wait: ldh a, (SC); bit 7, a; jr nz, wait
    code.extend_from_slice(&[0xF0, 0x01, 0xEA, 0x00, 0xC0]); // ldh a, (SB); ld ($C000), a
    code.extend_from_slice(&[0x3E, 0x01, 0xEA, 0x01, 0xC0]); // ld a, 1; ld ($C001), a
    code.extend_from_slice(&[0x18, 0xFE]); // jr -2
    code
}

fn gb_with(code: &[u8]) -> Box<Gb> {
    let mut gb = Box::new(Gb::new());
    assert!(gb.load_rom(gb_rom(code)));
    gb.reset();
    gb
}

fn run_gb_link(a: &mut Gb, b: &mut Gb, frames: usize) {
    for _ in 0..frames {
        assert!(run_linked_frame(&mut [a, b]), "GB link deadlocked");
    }
}

#[test]
fn gb_serial_exchanges_a_byte() {
    let link = GbSioLockstep::new();
    // Player 1 drives the clock; player 2 listens on the external clock.
    let mut p1 = gb_with(&gb_serial_program(0x42, 0x81, true));
    let mut p2 = gb_with(&gb_serial_program(0x99, 0x80, false));
    assert!(p1.attach_link(&link));
    assert!(p2.attach_link(&link));
    assert!(!Gb::new().attach_link(&link), "only two Game Boys fit on a cable");

    run_gb_link(&mut p1, &mut p2, 30);

    assert_eq!(p1.view8(0xC001), 1, "player 1's transfer never finished");
    assert_eq!(p2.view8(0xC001), 1, "player 2's transfer never finished");
    assert_eq!(p1.view8(0xC000), 0x99, "player 1 should receive player 2's byte");
    assert_eq!(p2.view8(0xC000), 0x42, "player 2 should receive player 1's byte");
    // The serial interrupt was raised on both sides.
    assert_ne!(p1.memory.io[0x0F] & 0x08, 0);
    assert_ne!(p2.memory.io[0x0F] & 0x08, 0);
}

#[test]
fn gb_serial_clock_owner_can_be_either_player() {
    // The second Game Boy attached starts the transfer, so it takes over as
    // the clock owner (node 0), as in mGBA.
    let link = GbSioLockstep::new();
    let mut p1 = gb_with(&gb_serial_program(0x5A, 0x80, false));
    let mut p2 = gb_with(&gb_serial_program(0xC3, 0x81, true));
    assert!(p1.attach_link(&link));
    assert!(p2.attach_link(&link));
    assert_eq!(p1.link().unwrap().id(), 0);

    run_gb_link(&mut p1, &mut p2, 30);

    assert_eq!(p2.link().unwrap().id(), 0, "the player that started the transfer owns the clock");
    assert_eq!(p1.view8(0xC000), 0xC3);
    assert_eq!(p2.view8(0xC000), 0x5A);
}

#[test]
fn gb_serial_without_partner_reads_ff() {
    // A lone node on the cable behaves like an unconnected port.
    let link = GbSioLockstep::new();
    let mut p1 = gb_with(&gb_serial_program(0x42, 0x81, false));
    assert!(p1.attach_link(&link));
    for _ in 0..10 {
        p1.run_frame();
    }
    assert_eq!(p1.view8(0xC000), 0xFF);
}

#[test]
fn gb_link_survives_reset_and_detach() {
    let link = GbSioLockstep::new();
    let mut p1 = gb_with(&gb_serial_program(0x11, 0x81, true));
    let mut p2 = gb_with(&gb_serial_program(0x22, 0x80, false));
    assert!(p1.attach_link(&link));
    assert!(p2.attach_link(&link));
    run_gb_link(&mut p1, &mut p2, 5);
    p1.reset();
    p2.reset();
    run_gb_link(&mut p1, &mut p2, 30);
    assert_eq!(p1.view8(0xC000), 0x22);
    assert_eq!(p2.view8(0xC000), 0x11);

    p2.detach_link();
    assert_eq!(link.lock().unwrap().attached(), 1);
    for _ in 0..5 {
        p1.run_frame();
        p2.run_frame();
    }
}

// ----- Game Boy Advance -----

/// A tiny ARM assembler for the test programs.
#[derive(Default)]
struct Arm {
    code: Vec<u32>,
}

const AL: u32 = 0xE;
const EQ: u32 = 0x0;
const NE: u32 = 0x1;

impl Arm {
    fn here(&self) -> usize {
        self.code.len()
    }
    /// Encode `imm` as an ARM rotated immediate.
    fn imm(imm: u32) -> u32 {
        for rot in 0..16 {
            let v = imm.rotate_left(rot * 2);
            if v < 0x100 {
                return (rot << 8) | v;
            }
        }
        panic!("{imm:#x} is not an ARM immediate");
    }
    fn mov(&mut self, rd: u32, imm: u32) {
        self.code.push(0xE3A0_0000 | (rd << 12) | Self::imm(imm));
    }
    fn orr(&mut self, rd: u32, rn: u32, imm: u32) {
        self.code.push(0xE380_0000 | (rn << 16) | (rd << 12) | Self::imm(imm));
    }
    /// Load a 32-bit constant with mov/orr.
    fn li(&mut self, rd: u32, value: u32) {
        self.mov(rd, value & 0xFF);
        for shift in [8, 16, 24] {
            let byte = (value >> shift) & 0xFF;
            if byte != 0 {
                self.orr(rd, rd, byte << shift);
            }
        }
    }
    fn strh(&mut self, rd: u32, rn: u32, off: u32) {
        assert!(off < 0x100);
        self.code.push(0xE1C0_00B0 | (rn << 16) | (rd << 12) | ((off >> 4) << 8) | (off & 0xF));
    }
    fn ldrh(&mut self, rd: u32, rn: u32, off: u32) {
        assert!(off < 0x100);
        self.code.push(0xE1D0_00B0 | (rn << 16) | (rd << 12) | ((off >> 4) << 8) | (off & 0xF));
    }
    fn str(&mut self, rd: u32, rn: u32, off: u32) {
        self.code.push(0xE580_0000 | (rn << 16) | (rd << 12) | off);
    }
    fn tst(&mut self, rn: u32, imm: u32) {
        self.code.push(0xE310_0000 | (rn << 16) | Self::imm(imm));
    }
    fn b(&mut self, cond: u32, target: usize) {
        let offset = (target as i32 - (self.here() as i32 + 2)) as u32 & 0x00FF_FFFF;
        self.code.push((cond << 28) | 0x0A00_0000 | offset);
    }
    /// Spin until `(halfword at [rn, off] & mask)` is nonzero (`cond` = EQ) or zero (`NE`).
    fn wait(&mut self, rn: u32, off: u32, mask: u32, cond: u32) {
        let top = self.here();
        self.ldrh(1, rn, off);
        self.tst(1, mask);
        self.b(cond, top);
    }
    fn delay(&mut self, count: u32) {
        self.li(2, count);
        let top = self.here();
        self.code.push(0xE252_2001); // subs r2, r2, #1
        self.b(NE, top);
    }
    fn halt_forever(&mut self) {
        let top = self.here();
        self.b(AL, top);
    }
}

// Halfword loads and stores take an 8-bit offset, so registers are addressed
// from 0x04000100.
const REG_BASE: u32 = 0x0400_0100;
const SIODATA32_LO: u32 = 0x20;
const SIOMULTI0: u32 = 0x20;
const SIOCNT: u32 = 0x28;
const SIOMLT_SEND: u32 = 0x2A;
const SIODATA8: u32 = 0x2A;
const RCNT: u32 = 0x34;
const EWRAM: u32 = 0x0200_0000;

fn gba_rom(prog: &Arm) -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];
    // b 0x080000C0
    rom[0..4].copy_from_slice(&0xEA00_002Eu32.to_le_bytes());
    rom[0xA0..0xAC].copy_from_slice(b"LINKTEST\0\0\0\0");
    rom[0xAC..0xB0].copy_from_slice(b"LNKT");
    rom[0xB2] = 0x96;
    for (i, w) in prog.code.iter().enumerate() {
        rom[0xC0 + i * 4..0xC4 + i * 4].copy_from_slice(&w.to_le_bytes());
    }
    rom
}

fn gba_with(prog: &Arm) -> Box<Gba> {
    let mut gba = Box::new(Gba::new());
    gba.log_enabled = true;
    assert!(gba.load_rom(gba_rom(prog)));
    gba.skip_bios_on_reset = true;
    gba.reset();
    gba
}

fn ewram32(gba: &Gba, offset: usize) -> u32 {
    u32::from_le_bytes(gba.memory.wram[offset..offset + 4].try_into().unwrap())
}

fn run_gba_link(players: &mut [Box<Gba>], frames: usize) {
    for _ in 0..frames {
        let mut refs: Vec<&mut Gba> = players.iter_mut().map(|b| b.as_mut()).collect();
        assert!(run_linked_frame(&mut refs), "GBA link deadlocked");
        for (i, p) in players.iter_mut().enumerate() {
            for (level, msg) in p.drain_log() {
                let bad = matches!(level, LogLevel::Error) || msg.contains("did not receive data");
                assert!(!bad, "player {}: {level:?} {msg}", i + 1);
            }
        }
    }
}

/// Normal mode: set SIOCNT to `mode_bits` (8- or 32-bit), load `data`, and on
/// the clock owner start the transfer; everyone waits for it to finish and
/// stores the received data and a done flag in EWRAM.
fn gba_normal_program(data: u32, normal32: bool, primary: bool) -> Arm {
    let mut a = Arm::default();
    a.li(0, REG_BASE);
    a.mov(1, 0);
    a.strh(1, 0, RCNT); // RCNT = 0: SIOCNT picks the mode
    let mode = if normal32 { 0x1000 } else { 0 };
    let clock = if primary { 1 } else { 0 }; // internal clock on the clock owner
    a.li(1, mode | clock);
    a.strh(1, 0, SIOCNT);
    if normal32 {
        a.li(1, data & 0xFFFF);
        a.strh(1, 0, SIODATA32_LO);
        a.li(1, data >> 16);
        a.strh(1, 0, SIODATA32_LO + 2);
    } else {
        a.li(1, data & 0xFF);
        a.strh(1, 0, SIODATA8);
    }
    if primary {
        a.delay(0x4000);
        a.li(1, mode | clock | 0x80);
        a.strh(1, 0, SIOCNT);
    } else {
        a.wait(0, SIOCNT, 0x80, EQ); // until the transfer starts
    }
    a.wait(0, SIOCNT, 0x80, NE); // until it ends
    a.li(3, EWRAM);
    if normal32 {
        a.ldrh(1, 0, SIODATA32_LO);
        a.ldrh(2, 0, SIODATA32_LO + 2);
        a.strh(1, 3, 0);
        a.strh(2, 3, 2);
    } else {
        a.ldrh(1, 0, SIODATA8);
        a.strh(1, 3, 0);
    }
    a.mov(1, 1);
    a.str(1, 3, 4);
    a.halt_forever();
    a
}

#[test]
fn gba_normal8_sends_a_byte_down_the_chain() {
    let link = GbaSioLockstepCoordinator::new();
    let mut players =
        vec![gba_with(&gba_normal_program(0x42, false, true)), gba_with(&gba_normal_program(0x99, false, false))];
    for (i, p) in players.iter_mut().enumerate() {
        p.attach_link(&link, Some(i as i32));
    }
    assert_eq!(link.lock().unwrap().attached(), 2);
    assert_eq!(players[0].link_player_id(), 0);
    assert_eq!(players[1].link_player_id(), 1);

    run_gba_link(&mut players, 30);

    assert_eq!(ewram32(&players[0], 4), 1, "player 1's transfer never finished");
    assert_eq!(ewram32(&players[1], 4), 1, "player 2's transfer never finished");
    // mGBA's lockstep driver passes normal-mode data from each player to the
    // next one; the clock owner's input isn't connected and reads 0xFF.
    assert_eq!(ewram32(&players[1], 0) & 0xFF, 0x42);
    assert_eq!(ewram32(&players[0], 0) & 0xFF, 0xFF);
}

#[test]
fn gba_normal32_sends_a_word_down_the_chain() {
    let link = GbaSioLockstepCoordinator::new();
    let mut players = vec![
        gba_with(&gba_normal_program(0xDEAD_BEEF, true, true)),
        gba_with(&gba_normal_program(0x1234_5678, true, false)),
    ];
    for (i, p) in players.iter_mut().enumerate() {
        p.attach_link(&link, Some(i as i32));
    }

    run_gba_link(&mut players, 30);

    assert_eq!(ewram32(&players[0], 4), 1);
    assert_eq!(ewram32(&players[1], 4), 1);
    assert_eq!(ewram32(&players[1], 0), 0xDEAD_BEEF);
    assert_eq!(ewram32(&players[0], 0), 0xFFFF_FFFF);
}

/// Multiplayer mode at 115200 bps: everyone puts `data` in SIOMLT_SEND; the
/// parent waits until all are ready and starts the transfer; everyone stores
/// SIOMULTI0-3, SIOCNT and a done flag in EWRAM.
fn gba_multi_program(data: u16, parent: bool) -> Arm {
    let mut a = Arm::default();
    a.li(0, REG_BASE);
    a.mov(1, 0);
    a.strh(1, 0, RCNT);
    a.li(1, 0x2003); // multiplayer, 115200 bps
    a.strh(1, 0, SIOCNT);
    a.li(1, data as u32);
    a.strh(1, 0, SIOMLT_SEND);
    if parent {
        a.delay(0x4000);
        a.wait(0, SIOCNT, 0x08, EQ); // until every GBA is in multiplayer mode
        a.li(1, 0x2083);
        a.strh(1, 0, SIOCNT);
    } else {
        a.wait(0, SIOCNT, 0x80, EQ);
    }
    a.wait(0, SIOCNT, 0x80, NE);
    a.li(3, EWRAM);
    for i in 0..4 {
        a.ldrh(1, 0, SIOMULTI0 + i * 2);
        a.strh(1, 3, i * 2);
    }
    a.ldrh(1, 0, SIOCNT);
    a.strh(1, 3, 8);
    a.mov(1, 1);
    a.str(1, 3, 12);
    a.halt_forever();
    a
}

fn check_multi(n: usize) {
    let link = GbaSioLockstepCoordinator::new();
    let data = [0x1111u16, 0x2222, 0x3333, 0x4444];
    let mut players: Vec<Box<Gba>> = (0..n).map(|i| gba_with(&gba_multi_program(data[i], i == 0))).collect();
    for (i, p) in players.iter_mut().enumerate() {
        p.attach_link(&link, Some(i as i32));
    }
    for (i, p) in players.iter_mut().enumerate() {
        assert_eq!(p.link_player_id(), i as i32);
    }

    run_gba_link(&mut players, 30);

    let mut expected = [0xFFFFu16; 4];
    expected[..n].copy_from_slice(&data[..n]);
    for (i, p) in players.iter().enumerate() {
        assert_eq!(ewram32(p, 12), 1, "player {}'s transfer never finished", i + 1);
        let got: Vec<u16> = (0..4).map(|j| u16::from_le_bytes([p.memory.wram[j * 2], p.memory.wram[j * 2 + 1]])).collect();
        assert_eq!(got, expected, "SIOMULTI on player {}", i + 1);
        let siocnt = u16::from_le_bytes([p.memory.wram[8], p.memory.wram[9]]);
        assert_eq!((siocnt >> 4) & 3, i as u16, "SIOCNT id on player {}", i + 1);
        assert_eq!(siocnt & 4 != 0, i != 0, "SIOCNT slave bit on player {}", i + 1);
        assert_ne!(siocnt & 8, 0, "SIOCNT ready bit on player {}", i + 1);
    }
}

#[test]
fn gba_multiplayer_two_players() {
    check_multi(2);
}

#[test]
fn gba_multiplayer_four_players() {
    check_multi(4);
}

#[test]
fn gba_link_survives_reset_and_detach() {
    let link = GbaSioLockstepCoordinator::new();
    let mut players = vec![gba_with(&gba_multi_program(0xAAAA, true)), gba_with(&gba_multi_program(0xBBBB, false))];
    for (i, p) in players.iter_mut().enumerate() {
        p.attach_link(&link, Some(i as i32));
    }
    run_gba_link(&mut players, 3);
    for p in players.iter_mut() {
        p.reset();
    }
    run_gba_link(&mut players, 30);
    for p in &players {
        assert_eq!(ewram32(p, 12), 1);
        assert_eq!(&p.memory.wram[0..4], &[0xAA, 0xAA, 0xBB, 0xBB]);
    }
    players[1].detach_link();
    assert_eq!(link.lock().unwrap().attached(), 1);
    // The remaining player keeps running on its own.
    let mut refs: Vec<&mut Gba> = vec![players[0].as_mut()];
    for _ in 0..5 {
        assert!(run_linked_frame(&mut refs));
    }
}

