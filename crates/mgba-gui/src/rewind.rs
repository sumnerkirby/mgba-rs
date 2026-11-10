//! Rewind buffer (mGBA's `mCoreRewindContext`).
//!
//! mGBA keeps a ring of full save states. Here the newest snapshot is kept
//! whole and every older one is stored as the XOR against its successor,
//! run-length encoded; consecutive states differ in few bytes, so this keeps
//! minutes of history in a few megabytes. Pushing and popping are O(state size).
//!
//! This Source Code Form is subject to the terms of the Mozilla Public
//! License, v. 2.0. If a copy of the MPL was not distributed with this
//! file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::VecDeque;

/// One saved moment: the core's state plus the frame shown at that point.
pub struct Snapshot {
    pub state: Vec<u8>,
    pub frame: Vec<u32>,
}

enum Entry {
    /// Delta against the next newer snapshot (or a full copy if sizes changed).
    Delta { state: Vec<u8>, frame: Vec<u8>, full: bool },
}

pub struct Rewind {
    older: VecDeque<Entry>,
    newest: Option<Snapshot>,
    pub capacity: usize,
}

fn frame_bytes(frame: &[u32]) -> Vec<u8> {
    frame.iter().flat_map(|p| p.to_le_bytes()).collect()
}

fn bytes_frame(bytes: &[u8]) -> Vec<u32> {
    bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn push_varint(out: &mut Vec<u8>, mut v: usize) {
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn read_varint(data: &[u8], pos: &mut usize) -> usize {
    let mut v = 0usize;
    let mut shift = 0;
    loop {
        let byte = data[*pos];
        *pos += 1;
        v |= ((byte & 0x7F) as usize) << shift;
        if byte & 0x80 == 0 {
            return v;
        }
        shift += 7;
    }
}

/// Encode `a XOR b` as (zero run, literal run, literals)*.
fn xor_encode(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    let n = a.len();
    while i < n {
        let start = i;
        while i < n && a[i] == b[i] {
            i += 1;
        }
        push_varint(&mut out, i - start);
        let lit_start = i;
        // A literal run ends at the first stretch of 4 equal bytes.
        while i < n && !(i + 4 <= n && a[i..i + 4] == b[i..i + 4]) {
            i += 1;
        }
        push_varint(&mut out, i - lit_start);
        out.extend(a[lit_start..i].iter().zip(&b[lit_start..i]).map(|(x, y)| x ^ y));
    }
    out
}

/// Apply an encoding from [`xor_encode`] to `base`, producing the other side.
fn xor_decode(delta: &[u8], base: &[u8]) -> Vec<u8> {
    let mut out = base.to_vec();
    let mut pos = 0;
    let mut i = 0;
    while pos < delta.len() {
        i += read_varint(delta, &mut pos);
        let lits = read_varint(delta, &mut pos);
        for k in 0..lits {
            out[i + k] ^= delta[pos + k];
        }
        pos += lits;
        i += lits;
    }
    out
}

impl Rewind {
    pub fn new(capacity: usize) -> Rewind {
        Rewind { older: VecDeque::new(), newest: None, capacity }
    }

    pub fn clear(&mut self) {
        self.older.clear();
        self.newest = None;
    }

    pub fn len(&self) -> usize {
        self.older.len() + self.newest.is_some() as usize
    }

    /// Approximate memory used, in bytes.
    pub fn memory(&self) -> usize {
        let older: usize = self
            .older
            .iter()
            .map(|Entry::Delta { state, frame, .. }| state.len() + frame.len())
            .sum();
        older + self.newest.as_ref().map(|s| s.state.len() + s.frame.len() * 4).unwrap_or(0)
    }

    pub fn push(&mut self, snap: Snapshot) {
        if let Some(prev) = self.newest.take() {
            let full = prev.state.len() != snap.state.len() || prev.frame.len() != snap.frame.len();
            let entry = if full {
                Entry::Delta { state: prev.state, frame: frame_bytes(&prev.frame), full: true }
            } else {
                Entry::Delta {
                    state: xor_encode(&prev.state, &snap.state),
                    frame: xor_encode(&frame_bytes(&prev.frame), &frame_bytes(&snap.frame)),
                    full: false,
                }
            };
            self.older.push_back(entry);
            while self.older.len() + 1 > self.capacity.max(1) {
                self.older.pop_front();
            }
        }
        self.newest = Some(snap);
    }

    /// Remove and return the newest snapshot; the one before it becomes newest.
    pub fn pop(&mut self) -> Option<Snapshot> {
        let newest = self.newest.take()?;
        if let Some(Entry::Delta { state, frame, full }) = self.older.pop_back() {
            let prev = if full {
                Snapshot { state, frame: bytes_frame(&frame) }
            } else {
                Snapshot {
                    state: xor_decode(&state, &newest.state),
                    frame: bytes_frame(&xor_decode(&frame, &frame_bytes(&newest.frame))),
                }
            };
            self.newest = Some(prev);
        }
        Some(newest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_deltas() {
        let mut r = Rewind::new(100);
        let mut states = vec![];
        for i in 0..20u8 {
            let mut state = vec![0u8; 5000];
            state[i as usize * 7] = i;
            state[4999] = i.wrapping_mul(3);
            let frame = vec![i as u32; 64];
            states.push((state.clone(), frame.clone()));
            r.push(Snapshot { state, frame });
        }
        for (state, frame) in states.iter().rev() {
            let s = r.pop().unwrap();
            assert_eq!(&s.state, state);
            assert_eq!(&s.frame, frame);
        }
        assert!(r.pop().is_none());
    }

    #[test]
    fn capacity_drops_oldest() {
        let mut r = Rewind::new(3);
        for i in 0..10u8 {
            r.push(Snapshot { state: vec![i; 10], frame: vec![] });
        }
        assert_eq!(r.len(), 3);
        assert_eq!(r.pop().unwrap().state[0], 9);
        assert_eq!(r.pop().unwrap().state[0], 8);
        assert_eq!(r.pop().unwrap().state[0], 7);
        assert!(r.pop().is_none());
    }
}
