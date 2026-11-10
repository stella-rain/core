//! The net session layer: the handshake, the packet mapping, the redundant send and ack, one
//! delay and window per party, frame advantage and hostile bytes.

mod support;

use stella_rain_core::input::Input;
use stella_rain_net::session::SessionError;
use stella_rain_net::session::party::{
    NetAdvance, NetConfig, NetConfigError, NetError, NetSession, NetWait, PartySettings, Phase,
    Refusal, StopReason,
};
use stella_rain_net::transport::TransportError;
use stella_rain_net::wire::{
    Ack, Bye, ByeReason, Checkpoint, Hello, InputFrame, MAX_FRAME_INPUTS, MAX_PACKET_LEN, Packet,
    WireInput,
};
use support::peers::{Peers, hello};
use support::simnet::{Impair, SimEndpoint, SimNet};
use support::wiretap::{Recording, Scripted, Tap};
use support::{Xorshift, engine, script, slot, straight};

fn solo(n: u8, tap: &Tap, settings: PartySettings) -> NetSession<Scripted> {
    let mut config = NetConfig::new(slot(0), hello());
    config.settings = settings;
    NetSession::new(engine(n), config, Scripted(tap.clone())).expect("a valid config")
}

fn frame(who: u8, from: u32, to: u32) -> Packet {
    Packet::Input(InputFrame {
        slot: slot(who),
        start_tick: from,
        inputs: (from..=to).map(|_| WireInput::default()).collect(),
    })
}

fn bye(who: u8, tick: u32, reason: ByeReason) -> Packet {
    Packet::Bye(Bye {
        slot: slot(who),
        tick,
        reason,
    })
}

fn clean(latency: u32) -> Impair {
    Impair::clean(latency)
}

/// `n` sessions over a simnet with the given hello per slot, each recording what it sent.
fn net_of(
    n: u8,
    hellos: &[Hello],
    impair: Impair,
) -> (SimNet, Vec<NetSession<Recording<SimEndpoint>>>, Vec<Tap>) {
    let net = SimNet::new(n, 1, impair);
    let taps: Vec<Tap> = (0..n).map(|_| Tap::new()).collect();
    let sessions = (0..n)
        .map(|i| {
            let config = NetConfig::new(slot(i), hellos[usize::from(i)]);
            let link = Recording::new(net.endpoint(i), taps[usize::from(i)].clone());
            NetSession::new(engine(n), config, link).expect("a valid config")
        })
        .collect();
    (net, sessions, taps)
}

#[test]
fn advance_waits_in_the_handshake_then_steps() {
    let mut peers = Peers::new(script(2, 612, 2, None), 2, 1, clean(1));
    let out = peers.sessions[0]
        .advance(&Input::default())
        .expect("advance");
    assert_eq!(
        out,
        NetAdvance {
            rolled_back: 0,
            waiting: Some(NetWait::Handshake)
        }
    );
    assert_eq!(peers.sessions[0].session().tick(), 0);
    assert_eq!(peers.sessions[0].phase(), Phase::Handshake);
    for _ in 0..10 {
        peers.round();
    }
    for i in 0..2 {
        assert_eq!(peers.sessions[i].phase(), Phase::Playing);
        let first = peers.calls[i].iter().find(|c| c.out.waiting.is_none());
        assert!(first.is_some(), "session {i} never stepped");
    }
}

#[test]
fn the_hello_is_redundant_under_loss_and_duplicates() {
    for seed in [1, 2] {
        let impair = Impair {
            loss: 300,
            duplicate: 300,
            reorder: 200,
            reorder_extra: 3,
            ..clean(2)
        };
        let mut peers = Peers::new(script(2, 612, 2, None), 2, seed, impair);
        for _ in 0..100 {
            peers.round();
        }
        for s in &peers.sessions {
            assert_eq!(s.phase(), Phase::Playing, "seed {seed}");
        }
    }
}

#[test]
fn three_peers_play_only_when_both_hellos_are_in() {
    let tap = Tap::new();
    let mut s = solo(3, &tap, PartySettings::default());
    tap.push(slot(1), &Packet::Hello(hello()));
    s.poll().expect("poll");
    assert_eq!(s.phase(), Phase::Handshake);
    tap.push(slot(2), &Packet::Hello(hello()));
    s.poll().expect("poll");
    assert_eq!(s.phase(), Phase::Playing);
}

