// Copyright (c) 2013-2024 Jeffrey Pfau
// Rust port of mGBA's GBA link-cable lockstep driver (src/gba/sio/lockstep.c,
// include/mgba/internal/gba/sio/lockstep.h).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::sio::{transfer_cycles, SIO_GPIO, SIO_JOYBUS, SIO_MULTI, SIO_NORMAL_32, SIO_NORMAL_8, SIO_UART};
use super::{glog, regs, Gba, LogLevel};
use crate::lockstep::{LinkedCore, ThreadWait};
use crate::timing::EventId;

/// `MAX_GBAS`
pub const MAX_GBAS: usize = 4;
/// `MAX_LOCKSTEP_EVENTS`
pub const MAX_LOCKSTEP_EVENTS: usize = 8;

const LOCKSTEP_INTERVAL: i32 = 4096;
const UNLOCKED_INTERVAL: i32 = 4096;
const HARD_SYNC_INTERVAL: i32 = 0x80000;
const fn target(p: i32) -> u32 {
    1u32.wrapping_shl(p as u32)
}
const TARGET_ALL: u32 = 0xF;
const TARGET_PRIMARY: u32 = 0x1;
const TARGET_SECONDARY: u32 = TARGET_ALL & !TARGET_PRIMARY;

/// `enum GBASIOLockstepEventType`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EventType {
    Attach,
    Detach,
    HardSync,
    ModeSet,
    TransferStart,
}

/// `struct GBASIOLockstepEvent` (`mode` and `finishCycle` share a union in mGBA).
#[derive(Clone, Copy, Debug)]
struct LockstepEvent {
    ty: EventType,
    timestamp: i32,
    player_id: i32,
    mode: i32,
    finish_cycle: i32,
}

impl LockstepEvent {
    fn new(ty: EventType, player_id: i32, timestamp: i32) -> Self {
        LockstepEvent { ty, timestamp, player_id, mode: 0, finish_cycle: 0 }
    }
}

/// `struct GBASIOLockstepPlayer`, plus the `mLockstepUser` callbacks a
/// frontend gives each driver (`requestedId`, and `sleep`/`wake` acting on the
/// core's thread).
#[derive(Clone, Debug)]
struct Player {
    player_id: i32,
    mode: i32,
    other_modes: [i32; MAX_GBAS],
    asleep: bool,
    cycle_offset: i32,
    /// The event queue, ordered by timestamp (a linked list over a fixed
    /// buffer of `MAX_LOCKSTEP_EVENTS` in mGBA).
    queue: Vec<LockstepEvent>,
    data_received: bool,

    /// `user->requestedId`
    requested_id: i32,
    /// `user->sleep` / `user->wake` act on this.
    thread: ThreadWait,
    /// The core's clock as of its last run loop, for when a partner needs it.
    time: i32,
}

/// `struct GBASIOLockstepCoordinator`: the link shared by 2–4 [`Gba`]s.
#[derive(Debug, Default)]
pub struct GbaSioLockstepCoordinator {
    players: BTreeMap<u32, Player>,
    next_id: u32,
    attached_players: [u32; MAX_GBAS],
    n_attached: i32,
    waiting: u32,
    transfer_active: bool,
    transfer_mode: i32,
    cycle: i32,
    next_hard_sync: i32,
    multi_data: [u16; 4],
    normal_data: [u32; 4],
}

/// `struct GBASIOLockstepDriver`: a [`Gba`]'s link-port driver.
#[derive(Clone, Debug)]
pub struct GbaSioDriver {
    coordinator: Option<Arc<Mutex<GbaSioLockstepCoordinator>>>,
    lockstep_id: u32,
    /// What `user->requestedId` returns (a negative id opts out).
    requested_id: i32,
}

impl GbaSioDriver {
    /// `GBASIOLockstepDriverCreate`. `requested_id` is the player number this
    /// GBA asks for (mGBA's Qt frontend hands them out in the order games
    /// are attached); `None` takes whatever is left.
    pub fn new(requested_id: Option<i32>) -> Self {
        GbaSioDriver { coordinator: None, lockstep_id: 0, requested_id: requested_id.unwrap_or(MAX_GBAS as i32 - 1) }
    }

