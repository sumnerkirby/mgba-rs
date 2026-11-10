# To do

What mGBA has that this port doesn't yet. Each item names the mGBA source to port.

## Save states

- **Game Boy states in mGBA's format** (`src/gb/serialize.c` and the serializers it calls in `src/gb/{memory,mbc,io,timer,video,audio,sio}.c`, including the MBC state union and the SM83 micro-op state). GBA states are done (`crates/mgba-core/src/gba/mgba_state.rs`).
- **Link-cable state in save states**: mGBA stores the GBA lockstep driver's state as extdata (`GBASIOLockstepDriverSaveState`/`LoadState` in `src/gba/sio/lockstep.c`). Neither this port's states nor the mGBA-format export include it, so states saved while linked don't restore the link.
- Writing PNG states with a screenshot (`_savePNGState`); loading them already works.
- RTC and cheat extdata (`EXTDATA_RTC`, `EXTDATA_CHEATS`) when loading and saving mGBA states.

### Checking against mGBA

The GBA format was verified by building mGBA's core from `mgba-master/` and comparing states. No CMake is needed: compile `src/{arm,sm83,gba,gb,core,util}` (plus `src/gba/{cart,cheats,sio,extra}`, `src/gb/{mbc,sio,extra}`, the software renderers, `src/feature/video-logger.c`, `src/util/vfs/vfs-{mem,fd}.c` and `src/third-party/inih/ini.c`) with `-DM_CORE_GBA -DM_CORE_GB -DDISABLE_THREADING -DENABLE_VFS -DENABLE_VFS_FD`, generate `version.c` from `src/core/version.c.in`, and link a small program that loads a ROM, runs frames and calls `mCoreSaveStateNamed`. Set the `volume` config to `0x100` (otherwise mGBA's audio samples are all zero), and note that `globalCycles` only counts when built with `-DENABLE_DEBUGGERS`. Generating the state's field offsets with `offsetof` from mGBA's headers avoids layout mistakes.

## Debugger

- Stack tracing: `backtrace`, `finish` and `stack` in the CLI (`ARMDebuggerUpdateStackTrace`, `src/debugger/stack-trace.c`).
- The access logger (`src/debugger/access-logger.c`).
- CLI `load`/`save` (save-state slots) and `source` (run a Lua script).
- Script breakpoints and watchpoints (`emu:setBreakpoint`, `setWatchpoint`, `setRangeWatchpoint`, `clearBreakpoint`).
- ELF symbol loading (`mDebuggerLoadELFSymbols`); armips `.sym` files are supported.

## Scripting

- The `input` API (keyboard and controller state), `socket`, `canvas` and `image` (overlays and screenshots), and `storage` (persistent script data).
- Events: `keysRead` at the moment the game reads KEYINPUT (scripts' keys are applied each frame instead), `crashed`, `sleep`, `savedataUpdated`, `alarm`, `rumble`, `memoryBlocksChanged`.
- `emu:screenshot`, `saveStateSlot`/`saveStateFile`/`loadStateSlot`/`loadStateFile`, `loadFile`, `autoloadSave`, `loadSaveFile`; rotation and solar sensor callbacks.

## Hardware and cartridges

- Gyro, tilt, solar and rumble sensors (recognized, not driven); a camera source for the Pocket Camera in the GUI.
- GBA unlicensed carts: Vast Fame and the multicart mapper (`src/gba/cart/unlicensed.c`, `vfame.c`), and Matrix memory carts (`matrix.c`).
- e-Reader (`src/gba/cart/ereader.c`), Game Boy Player (`src/gba/sio/gbp.c`), Battle Chip Gate (`src/gba/extra/battlechip.c`), Dolphin/GameCube JOY bus link (`src/gba/sio/dolphin.c`), Game Boy Printer (`src/gb/sio/printer.c`).
- Network link play (link cable between machines).

## Files

- ROM patches: IPS, UPS and BPS (`src/util/patch-*.c`).
- Multiboot images and ELF executables for the GBA; GBX files for the Game Boy.

## Frontend

- Audio and video recording.
- Audio output and game controllers on platforms other than Linux.
- Save states and rewind while a second player is linked.
- Memory search across banks (segments; mGBA has the same TODO).
