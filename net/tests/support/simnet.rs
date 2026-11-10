//! A seeded, test-only simulated network behind the `Transport` trait: latency, jitter, loss,
//! reordering and duplication, counted in rounds and never in clock time.
//!
//! It is not a session layer: it carries opaque messages between 2 or 3 slots and nothing more.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use stella_rain_core::id::PlayerIdx;
use stella_rain_net::transport::{Transport, TransportError};
use stella_rain_net::wire::MAX_PACKET_LEN;

use super::Xorshift;

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
    /// Deliveries with a lower `seq` than one already delivered on the same link. A duplicate
    /// that arrives after a later message counts too, so read it with `duplicate` off to see
    /// reordering alone.
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

/// A copy in flight.
struct Queued {
    from: u8,
    to: u8,
    bytes: Vec<u8>,
}

struct Inner {
    peers: u8,
    impair: Impair,
    round: u32,
    seq: u64,
    /// One generator per directed link, indexed `3 * from + to`.
    rngs: Vec<Xorshift>,
    /// Copies in flight, ordered by `(due round, seq)`.
    queue: BTreeMap<(u32, u64), Queued>,
    /// The highest `seq` delivered on each link, indexed `3 * from + to`.
    last_seq: [Option<u64>; 9],
    stats: Stats,
    trace: Vec<Delivery>,
}

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
        assert!((2..=3).contains(&peers), "a simnet has 2 or 3 peers");
        assert!(
            impair.reorder == 0 || impair.reorder_extra >= 1,
            "a reordered copy needs at least one extra round"
        );
        for permille in [impair.loss, impair.reorder, impair.duplicate] {
            assert!(permille <= 1000, "a permille is at most 1000");
        }
        let rngs = (0..9u64)
            .map(|link| {
                let (from, to) = (link / 3, link % 3);
                Xorshift::new(seed ^ (1 + 3 * from + to).wrapping_mul(0x9E37_79B9_7F4A_7C15))
            })
            .collect();
        SimNet {
            inner: Rc::new(RefCell::new(Inner {
                peers,
                impair,
                round: 0,
                seq: 0,
                rngs,
                queue: BTreeMap::new(),
                last_seq: [None; 9],
                stats: Stats::default(),
                trace: Vec::new(),
            })),
        }
    }

    /// The handle of the peer in `slot`.
    pub fn endpoint(&self, slot: u8) -> SimEndpoint {
        assert!(slot < self.inner.borrow().peers, "no such peer");
        SimEndpoint {
            slot,
            inner: Rc::clone(&self.inner),
        }
    }

    /// Moves time on by one round.
    pub fn step_round(&self) {
        self.inner.borrow_mut().round += 1;
    }

    /// The current round, from 0.
    pub fn round(&self) -> u32 {
        self.inner.borrow().round
    }

    /// The counters so far.
    pub fn stats(&self) -> Stats {
        self.inner.borrow().stats
    }

    /// Every delivery so far, in the order `recv` returned them.
    pub fn trace(&self) -> Vec<Delivery> {
        self.inner.borrow().trace.clone()
    }
}

impl Transport for SimEndpoint {
    fn send(&mut self, to: PlayerIdx, packet: &[u8]) -> Result<(), TransportError> {
        if packet.len() > MAX_PACKET_LEN {
            return Err(TransportError::TooLarge);
        }
        let mut net = self.inner.borrow_mut();
        let to = u8::try_from(to.get()).expect("a slot is small");
        if to == self.slot || to >= net.peers {
            return Err(TransportError::Closed);
        }
        net.stats.sent += 1;
        let link = usize::from(3 * self.slot + to);
        let impair = net.impair;
        let lost = net.rngs[link].below(1000) < u64::from(impair.loss);
        if lost {
            net.stats.lost += 1;
            return Ok(());
        }
        let copies = if net.rngs[link].below(1000) < u64::from(impair.duplicate) {
            net.stats.duplicated += 1;
            2
        } else {
            1
        };
        for _ in 0..copies {
            let rng = &mut net.rngs[link];
            let mut delay = impair.latency;
            delay += u32::try_from(rng.below(u64::from(impair.jitter) + 1)).expect("small");
            if rng.below(1000) < u64::from(impair.reorder) {
                delay +=
                    1 + u32::try_from(rng.below(u64::from(impair.reorder_extra))).expect("small");
            }
            net.seq += 1;
            let key = (net.round + delay, net.seq);
            net.queue.insert(
                key,
                Queued {
                    from: self.slot,
                    to,
                    bytes: packet.to_vec(),
                },
            );
        }
        Ok(())
    }

    fn recv(&mut self) -> Option<(PlayerIdx, Vec<u8>)> {
        let mut net = self.inner.borrow_mut();
        let round = net.round;
        let key = net
            .queue
            .range(..=(round, u64::MAX))
            .find(|(_, q)| q.to == self.slot)
            .map(|(key, _)| *key)?;
        let queued = net.queue.remove(&key).expect("the key was just found");
        let link = usize::from(3 * queued.from + queued.to);
        net.stats.delivered += 1;
        if net.last_seq[link].is_some_and(|last| key.1 < last) {
            net.stats.reordered += 1;
        }
        net.last_seq[link] = net.last_seq[link].max(Some(key.1));
        net.trace.push(Delivery {
            round,
            from: queued.from,
            to: queued.to,
            seq: key.1,
            len: queued.bytes.len(),
        });
        let from = PlayerIdx::new(queued.from).expect("a slot below 3");
        Some((from, queued.bytes))
    }
}
