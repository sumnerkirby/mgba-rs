// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's save-state container (src/core/serialize.c): a raw
// state followed by "extdata", or a PNG screenshot carrying the state and
// extdata in zlib-compressed `gbAs` and `gbAx` chunks.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;
use std::io::{Read, Write};

/// `enum mStateExtdataTag`
pub const EXTDATA_SCREENSHOT: u32 = 1;
pub const EXTDATA_SAVEDATA: u32 = 2;
pub const EXTDATA_CHEATS: u32 = 3;
pub const EXTDATA_RTC: u32 = 4;
pub const EXTDATA_MAX: u32 = 0x103;

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

/// `mStateExtdataSerialize`: header entries (tag, size, absolute offset),
/// a zero entry, then the data, appended after a raw state of `state_len`.
pub fn write_extdata(out: &mut Vec<u8>, extdata: &BTreeMap<u32, Vec<u8>>) {
    if extdata.is_empty() {
        return;
    }
    let header_size = 16 * (extdata.len() + 1);
    let mut position = out.len() + header_size;
    for (tag, data) in extdata {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(position as u64).to_le_bytes());
        position += data.len();
    }
    out.extend_from_slice(&[0u8; 16]);
    for data in extdata.values() {
        out.extend_from_slice(data);
    }
}

/// `mStateExtdataDeserialize`
fn read_extdata(file: &[u8], mut pos: usize) -> BTreeMap<u32, Vec<u8>> {
    let mut out = BTreeMap::new();
    while pos + 16 <= file.len() {
        let tag = u32::from_le_bytes(file[pos..pos + 4].try_into().unwrap());
        let size = i32::from_le_bytes(file[pos + 4..pos + 8].try_into().unwrap());
        let offset = u64::from_le_bytes(file[pos + 8..pos + 16].try_into().unwrap()) as usize;
        pos += 16;
        if tag == 0 {
            break;
        }
        if tag >= EXTDATA_MAX || size <= 0 {
            continue;
        }
        if let Some(data) = file.get(offset..offset + size as usize) {
            out.insert(tag, data.to_vec());
        }
    }
    out
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data).read_to_end(&mut out).ok()?;
    Some(out)
}

/// `mCoreExtractState` + `mCoreExtractExtdata`: the raw state (`state_size`
/// bytes) and the extdata from either container.
pub fn extract(file: &[u8], state_size: usize) -> Option<(Vec<u8>, BTreeMap<u32, Vec<u8>>)> {
    if file.starts_with(&PNG_SIGNATURE) {
        // `_loadPNGState`: walk the chunks for gbAs and gbAx.
        let mut state = None;
        let mut extdata = BTreeMap::new();
        let mut pos = 8;
        while pos + 8 <= file.len() {
            let len = u32::from_be_bytes(file[pos..pos + 4].try_into().unwrap()) as usize;
            let kind = &file[pos + 4..pos + 8];
            let data = file.get(pos + 8..pos + 8 + len)?;
            match kind {
                b"gbAs" => state = inflate(data),
                b"gbAx" if data.len() >= 8 => {
                    let tag = u32::from_le_bytes(data[0..4].try_into().unwrap());
                    if tag != 0 && tag < EXTDATA_MAX {
                        if let Some(d) = inflate(&data[8..]) {
                            extdata.insert(tag, d);
                        }
                    }
                }
                b"IEND" => break,
                _ => {}
            }
            pos += 12 + len;
        }
        let mut state = state?;
        state.resize(state_size, 0);
        return Some((state, extdata));
    }
    if file.len() < state_size {
        return None;
    }
    Some((file[..state_size].to_vec(), read_extdata(file, state_size)))
}

/// A raw state file: the state, then its extdata (what mGBA writes when
/// it isn't asked for a screenshot).
pub fn build_raw(state: &[u8], extdata: &BTreeMap<u32, Vec<u8>>) -> Vec<u8> {
    let mut out = state.to_vec();
    write_extdata(&mut out, extdata);
    out
}

/// zlib-compress (for writing PNG chunks).
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}
