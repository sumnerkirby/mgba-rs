// Copyright (c) 2013-2016 Jeffrey Pfau
// Rust port of mGBA's Game Boy link-cable lockstep driver
// (src/gb/sio/lockstep.c, include/mgba/internal/gb/sio/lockstep.h) together with
// the `struct mLockstep` callbacks that mGBA's Qt frontend supplies for it
// (MultiplayerController.cpp).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{Arc, Mutex};

use super::io::{REG_SC, SIO_CYCLES_PER_TRANSFER};
use super::{glog, Gb, GbEvent, LogLevel};
use crate::lockstep::{LinkedCore, LockstepPhase, ThreadWait};

/// `MAX_GBS`
pub const MAX_GBS: usize = 2;

const LOCKSTEP_INCREMENT: i32 = 512;

/// `struct GBSIOLockstepNode` (minus the driver vtable and timing event, which
/// are the owning [`Gb`]'s `sio_driver` field and [`GbEvent::SioLockstep`]).
#[derive(Clone, Copy, Debug, Default)]
struct Node {
    next_event: i32,
    event_diff: i32,
    id: i32,
    transfer_finished: bool,
}

/// The frontend's view of one linked core (`MultiplayerController::Player`).
#[derive(Clone, Copy, Debug)]
struct Player {
    awake: i32,
    cycles_posted: i32,
    wait_mask: u32,
    thread: ThreadWait,
}

impl Default for Player {
    fn default() -> Self {
        Player { awake: 1, cycles_posted: 0, wait_mask: 0, thread: ThreadWait::default() }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    node: Node,
    player: Player,
}

/// `struct GBSIOLockstep`, including its `struct mLockstep` base and the
/// frontend's per-player bookkeeping. Shared between the linked [`Gb`]s.
#[derive(Debug)]
pub struct GbSioLockstep {
    // struct mLockstep
    attached: i32,
    transfer_active: LockstepPhase,
    #[allow(dead_code)] // set like mGBA's, which never reads it for the GB either
    transfer_cycles: i32,
    // struct GBSIOLockstep
    /// Slot of the node with each id (`players[]`).
    players: [Option<usize>; MAX_GBS],
    pending_sb: [u8; MAX_GBS],
    master_claimed: bool,
    slots: Vec<Option<Slot>>,
}

/// A link-port driver: one node of a [`GbSioLockstep`] (`struct GBSIODriver`).
#[derive(Clone, Debug)]
pub struct GbSioDriver {
    lockstep: Arc<Mutex<GbSioLockstep>>,
    slot: usize,
}

impl GbSioLockstep {
    /// `mLockstepInit` + `GBSIOLockstepInit`
    pub fn new() -> Arc<Mutex<GbSioLockstep>> {
        Arc::new(Mutex::new(GbSioLockstep {
            attached: 0,
            transfer_active: LockstepPhase::Idle,
            transfer_cycles: 0,
            players: [None; MAX_GBS],
            pending_sb: [0xFF; MAX_GBS],
            master_claimed: false,
            slots: Vec::new(),
        }))
    }

    /// Number of attached Game Boys.
    pub fn attached(&self) -> usize {
        self.attached as usize
    }

    fn node(&mut self, slot: usize) -> &mut Node {
        &mut self.slots[slot].as_mut().expect("lockstep node").node
    }

    /// `MultiplayerController::player(id)`. mGBA's Qt frontend orders its list
    /// by node id only when a game attaches or detaches, so after the clock
    /// owner changes hands mid-session it can address the wrong thread; this
    /// always looks the player up by the node's current id.
    fn player(&mut self, id: i32) -> Option<&mut Player> {
        let slot = (*self.players.get(id as usize)?)?;
        self.slots[slot].as_mut().map(|s| &mut s.player)
    }

    // ----- struct mLockstep callbacks (MultiplayerController.cpp) -----

    /// `m_lockstep.signal`
    fn signal(&mut self, mask: u32) -> bool {
        let Some(player) = self.player(0) else { return false };
        let mut woke = false;
        player.wait_mask &= !mask;
        if player.wait_mask == 0 && player.awake < 1 {
            player.thread.stop_waiting();
            player.awake = 1;
            woke = true;
        }
        woke
    }

    /// `m_lockstep.wait`
    fn wait(&mut self, mask: u32) -> bool {
        let Some(player) = self.player(0) else { return false };
        let mut slept = false;
        player.wait_mask |= mask;
        if player.awake > 0 {
            player.thread.wait_from_thread();
            player.awake = 0;
            slept = true;
        }
        slept
    }

