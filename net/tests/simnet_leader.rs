//! The leader's Cancel and Restart under latency, jitter, loss, reordering and duplication, for 2
//! and 3 peers: every session ends the same run on the straight run's hash.

mod support;

use std::cell::Cell;
use std::rc::Rc;

use stella_rain_core::run_content::RunContent;
use stella_rain_net::session::party::{Phase, StopReason};
use stella_rain_net::wire::{CommandKind, Packet};
use support::peers::{Peers, ROUND_LIMIT};
use support::simnet::Impair;
use support::{equipment_setup, script, straight};

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

/// Rounds until slot 0 has confirmed 300 ticks, then it ends the run; the end tick.
fn stamp(peers: &mut Peers, kind: CommandKind) -> u32 {
    while peers.sessions[0].session().confirmed() < 300 {
        assert!(peers.net.round() < ROUND_LIMIT, "never confirmed 300");
        peers.round();
    }
    peers.sessions[0]
        .end_run(kind)
        .expect("the leader ends the run")
}

/// Every session handed out the same ended run of `t` ticks on the oracle's hash.
fn check_ended(peers: &mut Peers, oracle: &[u64], n: u8, t: u32, what: &str) {
    let mut bytes = Vec::new();
    for (i, s) in peers.sessions.iter_mut().enumerate() {
        let ended = s.take_ended_run().expect("an ended run");
        assert_eq!(
            (ended.run, ended.tick, ended.party),
            (0, t, n),
            "{what}: {i}"
        );
        assert_eq!(
            ended.ticks.len(),
            usize::try_from(t).expect("small"),
            "{what}: {i}"
        );
        let replay = ended
            .replay(&equipment_setup(), RunContent::bundled())
            .expect("a replay");
        assert_eq!(
            replay.final_hash,
            format!("{:016x}", oracle[t as usize]),
            "{what}: {i}"
        );
        bytes.push(replay.to_bytes());
    }
    assert!(
        bytes.windows(2).all(|w| w[0] == w[1]),
        "{what}: equal bytes"
    );
}

/// Runs a Cancel to its end and checks it.
fn finish_cancel(peers: &mut Peers, oracle: &[u64], n: u8, t: u32, what: &str) {
    assert!(
        peers.run(oracle),
        "{what}: not stopped in {ROUND_LIMIT} rounds"
    );
    for (i, s) in peers.sessions.iter().enumerate() {
        assert_eq!(s.phase(), Phase::Stopped, "{what}: session {i}");
        assert_eq!(
            s.stop_reason(),
            Some(StopReason::EndedByLeader(CommandKind::Cancel)),
            "{what}: session {i}"
        );
    }
    check_ended(peers, oracle, n, t, what);
}

/// Runs a Restart until every session is on run 1, then through run 1 to tick 612.
fn finish_restart(peers: &mut Peers, oracle: &[u64], n: u8, t: u32, what: &str) {
    while peers.sessions.iter().any(|s| s.run() == 0) {
        assert!(peers.net.round() < ROUND_LIMIT, "{what}: never restarted");
        peers.round();
        peers.check(oracle);
    }
    check_ended(peers, oracle, n, t, what);
    assert!(
        peers.run(oracle),
        "{what}: run 1 did not finish in {ROUND_LIMIT} rounds"
    );
    for _ in 0..60 {
        peers.round();
    }
    for (i, s) in peers.sessions.iter().enumerate() {
        assert_eq!(s.run(), 1, "{what}: session {i}");
        assert_eq!(s.phase(), Phase::Playing, "{what}: session {i}");
        assert_eq!(
            s.session().hash_after(TICKS),
            Some(oracle[TICKS as usize]),
            "{what}: {i}"
        );
    }
}

fn finish(peers: &mut Peers, kind: CommandKind, oracle: &[u64], n: u8, t: u32, what: &str) {
    match kind {
        CommandKind::Cancel => finish_cancel(peers, oracle, n, t, what),
        CommandKind::Restart => finish_restart(peers, oracle, n, t, what),
    }
}

