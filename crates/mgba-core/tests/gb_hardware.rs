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
