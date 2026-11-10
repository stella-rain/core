//! The net session: a [`Session`] behind a [`Transport`], with the handshake, the redundant send
//! and ack of inputs, frame advantage, the checkpoint exchange and the stop on desync.

use std::marker::PhantomData;

use stella_rain_core::engine::Engine;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;

use super::report::DesyncReport;
use super::{ConfigError, DEFAULT_DELAY, DEFAULT_WINDOW, Session, SessionError, Wait};
use crate::save::{CloneRing, Save};
use crate::transport::Transport;
use crate::wire::{Hello, PROTOCOL_VERSION};

/// How far ahead of a peer (in ticks of lead difference) a session may run before it waits.
pub const DEFAULT_MAX_SKEW: u32 = 4;
/// Every how many send rounds the newest checkpoints are sent.
pub const CHECKPOINT_SEND_EVERY: u32 = 4;
/// How many of the newest checkpoints each such send holds.
pub const CHECKPOINTS_IN_FLIGHT: usize = 2;
/// How many checkpoints are kept per slot.
pub const CHECKPOINTS_KEPT: usize = 8;

/// The one delay and window of a party. Every peer must be built from the same value: a frame
/// is accepted only up to `tick + 2 * delay + window + 1`, so peers whose delay plus window
/// differ by more than 2 refuse each other's inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartySettings {
    /// Ticks between a local input being given and being stepped.
    pub delay: u32,
    /// The most ticks predicted past the confirmed one.
    pub window: u32,
}

impl Default for PartySettings {
    fn default() -> PartySettings {
        PartySettings {
            delay: DEFAULT_DELAY,
            window: DEFAULT_WINDOW,
        }
    }
}

/// How a [`NetSession`] is set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetConfig {
    /// The slot this session plays.
    pub local: PlayerIdx,
    /// The delay and window of the party.
    pub settings: PartySettings,
    /// What this session says about itself in the handshake.
    pub hello: Hello,
    /// The frame-advantage skew at which a session waits; `u32::MAX` turns the wait off.
    pub max_skew: u32,
}

impl NetConfig {
    /// The default settings and skew for the slot `local`, with `hello`.
    pub fn new(local: PlayerIdx, hello: Hello) -> NetConfig {
        NetConfig {
            local,
            settings: PartySettings::default(),
            hello,
            max_skew: DEFAULT_MAX_SKEW,
        }
    }
}

/// Why a [`NetConfig`] is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetConfigError {
    /// The hello names protocol 0 or `sim_version` 0.
    BadHello,
    /// The inner session refused the settings.
    Session(ConfigError),
}

impl std::fmt::Display for NetConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetConfigError::BadHello => f.write_str("the hello names protocol or sim_version 0"),
            NetConfigError::Session(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for NetConfigError {}

/// Where a session is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for every peer's hello.
    Handshake,
    /// Playing.
    Playing,
    /// Stopped for good: a refusal or a desync.
    Stopped,
}

/// Why an [`NetSession::advance`] did not step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetWait {
    /// Some peer's hello is still missing.
    Handshake,
    /// This session runs too far ahead of a peer (frame advantage).
    Ahead,
    /// The window of predicted ticks is full.
    Window,
    /// A peer has not acknowledged the local inputs a frame can carry.
    Unacked,
}

impl From<Wait> for NetWait {
    fn from(wait: Wait) -> NetWait {
        match wait {
            Wait::Window => NetWait::Window,
            Wait::Unacked => NetWait::Unacked,
        }
    }
}

/// What one [`NetSession::advance`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetAdvance {
    /// Ticks re-simulated by this call.
    pub rolled_back: u32,
    /// Why no tick was stepped, if none was.
    pub waiting: Option<NetWait>,
}

/// What a peer's hello differs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The protocol version.
    Protocol,
    /// The simulation version.
    SimVersion,
    /// The content pack.
    Pack,
}

/// Why a session stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// This session refused the hello of `slot`.
    Refused {
        /// The peer.
        slot: PlayerIdx,
        /// What differed.
        why: Refusal,
    },
    /// The peer in the slot refused this session's hello.
    RefusedBy(PlayerIdx),
    /// This session's checkpoint hash differs from `slot`'s at `tick`.
    Desync {
        /// The checkpoint tick.
        tick: u32,
        /// The peer whose hash differed.
        slot: PlayerIdx,
    },
    /// The peer in `slot` announced a desync at `tick`.
    PeerDesync {
        /// The announcing peer.
        slot: PlayerIdx,
        /// The tick it announced.
        tick: u32,
    },
}

/// Why a [`NetSession`] call failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetError {
    /// The session has stopped.
    Stopped,
    /// The inner session refused a local call.
    Session(SessionError),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::Stopped => f.write_str("the session has stopped"),
            NetError::Session(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for NetError {}

/// Counters of what the layer did with packets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetStats {
    /// Packets taken from a peer's known slot and accepted.
    pub accepted: u64,
    /// Messages that did not decode.
    pub bad_packets: u64,
    /// Packets the session refused (a [`SessionError`] or a bad ack).
    pub refused: u64,
    /// Packets dropped for a slot that is not a remote slot, or before the sender's hello.
    pub dropped: u64,
    /// Sends the transport failed.
    pub send_errors: u64,
    /// Checkpoints compared and equal.
    pub checkpoints_compared: u64,
}

/// A [`Session`] behind a [`Transport`].
pub struct NetSession<T: Transport, S: Save = CloneRing> {
    session: Session<S>,
    transport: T,
    stats: NetStats,
    _hello: PhantomData<Hello>,
}

impl<T: Transport> NetSession<T, CloneRing> {
    /// A net session over `engine` at tick 0.
    pub fn new(
        engine: Engine,
        config: NetConfig,
        transport: T,
    ) -> Result<NetSession<T, CloneRing>, NetConfigError> {
        let _ = (engine, config, transport, PROTOCOL_VERSION);
        todo!()
    }
}

impl<T: Transport, S: Save> NetSession<T, S> {
    /// A net session that keeps its states in `save`.
    pub fn with_save(
        engine: Engine,
        config: NetConfig,
        transport: T,
        save: S,
    ) -> Result<NetSession<T, S>, NetConfigError> {
        let _ = (engine, config, transport, save);
        todo!()
    }

    /// Receives, steps one tick if the handshake, the frame advantage and the window allow, and
    /// sends a round.
    pub fn advance(&mut self, local: &Input) -> Result<NetAdvance, NetError> {
        let _ = local;
        todo!()
    }

    /// Receives, confirms what can be confirmed and sends a round; steps nothing.
    pub fn poll(&mut self) -> Result<(), NetError> {
        todo!()
    }

    /// Announces the local leave at the first tick without a local input and returns that tick.
    pub fn leave(&mut self) -> Result<u32, NetError> {
        todo!()
    }

    /// The phase.
    pub fn phase(&self) -> Phase {
        todo!()
    }

    /// Why the session stopped, once it has.
    pub fn stop_reason(&self) -> Option<StopReason> {
        todo!()
    }

    /// The inner session.
    pub fn session(&self) -> &Session<S> {
        &self.session
    }

    /// The counters.
    pub fn stats(&self) -> NetStats {
        self.stats
    }

    /// The desync report, once the session stopped on a desync.
    pub fn desync_report(&self) -> Option<&DesyncReport> {
        todo!()
    }

    /// The transport, for a caller that owns its own pump.
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }
}
