// Rust port of mGBA (https://mgba.io), a Game Boy Advance and Game Boy emulator.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub mod arm;
pub mod cheats;
pub mod gb;
pub mod gba;
pub mod sm83;
pub mod timing;

pub use gb::Gb;
pub use gba::Gba;
