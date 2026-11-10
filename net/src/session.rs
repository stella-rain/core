//! The rollback session: input queues, input delay, prediction, a save ring, rollback and
//! re-simulation within a window, all driven by local calls. It owns no transport, clock or
//! thread: the caller says when a tick is advanced and hands in what peers sent.
//!
//! What it does not do: it sends and receives nothing. The layer above it, [`party::NetSession`],
//! maps packets to [`Session::receive_inputs`], [`Session::ack`] and [`Session::leave`], holds
//! the handshake, balances frame advantage, exchanges checkpoints and stops on a desync.
use std::collections::VecDeque;
use std::fmt;

use stella_rain_core::engine::Engine;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::CHECKPOINT_TICKS;

use crate::predict::InputQueue;
use crate::save::{CloneRing, Save};
use crate::wire::{InputFrame, MAX_FRAME_INPUTS, WireInput};

pub mod party;
pub mod report;

/// The default window: the most ticks a session predicts past what it has confirmed.
pub const DEFAULT_WINDOW: u32 = 4;
/// The largest window a session accepts; it only bounds memory (one clone per tick of window).
pub const MAX_WINDOW: u32 = 16;
/// The default input delay, in ticks.
pub const DEFAULT_DELAY: u32 = 2;
/// The largest input delay a session accepts.
pub const MAX_DELAY: u32 = 16;

/// How a session is set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// The slot this session plays.
    pub local: PlayerIdx,
    /// Ticks between a local input being given and being stepped. Every peer of a party uses
    /// the same delay and window: a frame is accepted up to `tick + 2 * delay + window + 1`, so
    /// peers whose delay plus window differ by more than 2 refuse each other's inputs. Values
    /// of the party come from [`party::PartySettings`].
    pub delay: u32,
    /// The most ticks predicted past the confirmed one. A window of 0 is lockstep: it never
    /// rolls back, but it needs a delay of at least 1, because with delay 0 and a remote peer
    /// no input exists for the first tick before it is wanted.
    pub window: u32,
}

impl Config {
    /// The default delay and window for the slot `local`.
    pub fn new(local: PlayerIdx) -> Config {
        Config {
            local,
            delay: DEFAULT_DELAY,
            window: DEFAULT_WINDOW,
        }
    }
}

/// Why a [`Config`] is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// The engine has already run a tick.
    NotAtStart,
    /// The local slot is not in the engine's party.
    LocalAbsent,
    /// The window is over [`MAX_WINDOW`].
    WindowTooLarge,
    /// The delay is over [`MAX_DELAY`].
    DelayTooLarge,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ConfigError::NotAtStart => "the engine has already run a tick",
            ConfigError::LocalAbsent => "the local slot is not in the party",
            ConfigError::WindowTooLarge => "the window is over the maximum",
            ConfigError::DelayTooLarge => "the input delay is over the maximum",
        })
    }
}

impl std::error::Error for ConfigError {}

/// Why a call on a [`Session`] was refused. A refused call changes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// The save for a tick the window says is held is gone.
    SaveMissing,
    /// The slot is the local one, or not in the starting party.
    NotARemoteSlot,
    /// A frame's slot is not the sender's.
    SlotMismatch,
    /// An input at or after the sender's leave tick.
    SlotLeft,
    /// A tick further ahead than a peer within the window can be.
    TooFarAhead,
    /// A value different from the one already held.
    Conflict,
    /// An ack for inputs this session has not queued.
    AckBeyondSent,
    /// Slot 0 never leaves.
    FirstSlotLeaves,
    /// A leave at or below a tick the slot already sent an input for.
    LeaveAfterInputs,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SessionError::SaveMissing => "the save for a held tick is missing",
            SessionError::NotARemoteSlot => "the slot is not a remote slot of the party",
            SessionError::SlotMismatch => "the frame's slot is not the sender's",
            SessionError::SlotLeft => "an input at or after the slot's leave tick",
            SessionError::TooFarAhead => "a tick further ahead than the window allows",
            SessionError::Conflict => "a value different from the one held",
            SessionError::AckBeyondSent => "an ack beyond the inputs queued",
            SessionError::FirstSlotLeaves => "slot 0 never leaves",
            SessionError::LeaveAfterInputs => "a leave at or below a tick with an input",
        })
    }
}

