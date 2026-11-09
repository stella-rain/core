//! The rollback session: input queues, input delay, prediction, a save ring, rollback and
//! re-simulation within a window, all driven by local calls. It owns no transport, clock or
//! thread: the caller says when a tick is advanced and hands in what peers sent.
//!
//! What it does not do: it sends and receives nothing (a later layer maps packets to
//! [`Session::receive_inputs`], [`Session::ack`] and [`Session::leave`]), it holds no handshake,
//! it does not balance frame advantage between peers, it does not exchange checkpoints and it
//! does not stop on a desync.
#![allow(dead_code)]

use std::fmt;

use stella_rain_core::engine::Engine;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;

use crate::predict::InputQueue;
use crate::save::{CloneRing, Save};
use crate::wire::{InputFrame, WireInput};

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
    records: std::collections::VecDeque<Record>,
    record_base: u32,
    hashes: std::collections::VecDeque<u64>,
    hash_base: u32,
}

impl Session<CloneRing> {
    /// A session over `engine`, at tick 0, with a ring of `config.window + 1` clones.
    pub fn new(_engine: Engine, _config: Config) -> Result<Session<CloneRing>, ConfigError> {
        todo!()
    }
}

impl<S: Save> Session<S> {
    /// A session over `engine` that keeps its states in `save`.
    pub fn with_save(
        _engine: Engine,
        _config: Config,
        _save: S,
    ) -> Result<Session<S>, ConfigError> {
        todo!()
    }

    /// Queues the local input for tick `tick() + delay` and steps one tick, after repairing any
    /// tick a late input contradicts. Waits, changing nothing else, when the window is full or
    /// too many local inputs are unacknowledged.
    pub fn advance(&mut self, _local: &Input) -> Result<Advance, SessionError> {
        todo!()
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
    /// when there are none or they were pruned.
    pub fn local_frame(&self, _from_tick: u32) -> Option<InputFrame> {
        todo!()
    }

    /// The first tick of `slot`'s inputs not received; what an ack carries.
    pub fn next_tick(&self, _slot: PlayerIdx) -> u32 {
        todo!()
    }

    /// Ticks simulated.
    pub fn tick(&self) -> u32 {
        todo!()
    }

    /// Ticks whose inputs are known for every present remote slot.
    pub fn confirmed(&self) -> u32 {
        todo!()
    }

    /// The engine, at the tick the session is on.
    pub fn engine(&self) -> &Engine {
        todo!()
    }

    /// The configuration.
    pub fn config(&self) -> &Config {
        todo!()
    }

    /// The state hash after `ticks` ticks, for a confirmed tick kept for
    /// `CHECKPOINT_TICKS` behind the confirmed one.
    pub fn hash_after(&self, _ticks: u32) -> Option<u64> {
        todo!()
    }
}
