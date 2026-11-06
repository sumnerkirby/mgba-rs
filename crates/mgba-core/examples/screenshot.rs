//! Run a ROM headlessly for N frames and save the last frame as a PNG.
//! Usage: screenshot <rom> <frames> <out.png> [--bios FILE] [--skip-bios]
use mgba_core::Gba;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom = std::fs::read(&args[1]).unwrap();
    let frames: usize = args[2].parse().unwrap();
    let mut gba = Gba::new();
    if let Some(i) = args.iter().position(|a| a == "--bios") {
        assert!(gba.load_bios(&std::fs::read(&args[i + 1]).unwrap()));
    }
    gba.skip_bios_on_reset = args.iter().any(|a| a == "--skip-bios");
    gba.load_rom(rom);
    gba.reset();
    for _ in 0..frames {
        gba.run_frame();
    }
    let fb = gba.framebuffer();
    let mut enc = png::Encoder::new(std::fs::File::create(&args[3]).unwrap(), 240, 160);
    enc.set_color(png::ColorType::Rgb);
    let data: Vec<u8> = fb.iter().flat_map(|&p| [p as u8, (p >> 8) as u8, (p >> 16) as u8]).collect();
    enc.write_header().unwrap().write_image_data(&data).unwrap();
    println!("r12={:08X} pc={:08X} crashed={}", gba.cpu.gprs[12], gba.cpu.gprs[15], gba.crashed);
}