impl std::error::Error for SessionError {}

/// Why an [`Session::advance`] did not step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    /// The window of predicted ticks is full.
    Window,
    /// A peer has not acknowledged the local inputs a frame can carry.
    Unacked,
}

/// What one [`Session::advance`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Advance {
    /// Ticks re-simulated by this call, also on a waiting call.
    pub rolled_back: u32,
    /// Why no tick was stepped, if none was.
    pub waiting: Option<Wait>,
}

/// One tick as it was simulated.
#[derive(Clone, Copy, Debug)]
struct Record {
    inputs: [WireInput; 3],
    leaves: u8,
    hash: u64,
}

/// One confirmed tick as it was simulated, kept by [`Session::keep_confirmed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfirmedTick {
    /// The tick number, from 0.
    pub tick: u32,
    /// The inputs stepped, one per slot (a left or absent slot's is the default).
    pub inputs: [WireInput; 3],
    /// The slots whose leave is stamped on this tick, bit `i` for slot `i`.
    pub leaves: u8,
    /// The state hash after this tick.
    pub hash: u64,
}

/// A rollback session over one engine.
pub struct Session<S: Save = CloneRing> {
    config: Config,
    save: S,
    engine: Engine,
    start_present: u8,
    tick: u32,
    confirmed: u32,
    queues: [InputQueue; 3],
    leaves: [Option<u32>; 3],
    acked: [u32; 3],
    records: VecDeque<Record>,
    record_base: u32,
    hashes: VecDeque<u64>,
    hash_base: u32,
    keep: bool,
    log: Vec<ConfirmedTick>,
}

impl Session<CloneRing> {
    /// A session over `engine`, at tick 0, with a ring of `config.window + 1` clones.
    pub fn new(engine: Engine, config: Config) -> Result<Session<CloneRing>, ConfigError> {
        let ring = usize::try_from(config.window)
            .unwrap_or(usize::MAX)
            .saturating_add(1);
        Session::with_save(engine, config, CloneRing::new(ring))
    }
}

impl<S: Save> Session<S> {
    /// A session over `engine` that keeps its states in `save`.
    pub fn with_save(engine: Engine, config: Config, save: S) -> Result<Session<S>, ConfigError> {
        if engine.tick() != 0 {
            return Err(ConfigError::NotAtStart);
        }
        let start_present = engine.present();
        if start_present >> config.local.get() & 1 == 0 {
            return Err(ConfigError::LocalAbsent);
        }
        if config.window > MAX_WINDOW {
            return Err(ConfigError::WindowTooLarge);
        }
        if config.delay > MAX_DELAY {
            return Err(ConfigError::DelayTooLarge);
        }
        let mut queues = [InputQueue::new(), InputQueue::new(), InputQueue::new()];
        for t in 0..config.delay {
            // The first `delay` ticks of the local slot are neutral; every peer is sent them.
            let _ = queues[config.local.get()].set(t, WireInput::default());
        }
        Ok(Session {
            config,
            save,
            hashes: VecDeque::from([engine.state_hash()]),
            engine,
            start_present,
            tick: 0,
            confirmed: 0,
            queues,
            leaves: [None; 3],
            acked: [0; 3],
            records: VecDeque::new(),
            record_base: 0,
            hash_base: 0,
            keep: false,
            log: Vec::new(),
        })
    }

    /// Queues the local input for tick `tick() + delay` and steps one tick, after repairing any
    /// tick a late input contradicts. Waits, changing nothing else, when the window is full or
    /// too many local inputs are unacknowledged.
    pub fn advance(&mut self, local: &Input) -> Result<Advance, SessionError> {
        let rolled_back = self.reconcile()?;
        self.settle();
        if !self.may_step() {
            return Ok(Advance {
                rolled_back,
                waiting: Some(Wait::Window),
            });
        }
        let local_slot = self.config.local.get();
        let queues_input =
            self.leaves[local_slot].is_none_or(|l| self.queues[local_slot].next_tick() < l);
        if queues_input && self.unacked_full() {
            return Ok(Advance {
                rolled_back,
                waiting: Some(Wait::Unacked),
            });
        }
        if queues_input {
            let queue = &mut self.queues[local_slot];
            let at = queue.next_tick();
            let _ = queue.set(at, WireInput::from_input(local));
        }
        self.step_tick(self.tick);
        self.tick += 1;
        self.settle();
        Ok(Advance {
            rolled_back,
            waiting: None,
        })
    }