    /// `m_lockstep.addCycles`
    fn add_cycles(&mut self, id: i32, cycles: i32) {
        assert!(cycles >= 0, "lockstep: negative cycle count");
        if id == 0 {
            let Some(&Some(slot)) = self.players.get(1) else { return };
            let s = self.slots[slot].as_mut().expect("lockstep node");
            s.player.cycles_posted += cycles;
            if s.player.awake < 1 {
                s.node.next_event += s.player.cycles_posted;
            }
            s.player.thread.stop_waiting();
            s.player.awake = 1;
        } else if let Some(player) = self.player(id) {
            player.cycles_posted += cycles;
        }
    }

    /// `m_lockstep.useCycles`
    fn use_cycles(&mut self, id: i32, cycles: i32) -> i32 {
        let Some(player) = self.player(id) else { return 0 };
        player.cycles_posted -= cycles;
        if player.cycles_posted <= 0 {
            player.thread.wait_from_thread();
            player.awake = 0;
        }
        player.cycles_posted
    }

    /// `m_lockstep.unusedCycles`
    fn unused_cycles(&mut self, id: i32) -> i32 {
        self.player(id).map_or(0, |p| p.cycles_posted)
    }

    /// `m_lockstep.unload`
    fn unload(&mut self, id: i32) {
        if id != 0 {
            if let Some(player) = self.player(id) {
                player.cycles_posted = 0;
            }
            // release master GBA if it is waiting for this GBA
            if let Some(player) = self.player(0) {
                player.wait_mask &= !(1 << id);
                if player.wait_mask == 0 && player.awake < 1 {
                    player.thread.stop_waiting();
                    player.awake = 1;
                }
            }
        } else {
            let master_diff = match self.players[0] {
                Some(slot) => self.node(slot).event_diff,
                None => 0,
            };
            for i in 1..self.attached {
                let Some(slot) = self.players[i as usize] else { continue };
                let s = self.slots[slot].as_mut().expect("lockstep node");
                s.player.cycles_posted += master_diff;
                if s.player.awake < 1 {
                    s.node.next_event += s.player.cycles_posted;
                    s.player.thread.stop_waiting();
                    s.player.awake = 1;
                }
            }
        }
    }

    // ----- lockstep.c -----

    /// `GBSIOLockstepAttachNode` (with `GBSIOLockstepNodeCreate`)
    fn attach_node(lockstep: &Arc<Mutex<GbSioLockstep>>) -> Option<GbSioDriver> {
        let mut ls = lockstep.lock().unwrap();
        if ls.attached as usize == MAX_GBS {
            return None;
        }
        let slot = match ls.slots.iter().position(|s| s.is_none()) {
            Some(i) => i,
            None => {
                ls.slots.push(None);
                ls.slots.len() - 1
            }
        };
        let id = ls.attached;
        ls.slots[slot] = Some(Slot { node: Node { id, ..Node::default() }, player: Player::default() });
        ls.players[id as usize] = Some(slot);
        ls.attached += 1;
        Some(GbSioDriver { lockstep: lockstep.clone(), slot })
    }

    /// `GBSIOLockstepDetachNode`
    fn detach_node(&mut self, slot: usize) {
        if self.attached == 0 {
            return;
        }
        let attached = self.attached as usize;
        let mut i = 0;
        while i < attached {
            if self.players[i] != Some(slot) {
                i += 1;
                continue;
            }
            i += 1;
            while i < attached {
                self.players[i - 1] = self.players[i];
                let moved = self.players[i - 1].expect("lockstep node");
                self.node(moved).id = (i - 1) as i32;
                i += 1;
            }
            self.players[attached - 1] = None;
            self.attached -= 1;
            break;
        }
        self.slots[slot] = None;
    }
}

impl GbSioDriver {
    /// Whether this node's core has a wait pending (its thread would be asleep).
    pub fn waiting(&self) -> bool {
        let ls = self.lockstep.lock().unwrap();
        ls.slots[self.slot].as_ref().is_some_and(|s| s.player.thread.requested)
    }

    /// This node's player id (0 is the clock owner).
    pub fn id(&self) -> i32 {
        let ls = self.lockstep.lock().unwrap();
        ls.slots[self.slot].as_ref().map_or(-1, |s| s.node.id)
    }
}

impl Gb {
    /// Connect this Game Boy to a link cable (`GBSIOLockstepNodeCreate`,
    /// `GBSIOLockstepAttachNode` and `GBSIOSetDriver`, as mGBA's
    /// `MultiplayerController::attachGame` does). Fails when two are attached.
    pub fn attach_link(&mut self, lockstep: &Arc<Mutex<GbSioLockstep>>) -> bool {
        let Some(driver) = GbSioLockstep::attach_node(lockstep) else {
            return false;
        };
        self.sio_set_driver(Some(driver));
        true
    }