#[test]
fn a_hello_that_differs_refuses_the_party() {
    let good = hello();
    let cases = [
        (
            Hello {
                sim_version: good.sim_version + 1,
                ..good
            },
            Refusal::SimVersion,
        ),
        (
            Hello {
                pack_hash: [7; 32],
                ..good
            },
            Refusal::Pack,
        ),
        (
            Hello {
                protocol: good.protocol + 1,
                ..good
            },
            Refusal::Protocol,
        ),
    ];
    for (odd, why) in cases {
        let (net, mut sessions, taps) = net_of(2, &[good, odd], clean(1));
        for _ in 0..20 {
            net.step_round();
            for s in &mut sessions {
                let _ = s.advance(&Input::default());
            }
        }
        for s in &sessions {
            assert_eq!(s.phase(), Phase::Stopped);
            assert_eq!(s.session().tick(), 0);
        }
        assert_eq!(
            sessions[1].stop_reason(),
            Some(StopReason::Refused { slot: slot(0), why })
        );
        assert!(matches!(
            sessions[0].stop_reason(),
            Some(StopReason::Refused { .. } | StopReason::RefusedBy(_))
        ));
        let refused = taps
            .iter()
            .flat_map(Tap::sent)
            .any(|(_, p)| matches!(p, Packet::Bye(b) if b.reason == ByeReason::Refused));
        assert!(refused, "a Bye Refused was sent");
    }
    // Three peers, one odd: the third also stops.
    let odd = Hello {
        pack_hash: [7; 32],
        ..good
    };
    let (net, mut sessions, _) = net_of(3, &[good, good, odd], clean(1));
    for _ in 0..20 {
        net.step_round();
        for s in &mut sessions {
            let _ = s.advance(&Input::default());
        }
    }
    for s in &sessions {
        assert_eq!(s.phase(), Phase::Stopped);
    }
}

#[test]
fn a_hello_with_protocol_zero_is_a_bad_config() {
    let config = NetConfig::new(
        slot(0),
        Hello {
            protocol: 0,
            ..hello()
        },
    );
    let r = NetSession::new(engine(2), config, Scripted(Tap::new()));
    assert!(matches!(r, Err(NetConfigError::BadHello)));
    let config = NetConfig::new(
        slot(0),
        Hello {
            sim_version: 0,
            ..hello()
        },
    );
    let r = NetSession::new(engine(2), config, Scripted(Tap::new()));
    assert!(matches!(r, Err(NetConfigError::BadHello)));
}

#[test]
fn packets_map_onto_the_session() {
    let tap = Tap::new();
    let mut s = solo(2, &tap, PartySettings::default());
    // Before the hello, an input is dropped.
    tap.push(slot(1), &frame(1, 0, 5));
    s.poll().expect("poll");
    assert_eq!(s.session().next_tick(slot(1)), 0);
    assert_eq!(s.stats().accepted, 0);
    assert_eq!(s.stats().dropped, 1);
    tap.push(slot(1), &Packet::Hello(hello()));
    tap.push(slot(1), &frame(1, 0, 5));
    s.poll().expect("poll");
    assert_eq!(s.session().next_tick(slot(1)), 6);
    // A message from a slot that is not in the party is dropped.
    let dropped = s.stats().dropped;
    tap.push(
        slot(2),
        &Packet::Ack(Ack {
            slot: slot(0),
            next_tick: 0,
        }),
    );
    s.poll().expect("poll");
    assert_eq!(s.stats().dropped, dropped + 1);
    // An ack naming another slot is refused.
    tap.push(
        slot(1),
        &Packet::Ack(Ack {
            slot: slot(1),
            next_tick: 0,
        }),
    );
    s.poll().expect("poll");
    assert_eq!(s.stats().refused, 1);
}

#[test]
fn a_bye_left_is_a_leave_of_the_session() {
    let tap = Tap::new();
    let mut s = solo(3, &tap, PartySettings::default());
    tap.push(slot(1), &Packet::Hello(hello()));
    tap.push(slot(2), &Packet::Hello(hello()));
    // Slot 1 plays on; slot 2 sends 9 inputs and leaves at 9.
    tap.push(slot(2), &frame(2, 0, 8));
    tap.push(slot(2), &bye(2, 9, ByeReason::Left));
    let mut one = 0;
    for k in 0..14u32 {
        tap.push(slot(1), &frame(1, one, k + 2));
        one = k + 3;
        let own = s.session().next_tick(slot(0));
        tap.push(
            slot(1),
            &Packet::Ack(Ack {
                slot: slot(0),
                next_tick: own,
            }),
        );
        tap.push(
            slot(2),
            &Packet::Ack(Ack {
                slot: slot(0),
                next_tick: own,
            }),
        );
        let out = s.advance(&Input::default()).expect("advance");
        assert!(out.waiting.is_none(), "call {k}: {:?}", out.waiting);
    }
    assert_eq!(s.session().tick(), 14);
    assert_eq!(
        s.session().confirmed(),
        14,
        "only the leave lets it pass tick 9"
    );
}