    /// Repairs any tick a late input contradicts and confirms what is known, without stepping:
    /// returns the ticks re-simulated. A call for a peer that must not step (frame advantage,
    /// a finished peer).
    pub fn confirm(&mut self) -> Result<u32, SessionError> {
        let rolled_back = self.reconcile()?;
        self.settle();
        Ok(rolled_back)
    }

    /// Starts keeping the log of confirmed ticks, from the first one not yet confirmed. Off by
    /// default.
    pub fn keep_confirmed(&mut self) {
        self.keep = true;
    }

    /// Drains the log of confirmed ticks kept since the last call; empty unless
    /// [`Session::keep_confirmed`] was called.
    pub fn take_confirmed(&mut self) -> Vec<ConfirmedTick> {
        std::mem::take(&mut self.log)
    }

    /// Takes the inputs of the remote slot `from`. The frame is checked whole first: on an error
    /// nothing was inserted. An input older than what was pruned is ignored; one already held
    /// is a no-op if equal.
    pub fn receive_inputs(
        &mut self,
        from: PlayerIdx,
        frame: &InputFrame,
    ) -> Result<(), SessionError> {
        let s = self.remote_slot(from)?;
        if frame.slot != from {
            return Err(SessionError::SlotMismatch);
        }
        let Some(count) = u32::try_from(frame.inputs.len())
            .ok()
            .and_then(|n| n.checked_sub(1))
        else {
            return Ok(());
        };
        let last = frame
            .start_tick
            .checked_add(count)
            .ok_or(SessionError::TooFarAhead)?;
        if self.leaves[s].is_some_and(|l| last >= l) {
            return Err(SessionError::SlotLeft);
        }
        if last > self.farthest() {
            return Err(SessionError::TooFarAhead);
        }
        let ticks = frame.start_tick..=last;
        for (tick, input) in ticks.clone().zip(&frame.inputs) {
            self.queues[s]
                .check(tick, input)
                .map_err(|_| SessionError::Conflict)?;
        }
        for (tick, input) in ticks.zip(&frame.inputs) {
            let _ = self.queues[s].set(tick, *input);
        }
        Ok(())
    }

    /// The remote slot `from` has every local input below `next_tick`. An ack beyond the inputs
    /// queued is refused; one below the held ack is ignored.
    pub fn ack(&mut self, from: PlayerIdx, next_tick: u32) -> Result<(), SessionError> {
        let s = self.remote_slot(from)?;
        if next_tick > self.queues[self.config.local.get()].next_tick() {
            return Err(SessionError::AckBeyondSent);
        }
        self.acked[s] = self.acked[s].max(next_tick);
        Ok(())
    }

    /// The slot `from` leaves before tick `tick` is stepped; the local slot calls it with its
    /// own, and queues no input from that tick on. Slot 0 never leaves (the engine refuses it).
    /// A stamp behind the tick this session is on is a misprediction repaired by the next
    /// [`Session::advance`].
    pub fn leave(&mut self, from: PlayerIdx, tick: u32) -> Result<(), SessionError> {
        let s = from.get();
        if s == 0 {
            return Err(SessionError::FirstSlotLeaves);
        }
        if self.start_present >> s & 1 == 0 {
            return Err(SessionError::NotARemoteSlot);
        }
        if let Some(held) = self.leaves[s] {
            return if held == tick {
                Ok(())
            } else {
                Err(SessionError::Conflict)
            };
        }
        if self.queues[s].highest().is_some_and(|h| tick <= h) {
            return Err(SessionError::LeaveAfterInputs);
        }
        if tick > self.farthest() {
            return Err(SessionError::TooFarAhead);
        }
        self.leaves[s] = Some(tick);
        Ok(())
    }

