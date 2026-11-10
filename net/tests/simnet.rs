//! The simulated network: latency, jitter, loss, reordering and duplication are seeded, counted
//! in rounds, and the `Transport` contract holds under them.

mod support;

use stella_rain_net::transport::{Transport, TransportError};
use stella_rain_net::wire::MAX_PACKET_LEN;
use support::simnet::{Delivery, Impair, SimNet};
use support::slot;

/// Sends one message per round on link 0 -> 1 for `sends` rounds (the payload is the send
/// round), then `tail` more rounds; returns `(send round, delivery round, payload)` of each
/// copy received by slot 1, in the order received.
fn one_link(net: &SimNet, sends: u32, tail: u32) -> Vec<(u32, u32, Vec<u8>)> {
    let mut tx = net.endpoint(0);
    let mut rx = net.endpoint(1);
    let mut got = Vec::new();
    for _ in 0..sends + tail {
        let round = net.round();
        if round < sends {
            tx.send(slot(1), &round.to_le_bytes()).expect("sent");
        }
        while let Some((from, bytes)) = rx.recv() {
            assert_eq!(from, slot(0));
            let sent = u32::from_le_bytes(bytes[..4].try_into().expect("4 bytes"));
            got.push((sent, round, bytes));
        }
        net.step_round();
    }
    got
}

#[test]
fn latency_holds_a_message_for_that_many_rounds() {
    let net = SimNet::new(2, 1, Impair::clean(3));
    let mut tx = net.endpoint(0);
    let mut rx = net.endpoint(1);
    tx.send(slot(1), b"hi").expect("sent");
    for _ in 0..3 {
        assert_eq!(rx.recv(), None);
        net.step_round();
    }
    assert_eq!(rx.recv(), Some((slot(0), b"hi".to_vec())));
    assert_eq!(rx.recv(), None);
}

#[test]
fn zero_latency_is_readable_in_the_same_round_and_keeps_send_order() {
    let net = SimNet::new(2, 1, Impair::clean(0));
    let mut tx = net.endpoint(0);
    let mut rx = net.endpoint(1);
    tx.send(slot(1), b"a").expect("sent");
    tx.send(slot(1), b"b").expect("sent");
    assert_eq!(rx.recv(), Some((slot(0), b"a".to_vec())));
    assert_eq!(rx.recv(), Some((slot(0), b"b".to_vec())));
    assert_eq!(rx.recv(), None);
}

#[test]
fn jitter_spreads_delays_and_reorders_by_itself() {
    let net = SimNet::new(
        2,
        1,
        Impair {
            jitter: 3,
            ..Impair::clean(2)
        },
    );
    let got = one_link(&net, 400, 10);
    assert_eq!(got.len(), 400);
    let mut delays: Vec<u32> = got.iter().map(|(s, d, _)| d - s).collect();
    delays.sort_unstable();
    delays.dedup();
    assert_eq!(delays, vec![2, 3, 4, 5]);
    let stats = net.stats();
    assert_eq!((stats.lost, stats.duplicated), (0, 0));
    assert_eq!(stats.delivered, 400);
    assert!(stats.reordered > 0);
}

#[test]
fn loss_removes_messages_at_the_set_rate() {
    let all = SimNet::new(
        2,
        1,
        Impair {
            loss: 1000,
            ..Impair::clean(1)
        },
    );
    assert!(one_link(&all, 200, 5).is_empty());
    assert_eq!((all.stats().sent, all.stats().lost), (200, 200));

    let none = SimNet::new(2, 1, Impair::clean(1));
    assert_eq!(one_link(&none, 200, 5).len(), 200);

    let some = SimNet::new(
        2,
        1,
        Impair {
            loss: 300,
            ..Impair::clean(1)
        },
    );
    let got = one_link(&some, 5000, 5).len();
    assert!((3_300..=3_700).contains(&got), "delivered {got}");
    assert_eq!(some.stats().lost, 5000 - got as u64);
}