    /// Disconnect the link cable (`MultiplayerController::detachGame`).
    pub fn detach_link(&mut self) {
        let Some(driver) = self.sio_driver.clone() else { return };
        self.sio_set_driver(None);
        driver.lockstep.lock().unwrap().detach_node(driver.slot);
    }

    /// `GBSIOSetDriver`
    pub(crate) fn sio_set_driver(&mut self, driver: Option<GbSioDriver>) {
        if self.sio_driver.is_some() {
            self.sio_lockstep_node_deinit();
        }
        self.sio_driver = driver;
        if self.sio_driver.is_some() {
            self.sio_lockstep_node_init();
        }
    }

    /// `GBSIOLockstepNodeInit`
    pub(crate) fn sio_lockstep_node_init(&mut self) {
        let Some(driver) = self.sio_driver.clone() else { return };
        {
            let mut ls = driver.lockstep.lock().unwrap();
            let node = ls.node(driver.slot);
            glog!(self, LogLevel::Debug, "Lockstep {}: Node init", node.id);
            node.next_event = 0;
            node.event_diff = 0;
        }
        self.schedule(GbEvent::SioLockstep, 0);
    }

    /// `GBSIOLockstepNodeDeinit`
    pub(crate) fn sio_lockstep_node_deinit(&mut self) {
        let Some(driver) = self.sio_driver.clone() else { return };
        {
            let mut ls = driver.lockstep.lock().unwrap();
            let id = ls.node(driver.slot).id;
            ls.unload(id);
        }
        self.deschedule(GbEvent::SioLockstep);
    }

    /// `_finishTransfer`
    fn sio_lockstep_finish_transfer(&mut self, ls: &mut GbSioLockstep, slot: usize) {
        let node = *ls.node(slot);
        if node.transfer_finished {
            return;
        }
        self.sio.pending_sb = ls.pending_sb[(node.id == 0) as usize];
        if self.memory.io[REG_SC] & 0x80 != 0 {
            self.sio.remaining_bits = 8;
            self.deschedule(GbEvent::Sio);
            self.schedule(GbEvent::Sio, 0);
        }
        ls.node(slot).transfer_finished = true;
    }

    /// `_masterUpdate`
    fn sio_lockstep_master_update(&mut self, ls: &mut GbSioLockstep, slot: usize) -> i32 {
        let mut needs_to_wait = false;
        match ls.transfer_active {
            LockstepPhase::Idle => {
                // If the master hasn't initiated a transfer, it can keep going.
                ls.node(slot).next_event += LOCKSTEP_INCREMENT;
            }
            LockstepPhase::Starting => {
                // Start the transfer, but wait for the other GBs to catch up
                ls.node(slot).transfer_finished = false;
                needs_to_wait = true;
                ls.transfer_active = LockstepPhase::Started;
                ls.node(slot).next_event += 4;
            }
            LockstepPhase::Started => {
                // All the other GBs have caught up and are sleeping, we can all continue now
                ls.node(slot).next_event += 4;
                ls.transfer_active = LockstepPhase::Finishing;
            }
            LockstepPhase::Finishing => {
                // Finish the transfer
                // We need to make sure the other GBs catch up so they don't get behind
                // Split the cycles to avoid waiting too long
                ls.node(slot).next_event += self.sio.period * (2 - self.double_speed as i32) - 8;
                needs_to_wait = true;
                ls.transfer_active = LockstepPhase::Finished;
            }
            LockstepPhase::Finished => {
                // Everything's settled. We're done.
                self.sio_lockstep_finish_transfer(ls, slot);
                ls.master_claimed = false;
                ls.node(slot).next_event += LOCKSTEP_INCREMENT;
                ls.transfer_active = LockstepPhase::Idle;
            }
        }
        let mut mask = 0;
        for i in 1..ls.attached {
            mask |= 1 << i;
        }
        if mask != 0 {
            if needs_to_wait {
                // mGBA aborts if the master can't go to sleep.
                assert!(ls.wait(mask), "GB lockstep: master could not wait for the other Game Boy");
            } else {
                ls.signal(mask);
            }
        }
        // Tell the other GBs they can continue up to where we were
        let diff = ls.node(slot).event_diff;
        ls.add_cycles(0, diff);
        if needs_to_wait {
            return 0;
        }
        ls.node(slot).next_event
    }

