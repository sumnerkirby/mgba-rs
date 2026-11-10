//! Save states in mGBA's own format (GBA). The exporter and importer were
//! checked byte for byte against mGBA built from `mgba-master/`; these tests
//! keep the round trip, the PNG container and the safety checks working.

use std::path::Path;

use mgba_core::state_container;
use mgba_core::Gba;

fn cinema_rom() -> Option<Vec<u8>> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mgba-master/cinema/gba/irq/keyirq/test.gba")).ok()
}

fn gba(rom: Vec<u8>) -> Box<Gba> {
    let mut g = Box::new(Gba::new());
    assert!(g.load_rom(rom));
    g.skip_bios_on_reset = true;
    g.reset();
    g
}

#[test]
fn round_trip_continues_identically() {
    let Some(rom) = cinema_rom() else { return };
    let mut a = gba(rom.clone());
    for _ in 0..30 {
        a.run_frame();
    }
    let state = a.save_state_mgba();
    assert_eq!(&state[..4], &0x0100_000Bu32.to_le_bytes(), "mGBA version magic");
    let mut b = gba(rom);
    b.load_state_mgba(&state).unwrap();
    // Exporting right after loading gives the same state back.
    assert_eq!(b.save_state_mgba(), state);
    for _ in 0..30 {
        a.run_frame();
        b.run_frame();
    }
    assert_eq!(a.framebuffer(), b.framebuffer());
    assert_eq!(a.save_state_mgba(), b.save_state_mgba());
}

/// Wrap a raw state in a PNG the way mGBA's frontends save them: the state in
/// a zlib-compressed `gbAs` chunk, extdata in `gbAx` chunks.
fn png_container(raw_state: &[u8], extdata: &[(u32, Vec<u8>)]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&[0, 0, 0, 0]); // CRC isn't checked when loading
    }
    let mut png = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];
    chunk(&mut png, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
    chunk(&mut png, b"gbAs", &state_container::deflate(raw_state));
    for (tag, data) in extdata {
        let mut body = tag.to_le_bytes().to_vec();
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(&state_container::deflate(data));
        chunk(&mut png, b"gbAx", &body);
    }
    chunk(&mut png, b"IEND", &[]);
    png
}

#[test]
fn loads_png_states() {
    let Some(rom) = cinema_rom() else { return };
    let mut a = gba(rom.clone());
    for _ in 0..10 {
        a.run_frame();
    }
    let raw = a.serialize_mgba();
    let mut b = gba(rom);
    b.load_state_mgba(&png_container(&raw, &[(state_container::EXTDATA_RTC, vec![0; 16])])).unwrap();
    assert_eq!(b.serialize_mgba(), raw);
}

#[test]
fn rejects_other_games_and_garbage() {
    let mut rom = vec![0u8; 0x1000];
    rom[0xA0..0xAC].copy_from_slice(b"FIRSTGAME\0\0\0");
    rom[0xAC..0xB0].copy_from_slice(b"AAAA");
    let mut a = gba(rom.clone());
    a.run_frame();
    let state = a.save_state_mgba();
    rom[0xAC..0xB0].copy_from_slice(b"BBBB");
    let mut b = gba(rom);
    assert!(b.load_state_mgba(&state).unwrap_err().contains("different game"));
    assert!(b.load_state_mgba(b"not a state").is_err());
    // This port's own format isn't mistaken for mGBA's.
    assert!(b.load_state_mgba(&b.save_state()).is_err());
}
