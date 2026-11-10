//! A divergence is found and stops every session, under latency, jitter, loss, reordering and
//! duplication, for 2 and 3 peers.

mod support;

use stella_rain_net::session::party::{Phase, StopReason};
use support::peers::{Peers, ROUND_LIMIT, flip};
use support::simnet::Impair;
use support::{script, slot, straight};

const TICKS: u32 = 612;

fn five(loss: u16) -> Impair {
    Impair {
        latency: 3,
        jitter: 3,
        loss,
        reorder: 200,
        reorder_extra: 4,
        duplicate: 200,
    }
}

#[test]
fn a_tamper_under_every_impairment_stops_every_session_at_the_divergent_checkpoint() {
    for (seed, loss) in [(7, 100), (9, 300)] {
        for n in [2u8, 3] {
            let sc = script(n, TICKS, 2, None);
            let oracle = straight(&sc, n);
            let mut peers = Peers::new(sc, n, seed, five(loss));
            peers.set_tamper(flip(1, 0, 300..340));
            let finished = peers.run_with(|p| {
                // Before the tampered ticks every confirmed hash is the oracle's.
                p.check_below(&oracle, 299);
            });
            assert!(
                finished,
                "n {n} seed {seed}: not stopped in {ROUND_LIMIT} rounds"
            );
            for (i, s) in peers.sessions.iter().enumerate() {
                assert_eq!(s.phase(), Phase::Stopped, "n {n} seed {seed} session {i}");
                match s.stop_reason() {
                    Some(StopReason::Desync { tick, slot: other }) => {
                        assert_eq!(tick, 360);
                        assert_ne!(other, slot(u8::try_from(i).expect("small")));
                    }
                    Some(StopReason::PeerDesync { tick, .. }) => assert_eq!(tick, 360),
                    other => panic!("n {n} seed {seed} session {i}: {other:?}"),
                }
                assert!(s.session().tick() <= 420, "n {n} seed {seed} session {i}");
            }
        }
    }
}