    fn coordinator(&self) -> Arc<Mutex<GbaSioLockstepCoordinator>> {
        self.coordinator.clone().expect("GBA lockstep driver without a coordinator")
    }
}

impl GbaSioLockstepCoordinator {
    /// `GBASIOLockstepCoordinatorInit`
    pub fn new() -> Arc<Mutex<GbaSioLockstepCoordinator>> {
        Arc::new(Mutex::new(GbaSioLockstepCoordinator::default()))
    }

    /// `GBASIOLockstepCoordinatorAttached`
    pub fn attached(&self) -> usize {
        self.players.len()
    }

    fn player(&mut self, pid: u32) -> &mut Player {
        self.players.get_mut(&pid).expect("lockstep player")
    }

    /// `_verifyAwake`
    fn verify_awake(&self) {
        if cfg!(debug_assertions) {
            let mut asleep = 0;
            for &pid in &self.attached_players[..self.n_attached.max(0) as usize] {
                if pid == 0 {
                    continue;
                }
                if let Some(p) = self.players.get(&pid) {
                    asleep += p.asleep as i32;
                }
            }
            debug_assert!(asleep == 0 || asleep < self.n_attached);
        }
    }

    /// `_abortTransfer`
    fn abort_transfer(&mut self, pid: u32) {
        // TODO: Do we need to clean this up better?
        self.transfer_active = false;
        self.waiting = 0;

        if self.player(pid).player_id != 0 {
            let runner = self.attached_players[0];
            if self.players.contains_key(&runner) {
                self.player_wake(runner);
            }
        } else {
            self.wake_players();
        }
    }

    /// `_untilNextSync`
    fn until_next_sync(&self, pid: u32, now: i32) -> i32 {
        let player = &self.players[&pid];
        let mut cycle = self.cycle.wrapping_sub(lockstep_time(player, now));
        if player.player_id == 0 {
            if self.n_attached < 2 {
                cycle = cycle.wrapping_add(UNLOCKED_INTERVAL);
            } else {
                cycle = cycle.wrapping_add(LOCKSTEP_INTERVAL);
            }
        }
        cycle
    }

    /// `_advanceCycle`
    fn advance_cycle(&mut self, pid: u32, now: i32) {
        let new_cycle = lockstep_time(&self.players[&pid], now);
        debug_assert!(new_cycle.wrapping_sub(self.cycle) >= 0);
        self.next_hard_sync = self.next_hard_sync.wrapping_sub(new_cycle.wrapping_sub(self.cycle));
        self.cycle = new_cycle;
    }

    /// `_removePlayer`. `now` is the removed player's clock.
    fn remove_player(&mut self, pid: u32, now: i32) {
        let player = &self.players[&pid];
        let event = LockstepEvent::new(EventType::Detach, player.player_id, lockstep_time(player, now));
        let player_id = player.player_id;
        self.enqueue_event(&event, TARGET_ALL & !target(player_id));

        self.waiting = 0;
        self.transfer_active = false;

        self.players.remove(&pid);
        self.reconfig_players(None);

        let runner = self.attached_players[0];
        if self.players.contains_key(&runner) {
            self.player_wake(runner);
        }
        self.verify_awake();
    }

