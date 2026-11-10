//! 2 and 3 rollback sessions over the simulated network, with equipment actions and a leave in
//! the script, end on the same hash as a straight run.

mod support;

use stella_rain_core::input::EquipAction;
use stella_rain_net::session::party::{NetWait, PartySettings, Phase, StopReason};
use support::peers::{Peers, ROUND_LIMIT, flip};
use support::simnet::Impair;
use support::{Script, script, slot, straight};

const TICKS: u32 = 612;
const WINDOW: u32 = 4;

fn s(n: u8) -> Script {
    script(n, TICKS, 2, None)
}

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

/// Runs `peers` to the end and asserts the run converged; returns it. `stayers` are the
/// sessions that never leave.
fn converge(mut peers: Peers, oracle: &[u64], stayers: &[usize]) -> Peers {
    assert!(peers.run(oracle), "not done within {ROUND_LIMIT} rounds");
    let c = stayers
        .iter()
        .map(|i| peers.sessions[*i].session().confirmed())
        .min()
        .expect("a stayer");
    assert!(c >= TICKS - WINDOW, "confirmed only {c}");
    let want = oracle[c as usize];
    for i in stayers {
        assert_eq!(
            peers.sessions[*i].session().hash_after(c),
            Some(want),
            "session {i}"
        );
    }
    for (i, session) in peers.sessions.iter().enumerate() {
        assert_ne!(session.phase(), Phase::Stopped, "session {i}");
    }
    peers
}

fn kind(a: &EquipAction) -> u8 {
    match a {
        EquipAction::Equip { .. } => 0,
        EquipAction::Replace { .. } => 1,
        EquipAction::Extract { .. } => 2,
        EquipAction::Craft { .. } => 3,
        EquipAction::TreePick { .. } => 4,
    }
}

fn all(n: u8) -> Vec<usize> {
    (0..usize::from(n)).collect()
}

fn run_alone(n: u8, impair: Impair) -> Peers {
    let sc = s(n);
    let oracle = straight(&sc, n);
    converge(Peers::new(sc, n, 1, impair), &oracle, &all(n))
}

#[test]
fn latency_alone_converges() {
    for n in [2, 3] {
        run_alone(n, Impair::clean(3));
    }
}

#[test]
fn jitter_alone_converges() {
    for n in [2, 3] {
        let p = run_alone(
            n,
            Impair {
                jitter: 3,
                ..Impair::clean(3)
            },
        );
        // Jitter alone (no duplicates here) reorders, which needs at least two delay values.
        assert!(p.net.stats().reordered > 0);
    }
}

#[test]
fn loss_alone_converges() {
    for n in [2, 3] {
        let p = run_alone(
            n,
            Impair {
                loss: 300,
                ..Impair::clean(3)
            },
        );
        assert!(p.net.stats().lost > 0);
    }
}

#[test]
fn reorder_alone_converges() {
    for n in [2, 3] {
        let p = run_alone(
            n,
            Impair {
                reorder: 300,
                ..Impair::clean(3)
            },
        );
        assert!(p.net.stats().reordered > 0);
    }
}

#[test]
fn duplicate_alone_converges() {
    for n in [2, 3] {
        let p = run_alone(
            n,
            Impair {
                duplicate: 500,
                ..Impair::clean(3)
            },
        );
        assert!(p.net.stats().duplicated > 0);
    }
}

#[test]
fn all_five_converge_and_roll_back_over_equipment_actions() {
    for n in [2, 3] {
        for seed in [1, 2] {
            let sc = s(n);
            let actions: Vec<_> = sc
                .eff
                .iter()
                .flat_map(|row| row.iter().filter_map(|i| i.action))
                .collect();
            assert!(actions.len() >= 3, "{} actions", actions.len());
            let mut kinds: Vec<u8> = actions.iter().map(kind).collect();
            kinds.sort_unstable();
            kinds.dedup();
            // The 612-tick script of 2 peers holds only `Extract`; 3 peers also equip.
            assert!(kinds.len() >= if n == 3 { 2 } else { 1 });
            let oracle = straight(&sc, n);
            let p = converge(Peers::new(sc, n, seed, five(100)), &oracle, &all(n));
            assert!(p.total_rolled() > 0, "n {n} seed {seed}");
            assert!(p.rolled_over_an_action(), "n {n} seed {seed}");
        }
    }
}

