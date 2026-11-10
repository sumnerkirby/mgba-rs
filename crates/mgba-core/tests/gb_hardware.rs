//! Headless checks of Game Boy features the cinema suite doesn't cover:
//! sound output, battery-backed SRAM, the MBC3 real-time clock and save states.
//! The ROMs are assembled here from a few hand-written instructions.

use mgba_core::gb::mbc;
use mgba_core::Gb;

/// Build a 32 KiB ROM: entry point jumps to `code` placed at 0x150.
fn make_rom(cart_type: u8, ram_size: u8, cgb: u8, code: &[u8]) -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];
    rom[0x100..0x104].copy_from_slice(&[0x00, 0xC3, 0x50, 0x01]); // nop; jp $0150
    rom[0x134..0x13C].copy_from_slice(b"TESTROM\0");
    rom[0x143] = cgb;
    rom[0x147] = cart_type;
    rom[0x148] = 0; // 32 KiB
    rom[0x149] = ram_size;
    rom[0x150..0x150 + code.len()].copy_from_slice(code);
    rom
}

/// `ld a, n; ldh (reg), a`
fn ldh(reg: u8, value: u8) -> [u8; 4] {
    [0x3E, value, 0xE0, reg]
}

#[test]
fn square_channel_produces_sound() {
    let mut code = vec![];
    for (reg, v) in [(0x26, 0x80), (0x24, 0x77), (0x25, 0xFF), (0x11, 0x80), (0x12, 0xF0), (0x13, 0x00), (0x14, 0x87)] {
        code.extend_from_slice(&ldh(reg, v));
    }
    code.extend_from_slice(&[0x18, 0xFE]); // jr -2
    let mut gb = Gb::new();
    gb.load_rom(make_rom(0x00, 0, 0x00, &code));
    gb.reset();
    let mut audio = vec![];
    for _ in 0..30 {
        gb.run_frame();
        gb.drain_audio(&mut audio);
    }
    // 30 frames at ~59.7 Hz is about half a second of 32768 Hz audio.
    let expected = (30.0 / 59.7275 * gb.audio_sample_rate() as f64) as usize;
    assert!(audio.len() + 64 >= expected && audio.len() <= expected + 64, "got {} samples, expected ~{expected}", audio.len());
    let tail = &audio[audio.len() / 2..];
    let (min, max) = tail.iter().fold((i16::MAX, i16::MIN), |(lo, hi), s| (lo.min(s[0]), hi.max(s[0])));
    assert!(max as i32 - min as i32 > 1000, "square wave too quiet: {min}..{max}");
    // Silence the channel and the output settles.
    let mut silent = Gb::new();
    silent.load_rom(make_rom(0x00, 0, 0x00, &[0x18, 0xFE]));
    silent.reset();
    let mut quiet = vec![];
    for _ in 0..30 {
        silent.run_frame();
        silent.drain_audio(&mut quiet);
    }
    let tail = &quiet[quiet.len() / 2..];
    let (min, max) = tail.iter().fold((i16::MAX, i16::MIN), |(lo, hi), s| (lo.min(s[0]), hi.max(s[0])));
    assert!((max as i32 - min as i32) < 200, "silent ROM isn't quiet: {min}..{max}");
}

#[test]
fn battery_sram_round_trips() {
    // MBC1+RAM+BATTERY, 8 KiB RAM: enable RAM, write $42 to $A000 and $99 to $BFFF.
    let code = [
        0x3E, 0x0A, 0xEA, 0x00, 0x00, // ld a,$0A; ld ($0000),a
        0x3E, 0x42, 0xEA, 0x00, 0xA0, // ld a,$42; ld ($A000),a
        0x3E, 0x99, 0xEA, 0xFF, 0xBF, // ld a,$99; ld ($BFFF),a
        0x18, 0xFE,
    ];
    let rom = make_rom(0x03, 0x02, 0x00, &code);
    let mut gb = Gb::new();
    gb.load_rom(rom.clone());
    gb.reset();
    assert_eq!(gb.memory.mbc_type, mbc::MBC1);
    let mut flushed = false;
    for _ in 0..40 {
        gb.run_frame();
        flushed |= std::mem::take(&mut gb.savedata_updated);
    }
    assert!(flushed, "save write never settled");
    let save = gb.savedata().expect("cart has battery RAM");
    assert_eq!(save.len(), 0x2000);
    assert_eq!((save[0], save[0x1FFF]), (0x42, 0x99));

    // A fresh boot with the save loaded sees the data.
    let mut gb2 = Gb::new();
    gb2.load_rom(make_rom(0x03, 0x02, 0x00, &[0x3E, 0x0A, 0xEA, 0x00, 0x00, 0xFA, 0xFF, 0xBF, 0x18, 0xFE]));
    gb2.load_save(&save);
    gb2.reset();
    for _ in 0..2 {
        gb2.run_frame();
    }
    assert_eq!(gb2.cpu.a, 0x99);
}

