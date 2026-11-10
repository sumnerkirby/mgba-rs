// Copyright (c) 2013-2024 Jeffrey Pfau
// Rust port of mGBA's shared lockstep definitions (include/mgba/core/lockstep.h,
// src/core/lockstep.c) and of the part of a frontend that drives linked cores.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Link-cable plumbing shared by the Game Boy and GBA lockstep drivers.
//!
//! In mGBA every linked core runs on its own `mCoreThread`. When a lockstep
//! driver needs a core to stop and wait for its partners it calls
//! `mCoreThreadWaitFromThread` (a pending "wait" request on that thread) and
//! interrupts the CPU so the run loop returns; a partner later calls
//! `mCoreThreadStopWaiting` to cancel the request. Here all cores run on one
//! thread: the request is a flag ([`ThreadWait`]) and [`run_linked_frame`] steps
//! whichever cores have no wait pending, which is the same schedule as some
//! interleaving of mGBA's threads.

/// `enum mLockstepPhase`
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LockstepPhase {
    #[default]
    Idle,
    Starting,
    Started,
    Finishing,
    Finished,
}

/// The wait request of one core's (emulated) thread: `mCoreThreadWaitFromThread`
/// sets it, `mCoreThreadStopWaiting` clears it, and the core stops running at the
/// end of its current run loop while it is set.
#[derive(Clone, Copy, Debug, Default)]
pub struct ThreadWait {
    pub requested: bool,
}

impl ThreadWait {
    /// `mCoreThreadWaitFromThread`
    pub fn wait_from_thread(&mut self) {
        self.requested = true;
    }

    /// `mCoreThreadStopWaiting`
    pub fn stop_waiting(&mut self) {
        self.requested = false;
    }
}

/// A core that can take part in a link: what `mCoreThread`'s loop needs from it.
pub trait LinkedCore {
    /// `core->runLoop`
    fn run_loop(&mut self);
    /// Whether this core's thread has a wait request pending.
    fn link_waiting(&self) -> bool;
    /// Frames completed so far.
    fn link_frame_counter(&self) -> u32;
    /// Called after each run loop so partners can read this core's clock
    /// (mGBA reads it across threads; here the core is paused when they look).
    fn link_publish_time(&mut self) {}
}

/// Run linked cores until each has finished at least one frame, the way their
/// threads would in mGBA: a core whose thread is waiting is skipped until a
/// partner wakes it, and a core that is done with its frame keeps running if the
/// others are waiting on it. Returns `false` if every core ended up waiting
/// (the link deadlocked), after which the caller can keep calling it.
pub fn run_linked_frame<C: LinkedCore + ?Sized>(cores: &mut [&mut C]) -> bool {
    let start: Vec<u32> = cores.iter().map(|c| c.link_frame_counter()).collect();
    // Bound the work so a misbehaving link can't hang the caller forever.
    let mut budget = 4_000_000u32;
    loop {
        let pending = |cores: &[&mut C], i: usize| cores[i].link_frame_counter() == start[i];
        if !(0..cores.len()).any(|i| pending(cores, i)) {
            return true;
        }
        let pick = (0..cores.len())
            .find(|&i| pending(cores, i) && !cores[i].link_waiting())
            .or_else(|| (0..cores.len()).find(|&i| !cores[i].link_waiting()));
        let Some(i) = pick else {
            return false;
        };
        let frame = cores[i].link_frame_counter();
        // Run this core until its thread is asked to wait or its frame ends.
        loop {
            cores[i].run_loop();
            cores[i].link_publish_time();
            budget = budget.saturating_sub(1);
            if budget == 0 {
                return false;
            }
            if cores[i].link_waiting() || cores[i].link_frame_counter() != frame {
                break;
            }
        }
    }
}