    /// `_reconfigPlayers`. `caller` is the player whose core is running, with
    /// its clock; any other player's clock is read from where its core last
    /// stopped (mGBA reads it from the other thread).
    fn reconfig_players(&mut self, caller: Option<(u32, i32)>) {
        let players = self.players.len();
        self.attached_players = [0; MAX_GBAS];
        if players == 0 {
            // Reconfiguring player IDs with no players attached somehow?
        } else if players == 1 {
            let (&p0, player) = self.players.iter_mut().next().expect("one player");
            self.attached_players[0] = p0;
            self.cycle = match caller {
                Some((pid, now)) if pid == p0 => now,
                _ => player.time,
            };
            self.next_hard_sync = HARD_SYNC_INTERVAL;

            if player.player_id != 0 {
                player.player_id = 0;
            }

            if !self.transfer_active {
                self.transfer_mode = self.players[&p0].mode;
            }
        } else {
            let mut player_preferences: [Vec<u32>; MAX_GBAS] = Default::default();

            // Collect the first four players' requested player IDs so we can sort through them later
            let mut seen = 0;
            for (&pid, player) in &self.players {
                if seen >= MAX_GBAS {
                    break;
                }
                let mut requested = player.requested_id;
                if requested < 0 {
                    continue;
                }
                if requested >= MAX_GBAS as i32 {
                    requested = MAX_GBAS as i32 - 1;
                }
                player_preferences[requested as usize].push(pid);
                seen += 1;
            }

            // Now sort each requested player ID to figure out who gets which ID
            seen = 0;
            for i in 0..MAX_GBAS {
                for j in 0..=i {
                    while !player_preferences[j].is_empty() && seen < MAX_GBAS {
                        let pid = player_preferences[j].remove(0);
                        let Some(player) = self.players.get_mut(&pid) else {
                            continue;
                        };
                        self.attached_players[seen] = pid;
                        if player.player_id != seen as i32 {
                            player.player_id = seen as i32;
                        }
                        seen += 1;
                    }
                }
            }
        }

        let mut n_attached = 0;
        for i in 0..MAX_GBAS {
            let pid = self.attached_players[i];
            if pid == 0 {
                continue;
            }
            if !self.players.contains_key(&pid) {
                self.attached_players[i] = 0;
            } else {
                n_attached += 1;
            }
        }
        self.n_attached = n_attached;
    }

    /// `_enqueueEvent`
    fn enqueue_event(&mut self, event: &LockstepEvent, target_mask: u32) {
        for i in 0..self.n_attached.max(0) as usize {
            if target_mask & target(i as i32) == 0 {
                continue;
            }
            let pid = self.attached_players[i];
            let player = self.player(pid);
            if player.queue.len() >= MAX_LOCKSTEP_EVENTS {
                // mGBA asserts here ("No free events").
                debug_assert!(false, "GBA lockstep: no free events");
            }
            let pos = player
                .queue
                .iter()
                .position(|next| event.timestamp.wrapping_sub(next.timestamp) < 0)
                .unwrap_or(player.queue.len());
            player.queue.insert(pos, *event);
        }
    }

    /// `GBASIOLockstepCoordinatorWakePlayers`
    fn wake_players(&mut self) {
        for i in 1..self.n_attached.max(0) as usize {
            let pid = self.attached_players[i];
            if pid == 0 {
                continue;
            }
            self.player_wake(pid);
        }
    }

    /// `GBASIOLockstepPlayerWake`
    fn player_wake(&mut self, pid: u32) {
        let player = self.player(pid);
        if !player.asleep {
            return;
        }
        player.asleep = false;
        player.thread.stop_waiting();
    }
}

/// `GBASIOLockstepTime`, given the player's core clock.
fn lockstep_time(player: &Player, now: i32) -> i32 {
    now.wrapping_sub(player.cycle_offset)
}

impl Gba {
    /// Connect this GBA to a link cable (`GBASIOLockstepDriverCreate`,
    /// `GBASIOLockstepCoordinatorAttach` and `setPeripheral(mPERIPH_GBA_LINK_PORT)`).
    /// `requested_id` is the player number asked for (0–3).
    pub fn attach_link(&mut self, coordinator: &Arc<Mutex<GbaSioLockstepCoordinator>>, requested_id: Option<i32>) {
        let mut driver = GbaSioDriver::new(requested_id);
        driver.coordinator = Some(coordinator.clone());
        self.sio_set_driver(Some(driver));
    }

    /// Disconnect the link cable (`setPeripheral(NULL)` and
    /// `GBASIOLockstepCoordinatorDetach`).
    pub fn detach_link(&mut self) {
        self.sio_set_driver(None);
    }

    /// The link-cable driver, if one is attached.
    pub fn link(&self) -> Option<&GbaSioDriver> {
        self.sio_driver.as_ref()
    }

    /// This GBA's multiplayer id (0 is the clock owner), as the driver reports it.
    pub fn link_player_id(&mut self) -> i32 {
        self.sio_driver_device_id()
    }