#[test]
fn mbc3_rtc_save_has_mgba_suffix() {
    let rom = make_rom(0x10, 0x03, 0x00, &[0x18, 0xFE]); // MBC3+TIMER+RAM+BATTERY, 32 KiB RAM
    let mut gb = Gb::new();
    gb.rtc_fake_epoch_ms = Some(1_200_000_000);
    gb.load_rom(rom);
    gb.reset();
    assert_eq!(gb.memory.mbc_type, mbc::MBC3_RTC);
    gb.run_frame();
    let save = gb.savedata().unwrap();
    // SRAM followed by mGBA's 48-byte struct GBMBCRTCSaveBuffer
    assert_eq!(save.len(), 0x8000 + 48);
    let unix = u64::from_le_bytes(save[0x8000 + 40..].try_into().unwrap());
    assert_eq!(unix, 1_200_000);
    // Pretend the save was written earlier, with the clock's seconds register at 7.
    let mut save = save;
    save[0x8000 + 40..].copy_from_slice(&1_000_000u64.to_le_bytes());
    save[0x8000 + 20] = 7;
    let mut gb2 = Gb::new();
    gb2.load_rom(make_rom(0x10, 0x03, 0x00, &[0x18, 0xFE]));
    gb2.rtc_fake_epoch_ms = Some(1_200_000_000);
    gb2.load_save(&save);
    assert_eq!(gb2.memory.rtc_last_latch, 1_000_000);
    // The RTC block survives a reset, as mGBA re-reads it from the save file.
    gb2.reset();
    assert_eq!(gb2.memory.rtc_last_latch, 1_000_000);
    assert_eq!(gb2.memory.rtc_regs[0], 7);
}

#[test]
fn cgb_rom_runs_in_color_mode() {
    let mut gb = Gb::new();
    gb.load_rom(make_rom(0x00, 0, 0x80, &[0x18, 0xFE]));
    gb.reset();
    assert_eq!(gb.model, mgba_core::gb::model::CGB);
    assert_eq!(gb.cpu.a, 0x11, "CGB boot leaves A=$11");
    let mut dmg = Gb::new();
    dmg.load_rom(make_rom(0x00, 0, 0x00, &[0x18, 0xFE]));
    dmg.reset();
    assert_eq!(dmg.model, mgba_core::gb::model::DMG);
    assert_eq!(dmg.cpu.a, 0x01);
}

/// A ROM of `banks` 16 KiB banks whose first byte of each bank holds its number.
fn banked_rom(cart_type: u8, ram_size: u8, banks: usize) -> Vec<u8> {
    let mut rom = make_rom(cart_type, ram_size, 0x00, &[0x18, 0xFE]);
    rom.resize(banks * 0x4000, 0);
    for b in 1..banks {
        rom[b * 0x4000] = b as u8;
    }
    rom[0x148] = (banks / 2).trailing_zeros() as u8;
    rom
}

#[test]
fn wisdom_tree_switches_32k_banks() {
    let mut rom = banked_rom(0x00, 0, 8);
    // _isWisdomTree: blank title/header fields, checksum $E7 and the name in ROM
    for b in &mut rom[0x134..0x14C] {
        *b = 0;
    }
    rom[0x14D] = 0xE7;
    rom[0x400..0x40B].copy_from_slice(b"WISDOM TREE");
    rom[0x0000] = 0xAA;
    let mut gb = Gb::new();
    gb.load_rom(rom);
    gb.reset();
    assert_eq!(gb.memory.mbc_type, mbc::UNL_WISDOM_TREE);
    gb.store8(0x0001, 0); // address bits select 32 KiB bank 1 -> banks 2 and 3
    assert_eq!(gb.load8(0x0000), 2);
    assert_eq!(gb.load8(0x4000), 3);
}

