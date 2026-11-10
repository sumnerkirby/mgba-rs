// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's event scheduler (src/core/timing.c).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Every schedulable event in the system. In mGBA these are separate
/// `struct mTimingEvent`s with callbacks; here the owner dispatches on the id.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventId {
    Irq,
    Video,
    AudioFrame,
    AudioSample,
    Timer0,
    Timer1,
    Timer2,
    Timer3,
    Dma,
    SavedataDust,
    Sio,
}

pub const EVENT_COUNT: usize = 11;

impl EventId {
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn priority(self) -> u32 {
        match self {
            EventId::Irq => 0,
            EventId::Video => 8,
            EventId::AudioFrame => 0x10,
            EventId::AudioSample => 0x18,
            EventId::Timer0 => 0x20,
            EventId::Timer1 => 0x21,
            EventId::Timer2 => 0x22,
            EventId::Timer3 => 0x23,
            EventId::Dma => 0x40,
            EventId::SavedataDust => 0x70,
            EventId::Sio => 0x80,
        }
    }
}

impl TimingEvent for EventId {
    #[inline]
    fn index(self) -> usize {
        self as usize
    }
    #[inline]
    fn priority(self) -> u32 {
        EventId::priority(self)
    }
}

/// An event identifier usable with [`Timing`]: each system (GBA, GB) has its
/// own enum, standing in for mGBA's per-event `struct mTimingEvent`.
pub trait TimingEvent: Copy + PartialEq + std::fmt::Debug {
    fn index(self) -> usize;
    fn priority(self) -> u32;
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub struct Event<E = EventId> {
    pub id: E,
    pub when: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[serde(bound(serialize = "E: serde::Serialize", deserialize = "E: serde::Deserialize<'de>"))]
pub struct Timing<E = EventId, const N: usize = EVENT_COUNT> {
    pub root: Vec<Event<E>>,
    pub reroot: Vec<Event<E>>,
    pub global_cycles: u64,
    pub master_cycles: u32,
    /// The `when` field of each event struct; retained after it fires, like mGBA's.
    #[serde(with = "serde_big_array::BigArray")]
    pub whens: [u32; N],
}

impl<E, const N: usize> Default for Timing<E, N> {
    fn default() -> Self {
        Timing { root: Vec::new(), reroot: Vec::new(), global_cycles: 0, master_cycles: 0, whens: [0; N] }
    }
}

impl<E: TimingEvent, const N: usize> Timing<E, N> {
    pub fn clear(&mut self) {
        self.root.clear();
        self.reroot.clear();
        self.global_cycles = 0;
        self.master_cycles = 0;
    }

    /// `mTimingInterrupt`
    pub fn interrupt(&mut self) {
        if self.root.is_empty() {
            return;
        }
        self.reroot = std::mem::take(&mut self.root);
    }

    fn active_list(&mut self) -> &mut Vec<Event<E>> {
        if !self.reroot.is_empty() {
            &mut self.reroot
        } else {
            &mut self.root
        }
    }

    /// `mTimingSchedule`. `relative` is the CPU's current cycle count and
    /// `next_event` the CPU's next-event marker.
    pub fn schedule(&mut self, id: E, when: i32, relative: i32, next_event: &mut i32) {
        self.deschedule(id);
        let next = when.wrapping_add(relative);
        let abs = (next as u32).wrapping_add(self.master_cycles);
        if next < *next_event {
            *next_event = next;
        }
        let master = self.master_cycles;
        let priority = id.priority();
        let list = self.active_list();
        let mut pos = list.len();
        for (i, e) in list.iter().enumerate() {
            let next_when = e.when.wrapping_sub(master) as i32;
            if next_when > next || (next_when == next && e.id.priority() > priority) {
                pos = i;
                break;
            }
        }
        list.insert(pos, Event { id, when: abs });
        self.whens[id.index()] = abs;
    }

    /// `mTimingDeschedule`
    pub fn deschedule(&mut self, id: E) {
        let list = self.active_list();
        if let Some(pos) = list.iter().position(|e| e.id == id) {
            list.remove(pos);
        }
    }

    /// `mTimingIsScheduled`
    pub fn is_scheduled(&self, id: E) -> bool {
        let list = if self.root.is_empty() { &self.reroot } else { &self.root };
        list.iter().any(|e| e.id == id)
    }

    /// `mTimingCurrentTime`
    #[inline]
    pub fn current_time(&self, relative: i32) -> i32 {
        self.master_cycles.wrapping_add(relative as u32) as i32
    }

    /// `mTimingNextEvent`
    pub fn next_event(&self, relative: i32) -> i32 {
        match self.root.first() {
            None => i32::MAX,
            Some(e) => e.when.wrapping_sub(self.master_cycles).wrapping_sub(relative as u32) as i32,
        }
    }

    /// `mTimingUntil`
    pub fn until(&self, id: E, relative: i32) -> i32 {
        self.whens[id.index()].wrapping_sub(self.master_cycles).wrapping_sub(relative as u32) as i32
    }

    /// Absolute `when` of a scheduled event (for serialization).
    pub fn event_when(&self, id: E) -> Option<u32> {
        let list = if self.root.is_empty() { &self.reroot } else { &self.root };
        list.iter().find(|e| e.id == id).map(|e| e.when)
    }
}
