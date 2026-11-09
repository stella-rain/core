//! The rollback session: input queues, input delay, prediction, a save ring, rollback and
//! re-simulation within a window, all driven by local calls. It owns no transport, clock or
//! thread: the caller says when a tick is advanced and hands in what peers sent.
//!
//! What it does not do: it sends and receives nothing (a later layer maps packets to
//! [`Session::receive_inputs`], [`Session::ack`] and [`Session::leave`]), it holds no handshake,
//! it does not balance frame advantage between peers, it does not exchange checkpoints and it
//! does not stop on a desync.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::fmt;

use stella_rain_core::engine::Engine;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::CHECKPOINT_TICKS;

use crate::predict::InputQueue;
use crate::save::{CloneRing, Save};
use crate::wire::{InputFrame, MAX_FRAME_INPUTS, WireInput};

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
    /// Ticks between a local input being given and being stepped.
    pub delay: u32,
    /// The most ticks predicted past the confirmed one.
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
        })
    }

    /// Queues the local input for tick `tick() + delay` and steps one tick, after repairing any
    /// tick a late input contradicts. Waits, changing nothing else, when the window is full or
    /// too many local inputs are unacknowledged.
    pub fn advance(&mut self, local: &Input) -> Result<Advance, SessionError> {
        let local_slot = self.config.local.get();
        let at = self.queues[local_slot].next_tick();
        let _ = self.queues[local_slot].set(at, WireInput::from_input(local));
        self.step_tick(self.tick);
        self.tick += 1;
        self.settle();
        Ok(Advance {
            rolled_back: 0,
            waiting: None,
        })
    }

    /// Takes the inputs of the remote slot `from`.
    pub fn receive_inputs(
        &mut self,
        _from: PlayerIdx,
        _frame: &InputFrame,
    ) -> Result<(), SessionError> {
        todo!()
    }

    /// The remote slot `from` has every local input below `next_tick`.
    pub fn ack(&mut self, _from: PlayerIdx, _next_tick: u32) -> Result<(), SessionError> {
        todo!()
    }

    /// The slot `from` leaves before tick `tick` is stepped.
    pub fn leave(&mut self, _from: PlayerIdx, _tick: u32) -> Result<(), SessionError> {
        todo!()
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

    /// Moves the confirmed tick and drops what is behind it.
    fn settle(&mut self) {
        self.confirmed = self.confirmed.max(self.tick);
        while self.record_base < self.confirmed {
            self.records.pop_front();
            self.record_base += 1;
        }
        let keep_from = self.confirmed.saturating_sub(CHECKPOINT_TICKS);
        while self.hash_base < keep_from {
            self.hashes.pop_front();
            self.hash_base += 1;
        }
        self.save.forget_before(self.confirmed);
        let local = self.config.local.get();
        for (s, queue) in self.queues.iter_mut().enumerate() {
            if s != local {
                queue.prune_below(self.confirmed);
            }
        }
        self.queues[local].prune_below(self.confirmed);
    }
}
