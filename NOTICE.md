# Notices and credits

## mGBA

mGBA-rs is a Rust translation of [mGBA](https://mgba.io), Copyright © 2013–2026 Jeffrey Pfau and contributors, distributed under the Mozilla Public License 2.0. As a translation of mGBA's source it is a derivative work and is distributed under the same license (see [`LICENSE`](LICENSE)). The source files carry mGBA's copyright and MPL headers and name the mGBA files and functions they port.

This includes mGBA's high-level BIOS replacement (`crates/mgba-core/src/gba/hle_bios.bin`, built from mGBA's `src/gba/hle-bios.s`), its per-game override tables, cheat code decryption tables, and the Super Game Boy default border, all of which come from mGBA's source.

The port was made with AI assistance (see "How this port was made" in the README). It is not affiliated with or endorsed by the mGBA project.

mGBA's own third-party components (inih, LZMA SDK, MurmurHash3, SQLite and others listed in mGBA's README) are not used or translated here.

## Lua

The `mgba-script` crate builds Lua 5.4 from source (via the `lua-src` crate):

> Lua 5.4 — Copyright © 1994–2024 Lua.org, PUC-Rio. Licensed under the MIT License.

## Rust crates

The workspace depends on crates from crates.io, built into the `mgba-rs` binary. They are all under permissive licenses compatible with the MPL: MIT and/or Apache-2.0 for the large majority, plus Zlib, BSD-2-Clause, BSD-3-Clause, ISC, BSL-1.0, Unlicense, 0BSD and Unicode-3.0 for some. A few offer a choice that includes a copyleft license (GPL-2.0 or LGPL-2.1); this project uses them under their permissive option. The direct dependencies are:

| Crate | Used for | License |
| --- | --- | --- |
| serde, bincode, serde-big-array | this port's save states | MIT OR Apache-2.0 / MIT |
| flate2 (miniz_oxide) | mGBA save-state compression | MIT OR Apache-2.0 |
| mlua, lua-src | Lua scripting | MIT |
| eframe, egui | GUI | MIT OR Apache-2.0 |
| rfd | file dialogs | MIT |
| png | screenshots | MIT OR Apache-2.0 |
| libloading, libc | ALSA audio and controller input on Linux | ISC / MIT OR Apache-2.0 |

Run `cargo metadata` (or a tool such as `cargo about`) for the complete list with each crate's license text.

## Fonts

egui embeds its default fonts (from the `epaint_default_fonts` crate) in the GUI binary:

- Ubuntu Light — Ubuntu Font Licence 1.0
- Hack — MIT License (Copyright 2018 Source Foundry Authors; DejaVu portions in the public domain)
- Noto Emoji — SIL Open Font License 1.1
- emoji-icon-font — MIT License

## Test data

The tests read mGBA's regression suite (test ROMs, screenshots and video logs) from a separate `mgba-master/` checkout at test time. That data belongs to the mGBA project and its respective authors and is not part of this repository.