#[test]
fn a_leave_under_loss_converges_for_the_stayers() {
    for (seed, loss) in [(7, 100), (9, 300)] {
        let sc = script(3, TICKS, 2, Some((slot(2), 300)));
        let oracle = straight(&sc, 3);
        let p = converge(
            Peers::new(sc, 3, seed, five(loss)).leaver(2, 300),
            &oracle,
            &[0, 1],
        );
        assert_eq!(p.sessions[2].session().tick(), 300);
        assert_eq!(p.sessions[0].session().tick(), TICKS);
        assert_eq!(p.sessions[1].session().tick(), TICKS);
        assert!(p.rolled_across(300, 2), "seed {seed}");
    }
}

#[test]
fn the_same_seed_gives_the_same_run_and_another_seed_a_different_one() {
    let run = |seed| {
        let sc = script(3, TICKS, 2, Some((slot(2), 300)));
        let oracle = straight(&sc, 3);
        let p = converge(
            Peers::new(sc, 3, seed, five(100)).leaver(2, 300),
            &oracle,
            &[0, 1],
        );
        let confirmed: Vec<_> = p
            .sessions
            .iter()
            .map(|x| {
                let x = x.session();
                (x.confirmed(), x.hash_after(x.confirmed()))
            })
            .collect();
        (p.net.trace(), confirmed, p.total_rolled())
    };
    assert_eq!(run(7), run(7));
    assert_ne!(run(7).0, run(8).0);
}

#[test]
fn a_tampered_input_stops_both_peers_on_the_divergent_checkpoint() {
    let sc = s(2);
    let oracle = straight(&sc, 2);
    let mut peers = Peers::new(sc, 2, 1, five(100));
    peers.set_tamper(flip(1, 0, 300..340));
    // `check` would flag the divergent hashes before the checkpoint finds them.
    let stopped = peers.run_with(|p| p.check_below(&oracle, 299));
    assert!(stopped, "not stopped within {ROUND_LIMIT} rounds");
    for (i, session) in peers.sessions.iter().enumerate() {
        match session.stop_reason() {
            Some(StopReason::Desync { tick, .. } | StopReason::PeerDesync { tick, .. }) => {
                assert_eq!(tick, 360, "session {i}");
            }
            other => panic!("session {i}: {other:?}"),
        }
    }
    for i in 0..2 {
        assert_eq!(peers.sessions[i].phase(), Phase::Stopped, "session {i}");
        assert!(peers.sessions[i].session().tick() <= 420, "session {i}");
    }
}

#[test]
fn uneven_clocks_make_the_faster_peer_wait_and_roll_back_less() {
    let run = |max_skew: u32| {
        let sc = s(2);
        let oracle = straight(&sc, 2);
        let mut peers = Peers::new(sc, 2, 1, Impair::clean(3))
            .max_skew(max_skew)
            .calls_per_round(0, 2);
        let mut worst = 0;
        let mut rounds = 0u32;
        while !peers.done() {
            assert!(peers.net.round() < 300 + ROUND_LIMIT, "not done");
            peers.round();
            peers.check(&oracle);
            rounds += 1;
            if rounds > 30 {
                let (a, b) = (
                    peers.sessions[0].session().tick(),
                    peers.sessions[1].session().tick(),
                );
                worst = worst.max(a.abs_diff(b));
            }
        }
        (peers, worst)
    };
    let (skewed, worst) = run(stella_rain_net::session::party::DEFAULT_MAX_SKEW);
    assert!(skewed.waits_of(Some(0), Some(NetWait::Ahead)) > 0);
    assert_eq!(skewed.waits_of(Some(1), Some(NetWait::Ahead)), 0);
    assert!(worst <= 4, "the ticks drifted {worst} apart");
    let (free, _) = run(u32::MAX);
    assert!(free.rolled_by(0) > skewed.rolled_by(0));
}

#[test]
fn a_non_default_party_setting_converges() {
    let sc = script(3, TICKS, 3, None);
    let oracle = straight(&sc, 3);
    let peers = Peers::new(sc, 3, 2, five(100)).settings(PartySettings {
        delay: 3,
        window: 6,
    });
    converge(peers, &oracle, &all(3));
}

#[test]
fn a_lost_first_hello_still_reaches_playing_by_round_60() {
    for n in [2, 3] {
        let mut peers = Peers::new(s(n), n, 2, five(300));
        for _ in 0..60 {
            peers.round();
        }
        for (i, session) in peers.sessions.iter().enumerate() {
            assert_ne!(session.phase(), Phase::Handshake, "n {n} session {i}");
        }
    }
}