    fn lockstep_handles(&self) -> Option<(Arc<Mutex<GbaSioLockstepCoordinator>>, u32)> {
        let d = self.sio_driver.as_ref()?;
        Some((d.coordinator(), d.lockstep_id))
    }

    /// `GBASIOLockstepPlayerSleep` (always called for the player whose core is running).
    fn lockstep_player_sleep(&mut self, c: &mut GbaSioLockstepCoordinator, pid: u32) {
        let player = c.player(pid);
        if player.asleep {
            return;
        }
        player.asleep = true;
        player.thread.wait_from_thread();
        self.cpu.next_event = 0;
        self.interrupt();
    }

    /// `GBASIOLockstepCoordinatorWaitOnPlayers`
    fn lockstep_wait_on_players(&mut self, c: &mut GbaSioLockstepCoordinator, pid: u32) {
        if c.waiting != 0 {
            glog!(self, LogLevel::Error, "Multiplayer desynchronized: coordinator still waiting");
        }
        if c.players[&pid].asleep {
            glog!(self, LogLevel::Error, "Multiplayer desynchronized: player asleep");
        }
        if c.players[&pid].player_id != 0 {
            glog!(
                self,
                LogLevel::Error,
                "Multiplayer desynchronized: invalid player {} attempting to coordinate",
                c.players[&pid].player_id
            );
        }
        if c.n_attached < 2 {
            return;
        }

        let now = self.current_time();
        c.advance_cycle(pid, now);
        glog!(self, LogLevel::Debug, "Primary waiting for players to ack");
        c.waiting = ((1u32 << c.n_attached) - 1) & !target(c.players[&pid].player_id);
        self.lockstep_player_sleep(c, pid);
        c.wake_players();

        c.verify_awake();
    }

    /// `GBASIOLockstepCoordinatorAckPlayer`
    fn lockstep_ack_player(&mut self, c: &mut GbaSioLockstepCoordinator, pid: u32) {
        let player_id = c.players[&pid].player_id;
        if player_id == 0 {
            return;
        }
        c.waiting &= !target(player_id);
        if c.waiting == 0 {
            glog!(self, LogLevel::Debug, "All players acked, waking primary");
            if c.transfer_active {
                for i in 0..c.n_attached.max(0) as usize {
                    let other = c.attached_players[i];
                    if other == 0 {
                        continue;
                    }
                    c.player(other).data_received = true;
                }

                c.transfer_active = false;
            }

            let runner = c.attached_players[0];
            c.player_wake(runner);
        }
        self.lockstep_player_sleep(c, pid);
    }

    /// `_setData`
    fn lockstep_set_data(&mut self, c: &mut GbaSioLockstepCoordinator, id: usize) {
        let io = &self.memory.io;
        match c.transfer_mode {
            SIO_MULTI => c.multi_data[id] = io[(regs::SIOMLT_SEND >> 1) as usize],
            SIO_NORMAL_8 => c.normal_data[id] = io[(regs::SIODATA8 >> 1) as usize] as u32,
            SIO_NORMAL_32 => {
                c.normal_data[id] = io[(regs::SIODATA32_LO >> 1) as usize] as u32;
                c.normal_data[id] |= (io[(regs::SIODATA32_HI >> 1) as usize] as u32) << 16;
            }
            SIO_UART | SIO_GPIO | SIO_JOYBUS => {
                glog!(self, LogLevel::Warn, "Unsupported mode {} in lockstep", c.transfer_mode);
                // TODO: Should we handle this or just abort?
            }
            _ => {}
        }
    }

    /// `_setReady`
    fn lockstep_set_ready(&mut self, c: &mut GbaSioLockstepCoordinator, pid: u32, player_id: i32, mode: i32) {
        debug_assert!((0..MAX_GBAS as i32).contains(&player_id));
        let n_attached = c.n_attached;
        let active = c.player(pid);
        active.other_modes[player_id as usize & 3] = mode;
        let mut ready = true;
        let mut i = 0;
        while ready && i < n_attached {
            ready = active.other_modes[i as usize] == active.mode;
            i += 1;
        }
        if active.mode == SIO_MULTI {
            self.sio.siocnt = (self.sio.siocnt & !(1 << 3)) | ((ready as u16) << 3);
            self.sio.rcnt = (self.sio.rcnt & !(1 << 1)) | ((ready as u16) << 1);
        }
    }

