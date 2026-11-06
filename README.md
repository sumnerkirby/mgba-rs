# mGBA-rs

A Rust rewrite of [mGBA](https://mgba.io)'s Game Boy Advance emulator, with a desktop GUI.

This is a faithful port: the CPU, memory map, scheduler, DMA, timers, audio and the software renderer follow mGBA's C code (`mgba-master/src`) function by function, so timing and accuracy track the original. It's checked against mGBA's own test suite (see [Testing](#testing)).

## Layout

| Path | What it is |
| --- | --- |
| `crates/mgba-core` | The emulator library (no system dependencies) |
| `crates/mgba-core/src/arm` | ARM7TDMI interpreter (`src/arm/*.c`) |
| `crates/mgba-core/src/gba` | GBA system: memory, I/O, DMA, timers, video timing, audio, HLE BIOS, save memory, RTC, overrides (`src/gba/*.c`) |
| `crates/mgba-core/src/gba/renderer` | Software renderer (`src/gba/renderers/video-software.c` and friends) |
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

You can also start it without arguments and use **File → Open ROM** or drag a ROM onto the window.

### Default controls

| GBA | Key |
| --- | --- |
| D-pad | Arrow keys |
| A / B | X / Z |
| L / R | A / S |
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

- Battery saves are read from and written to `<rom name>.sav` next to the ROM, as mGBA does, so existing mGBA saves work.
- Save states go to `<rom name>.ss1` … `.ss9`. They use this port's own format and are not interchangeable with mGBA's.
- Screenshots are written next to the ROM as PNG.
- A real GBA BIOS is optional (**Emulation → Choose BIOS file**). Without one, the built-in HLE BIOS from mGBA is used.

## What's ported

- ARM and Thumb instruction sets, including mGBA's cycle counting, prefetch-buffer stall model and open-bus behavior
- Memory map with waitstates, VRAM access stalls, and EEPROM/Flash/SRAM save detection
- Event scheduler, IRQs, halt/stop, DMA (including sound FIFO and video-capture timing), timers with cascade
- PPU timing and the full software renderer: text, affine and bitmap modes, sprites (affine, mosaic, OBJ window, per-line cycle limit), windows, alpha/brightness blending, mosaic, stereo mode
- Audio: the four PSG channels and both Direct Sound FIFOs with mGBA's mixing and SOUNDBIAS resolution handling
- High-level BIOS emulation (division, sqrt, arctan, affine set-up, LZ77/Huffman/RLE/diff decompression, CpuSet, register RAM reset and more)
- Cartridge GPIO with the real-time clock, and mGBA's per-game override table (save types, RTC, idle loops)

## Not ported yet

- The Game Boy / Game Boy Color core (planned as the next milestone)
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

`trace` and `steptrace` print CPU state per frame or per instruction for debugging.

## License

mGBA is © Jeffrey Pfau and contributors, licensed under the Mozilla Public License 2.0. This port is a derivative work and is distributed under the same license; see `LICENSE`.