#[test]
fn huc3_clock_counts_minutes_and_saves() {
    let mut gb = Gb::new();
    gb.rtc_fake_epoch_ms = Some(0);
    gb.load_rom(banked_rom(0xFE, 0x02, 4));
    gb.reset();
    assert_eq!(gb.memory.mbc_type, mbc::HUC3);
    // Pretend the clock was last latched 61 minutes before boot.
    gb.memory.rtc_last_latch = -61 * 60;
    let cmd = |gb: &mut Gb, v: u8| {
        gb.store8(0x0000, 0x0B); // mode IN
        gb.store8(0xA000, v);
        gb.store8(0x0000, 0x0D); // mode COMMIT
        gb.store8(0xA000, 0);
    };
    cmd(&mut gb, 0x60); // latch: copy the clock to registers 0..6
    // read registers 0..2 (minutes, low nibble first) with auto-increment
    cmd(&mut gb, 0x40);
    cmd(&mut gb, 0x50);
    let mut minutes = 0u32;
    for i in 0..3 {
        cmd(&mut gb, 0x10);
        gb.store8(0x0000, 0x0C); // mode OUT
        minutes |= ((gb.load8(0xA000) & 0xF) as u32) << (4 * i);
    }
    assert_eq!(minutes, 61);
    let save = gb.savedata().unwrap();
    assert_eq!(save.len(), 0x2000 + 0x88, "HuC-3 saves end with mGBA's 136-byte clock block");
}

#[test]
fn tama5_ram_round_trips() {
    let mut gb = Gb::new();
    gb.load_rom(banked_rom(0xFD, 0x00, 4));
    gb.reset();
    assert_eq!(gb.memory.mbc_type, mbc::TAMA5);
    let reg = |gb: &mut Gb, r: u8, v: u8| {
        gb.store8(0xA001, r);
        gb.store8(0xA000, v);
    };
    // write $5A to RAM address $13
    reg(&mut gb, 4, 0xA); // WRITE_LO
    reg(&mut gb, 5, 0x5); // WRITE_HI
    reg(&mut gb, 6, 0x1); // ADDR_HI: RAM write, address bit 4
    reg(&mut gb, 7, 0x3); // ADDR_LO: commit
    assert_eq!(gb.memory.sram[0x13], 0x5A);
    // read it back
    reg(&mut gb, 6, 0x3); // ADDR_HI: RAM read, address bit 4
    reg(&mut gb, 7, 0x3);
    gb.store8(0xA001, 0xC); // READ_LO
    let lo = gb.load8(0xA000) & 0xF;
    gb.store8(0xA001, 0xD); // READ_HI
    let hi = gb.load8(0xA000) & 0xF;
    assert_eq!((hi << 4) | lo, 0x5A);
    assert_eq!(gb.savedata().unwrap().len(), 0x20 + 40, "TAMA5 saves end with mGBA's 40-byte clock block");
}

#[test]
fn pocket_camera_captures_from_image() {
    let mut gb = Gb::new();
    gb.load_rom(banked_rom(0xFC, 0x04, 4));
    gb.reset();
    assert_eq!(gb.memory.mbc_type, mbc::POCKETCAM);
    gb.camera_image = Some(vec![0; 128 * 112]); // all black
    gb.store8(0x0000, 0x0A);
    gb.store8(0x4000, 0x10); // camera registers
    gb.store8(0xA002, 0x01); // exposure
    gb.store8(0xA003, 0x00);
    for i in 0..48 {
        gb.store8(0xA006 + i, 0x80); // dither thresholds
    }
    gb.store8(0xA000, 0x01); // capture
    // Black pixels fall below every threshold: color 3 (both bitplanes set).
    assert_eq!(&gb.memory.sram[0x100..0x104], &[0xFF, 0xFF, 0xFF, 0xFF]);
}
