//! `n` rollback sessions that exchange real `Packet` bytes over a [`SimNet`], so a test can see
//! them converge on the straight run's hash under latency, jitter, loss, reordering and
//! duplication.
//!
//! It is not a session layer: there is no handshake, no frame advantage and no checkpoint
//! exchange, and the redundant send (every round, each peer's unacknowledged inputs, an ack and,
//! once announced, a bye) is a test stand-in for the one the session layer will own.
#![allow(dead_code)]

use stella_rain_net::session::{Config, Session, Wait};
use stella_rain_net::wire::InputFrame;

use super::simnet::{Impair, SimEndpoint, SimNet};
use super::{CallStat, Script};

/// The most rounds a run may take.
pub const ROUND_LIMIT: u32 = 4000;

/// A change made to a frame before it is encoded: `from`, `to`, the frame.
pub type FrameTamper = Box<dyn FnMut(usize, usize, &mut InputFrame)>;

/// The sessions, the network between them and the books of the run.
pub struct Peers {
    /// The sessions, by slot.
    pub sessions: Vec<Session>,
    /// Every `advance` call of each session.
    pub calls: Vec<Vec<CallStat>>,
    /// The network.
    pub net: SimNet,
    /// The script every peer plays.
    pub script: Script,
    /// Changes a frame as it is sent, if set.
    pub tamper: Option<FrameTamper>,
    eps: Vec<SimEndpoint>,
    delay: u32,
    stops: Vec<u32>,
    leaver: Option<(usize, u32)>,
    announced: bool,
    taken: Vec<u32>,
    acked: Vec<[u32; 3]>,
}

impl Peers {
    /// `n` sessions on `script` over a net with `seed` and `impair`: delay 2, window 4, each
    /// stopping at tick 612.
    pub fn new(script: Script, n: u8, seed: u64, impair: Impair) -> Peers {
        todo!("{} {n} {seed} {impair:?}", script.n)
    }

    /// Slot `who` leaves before tick `at`: it announces the leave `delay` calls ahead, stops
    /// advancing at `at` and keeps sending.
    pub fn leaver(self, who: usize, at: u32) -> Peers {
        todo!("{who} {at}")
    }

    /// Whether every session has stopped.
    pub fn done(&self) -> bool {
        todo!()
    }

    /// Runs rounds, [`Peers::check`] after each, until `done` or [`ROUND_LIMIT`]; whether it
    /// finished.
    pub fn run(&mut self, oracle: &[u64]) -> bool {
        todo!("{}", oracle.len())
    }

    /// One round.
    pub fn round(&mut self) {
        todo!()
    }

    /// The window holds and every confirmed hash is the straight run's.
    pub fn check(&self, oracle: &[u64]) {
        todo!("{}", oracle.len())
    }

    /// Whether any call re-simulated a tick that holds an action.
    pub fn rolled_over_an_action(&self) -> bool {
        todo!()
    }

    /// Whether any call of a session other than `except` re-simulated across `tick`: it began
    /// at or before `tick` and had stepped past it.
    pub fn rolled_across(&self, tick: u32, except: usize) -> bool {
        todo!("{tick} {except}")
    }

    /// The most ticks one call re-simulated.
    pub fn max_rolled(&self) -> u32 {
        todo!()
    }

    /// The ticks re-simulated over the run.
    pub fn total_rolled(&self) -> u64 {
        todo!()
    }

    /// How many calls waited, for any reason, or for `reason` only.
    pub fn waits(&self, reason: Option<Wait>) -> usize {
        todo!("{reason:?}")
    }
}