    /// `_slaveUpdate`
    fn sio_lockstep_slave_update(&mut self, ls: &mut GbSioLockstep, slot: usize) -> i32 {
        let id = ls.node(slot).id;
        let event_diff = ls.node(slot).event_diff;
        let mut signal = false;
        match ls.transfer_active {
            LockstepPhase::Idle => ls.add_cycles(id, LOCKSTEP_INCREMENT),
            LockstepPhase::Starting | LockstepPhase::Finishing => {}
            LockstepPhase::Started => {
                if ls.unused_cycles(id) <= event_diff {
                    ls.node(slot).transfer_finished = false;
                    signal = true;
                }
            }
            LockstepPhase::Finished => {
                if ls.unused_cycles(id) <= event_diff {
                    self.sio_lockstep_finish_transfer(ls, slot);
                    signal = true;
                }
            }
        }
        if signal {
            ls.signal(1 << id);
        }
        0
    }

    /// `_GBSIOLockstepNodeProcessEvents`
    pub(crate) fn sio_lockstep_process_events(&mut self, cycles_late: u32) {
        let Some(driver) = self.sio_driver.clone() else { return };
        let slot = driver.slot;
        let mut guard = driver.lockstep.lock().unwrap();
        let ls = &mut *guard;
        if ls.attached < 2 {
            let when = (SIO_CYCLES_PER_TRANSFER[0] >> 1) * (2 - self.double_speed as i32) - cycles_late as i32;
            drop(guard);
            self.schedule(GbEvent::SioLockstep, when);
            return;
        }
        let mut cycles;
        ls.node(slot).next_event -= cycles_late as i32;
        if ls.node(slot).next_event <= 0 {
            let id = ls.node(slot).id;
            if id == 0 {
                cycles = self.sio_lockstep_master_update(ls, slot);
            } else {
                cycles = self.sio_lockstep_slave_update(ls, slot);
                let diff = ls.node(slot).event_diff;
                cycles += ls.use_cycles(id, diff);
            }
            ls.node(slot).event_diff = 0;
        } else {
            cycles = ls.node(slot).next_event;
        }

        if cycles > 0 {
            let node = ls.node(slot);
            node.next_event = 0;
            node.event_diff += cycles;
            drop(guard);
            self.deschedule(GbEvent::SioLockstep);
            self.schedule(GbEvent::SioLockstep, cycles);
        } else {
            drop(guard);
            self.interrupt();
            self.schedule(GbEvent::SioLockstep, cycles_late as i32 + 1);
        }
    }

    /// `GBSIOLockstepNodeWriteSB`
    pub(crate) fn sio_lockstep_write_sb(&mut self, value: u8) {
        let Some(driver) = self.sio_driver.clone() else { return };
        let mut ls = driver.lockstep.lock().unwrap();
        let id = ls.node(driver.slot).id;
        ls.pending_sb[id as usize] = value;
    }

    /// `GBSIOLockstepNodeWriteSC`
    pub(crate) fn sio_lockstep_write_sc(&mut self, value: u8) -> u8 {
        let Some(driver) = self.sio_driver.clone() else { return value };
        let slot = driver.slot;
        let mut ls = driver.lockstep.lock().unwrap();
        if (value & 0x81) == 0x81 && ls.attached > 1 {
            if !ls.master_claimed {
                ls.master_claimed = true;
                let id = ls.node(slot).id;
                if id != 0 {
                    let old_master = ls.players[0].expect("lockstep master");
                    ls.node(old_master).id = 1;
                    ls.node(slot).id = 0;
                    ls.players[1] = Some(old_master);
                    ls.players[0] = Some(slot);
                    ls.pending_sb.swap(0, 1);
                }
                ls.transfer_active = LockstepPhase::Starting;
                ls.transfer_cycles = SIO_CYCLES_PER_TRANSFER[((value >> 1) & 1) as usize];
                drop(ls);
                self.deschedule(GbEvent::Sio);
                self.deschedule(GbEvent::SioLockstep);
                self.schedule(GbEvent::SioLockstep, 0);
            } else {
                glog!(self, LogLevel::Debug, "GBSIOLockstepNodeWriteSC() failed to write to masterClaimed");
            }
        }
        value
    }

    /// The link-cable node this Game Boy is attached through, if any.
    pub fn link(&self) -> Option<&GbSioDriver> {
        self.sio_driver.as_ref()
    }
}

impl LinkedCore for Gb {
    /// `_GBCoreRunLoop`
    fn run_loop(&mut self) {
        crate::sm83::run(self);
    }

    fn link_waiting(&self) -> bool {
        self.sio_driver.as_ref().is_some_and(|d| d.waiting())
    }

    fn link_frame_counter(&self) -> u32 {
        self.video.frame_counter
    }
}
