//! The `Transport` interface, through a test-only loopback.

use std::collections::VecDeque;

use stella_rain_core::id::PlayerIdx;
use stella_rain_net::transport::{Transport, TransportError};
use stella_rain_net::wire::{Ack, MAX_PACKET_LEN, Packet};

/// A queue per slot; a sent message lands in the queue of the slot it is addressed to and is
/// received as coming from `from`.
struct Loopback {
    from: PlayerIdx,
    queues: [VecDeque<Vec<u8>>; 3],
    open: bool,
}

impl Loopback {
    fn new(from: PlayerIdx) -> Loopback {
        Loopback {
            from,
            queues: Default::default(),
            open: true,
        }
    }

    fn close(&mut self) {
        self.open = false;
    }
}

impl Transport for Loopback {
    fn send(&mut self, to: PlayerIdx, packet: &[u8]) -> Result<(), TransportError> {
        if !self.open {
            return Err(TransportError::Closed);
        }
        if packet.len() > MAX_PACKET_LEN {
            return Err(TransportError::TooLarge);
        }
        self.queues[to.get()].push_back(packet.to_vec());
        Ok(())
    }

    fn recv(&mut self) -> Option<(PlayerIdx, Vec<u8>)> {
        for q in &mut self.queues {
            if let Some(m) = q.pop_front() {
                return Some((self.from, m));
            }
        }
        None
    }
}

#[test]
fn one_message_in_is_one_message_out() {
    let mut l = Loopback::new(PlayerIdx::First);
    let t: &mut dyn Transport = &mut l;
    t.send(PlayerIdx::Second, &[1, 2, 3]).unwrap();
    assert_eq!(t.recv(), Some((PlayerIdx::First, vec![1, 2, 3])));
    assert_eq!(t.recv(), None);
}

#[test]
fn a_message_over_the_maximum_is_too_large() {
    let mut l = Loopback::new(PlayerIdx::First);
    let t: &mut dyn Transport = &mut l;
    assert_eq!(
        t.send(PlayerIdx::Second, &vec![0; MAX_PACKET_LEN + 1]),
        Err(TransportError::TooLarge)
    );
    assert_eq!(t.send(PlayerIdx::Second, &vec![0; MAX_PACKET_LEN]), Ok(()));
}

#[test]
fn a_closed_transport_refuses_to_send() {
    let mut l = Loopback::new(PlayerIdx::First);
    l.close();
    let t: &mut dyn Transport = &mut l;
    assert_eq!(t.send(PlayerIdx::Third, &[0]), Err(TransportError::Closed));
}

#[test]
fn an_encoded_packet_arrives_as_the_packet_that_was_sent() {
    let mut l = Loopback::new(PlayerIdx::Third);
    let t: &mut dyn Transport = &mut l;
    let ack = Packet::Ack(Ack {
        slot: PlayerIdx::Second,
        next_tick: 259,
    });
    t.send(PlayerIdx::First, &ack.encode().unwrap()).unwrap();
    let (from, bytes) = t.recv().unwrap();
    assert_eq!(from, PlayerIdx::Third);
    assert_eq!(Packet::decode(&bytes), Ok(ack));
}

#[test]
fn the_errors_display_and_are_errors() {
    for e in [TransportError::Closed, TransportError::TooLarge] {
        assert!(!e.to_string().is_empty());
        let _: &dyn std::error::Error = &e;
    }
}