#[test]
fn a_cancel_under_every_impairment() {
    for (seed, loss) in [(7, 100), (9, 300)] {
        for n in [2u8, 3] {
            let sc = script(n, TICKS, 2, None);
            let oracle = straight(&sc, n);
            let mut peers = Peers::new(sc, n, seed, five(loss));
            let t = stamp(&mut peers, CommandKind::Cancel);
            assert!(t >= 300);
            finish(
                &mut peers,
                CommandKind::Cancel,
                &oracle,
                n,
                t,
                &format!("n {n} seed {seed}"),
            );
        }
    }
}

#[test]
fn a_restart_under_every_impairment() {
    for (seed, loss) in [(7, 100), (9, 300)] {
        for n in [2u8, 3] {
            let sc = script(n, TICKS, 2, None);
            let oracle = straight(&sc, n);
            let mut peers = Peers::new(sc, n, seed, five(loss));
            let t = stamp(&mut peers, CommandKind::Restart);
            assert!(t >= 300);
            finish(
                &mut peers,
                CommandKind::Restart,
                &oracle,
                n,
                t,
                &format!("n {n} seed {seed}"),
            );
        }
    }
}

#[test]
fn a_lost_command_is_resent() {
    for kind in [CommandKind::Cancel, CommandKind::Restart] {
        for (seed, loss) in [(7, 100), (9, 300)] {
            for n in [2u8, 3] {
                let sc = script(n, TICKS, 2, None);
                let oracle = straight(&sc, n);
                let mut peers = Peers::new(sc, n, seed, five(loss));
                let lost = Rc::new(Cell::new([0u32; 3]));
                let counts = Rc::clone(&lost);
                peers.set_drop_if(Box::new(move |_, to, p| {
                    let mut c = counts.get();
                    let drop = matches!(p, Packet::Command(_)) && c[to] < 3;
                    if drop {
                        c[to] += 1;
                        counts.set(c);
                    }
                    drop
                }));
                let t = stamp(&mut peers, kind);
                finish(
                    &mut peers,
                    kind,
                    &oracle,
                    n,
                    t,
                    &format!("{kind:?} n {n} seed {seed}"),
                );
            }
        }
    }
}

#[test]
fn lost_acks_cost_nothing_but_time() {
    for kind in [CommandKind::Cancel, CommandKind::Restart] {
        for (seed, loss) in [(7, 100), (9, 300)] {
            for n in [2u8, 3] {
                let sc = script(n, TICKS, 2, None);
                let oracle = straight(&sc, n);
                let mut peers = Peers::new(sc, n, seed, five(loss));
                let rounds = peers.round_cell();
                let from = Rc::new(Cell::new(u32::MAX));
                let seen = Rc::clone(&from);
                peers.set_drop_if(Box::new(move |_, _, p| {
                    let lo = seen.get();
                    matches!(p, Packet::Ack(_))
                        && (lo..lo.saturating_add(60)).contains(&rounds.get())
                }));
                while peers.sessions[0].session().confirmed() < 300 {
                    assert!(peers.net.round() < ROUND_LIMIT);
                    peers.round();
                }
                from.set(peers.net.round());
                let t = peers.sessions[0]
                    .end_run(kind)
                    .expect("the leader ends the run");
                finish(
                    &mut peers,
                    kind,
                    &oracle,
                    n,
                    t,
                    &format!("{kind:?} n {n} seed {seed}"),
                );
            }
        }
    }
}

#[test]
fn reordered_old_run_packets_are_dropped() {
    let impair = Impair {
        latency: 2,
        reorder: 500,
        reorder_extra: 6,
        ..Impair::clean(2)
    };
    for seed in [3u64, 7, 9, 11, 13] {
        let sc = script(3, TICKS, 2, None);
        let oracle = straight(&sc, 3);
        let mut peers = Peers::new(sc, 3, seed, impair);
        let t = stamp(&mut peers, CommandKind::Restart);
        let dropped = |p: &Peers| p.sessions.iter().map(|s| s.stats().dropped).sum::<u64>();
        let at_stamp = dropped(&peers);
        finish(
            &mut peers,
            CommandKind::Restart,
            &oracle,
            3,
            t,
            &format!("seed {seed}"),
        );
        assert!(
            dropped(&peers) > at_stamp,
            "seed {seed}: no old-run packet was dropped"
        );
    }
}
