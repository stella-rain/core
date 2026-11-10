//! `Transport` test doubles: a [`Tap`] that records what a session sent and holds what a test
//! wants it to receive; [`Scripted`] (a transport that is only a tap: a black hole that answers
//! with what the test queued) and [`Recording`] (wraps another transport and records what it
//! sent).
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use stella_rain_core::id::PlayerIdx;
use stella_rain_net::transport::{Transport, TransportError};
use stella_rain_net::wire::Packet;

#[derive(Default)]
struct Inner {
    sent: Vec<(PlayerIdx, Vec<u8>)>,
    inbox: VecDeque<(PlayerIdx, Vec<u8>)>,
    fail: Option<TransportError>,
}

/// A shared record of what one session sent and a queue of what it will receive.
#[derive(Clone, Default)]
pub struct Tap(Rc<RefCell<Inner>>);

impl Tap {
    /// A new, empty tap.
    pub fn new() -> Tap {
        Tap::default()
    }

    /// Queues raw bytes as arriving from `from`.
    pub fn push_bytes(&self, from: PlayerIdx, bytes: Vec<u8>) {
        self.0.borrow_mut().inbox.push_back((from, bytes));
    }

    /// Queues `packet` as arriving from `from`.
    pub fn push(&self, from: PlayerIdx, packet: &Packet) {
        self.push_bytes(from, packet.encode().expect("a short packet"));
    }

    /// Every message sent, decoded, with its destination.
    pub fn sent(&self) -> Vec<(PlayerIdx, Packet)> {
        self.0
            .borrow()
            .sent
            .iter()
            .map(|(to, b)| {
                (
                    *to,
                    Packet::decode(b).expect("the layer sends valid packets"),
                )
            })
            .collect()
    }

    /// The longest message sent.
    pub fn max_len(&self) -> usize {
        self.0
            .borrow()
            .sent
            .iter()
            .map(|(_, b)| b.len())
            .max()
            .unwrap_or(0)
    }

    /// How many messages were sent.
    pub fn count(&self) -> usize {
        self.0.borrow().sent.len()
    }

    /// From now on every send fails with `error` (`None`: succeeds again).
    pub fn fail_with(&self, error: Option<TransportError>) {
        self.0.borrow_mut().fail = error;
    }

    fn record(&self, to: PlayerIdx, bytes: &[u8]) {
        self.0.borrow_mut().sent.push((to, bytes.to_vec()));
    }
}

/// A transport that is a tap and nothing else: sends are recorded and go nowhere.
pub struct Scripted(pub Tap);

impl Transport for Scripted {
    fn send(&mut self, to: PlayerIdx, packet: &[u8]) -> Result<(), TransportError> {
        if let Some(e) = self.0.0.borrow().fail {
            return Err(e);
        }
        self.0.record(to, packet);
        Ok(())
    }

    fn recv(&mut self) -> Option<(PlayerIdx, Vec<u8>)> {
        self.0.0.borrow_mut().inbox.pop_front()
    }
}

/// A transport that records what `inner` sends.
pub struct Recording<T> {
    inner: T,
    tap: Tap,
}

impl<T> Recording<T> {
    /// Wraps `inner`, recording into `tap`.
    pub fn new(inner: T, tap: Tap) -> Recording<T> {
        Recording { inner, tap }
    }
}

impl<T: Transport> Transport for Recording<T> {
    fn send(&mut self, to: PlayerIdx, packet: &[u8]) -> Result<(), TransportError> {
        self.inner.send(to, packet)?;
        self.tap.record(to, packet);
        Ok(())
    }

    fn recv(&mut self) -> Option<(PlayerIdx, Vec<u8>)> {
        self.inner.recv()
    }
}
