# mGBA-rs

A Rust rewrite of [mGBA](https://mgba.io)'s Game Boy Advance and Game Boy / Game Boy Color emulators, with a desktop GUI.

This is a faithful port: the CPUs, memory maps, scheduler, DMA, timers, audio and the software renderers follow mGBA's C code (`mgba-master/src`) function by function, so timing and accuracy track the original. It's checked against mGBA's own test suite (see [Testing](#testing)).

## How this port was made

This port was reconstructed with AI assistance: the C sources were translated to Rust function by function by an AI coding assistant (Anthropic's Claude), under human direction, and checked against mGBA's own regression suite and, for save states, against mGBA itself. Some emulator porters and decompilation communities don't want AI-generated reconstructions mixed into their projects, so this is a separate repository rather than a fork of mGBA, and nothing here has been submitted upstream.

mGBA-rs is not affiliated with or endorsed by mGBA or its authors. Please report problems with this port here, not to the mGBA project. All credit for the emulation itself — the hardware research, the design and the code being ported — belongs to Jeffrey Pfau and mGBA's contributors.

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
| `crates/mgba-core/src/lockstep.rs` | Link-cable plumbing shared by both systems (`src/core/lockstep.c`) and a runner that steps linked cores together |
| `crates/mgba-core/src/debugger` | Debugger core, expression parser, CLI debugger and GDB stub (`src/debugger/*.c`, `src/arm/debugger`, `src/sm83/debugger`) |
| `crates/mgba-core/src/core.rs` | The tool-facing parts of mGBA's `struct mCore` (memory blocks, bus/raw access, registers, stepping) |
| `crates/mgba-core/src/mem_search.rs` | Memory search (`src/core/mem-search.c`) |
| `crates/mgba-core/src/gba/mgba_state.rs`, `state_container.rs` | mGBA's GBA save-state format and container (`src/gba/serialize.c`, `src/core/serialize.c`) |
| `crates/mgba-script` | Lua scripting with mGBA's script API (mlua with a vendored Lua 5.4) |
| `crates/mgba-gui` | `mgba-rs`, the desktop app (egui/eframe) |
| `mgba-master/` | The original mGBA source, used for reference and test data (not included; see [Testing](#testing)) |

## Building and running

Needs a current stable Rust toolchain and a C compiler (Lua 5.4 is built from source for scripting).

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

Keys can be remapped in **Settings → Controls**. Game controllers work too (Linux, read directly from `/dev/input`, no extra libraries): by default the bottom face button is A, the right one is B, the shoulders are L/R and both the d-pad and left stick steer, as in mGBA. Controller buttons can be rebound in the same window.

### Two players (link cable)

**Link → Start player 2 with this game** (or **…with another game**) starts a second Game Boy or GBA in the same window, beside the first, connected by a link cable. Both must be the same system. You can also start it from the command line:

```bash
./target/release/mgba-rs game.gba --link            # player 2 runs the same game
./target/release/mgba-rs game.gba --link other.gba  # player 2 runs another game
```

Player 2's default keys are I/J/K/L for the d-pad, M / N for A / B, U / O for L / R, P for Start and Y for Select (remappable in **Settings → Controls**). With two controllers plugged in, the second one plays as player 2. Only player 1's sound is played. Save states and rewind are off while player 2 is connected, since restoring one side alone would desynchronize the link. When both players run the same game, player 2 keeps its battery save in `<rom name>.sa2`, as mGBA does.

| Action | Shortcut |
| --- | --- |
| Open ROM | Ctrl+O |
| Pause / resume | Ctrl+P |
| Reset | Ctrl+R |
| Fast forward | hold Tab (or toggle in the Emulation menu) |
| Rewind | hold ` (backtick); history length is set in the Emulation menu |
| Load / save state | F1–F9 / Shift+F1–F9 |
| Screenshot | F12 |
| Fullscreen | F11 |

### Files

- Battery saves are read from and written to `<rom name>.sav` next to the ROM, as mGBA does, so existing mGBA saves work. For Game Boy MBC3 carts with a clock, the RTC block mGBA appends to the save is read and written too.
- Save states go to `<rom name>.ss1` … `.ss9` in this port's own format. For GBA games the slots also load mGBA's state files (raw or PNG), so mGBA's `.ss1`–`.ss9` next to a ROM work as they are; **File → Save state for mGBA…** writes one mGBA can load, and **File → Load state file…** opens either kind. Game Boy states aren't exchangeable yet.
- Screenshots are written next to the ROM as PNG.
- Cheats live in `<rom name>.cheats` next to the ROM, in mGBA's cheats file format (**Settings → Cheats…** to add, toggle or remove them).
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
- Idle-loop handling as mGBA's `idleOptimization` setting: skip known idle loops, detect them automatically (`_analyzeForIdleLoop`), or run them (**Emulation → Idle loops**)
- AGBPrint, the debug-print interface of Nintendo's development cartridges; output goes to the emulator log
- Link cable between 2–4 GBAs in normal (8- and 32-bit) and multiplayer modes, using mGBA's lockstep driver (`src/gba/sio/lockstep.c`). As in mGBA, normal-mode data is passed from each player to the next, so the clock owner reads 0xFF
- Cheats: mGBA's cheat engine with GameShark / Action Replay v1–v2 (encrypted or raw), Pro Action Replay v3, CodeBreaker (including its encryption) and VBA-style codes, plus ROM patches; the format is detected automatically. Codes with a hook run when the game reaches the hooked address, through a `BKPT` patched into the ROM as mGBA does
- Save states in mGBA's own format, both ways (see [Files](#files))

### Game Boy / Game Boy Color

- SM83 CPU with mGBA's micro-op (M-cycle) model, including the HALT bug, EI delay and double speed
- Memory map with OAM DMA bus conflicts, CGB HDMA/GDMA, WRAM banking and open-bus cartridge behavior
- MBC1 (incl. multicarts), MBC2, MBC3 with RTC, MBC5 (incl. rumble flag), MBC6, MBC7 (EEPROM; no tilt sensor), MMM01, HuC-1, HuC-3 (with clock), TAMA5 (with clock), the Pocket Camera (captures an image supplied by the frontend; the GUI has no camera yet), M161, and the unlicensed Wisdom Tree, PKJD, NT, BBD, Hitek, Li Cheng, GGB-81, Sachen and Sintax mappers, with mGBA's detection and override table
- Timer with mGBA's DIV/TIMA edge timing, and the serial port with a link cable between two Game Boys (mGBA's lockstep driver, `src/gb/sio/lockstep.c`, plus the cycle bookkeeping its Qt frontend supplies)
- PPU mode timing and STAT/LYC interrupts, and the software renderer for DMG and CGB (priorities, banks, flips, window quirks, AGB color correction)
- Super Game Boy models: palette and attribute commands, VRAM transfers, multiplayer requests and borders (256x224 output with the default or game-supplied border; toggle in the Video menu)
- APU in DMG and CGB styles (sweep, envelopes, wave RAM quirks, LFSR noise), mixed like mGBA at 131072 Hz and averaged to 32768 Hz for output
- Model detection and selection (DMG, MGB, SGB, SGB2, CGB, AGB) and optional boot ROMs
- Cheats: GameShark, CodeBreaker, Game Genie (including bank-checked ROM patches) and VBA-style codes
- Playback of mGBA video logs recorded from a Game Boy (`Gb::load_mvl_state` / `Gb::mvl_packet`)

### Frontend

- Rewind (hold `), with history stored as compressed differences between save states so 30 seconds costs tens of megabytes, not hundreds
- Game controller input and rebinding (Linux)
- Two linked players side by side, with their own keyboard layout and controller
- Memory search, Lua scripts and the debugger under **Tools** (see below)

## Developer tools

**Debugger.** `mgba-rs game.gba -d` (or **Tools → Debugger in terminal**) attaches mGBA's command-line debugger to the terminal and stops before the first instruction; type `help` for the commands. Breakpoints (`break`, with an optional condition), software breakpoints on the GBA (`break/a`, `break/t`), read/write/change watchpoints and ranges, stepping (`next`, `trace`, `frame`), `print` with mGBA's expression syntax (registers, I/O register names like `DISPCNT`, `$bank:addr` segments, `*address`), memory dumps (`x/1`, `x/2`, `x/4`), writes, disassembly of ARM, Thumb and SM83 code, and symbols (`load-symbols` reads armips `.sym` files) all work as in mGBA.

**GDB.** `mgba-rs game.gba -g` (or **Tools → Start GDB server**) listens on `localhost:2345`, as mGBA's `-g` does; connect with `target remote localhost:2345` from an ARM GDB. Like mGBA's stub it supports registers, memory, breakpoints, watchpoints, stepping and the target/memory-map descriptions, for GBA games.

**Lua scripts.** `mgba-rs game.gba --script file.lua` (or **Tools → Load Lua script…**) runs scripts written for mGBA's scripting API: `emu` (`read8/16/32`, `write8/16/32`, `readRange`, `readRegister`, `writeRegister`, `getGameTitle`, `getGameCode`, `platform`, `frequency`, `frameCycles`, `currentFrame`, `currentCycle`, `romSize`, `checksum`, `reset`, `runFrame`, `step`, key functions, `saveStateBuffer`/`loadStateBuffer`), `emu.memory.<domain>` (`wram`, `iwram`, `vram`, `cart0`, … with `read*`, `write*`, `readRange`, `base`, `bound`, `size`, `name`), `console` (`log`, `warn`, `error`, `createBuffer`), `callbacks` (`add`, `oneshot`, `remove`; the `frame`, `reset`, `start` and `stop` events), `C` constants and `util`. Output and text buffers appear in the **Scripting** window.

**Memory search.** **Tools → Memory search…** is mGBA's cheat finder: search for a value (or guess it as decimal, hex or a scaled value) and refine the results as it changes.

See [TODO.md](TODO.md) for what isn't ported yet.

## Testing

mGBA ships a regression suite in `mgba-master/cinema`: test ROMs with expected screenshots, and recorded video logs (`.mvl`) of real games that exercise the renderer. It isn't part of this repository; clone mGBA into `mgba-master/` at the top of the workspace:

```bash
git clone https://github.com/mgba-emu/mgba.git mgba-master
```

```bash
cargo test --release
```

Without `mgba-master/`, the tests that need mGBA's test data print a note and skip; everything else still runs.

This runs every GBA ROM test and every video log and requires a pixel-exact match with mGBA's baselines (currently 3 ROM tests and 23 video logs, all passing), plus a save-state determinism check.

It also runs the Game Boy ROM tests in `cinema/gb` (blargg, mooneye-gb, acid2, samesuite, ramg-mbc3): 236 tests, of which 148 match mGBA's correct baseline and the other 88 are tests mGBA itself is marked as failing (`fail=1`). For those, 87 match mGBA's recorded wrong output (`xbaseline_*.png`) pixel for pixel, and one has no reference image at all, which mGBA's runner also counts as an expected failure. No test differs from mGBA. The 10 Game Boy video logs in `cinema/gb/window` (including a Super Game Boy one with its border) replay pixel-exactly too.

Headless link-cable checks (`tests/link.rs`) connect two Game Boys and two to four GBAs and verify the bytes, words and multiplayer halfwords each side receives, including after a reset and a disconnect.

`tests/devtools.rs` drives the CLI debugger (GBA and GB) and the GDB stub over a loopback socket, and checks idle-loop detection, AGBPrint, cheat hooks and memory search. `tests/mgba_state.rs` covers mGBA-format states; the format itself was verified against mGBA built from `mgba-master/` (states match byte for byte, and each emulator continues the other's states to identical frames). The ARM, Thumb and SM83 disassemblers and the expression parser have unit tests, and `mgba-script` tests run mGBA-style Lua scripts.

Other headless checks cover GB sound output, battery saves, the MBC3/HuC-3/TAMA5 clock blocks, the newer mappers, the Pocket Camera, cheats in every format, and save states.

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

```bash
cargo run --release -p mgba-core --example link -- game.gba --players 4 --frames 600
```

```bash
cargo run --release -p mgba-core --example mgba_state -- import game.gba game.ss1 60 /tmp/frame.rgb
```

`trace` and `steptrace` print CPU state per frame or per instruction for debugging.

## License and credits

mGBA is Copyright © 2013–2026 Jeffrey Pfau and contributors and is distributed under the [Mozilla Public License 2.0](https://www.mozilla.org/MPL/2.0/). This port translates mGBA's source code, so it is a derivative work ("Covered Software" under the MPL) and is distributed under the same license; see [`LICENSE`](LICENSE). Each source file keeps mGBA's copyright notice and the MPL header, and names the mGBA files and functions it ports. The built-in HLE BIOS (`crates/mgba-core/src/gba/hle_bios.bin`) is assembled from mGBA's `src/gba/hle-bios.s` and is covered by the same license.

The app uses third-party Rust crates and Lua 5.4 under their own permissive licenses; see [`NOTICE.md`](NOTICE.md). mGBA's test ROMs and video logs are used from `mgba-master/` at test time and are not part of this repository. No game ROMs, BIOS dumps or boot ROMs are included.