#[test]
fn inputs_are_resent_until_acked_and_the_frame_is_bounded() {
    let tap = Tap::new();
    let mut s = solo(2, &tap, PartySettings::default());
    tap.push(slot(1), &Packet::Hello(hello()));
    let mut sizes = Vec::new();
    let mut waited = None;
    let mut remote = 0;
    for k in 0..60u32 {
        tap.push(slot(1), &frame(1, remote, k + 2));
        remote = k + 3;
        let out = s.advance(&Input::default()).expect("advance");
        if out.waiting == Some(NetWait::Unacked) {
            waited = Some(k);
            break;
        }
        assert!(out.waiting.is_none(), "call {k}: {:?}", out.waiting);
        let last = tap
            .sent()
            .into_iter()
            .filter_map(|(_, p)| {
                if let Packet::Input(f) = p {
                    Some(f)
                } else {
                    None
                }
            })
            .next_back()
            .expect("an input frame");
        assert_eq!(last.start_tick, 0);
        sizes.push(last.inputs.len());
        let ack = tap
            .sent()
            .into_iter()
            .filter_map(|(_, p)| {
                if let Packet::Ack(a) = p {
                    Some(a)
                } else {
                    None
                }
            })
            .next_back()
            .expect("an ack");
        assert_eq!(ack.next_tick, s.session().next_tick(slot(1)));
    }
    assert!(waited.is_some(), "never waited for acks");
    assert!(
        sizes
            .windows(2)
            .all(|w| w[1] == (w[0] + 1).min(MAX_FRAME_INPUTS))
    );
    assert_eq!(*sizes.last().expect("sizes"), MAX_FRAME_INPUTS);
    // An ack moves the start of the frame.
    tap.push(
        slot(1),
        &Packet::Ack(Ack {
            slot: slot(0),
            next_tick: 20,
        }),
    );
    tap.push(slot(1), &frame(1, remote, remote + 2));
    let out = s.advance(&Input::default()).expect("advance");
    assert!(out.waiting.is_none());
    let last = tap
        .sent()
        .into_iter()
        .filter_map(|(_, p)| {
            if let Packet::Input(f) = p {
                Some(f)
            } else {
                None
            }
        })
        .next_back()
        .expect("a frame");
    assert_eq!(last.start_tick, 20);
    assert!(tap.max_len() <= MAX_PACKET_LEN);
    assert_eq!(s.stats().send_errors, 0);
}

#[test]
fn a_failed_send_is_counted_and_the_next_round_sends_again() {
    let tap = Tap::new();
    let mut s = solo(2, &tap, PartySettings::default());
    tap.fail_with(Some(TransportError::Closed));
    s.poll().expect("poll");
    assert!(s.stats().send_errors > 0);
    assert_eq!(tap.count(), 0);
    tap.fail_with(None);
    s.poll().expect("poll");
    assert!(tap.count() > 0);
}

#[test]
fn one_delay_and_window_per_party() {
    let settings = PartySettings {
        delay: 3,
        window: 6,
    };
    let sc = script(2, 612, 3, None);
    let oracle = straight(&sc, 2);
    let mut peers = Peers::new(sc, 2, 1, clean(2)).settings(settings);
    assert!(peers.run(&oracle));
    for s in &peers.sessions {
        assert_eq!(s.session().config().delay, 3);
        assert_eq!(s.session().config().window, 6);
    }
    // The bound that makes one value per party a rule: tick + 2 * delay + window + 1.
    let tap = Tap::new();
    let mut s = solo(2, &tap, settings);
    tap.push(slot(1), &Packet::Hello(hello()));
    tap.push(slot(1), &frame(1, 0, 13));
    s.poll().expect("poll");
    assert_eq!(s.session().next_tick(slot(1)), 14);
    tap.push(slot(1), &frame(1, 0, 14));
    s.poll().expect("poll");
    assert_eq!(s.session().next_tick(slot(1)), 14);
    assert_eq!(s.stats().refused, 1);
    let _ = SessionError::TooFarAhead;
}