    /// `_hardSync`
    fn lockstep_hard_sync(&mut self, c: &mut GbaSioLockstepCoordinator, pid: u32) {
        debug_assert!(c.players[&pid].player_id == 0);
        let now = self.current_time();
        let event = LockstepEvent::new(EventType::HardSync, 0, lockstep_time(&c.players[&pid], now));
        c.enqueue_event(&event, TARGET_SECONDARY);
        self.lockstep_wait_on_players(c, pid);
    }

    // ----- struct GBASIODriver callbacks -----

    /// `GBASIOSetDriver`
    pub(crate) fn sio_set_driver(&mut self, driver: Option<GbaSioDriver>) {
        if self.sio_driver.is_some() {
            self.sio_driver_deinit();
        }
        self.sio_driver = driver;
        if self.sio_driver.is_some() {
            // GBASIOLockstepDriverInit
            self.sio_driver_reset();
        }
    }

    /// `GBASIOLockstepDriverDeinit`
    fn sio_driver_deinit(&mut self) {
        let Some((coordinator, pid)) = self.lockstep_handles() else { return };
        {
            let mut c = coordinator.lock().unwrap();
            if c.players.contains_key(&pid) {
                let now = self.current_time();
                c.remove_player(pid, now);
            }
        }
        self.deschedule(EventId::SioLockstep);
        if let Some(d) = self.sio_driver.as_mut() {
            d.lockstep_id = 0;
        }
    }

    /// `GBASIOLockstepDriverReset`
    pub(crate) fn sio_driver_reset(&mut self) {
        let Some(driver) = self.sio_driver.clone() else { return };
        let coordinator = driver.coordinator();
        let mut guard = coordinator.lock().unwrap();
        let c = &mut *guard;
        let now = self.current_time();
        let pid;
        if driver.lockstep_id == 0 {
            let player = Player {
                player_id: -1,
                mode: self.sio.mode,
                other_modes: [0; MAX_GBAS],
                asleep: false,
                cycle_offset: 0,
                queue: Vec::with_capacity(MAX_LOCKSTEP_EVENTS),
                data_received: false,
                requested_id: driver.requested_id,
                thread: ThreadWait::default(),
                time: now,
            };

            loop {
                if c.next_id == u32::MAX {
                    c.next_id = 0;
                }
                c.next_id += 1;
                let id = c.next_id;
                if !c.players.contains_key(&id) {
                    c.players.insert(id, player);
                    self.sio_driver.as_mut().unwrap().lockstep_id = id;
                    pid = id;
                    break;
                }
            }
            c.reconfig_players(Some((pid, now)));
            let cycle = c.cycle;
            let player = c.player(pid);
            player.cycle_offset = now.wrapping_sub(cycle);
            if player.player_id != 0 {
                let event = LockstepEvent::new(EventType::Attach, player.player_id, lockstep_time(player, now));
                let player_id = player.player_id;
                c.enqueue_event(&event, TARGET_ALL & !target(player_id));
            }
        } else {
            pid = driver.lockstep_id;
            let cycle = c.cycle;
            c.player(pid).cycle_offset = now.wrapping_sub(cycle);
        }

        if c.transfer_active {
            c.abort_transfer(pid);
            c.player(pid).asleep = false;
        }
        if c.players[&pid].player_id == 0 && c.n_attached > 1 {
            c.waiting = 0;
            // We will immediately go back to sleep when the initial mode gets set,
            // so we need to clear this here to avoid triggering an assert later.
            c.player(pid).asleep = false;
            c.wake_players();
        }

        if self.timing.is_scheduled(EventId::SioLockstep) {
            return;
        }

        let next_event;
        let (player_id, mode) = (c.players[&pid].player_id, c.players[&pid].mode);
        self.lockstep_set_ready(c, pid, player_id, mode);
        if c.players.len() == 1 {
            c.cycle = now;
            next_event = LOCKSTEP_INTERVAL;
        } else {
            let transfer_mode = c.transfer_mode;
            self.lockstep_set_ready(c, pid, 0, transfer_mode);
            next_event = c.until_next_sync(pid, now);
        }
        drop(guard);
        self.schedule(EventId::SioLockstep, next_event);
    }

