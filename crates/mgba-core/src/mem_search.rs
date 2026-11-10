// Copyright (c) 2013-2017 Jeffrey Pfau
// Rust port of mGBA's memory search (src/core/mem-search.c,
// include/mgba/core/mem-search.h), the engine behind its cheat finder.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::core::{MCore, MemoryBlock};

/// `enum mCoreMemorySearchType`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchType {
    Int,
    String,
    Guess,
}

/// `enum mCoreMemorySearchOp`
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SearchOp {
    Equal,
    Greater,
    Less,
    Any,
    Delta,
    DeltaPositive,
    DeltaNegative,
    DeltaAny,
}

/// `struct mCoreMemorySearchParams`
#[derive(Clone, Debug)]
pub struct SearchParams {
    pub memory_flags: u32,
    pub ty: SearchType,
    pub op: SearchOp,
    /// -1 for any alignment.
    pub align: i32,
    /// 1, 2 or 4; -1 lets a guess pick from the value.
    pub width: i32,
    pub value_int: i32,
    pub value_str: String,
}

/// `struct mCoreMemorySearchResult`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchResult {
    pub address: u32,
    pub segment: i32,
    pub guess_divisor: u32,
    pub guess_multiplier: u32,
    pub ty: SearchType,
    pub width: i32,
    pub old_value: i32,
}

/// `_op`
fn op(value: i32, matching: i32, op: SearchOp) -> bool {
    match op {
        SearchOp::Greater => value > matching,
        SearchOp::Less => value < matching,
        SearchOp::Equal | SearchOp::Delta => value == matching,
        SearchOp::DeltaPositive => value > 0,
        SearchOp::DeltaNegative => value < 0,
        SearchOp::DeltaAny => value != 0,
        SearchOp::Any => true,
    }
}

/// `_search32` / `_search16` / `_search8`
fn search_width(
    mem: &[u8],
    block: &MemoryBlock,
    width: usize,
    value: i64,
    sop: SearchOp,
    out: &mut Vec<SearchResult>,
    limit: usize,
) -> usize {
    let mut found = 0;
    let mut i = 0;
    let matching = match width {
        4 => value as u32 as i32,
        2 => value as u16 as i32,
        _ => value as u8 as i32,
    };
    while (limit == 0 || found < limit) && i + width <= mem.len() {
        let v = match width {
            4 => u32::from_le_bytes([mem[i], mem[i + 1], mem[i + 2], mem[i + 3]]) as i32,
            2 => u16::from_le_bytes([mem[i], mem[i + 1]]) as i32,
            _ => mem[i] as i32,
        };
        if op(v, matching, sop) {
            out.push(SearchResult {
                address: block.start.wrapping_add(i as u32),
                segment: -1, // TODO
                guess_divisor: 1,
                guess_multiplier: 1,
                ty: SearchType::Int,
                width: width as i32,
                old_value: v,
            });
            found += 1;
        }
        i += width;
    }
    found
}

/// `_searchInt`
fn search_int(mem: &[u8], block: &MemoryBlock, p: &SearchParams, out: &mut Vec<SearchResult>, limit: usize) -> usize {
    if p.align == p.width || p.align == -1 {
        match p.width {
            4 | 2 | 1 => return search_width(mem, block, p.width as usize, p.value_int as i64, p.op, out, limit),
            _ => {}
        }
    }
    0
}

/// `_searchStr`
fn search_str(mem: &[u8], block: &MemoryBlock, value: &[u8], out: &mut Vec<SearchResult>, limit: usize) -> usize {
    let mut found = 0;
    let len = value.len();
    let mut i = 0;
    while (limit == 0 || found < limit) && i + len < mem.len() {
        if &mem[i..i + len] == value {
            out.push(SearchResult {
                address: block.start.wrapping_add(i as u32),
                segment: -1,
                guess_divisor: 1,
                guess_multiplier: 1,
                ty: SearchType::String,
                width: len as i32,
                old_value: 0,
            });
            found += 1;
        }
        i += 1;
    }
    found
}

/// The width a guess picks for `value`.
fn guess_width(p: &SearchParams, value: i64) -> usize {
    if (p.width == -1 && value > 0x10000) || p.width == 4 {
        4
    } else if (p.width == -1 && value > 0x100) || p.width == 2 {
        2
    } else {
        1
    }
}

/// `strtoll` that must consume the whole string.
fn parse_full(s: &str, radix: u32) -> Option<i64> {
    let t = s.trim_start();
    if t.is_empty() {
        return None;
    }
    let t = if radix == 16 { t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t) } else { t };
    i64::from_str_radix(t, radix).ok()
}

