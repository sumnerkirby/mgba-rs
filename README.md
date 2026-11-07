# mGBA-rs

A Rust rewrite of [mGBA](https://mgba.io)'s Game Boy Advance and Game Boy / Game Boy Color emulators, with a desktop GUI.

This is a faithful port: the CPUs, memory maps, scheduler, DMA, timers, audio and the software renderers follow mGBA's C code (`mgba-master/src`) function by function, so timing and accuracy track the original. It's checked against mGBA's own test suite (see [Testing](#testing)).

## Layout

| Path | What it is |
| --- | --- |
| `crates/mgba-core` | The emulator library (no system dependencies) |
| `crates/mgba-core/src/arm` | ARM7TDMI interpreter (`src/arm/*.c`) |
| `crates/mgba-core/src/gba` | GBA system: memory, I/O, DMA, timers, video timing, audio, HLE BIOS, save memory, RTC, overrides (`src/gba/*.c`) |
| `crates/mgba-core/src/gba/renderer` | Software renderer (`src/gba/renderers/video-software.c` and friends) |
| `crates/mgba-core/src/sm83` | SM83 (Game Boy CPU) interpreter (`src/sm83/*.c`) |
| `crates/mgba-core/src/gb` | Game Boy / Color system: memory, MBCs, I/O, timer, PPU timing, software renderer, APU (`src/gb/*.c`) |
| `crates/mgba-core/src/timing.rs` | The event scheduler both systems share (`src/core/timing.c`) |
| `crates/mgba-gui` | `mgba-rs`, the desktop app (egui/eframe) |
| `mgba-master/` | The original mGBA source, kept for reference and for its test data |

## Building and running

Needs a current stable Rust toolchain.

```bash
cargo build --release
```

```bash
./target/release/mgba-rs path/to/game.gba
```

Game Boy and Game Boy Color games (`.gb`, `.gbc`) open the same way; the core is picked from the ROM header (falling back to the file extension).

You can also start it without arguments and use **File → Open ROM** or drag a ROM onto the window.

### Default controls

| GBA / GB | Key |
| --- | --- |
| D-pad | Arrow keys |
| A / B | X / Z |
| L / R (GBA only) | A / S |
| Start / Select | Enter / Backspace |

Keys can be remapped in **Settings → Controls**.

| Action | Shortcut |
| --- | --- |
| Open ROM | Ctrl+O |
| Pause / resume | Ctrl+P |
| Reset | Ctrl+R |
| Fast forward | hold Tab (or toggle in the Emulation menu) |
| Load / save state | F1–F9 / Shift+F1–F9 |
| Screenshot | F12 |
| Fullscreen | F11 |

### Files

- Battery saves are read from and written to `<rom name>.sav` next to the ROM, as mGBA does, so existing mGBA saves work. For Game Boy MBC3 carts with a clock, the RTC block mGBA appends to the save is read and written too.
- Save states go to `<rom name>.ss1` … `.ss9`. They use this port's own format and are not interchangeable with mGBA's.
- Screenshots are written next to the ROM as PNG.
- A real GBA BIOS is optional (**Emulation → Choose GBA BIOS file**). Without one, the built-in HLE BIOS from mGBA is used.
- Game Boy and Game Boy Color boot ROMs are optional too (**Emulation → Game Boy boot ROMs**). Without one the boot sequence is skipped and registers are set as the boot ROM would leave them, like mGBA. **Emulation → Game Boy model** forces a model (DMG, MGB, SGB, SGB2, CGB, AGB); otherwise it follows the cartridge header.

## What's ported

- ARM and Thumb instruction sets, including mGBA's cycle counting, prefetch-buffer stall model and open-bus behavior
- Memory map with waitstates, VRAM access stalls, and EEPROM/Flash/SRAM save detection
- Event scheduler, IRQs, halt/stop, DMA (including sound FIFO and video-capture timing), timers with cascade
- PPU timing and the full software renderer: text, affine and bitmap modes, sprites (affine, mosaic, OBJ window, per-line cycle limit), windows, alpha/brightness blending, mosaic, stereo mode
- Audio: the four PSG channels and both Direct Sound FIFOs with mGBA's mixing and SOUNDBIAS resolution handling
- High-level BIOS emulation (division, sqrt, arctan, affine set-up, LZ77/Huffman/RLE/diff decompression, CpuSet, register RAM reset and more)
- Cartridge GPIO with the real-time clock, and mGBA's per-game override table (save types, RTC, idle loops)

### Game Boy / Game Boy Color

- SM83 CPU with mGBA's micro-op (M-cycle) model, including the HALT bug, EI delay and double speed
- Memory map with OAM DMA bus conflicts, CGB HDMA/GDMA, WRAM banking and open-bus cartridge behavior
- MBC1 (incl. multicarts), MBC2, MBC3 with RTC, MBC5 (incl. rumble flag), MBC6, MBC7 (EEPROM; no tilt sensor), MMM01, HuC-1, M161
- Timer with mGBA's DIV/TIMA edge timing, serial port (no link partner)
- PPU mode timing and STAT/LYC interrupts, and the software renderer for DMG and CGB (priorities, banks, flips, window quirks, AGB color correction)
- Super Game Boy models: palette and attribute commands, VRAM transfers and multiplayer requests
- APU in DMG and CGB styles (sweep, envelopes, wave RAM quirks, LFSR noise), mixed like mGBA at 131072 Hz and averaged to 32768 Hz for output
- Model detection and selection (DMG, MGB, SGB, SGB2, CGB, AGB) and optional boot ROMs

## Not ported yet

- Game Boy: SGB borders (output is always 160x144), HuC-3, TAMA5, Pocket Camera and the unlicensed mappers, GBX files, the override table beyond the M161/MBC3 entries, link cable, tilt and rumble devices, cheats and GB video logs (`cinema/gb/window`)
- Link cable / multiplayer (serial transfers complete as if nothing is connected), e-Reader, GB Player
- Gyro, tilt, solar and rumble sensors (they're recognized, but not driven)
- Debugger, Lua scripting, cheats, ROM patches, multiboot/ELF images, idle-loop auto-detection, AGBPrint
- Rewind, gamepad input and audio/video recording in the GUI
- Audio output is currently Linux-only (ALSA, loaded at runtime so building needs no system headers; PipeWire and PulseAudio both serve it). On other platforms the GUI runs silently.

## Testing

mGBA ships a regression suite in `mgba-master/cinema`: test ROMs with expected screenshots, and recorded video logs (`.mvl`) of real games that exercise the renderer. With `mgba-master/` next to the workspace:

```bash
cargo test --release
```

This runs every GBA ROM test and every video log and requires a pixel-exact match with mGBA's baselines (currently 3 ROM tests and 23 video logs, all passing), plus a save-state determinism check.

It also runs the Game Boy ROM tests in `cinema/gb` (blargg, mooneye-gb, acid2, samesuite, ramg-mbc3): 236 tests, of which 148 match mGBA's correct baseline and the other 88 are tests mGBA itself is marked as failing (`fail=1`). For those, 87 match mGBA's recorded wrong output (`xbaseline_*.png`) pixel for pixel, and one has no reference image at all, which mGBA's runner also counts as an expected failure. No test differs from mGBA. A few headless checks cover GB sound output, battery saves, the MBC3 RTC save block and save states.

Developer tools live in `crates/mgba-core/examples`:

```bash
cargo run --release -p mgba-core --example cinema -- mgba-master/cinema/gba/irq/keyirq --dump /tmp/frames
```

```bash
cargo run --release -p mgba-core --example mvl -- mgba-master/cinema/gba/bg/lady-sia
```

```bash
cargo run --release -p mgba-core --example bench -- game.gba 3000
```

```bash
cargo run --release -p mgba-core --example gb -- game.gb 600 /tmp/frame.png
```

`trace` and `steptrace` print CPU state per frame or per instruction for debugging.

## License

mGBA is © Jeffrey Pfau and contributors, licensed under the Mozilla Public License 2.0. This port is a derivative work and is distributed under the same license; see `LICENSE`.