    /// The local inputs from `from_tick`, at most [`crate::wire::MAX_FRAME_INPUTS`], or `None`
    /// when there are none or they were pruned. Inputs below the confirmed tick that every
    /// remote peer has acknowledged are dropped, so a session with no remote peer keeps only
    /// the inputs from its confirmed tick on.
    pub fn local_frame(&self, from_tick: u32) -> Option<InputFrame> {
        let queue = &self.queues[self.config.local.get()];
        if from_tick < queue.floor() {
            return None;
        }
        let inputs = queue.run_from(from_tick, MAX_FRAME_INPUTS);
        (!inputs.is_empty()).then_some(InputFrame {
            slot: self.config.local,
            start_tick: from_tick,
            inputs,
        })
    }

    /// The first tick of `slot`'s inputs not received; what an ack carries.
    pub fn next_tick(&self, slot: PlayerIdx) -> u32 {
        self.queues[slot.get()].next_tick()
    }

    /// Ticks simulated.
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// Ticks whose inputs are known for every present remote slot.
    pub fn confirmed(&self) -> u32 {
        self.confirmed
    }

    /// The engine, at the tick the session is on.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The state hash after `ticks` ticks, for a confirmed tick kept for
    /// `CHECKPOINT_TICKS` behind the confirmed one.
    pub fn hash_after(&self, ticks: u32) -> Option<u64> {
        if ticks > self.confirmed {
            return None;
        }
        let at = usize::try_from(ticks.checked_sub(self.hash_base)?).ok()?;
        self.hashes.get(at).copied()
    }

    /// The inputs stepped at tick `t` and the slots whose leave is stamped `t`, as best known.
    fn inputs_for(&self, t: u32) -> ([WireInput; 3], u8) {
        let mut inputs = [WireInput::default(); 3];
        let mut mask = 0u8;
        for (s, input) in inputs.iter_mut().enumerate() {
            if self.start_present >> s & 1 == 0 {
                continue;
            }
            match self.leaves[s] {
                Some(l) if l <= t => {}
                _ => *input = self.queues[s].predicted(t),
            }
            if self.leaves[s] == Some(t) {
                mask |= 1 << s;
            }
        }
        (inputs, mask)
    }

    /// Simulates tick `t` and records it, in the order a recorder writes: the state is saved,
    /// the leaves stamped `t` happen, the actions are made in slot order (a refusal is ignored),
    /// then the tick steps on inputs with the action cleared.
    fn step_tick(&mut self, t: u32) {
        self.save.save(t, &self.engine);
        let (inputs, leaves) = self.inputs_for(t);
        for s in 0..3u8 {
            if leaves >> s & 1 == 1
                && let Some(who) = PlayerIdx::new(s)
            {
                let _ = self.engine.leave(who);
            }
        }
        let mut stepped = [Input::default(); 3];
        for (s, wire) in inputs.iter().enumerate() {
            stepped[s] = wire.to_input();
            if let (Some(who), Some(action)) =
                (PlayerIdx::new(u8::try_from(s).unwrap_or(3)), wire.action)
                && self.engine.present() >> s & 1 == 1
            {
                let _ = self.engine.act(who, action);
            }
        }
        self.engine.step(&stepped);
        let record = Record {
            inputs,
            leaves,
            hash: self.engine.state_hash(),
        };
        self.put_record(t, record);
    }

    /// Writes the record and the hash of tick `t`, over the old ones on a re-simulation.
    fn put_record(&mut self, t: u32, record: Record) {
        let at = usize::try_from(t - self.record_base).unwrap_or(usize::MAX);
        if at < self.records.len() {
            self.records[at] = record;
        } else {
            self.records.push_back(record);
        }
        let hash_at = usize::try_from(t + 1 - self.hash_base).unwrap_or(usize::MAX);
        if hash_at < self.hashes.len() {
            self.hashes[hash_at] = record.hash;
        } else {
            self.hashes.push_back(record.hash);
        }
    }