    /// `GBASIOLockstepDriverSetMode`
    pub(crate) fn sio_driver_set_mode(&mut self, mode: i32) {
        let Some((coordinator, pid)) = self.lockstep_handles() else { return };
        let mut guard = coordinator.lock().unwrap();
        let c = &mut *guard;
        if mode != c.players[&pid].mode {
            glog!(self, LogLevel::Debug, "Switching mode from {} to {}", c.players[&pid].mode, mode);
            let now = self.current_time();
            let player = c.player(pid);
            player.mode = mode;
            let mut event = LockstepEvent::new(EventType::ModeSet, player.player_id, lockstep_time(player, now));
            event.mode = mode;
            let player_id = player.player_id;
            if player_id == 0 {
                c.transfer_mode = mode;
                self.lockstep_wait_on_players(c, pid);
            }
            self.lockstep_set_ready(c, pid, player_id, mode);
            c.enqueue_event(&event, TARGET_ALL & !target(player_id));
        }
    }

    /// `GBASIOLockstepDriverConnectedDevices`
    pub(crate) fn sio_driver_connected_devices(&self) -> i32 {
        let Some((coordinator, pid)) = self.lockstep_handles() else { return 0 };
        if pid == 0 {
            return 0;
        }
        let c = coordinator.lock().unwrap();
        c.n_attached - 1
    }

    /// `GBASIOLockstepDriverDeviceId`
    pub(crate) fn sio_driver_device_id(&self) -> i32 {
        let Some((coordinator, pid)) = self.lockstep_handles() else { return 0 };
        let c = coordinator.lock().unwrap();
        match c.players.get(&pid) {
            Some(p) if p.player_id >= 0 => p.player_id,
            _ => 0,
        }
    }

    /// `GBASIOLockstepDriverStart`
    pub(crate) fn sio_driver_start(&mut self) -> bool {
        let Some((coordinator, pid)) = self.lockstep_handles() else { return true };
        let mut guard = coordinator.lock().unwrap();
        let c = &mut *guard;
        if c.transfer_active {
            glog!(self, LogLevel::GameError, "Transfer restarted unexpectedly");
            return false;
        }
        if c.n_attached < 2 {
            glog!(self, LogLevel::Debug, "Attempted to start transfer with no secondary players");
            return false;
        }
        if c.players[&pid].player_id != 0 {
            glog!(self, LogLevel::Debug, "Secondary player attempted to start transfer");
            return false;
        }
        glog!(self, LogLevel::Debug, "Transfer starting at {:08X}", c.cycle);
        c.multi_data = [0xFFFF; 4];
        self.lockstep_set_data(c, 0);

        let now = self.current_time();
        let player = &c.players[&pid];
        let timestamp = lockstep_time(player, now);
        let mut event = LockstepEvent::new(EventType::TransferStart, 0, timestamp);
        event.finish_cycle = timestamp.wrapping_add(transfer_cycles(player.mode, self.sio.siocnt, c.n_attached - 1));
        c.enqueue_event(&event, TARGET_SECONDARY);
        self.lockstep_wait_on_players(c, pid);
        c.transfer_active = true;
        true
    }

    /// `GBASIOLockstepDriverFinishMultiplayer`
    pub(crate) fn sio_driver_finish_multiplayer(&mut self, data: &mut [u16; 4]) {
        let Some((coordinator, pid)) = self.lockstep_handles() else { return };
        let mut guard = coordinator.lock().unwrap();
        let c = &mut *guard;
        if c.transfer_mode == SIO_MULTI {
            if !c.players[&pid].data_received {
                glog!(self, LogLevel::Warn, "MULTI did not receive data. Are we running behind?");
                *data = [0xFFFF; 4];
            } else {
                glog!(
                    self,
                    LogLevel::Debug,
                    "MULTI transfer finished: {:04X} {:04X} {:04X} {:04X}",
                    c.multi_data[0],
                    c.multi_data[1],
                    c.multi_data[2],
                    c.multi_data[3]
                );
                *data = c.multi_data;
            }
            c.player(pid).data_received = false;
            if c.players[&pid].player_id == 0 {
                self.lockstep_hard_sync(c, pid);
            }
        }
    }