/// `_searchGuess`: the value as decimal and as hex, and scaled down by
/// powers of 10 (or 16) for games that store it divided.
fn search_guess(mem: &[u8], block: &MemoryBlock, p: &SearchParams, out: &mut Vec<SearchResult>, limit: usize) -> usize {
    let mut found = 0;
    for (radix, base) in [(10u32, 10i64), (16, 16)] {
        let Some(mut value) = parse_full(&p.value_str, radix) else { continue };
        let w = guess_width(p, value);
        found += search_width(mem, block, w, value, p.op, out, if limit != 0 { limit - found.min(limit) } else { 0 });
        let mut divisor = 1u32;
        while value != 0 && value % base == 0 {
            value /= base;
            divisor = divisor.wrapping_mul(base as u32);
            let mut tmp = Vec::new();
            let w = guess_width(p, value);
            found += search_width(mem, block, w, value, p.op, &mut tmp, if limit != 0 { limit - found.min(limit) } else { 0 });
            for mut r in tmp {
                r.guess_divisor = divisor;
                out.push(r);
            }
        }
    }
    found
}

/// `_search`
fn search_block(mem: &[u8], block: &MemoryBlock, p: &SearchParams, out: &mut Vec<SearchResult>, limit: usize) -> usize {
    match p.ty {
        SearchType::Int => search_int(mem, block, p, out, limit),
        SearchType::String => {
            let bytes = p.value_str.as_bytes();
            let len = (p.width.max(0) as usize).min(bytes.len());
            search_str(mem, block, &bytes[..len], out, limit)
        }
        SearchType::Guess => search_guess(mem, block, p, out, limit),
    }
}

/// `mCoreMemorySearch`: search every memory block matching `memory_flags`.
/// `limit` 0 means unlimited.
pub fn search(core: &dyn MCore, p: &SearchParams, limit: usize) -> Vec<SearchResult> {
    let mut out = Vec::new();
    let mut found = 0;
    for block in core.list_memory_blocks() {
        if limit != 0 && found >= limit {
            break;
        }
        if block.flags & p.memory_flags == 0 {
            continue;
        }
        let Some(mut mem) = core.memory_block(block.id) else { continue };
        let span = block.end.wrapping_sub(block.start) as usize;
        if mem.len() > span {
            mem.truncate(span); // TODO: Segments
        }
        found += search_block(&mem, &block, p, &mut out, if limit != 0 { limit - found } else { 0 });
    }
    out
}

/// `_testSpecificGuess`
fn test_specific_guess(core: &mut dyn MCore, res: &mut SearchResult, op_value: i64, sop: SearchOp) -> bool {
    let offset: i64 = if sop >= SearchOp::Delta { res.old_value as i64 } else { 0 };
    res.old_value = res.old_value.wrapping_add(op_value as i32);
    let check = |value: i64, res: &SearchResult| {
        op(
            (value * res.guess_divisor as i64 / res.guess_multiplier.max(1) as i64 - offset) as i32,
            op_value as i32,
            sop,
        )
    };
    let value = core.raw_read8(res.address, res.segment) as i64;
    if check(value, res) {
        res.old_value = value as i32;
        return true;
    }
    if res.address & 1 == 0 && (res.width >= 2 || res.width == -1) {
        let value = core.raw_read16(res.address, res.segment) as i64;
        if check(value, res) {
            res.old_value = value as i32;
            return true;
        }
    }
    if res.address & 3 == 0 && (res.width >= 4 || res.width == -1) {
        let value = core.raw_read32(res.address, res.segment) as i64;
        if check(value, res) {
            res.old_value = value as i32;
            return true;
        }
    }
    res.old_value = res.old_value.wrapping_sub(op_value as i32);
    false
}

/// `_testGuess`
fn test_guess(core: &mut dyn MCore, res: &mut SearchResult, p: &SearchParams) -> bool {
    if let Some(v) = parse_full(&p.value_str, 10) {
        if test_specific_guess(core, res, v, p.op) {
            return true;
        }
    }
    if let Some(v) = parse_full(&p.value_str, 16) {
        if test_specific_guess(core, res, v, p.op) {
            return true;
        }
    }
    false
}

/// `mCoreMemorySearchRepeat`: narrow earlier results with a new condition
/// (deltas compare against each result's previous value).
pub fn search_repeat(core: &mut dyn MCore, p: &SearchParams, inout: &mut Vec<SearchResult>) {
    let mut i = 0;
    while i < inout.len() {
        let mut res = inout[i];
        let keep = match res.ty {
            SearchType::Int => match p.ty {
                SearchType::Guess => test_guess(core, &mut res, p),
                SearchType::Int => {
                    let value = match p.width {
                        1 => core.raw_read8(res.address, res.segment) as i32,
                        2 => core.raw_read16(res.address, res.segment) as i32,
                        4 => core.raw_read32(res.address, res.segment) as i32,
                        _ => 0,
                    };
                    let mut op_value = value;
                    if p.op >= SearchOp::Delta {
                        op_value = op_value.wrapping_sub(res.old_value);
                    }
                    let keep = op(op_value, p.value_int, p.op);
                    if keep {
                        res.old_value = value;
                    }
                    keep
                }
                SearchType::String => true,
            },
            // TODO
            SearchType::String | SearchType::Guess => true,
        };
        if keep {
            inout[i] = res;
            i += 1;
        } else {
            // mGBA moves the last result into the hole.
            inout.swap_remove(i);
        }
    }
}