    /// The slot index of `from` if it is a remote slot of the starting party.
    fn remote_slot(&self, from: PlayerIdx) -> Result<usize, SessionError> {
        let s = from.get();
        if s == self.config.local.get() || self.start_present >> s & 1 == 0 {
            return Err(SessionError::NotARemoteSlot);
        }
        Ok(s)
    }

    /// The last tick a peer within the window can have sent an input or a leave for.
    fn farthest(&self) -> u32 {
        self.tick
            .saturating_add(self.config.delay.saturating_mul(2))
            .saturating_add(self.config.window)
            .saturating_add(1)
    }

    /// The remote slots of the starting party.
    fn remote_slots(&self) -> impl Iterator<Item = usize> + '_ {
        let local = self.config.local.get();
        (0..3).filter(move |s| *s != local && self.start_present >> s & 1 == 1)
    }

    /// The remote slots still in the run at the tick this session is on.
    fn peers(&self) -> impl Iterator<Item = usize> + '_ {
        self.remote_slots()
            .filter(|s| self.leaves[*s].is_none_or(|l| l > self.tick))
    }

    /// The first tick whose inputs some remote slot has not sent; `None` with no remote slot.
    /// A slot with a stamped leave is known once it has sent every input below the stamp.
    fn known(&self) -> Option<u32> {
        self.remote_slots()
            .filter(|s| self.leaves[*s].is_none_or(|l| self.queues[*s].next_tick() < l))
            .map(|s| self.queues[s].next_tick())
            .min()
    }

    /// Whether queueing one more local input would leave a peer more unacknowledged inputs than
    /// a frame carries.
    fn unacked_full(&self) -> bool {
        let after = self.queues[self.config.local.get()]
            .next_tick()
            .saturating_add(1);
        self.peers().any(|s| {
            usize::try_from(after.saturating_sub(self.acked[s])).unwrap_or(usize::MAX)
                > MAX_FRAME_INPUTS
        })
    }

    /// Whether the next tick may be stepped: all its remote inputs are known, or fewer than
    /// `window` ticks are predicted already.
    fn may_step(&self) -> bool {
        self.known()
            .is_none_or(|k| self.tick < k || self.tick - k < self.config.window)
    }

    /// Re-simulates from the earliest tick whose stepped inputs differ from what is known now,
    /// and returns how many ticks that was.
    fn reconcile(&mut self) -> Result<u32, SessionError> {
        let first = (self.record_base..self.tick).find(|t| {
            let at = usize::try_from(*t - self.record_base).unwrap_or(usize::MAX);
            let (inputs, leaves) = self.inputs_for(*t);
            self.records
                .get(at)
                .is_none_or(|r| r.inputs != inputs || r.leaves != leaves)
        });
        let Some(from) = first else {
            return Ok(0);
        };
        if !self.save.load(from, &mut self.engine) {
            return Err(SessionError::SaveMissing);
        }
        for t in from..self.tick {
            self.step_tick(t);
        }
        Ok(self.tick - from)
    }

    /// Moves the confirmed tick and drops what is behind it.
    fn settle(&mut self) {
        let confirmed = self.known().map_or(self.tick, |k| k.min(self.tick));
        self.confirmed = self.confirmed.max(confirmed);
        while self.record_base < self.confirmed {
            if let Some(r) = self.records.pop_front()
                && self.keep
            {
                self.log.push(ConfirmedTick {
                    tick: self.record_base,
                    inputs: r.inputs,
                    leaves: r.leaves,
                    hash: r.hash,
                });
            }
            self.record_base += 1;
        }
        let keep_from = self.confirmed.saturating_sub(CHECKPOINT_TICKS);
        while self.hash_base < keep_from {
            self.hashes.pop_front();
            self.hash_base += 1;
        }
        self.save.forget_before(self.confirmed);
        let acked = self
            .peers()
            .map(|s| self.acked[s])
            .min()
            .map_or(self.confirmed, |a| a.min(self.confirmed));
        let local = self.config.local.get();
        for (s, queue) in self.queues.iter_mut().enumerate() {
            queue.prune_below(if s == local { acked } else { self.confirmed });
        }
    }
}