    /// `GBASIOLockstepDriverFinishNormal8`
    pub(crate) fn sio_driver_finish_normal8(&mut self) -> u8 {
        let Some((coordinator, pid)) = self.lockstep_handles() else { return 0 };
        let mut data = 0xFF;
        let mut guard = coordinator.lock().unwrap();
        let c = &mut *guard;
        if c.transfer_mode == SIO_NORMAL_8 {
            let player_id = c.players[&pid].player_id;
            if player_id > 0 {
                if !c.players[&pid].data_received {
                    glog!(self, LogLevel::Warn, "NORMAL did not receive data. Are we running behind?");
                } else {
                    data = c.normal_data[player_id as usize - 1] as u8;
                    glog!(self, LogLevel::Debug, "NORMAL8 transfer finished: {:02X}", data);
                }
            }
            c.player(pid).data_received = false;
            if player_id == 0 {
                self.lockstep_hard_sync(c, pid);
            }
        }
        data
    }

    /// `GBASIOLockstepDriverFinishNormal32`
    pub(crate) fn sio_driver_finish_normal32(&mut self) -> u32 {
        let Some((coordinator, pid)) = self.lockstep_handles() else { return 0 };
        let mut data = 0xFFFF_FFFF;
        let mut guard = coordinator.lock().unwrap();
        let c = &mut *guard;
        if c.transfer_mode == SIO_NORMAL_32 {
            let player_id = c.players[&pid].player_id;
            if player_id > 0 {
                if !c.players[&pid].data_received {
                    glog!(self, LogLevel::Warn, "Did not receive data. Are we running behind?");
                } else {
                    data = c.normal_data[player_id as usize - 1];
                    glog!(self, LogLevel::Debug, "NORMAL32 transfer finished: {:08X}", data);
                }
            }
            c.player(pid).data_received = false;
            if player_id == 0 {
                self.lockstep_hard_sync(c, pid);
            }
        }
        data
    }