#[test]
fn hostile_bytes_never_panic_and_are_all_accounted_for() {
    let tap = Tap::new();
    let mut s = solo(2, &tap, PartySettings::default());
    tap.push(slot(1), &Packet::Hello(hello()));
    let mut rng = Xorshift::new(99);
    let valid = |rng: &mut Xorshift| -> Packet {
        match rng.below(5) {
            0 => Packet::Hello(hello()),
            1 => {
                let start = u32::try_from(rng.below(40)).expect("small");
                frame(
                    1,
                    start,
                    start + u32::try_from(rng.below(20)).expect("small"),
                )
            }
            2 => Packet::Ack(Ack {
                slot: slot(0),
                next_tick: u32::try_from(rng.below(50)).expect("small"),
            }),
            3 => Packet::Checkpoint(Checkpoint {
                slot: slot(1),
                tick: 60 * u32::try_from(1 + rng.below(9)).expect("small"),
                hash: rng.next(),
            }),
            _ => bye(
                1,
                u32::try_from(rng.below(100)).expect("small"),
                ByeReason::Left,
            ),
        }
    };
    let total = 400u64;
    for _ in 0..total {
        let from = slot(u8::try_from(rng.below(3)).expect("small"));
        let mut bytes = match rng.below(4) {
            0 => (0..rng.below(80))
                .map(|_| u8::try_from(rng.below(256)).expect("byte"))
                .collect(),
            _ => valid(&mut rng).encode().expect("a short packet"),
        };
        match rng.below(4) {
            0 if !bytes.is_empty() => {
                let at = usize::try_from(rng.below(bytes.len() as u64)).expect("small");
                bytes[at] ^= 1 << rng.below(8);
            }
            1 => bytes.truncate(usize::try_from(rng.below(bytes.len() as u64 + 1)).expect("small")),
            2 => bytes.extend([1, 2, 3]),
            _ => {}
        }
        tap.push_bytes(from, bytes);
    }
    s.poll().expect("poll");
    for _ in 0..4 {
        let _ = s.poll();
    }
    let st = s.stats();
    assert_eq!(
        st.accepted + st.bad_packets + st.refused + st.dropped,
        total + 1
    );
    // A clean run after hostile traffic still converges (a hostile Bye may stop a session on its
    // own, so the clean run is a separate one).
    let sc = script(2, 612, 2, None);
    let oracle = straight(&sc, 2);
    assert!(Peers::new(sc, 2, 1, clean(2)).run(&oracle));
}

#[test]
fn a_session_that_runs_ahead_waits_for_a_peer_that_does_not_play() {
    let run = |max_skew: u32| {
        let mut peers = Peers::new(script(2, 612, 2, None), 2, 1, clean(1))
            .max_skew(max_skew)
            .calls_per_round(1, 0);
        for _ in 0..20 {
            peers.round();
        }
        peers
    };
    let skewed = run(4);
    let free = run(u32::MAX);
    assert!(skewed.waits_of(Some(0), Some(NetWait::Ahead)) > 0);
    assert_eq!(free.waits_of(Some(0), Some(NetWait::Ahead)), 0);
    assert!(free.waits_of(Some(0), Some(NetWait::Window)) > 0);
    assert!(skewed.sessions[0].session().tick() < free.sessions[0].session().tick());
    assert_eq!(skewed.rolled_by(0), 0);
    assert_eq!(free.rolled_by(0), 0);
}

#[test]
fn a_balanced_run_never_waits_ahead() {
    let mut peers = Peers::new(script(2, 612, 2, None), 2, 1, clean(3));
    for _ in 0..200 {
        peers.round();
    }
    assert_eq!(peers.waits(Some(NetWait::Ahead)), 0);
}

#[test]
fn a_stopped_session_keeps_announcing_its_stop_from_advance() {
    let tap = Tap::new();
    let mut s = solo(2, &tap, PartySettings::default());
    tap.push(
        slot(1),
        &Packet::Hello(Hello {
            pack_hash: [7; 32],
            ..hello()
        }),
    );
    s.poll().expect("poll");
    assert_eq!(s.phase(), Phase::Stopped);
    let sent = tap.count();
    assert_eq!(s.advance(&Input::default()), Err(NetError::Stopped));
    assert!(tap.count() > sent, "the Bye is sent again");
}

#[test]
fn a_desync_bye_before_the_hello_is_dropped() {
    let tap = Tap::new();
    let mut s = solo(2, &tap, PartySettings::default());
    tap.push(slot(1), &bye(1, 60, ByeReason::Desync));
    s.poll().expect("poll");
    assert_eq!(s.phase(), Phase::Handshake);
    assert!(s.desync_report().is_none());
}

#[test]
fn a_checkpoint_from_the_far_future_is_refused() {
    let tap = Tap::new();
    let mut s = solo(2, &tap, PartySettings::default());
    tap.push(slot(1), &Packet::Hello(hello()));
    tap.push(
        slot(1),
        &Packet::Checkpoint(Checkpoint {
            slot: slot(1),
            tick: 60_000,
            hash: 1,
        }),
    );
    s.poll().expect("poll");
    assert_eq!(s.stats().refused, 1);
}
