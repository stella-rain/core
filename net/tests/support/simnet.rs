//! A seeded, test-only simulated network behind the `Transport` trait: latency, jitter, loss,
//! reordering and duplication, counted in rounds and never in clock time.
//!
//! It is not a session layer: it carries opaque messages between 2 or 3 slots and nothing more.
#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use stella_rain_core::id::PlayerIdx;
use stella_rain_net::transport::{Transport, TransportError};

/// What the network does to a message. Every rate is in permille (0 to 1000).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Impair {
    /// Rounds every copy takes at least.
    pub latency: u32,
    /// Each copy adds `0..=jitter` rounds, uniformly.
    pub jitter: u32,
    /// Chance in permille that a message vanishes.
    pub loss: u16,
    /// Chance in permille that a copy is held back `1..=reorder_extra` extra rounds.
    pub reorder: u16,
    /// The most extra rounds a reordered copy is held back.
    pub reorder_extra: u32,
    /// Chance in permille that a second copy is sent, delayed on its own.
    pub duplicate: u16,
}

impl Impair {
    /// Only `latency` rounds of delay: no jitter, loss, reordering or duplication.
    pub fn clean(latency: u32) -> Impair {
        Impair {
            latency,
            jitter: 0,
            loss: 0,
            reorder: 0,
            reorder_extra: 4,
            duplicate: 0,
        }
    }
}

/// What the network did so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Sends that were accepted.
    pub sent: u64,
    /// Messages that vanished.
    pub lost: u64,
    /// Extra copies queued.
    pub duplicated: u64,
    /// Copies `recv` returned.
    pub delivered: u64,
    /// Deliveries with a lower `seq` than one already delivered on the same link.
    pub reordered: u64,
}

/// One delivery, as `recv` returned it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delivery {
    /// The round `recv` ran in.
    pub round: u32,
    /// The sender's slot.
    pub from: u8,
    /// The receiver's slot.
    pub to: u8,
    /// The copy's number over every copy ever queued.
    pub seq: u64,
    /// The message length in bytes.
    pub len: usize,
}

struct Inner {}

/// The network: the clock (`step_round`) and the books (`stats`, `trace`).
pub struct SimNet {
    inner: Rc<RefCell<Inner>>,
}

/// One slot's handle on the network; clone it freely.
#[derive(Clone)]
pub struct SimEndpoint {
    slot: u8,
    inner: Rc<RefCell<Inner>>,
}

impl SimNet {
    /// A network for 2 or 3 peers; a permille above 1000 or another peer count panics.
    pub fn new(peers: u8, seed: u64, impair: Impair) -> SimNet {
        todo!("{peers} {seed} {impair:?}")
    }

    /// The handle of the peer in `slot`.
    pub fn endpoint(&self, slot: u8) -> SimEndpoint {
        todo!("{slot}")
    }

    /// Moves time on by one round.
    pub fn step_round(&self) {
        todo!()
    }

    /// The current round, from 0.
    pub fn round(&self) -> u32 {
        todo!()
    }

    /// The counters so far.
    pub fn stats(&self) -> Stats {
        todo!()
    }

    /// Every delivery so far, in the order `recv` returned them.
    pub fn trace(&self) -> Vec<Delivery> {
        todo!()
    }
}

impl Transport for SimEndpoint {
    fn send(&mut self, to: PlayerIdx, packet: &[u8]) -> Result<(), TransportError> {
        todo!("{} {to:?} {}", self.slot, packet.len())
    }

    fn recv(&mut self) -> Option<(PlayerIdx, Vec<u8>)> {
        todo!("{}", self.slot)
    }
}