    /// `_lockstepEvent`
    pub(crate) fn sio_lockstep_event(&mut self, cycles_late: u32) {
        let Some((coordinator, pid)) = self.lockstep_handles() else { return };
        let mut guard = coordinator.lock().unwrap();
        let c = &mut *guard;
        let now = self.current_time();
        let player_id = c.players[&pid].player_id;
        if !(0..4).contains(&player_id) {
            glog!(self, LogLevel::Error, "Invalid multiplayer ID {}", player_id);
        }

        let mut was_detach = false;
        if let Some(first) = c.players[&pid].queue.first() {
            if first.ty == EventType::Detach {
                glog!(
                    self,
                    LogLevel::Debug,
                    "Player {} detached at timestamp {:X}, picking up the pieces",
                    first.player_id,
                    first.timestamp
                );
                was_detach = true;
            }
        }
        if player_id == 0 && lockstep_time(&c.players[&pid], now).wrapping_sub(c.cycle) >= 0 {
            // We are the clock owner; advance the shared clock. However, if we just became
            // the clock owner (by the previous one disconnecting) we might be slightly
            // behind the shared clock. We should wait a bit if needed in that case.
            c.advance_cycle(pid, now);
            if !c.transfer_active {
                c.wake_players();
            }
            if c.next_hard_sync < 0 {
                if c.waiting == 0 {
                    self.lockstep_hard_sync(c, pid);
                }
                c.next_hard_sync = c.next_hard_sync.wrapping_add(HARD_SYNC_INTERVAL);
            }
        }

        let mut next_event = c.until_next_sync(pid, now);
        loop {
            let Some(&event) = c.players[&pid].queue.first() else { break };
            let time = lockstep_time(&c.players[&pid], now);
            if event.timestamp.wrapping_sub(time) > 0 {
                break;
            }
            c.player(pid).queue.remove(0);
            let player_id = c.players[&pid].player_id;
            let mut reply = LockstepEvent::new(EventType::ModeSet, player_id, time);
            glog!(
                self,
                LogLevel::Debug,
                "Got event of type {:?} from {} at timestamp {:X}",
                event.ty,
                event.player_id,
                event.timestamp
            );
            match event.ty {
                EventType::Attach => {
                    self.lockstep_set_ready(c, pid, event.player_id, -1);
                    if player_id == 0 {
                        self.sio.siocnt &= !(1 << 2); // GBASIOMultiplayerClearSlave
                    }
                    reply.mode = c.players[&pid].mode;
                    reply.ty = EventType::ModeSet;
                    c.enqueue_event(&reply, target(event.player_id));
                }
                EventType::HardSync => {
                    self.lockstep_ack_player(c, pid);
                }
                EventType::TransferStart => {
                    self.lockstep_set_data(c, player_id as usize);
                    next_event = event.finish_cycle.wrapping_sub(time).wrapping_sub(cycles_late as i32);
                    self.sio.siocnt |= 0x80;
                    self.deschedule(EventId::Sio);
                    self.schedule(EventId::Sio, next_event);
                    self.lockstep_ack_player(c, pid);
                }
                EventType::ModeSet => {
                    if c.transfer_active && c.players[&pid].mode != event.mode {
                        glog!(self, LogLevel::Debug, "Switching modes while transfer is active");
                        c.abort_transfer(pid);
                    }
                    self.lockstep_set_ready(c, pid, event.player_id, event.mode);
                    if event.player_id == 0 {
                        self.lockstep_ack_player(c, pid);
                    }
                }
                EventType::Detach => {
                    self.lockstep_set_ready(c, pid, event.player_id, -1);
                    let mode = c.players[&pid].mode;
                    self.lockstep_set_ready(c, pid, player_id, mode);
                    reply.mode = mode;
                    reply.ty = EventType::ModeSet;
                    c.enqueue_event(&reply, !target(event.player_id));
                    if mode == SIO_MULTI {
                        let id = (player_id as u16) & 3;
                        self.sio.siocnt = (self.sio.siocnt & !(3 << 4)) | (id << 4);
                        let slave = player_id != 0 || c.n_attached < 2;
                        self.sio.siocnt = (self.sio.siocnt & !(1 << 2)) | ((slave as u16) << 2);
                    }
                    was_detach = true;
                }
            }
        }
        let player = &c.players[&pid];
        let time = lockstep_time(player, now);
        if let Some(first) = player.queue.first() {
            if first.timestamp.wrapping_sub(time) < next_event {
                next_event = first.timestamp.wrapping_sub(time);
            }
        }

        if player.player_id != 0 && next_event <= LOCKSTEP_INTERVAL && (player.queue.is_empty() || was_detach) {
            self.lockstep_player_sleep(c, pid);
            // XXX: Is there a better way to gain sync lock at the beginning?
            if next_event < 4 {
                next_event = 4;
            }
            c.verify_awake();
        }
        drop(guard);

        debug_assert!(next_event > 0);
        self.schedule(EventId::SioLockstep, next_event);
    }
}

impl LinkedCore for Gba {
    /// `_GBACoreRunLoop`
    fn run_loop(&mut self) {
        crate::arm::run_loop(self);
    }

    fn link_waiting(&self) -> bool {
        let Some((coordinator, pid)) = self.lockstep_handles() else { return false };
        let c = coordinator.lock().unwrap();
        c.players.get(&pid).is_some_and(|p| p.thread.requested)
    }

    fn link_frame_counter(&self) -> u32 {
        self.video.frame_counter
    }

    fn link_publish_time(&mut self) {
        let Some((coordinator, pid)) = self.lockstep_handles() else { return };
        let now = self.current_time();
        let mut c = coordinator.lock().unwrap();
        if let Some(p) = c.players.get_mut(&pid) {
            p.time = now;
        }
    }
}