#[test]
fn reorder_holds_some_copies_back_and_a_later_one_overtakes() {
    let net = SimNet::new(
        2,
        1,
        Impair {
            reorder: 500,
            reorder_extra: 4,
            ..Impair::clean(2)
        },
    );
    let got = one_link(&net, 400, 10);
    assert_eq!(got.len(), 400);
    let delays: Vec<u32> = got.iter().map(|(s, d, _)| d - s).collect();
    assert!(delays.iter().all(|d| (2..=6).contains(d)), "{delays:?}");
    assert!(delays.contains(&2) && delays.iter().any(|d| *d > 2));
    assert!(net.stats().reordered > 0);

    let control = SimNet::new(2, 1, Impair::clean(2));
    let got = one_link(&control, 400, 10);
    assert_eq!(control.stats().reordered, 0);
    let order: Vec<u32> = got.iter().map(|(s, _, _)| *s).collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn duplicate_sends_a_second_copy_that_sorts_after_the_first() {
    let net = SimNet::new(
        2,
        1,
        Impair {
            duplicate: 1000,
            ..Impair::clean(1)
        },
    );
    let got = one_link(&net, 200, 5);
    assert_eq!(got.len(), 400);
    for sent in 0..200u32 {
        let copies: Vec<_> = got.iter().filter(|(s, _, _)| *s == sent).collect();
        assert_eq!(copies.len(), 2, "payload {sent}");
        assert_eq!(copies[0].2, copies[1].2);
    }
    let seqs: Vec<u64> = net.trace().iter().map(|d| d.seq).collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "a copy sorts after its original"
    );
    assert_eq!(net.stats().duplicated, 200);

    let off = SimNet::new(2, 1, Impair::clean(1));
    assert_eq!(one_link(&off, 200, 5).len(), 200);

    let some = SimNet::new(
        2,
        1,
        Impair {
            duplicate: 300,
            ..Impair::clean(1)
        },
    );
    let got = one_link(&some, 5000, 5).len();
    assert!((6_300..=6_700).contains(&got), "delivered {got}");
}

fn all_five() -> Impair {
    Impair {
        latency: 2,
        jitter: 3,
        loss: 100,
        reorder: 200,
        reorder_extra: 4,
        duplicate: 200,
    }
}

/// 3 peers, every peer sends one message to each other peer per round, for `rounds` rounds;
/// `extra` also sends on 2 -> 1.
fn chatter(seed: u64, rounds: u32, extra: bool) -> Vec<Delivery> {
    let net = SimNet::new(3, seed, all_five());
    let mut eps: Vec<_> = (0..3).map(|i| net.endpoint(i)).collect();
    for _ in 0..rounds {
        let round = net.round();
        for (from, ep) in eps.iter_mut().enumerate() {
            for to in 0..3u8 {
                if usize::from(to) == from || (!extra && from == 2 && to == 1) {
                    continue;
                }
                ep.send(slot(to), &round.to_le_bytes()).expect("sent");
            }
        }
        for ep in &mut eps {
            while ep.recv().is_some() {}
        }
        net.step_round();
    }
    net.trace()
}

#[test]
fn a_seed_fixes_the_run_and_another_seed_changes_it() {
    assert_eq!(chatter(7, 600, true), chatter(7, 600, true));
    assert_ne!(chatter(7, 600, true), chatter(8, 600, true));
}

#[test]
fn a_link_does_not_depend_on_traffic_on_other_links() {
    // Every message on 2 -> 1 is sent too in the second run's twin, so compare link 0 -> 1 with
    // and without it. `seq` counts every copy of the net, so compare round, len and order.
    let link = |t: Vec<Delivery>| -> Vec<(u32, usize)> {
        t.into_iter()
            .filter(|d| d.from == 0 && d.to == 1)
            .map(|d| (d.round, d.len))
            .collect()
    };
    let with = link(chatter(7, 600, true));
    let without = link(chatter(7, 600, false));
    assert!(!with.is_empty());
    assert_eq!(with, without);
}

#[test]
fn the_transport_contract_holds() {
    let net = SimNet::new(2, 1, Impair::clean(1));
    let mut a = net.endpoint(0);
    let mut b = net.endpoint(1);
    assert_eq!(a.send(slot(1), &vec![0; MAX_PACKET_LEN]), Ok(()));
    assert_eq!(net.stats().sent, 1);
    assert_eq!(
        a.send(slot(1), &vec![0; MAX_PACKET_LEN + 1]),
        Err(TransportError::TooLarge)
    );
    assert_eq!(net.stats().sent, 1);
    assert_eq!(a.send(slot(0), b"x"), Err(TransportError::Closed));
    assert_eq!(a.send(slot(2), b"x"), Err(TransportError::Closed));
    assert_eq!(b.recv(), None);
    net.step_round();
    // The slot comes from the path: bytes that claim another sender change nothing.
    let mut c = net.endpoint(1);
    c.send(slot(0), &[0x05, 2, 2, 2, 2, 2, 2]).expect("sent");
    assert_eq!(
        b.recv().map(|(from, bytes)| (from, bytes.len())),
        Some((slot(0), MAX_PACKET_LEN))
    );
    net.step_round();
    assert_eq!(
        a.recv().map(|(from, bytes)| (from, bytes.len())),
        Some((slot(1), 7))
    );
}

#[test]
#[should_panic(expected = "permille")]
fn a_permille_above_1000_panics() {
    let _ = SimNet::new(
        2,
        1,
        Impair {
            loss: 1001,
            ..Impair::clean(1)
        },
    );
}
