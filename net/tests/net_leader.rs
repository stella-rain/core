//! The leader ends a run over the net: `end_run` for slot 0, what every session refuses, the
//! Cancel that stops every peer with its ended run, and the Restart behind a barrier.

mod support;

use stella_rain_net::session::party::{
    EndRunError, MIN_RESTART_TICK, NetError, NetStats, NetWait, Phase, StopReason,
};
use stella_rain_net::transport::Transport;
use stella_rain_net::wire::{Ack, CommandKind, InputFrame, LeaderCommand, Packet, WireInput};
use support::peers::{Peers, ROUND_LIMIT};
use support::simnet::Impair;
use support::{script, slot, straight};

fn clean(latency: u32) -> Impair {
    Impair::clean(latency)
}

/// `n` peers on a net without latency (a forged packet is due at once), run until every session has stepped `ticks`.
fn warm(n: u8, ticks: u32) -> Peers {
    let mut peers = Peers::new(script(n, 612, 2, None), n, 1, clean(0));
    while peers
        .sessions
        .iter()
        .any(|s| s.session().tick() < ticks || s.phase() != Phase::Playing)
    {
        assert!(peers.net.round() < ROUND_LIMIT, "warm-up did not finish");
        peers.round();
    }
    peers
}

/// Rounds until slot 0 has confirmed `at` ticks, then slot 0 ends the run with `kind`.
fn stamp(peers: &mut Peers, kind: CommandKind, at: u32) -> u32 {
    while peers.sessions[0].session().confirmed() < at {
        assert!(peers.net.round() < ROUND_LIMIT, "never confirmed {at}");
        peers.round();
    }
    peers.sessions[0]
        .end_run(kind)
        .expect("the leader ends the run")
}

/// Sends `packet` from `from` to session `to` and has `to` receive just it; its counters before
/// and after.
fn deliver(peers: &mut Peers, from: u8, to: usize, packet: &Packet) -> (NetStats, NetStats) {
    peers.sessions[to].poll().expect("drain");
    let before = peers.sessions[to].stats();
    let bytes = packet.encode().expect("a short packet");
    peers
        .net
        .endpoint(from)
        .send(slot(u8::try_from(to).expect("small")), &bytes)
        .expect("sent");
    peers.sessions[to].poll().expect("poll");
    (before, peers.sessions[to].stats())
}

fn command(run: u8, who: u8, tick: u32, kind: CommandKind) -> Packet {
    Packet::Command(LeaderCommand {
        run,
        slot: slot(who),
        tick,
        kind,
    })
}

#[test]
fn min_restart_tick_is_60() {
    assert_eq!(MIN_RESTART_TICK, 60);
}

// A. Local refusals.

#[test]
fn a_non_leader_cannot_end_the_run() {
    let mut peers = warm(2, 70);
    for kind in [CommandKind::Cancel, CommandKind::Restart] {
        assert_eq!(
            peers.sessions[1].end_run(kind),
            Err(EndRunError::NotLeader),
            "{kind:?}"
        );
        assert_eq!(peers.sessions[1].ending(), None, "{kind:?}");
    }
}

#[test]
fn a_restart_before_tick_60_is_refused_and_a_cancel_is_not() {
    let mut peers = warm(2, 8);
    let leader = &mut peers.sessions[0];
    let confirmed = leader.session().confirmed();
    assert!((1..MIN_RESTART_TICK).contains(&confirmed), "{confirmed}");
    assert_eq!(
        leader.end_run(CommandKind::Restart),
        Err(EndRunError::TooEarly)
    );
    assert_eq!(leader.ending(), None);
    let t = leader.end_run(CommandKind::Cancel).expect("a cancel");
    assert_eq!(t, confirmed);
    assert!(t < MIN_RESTART_TICK);

    let mut peers = warm(2, 8);
    while peers.sessions[0].session().confirmed() < MIN_RESTART_TICK {
        peers.round();
    }
    let confirmed = peers.sessions[0].session().confirmed();
    let t = peers.sessions[0]
        .end_run(CommandKind::Restart)
        .expect("a restart at 60");
    assert_eq!(t, confirmed);
    assert!(t >= MIN_RESTART_TICK);
}

#[test]
fn a_restart_is_refused_while_a_peer_has_left() {
    let mut peers = Peers::new(script(3, 612, 2, None), 3, 1, clean(1)).leaver(2, 200);
    while peers.sessions[0].session().tick() < 210 {
        assert!(peers.net.round() < ROUND_LIMIT);
        peers.round();
    }
    assert_eq!(
        peers.sessions[0].end_run(CommandKind::Restart),
        Err(EndRunError::PeerLeft)
    );
    assert_eq!(peers.sessions[0].ending(), None);
    assert!(peers.sessions[0].end_run(CommandKind::Cancel).is_ok());
}

#[test]
fn the_other_refusals() {
    let mut peers = Peers::new(script(2, 612, 2, None), 2, 1, clean(1));
    assert_eq!(
        peers.sessions[0].end_run(CommandKind::Cancel),
        Err(EndRunError::NotPlaying)
    );
    let mut peers = warm(2, 100);
    while peers.sessions[0].session().confirmed() < MIN_RESTART_TICK {
        peers.round();
    }
    peers.sessions[0]
        .end_run(CommandKind::Restart)
        .expect("a restart");
    for kind in [CommandKind::Cancel, CommandKind::Restart] {
        assert_eq!(
            peers.sessions[0].end_run(kind),
            Err(EndRunError::AlreadyEnding),
            "{kind:?}"
        );
    }
    let mut peers = warm(2, 100);
    peers.sessions[0]
        .end_run(CommandKind::Cancel)
        .expect("a cancel");
    for kind in [CommandKind::Cancel, CommandKind::Restart] {
        assert_eq!(
            peers.sessions[0].end_run(kind),
            Err(EndRunError::Stopped),
            "{kind:?}"
        );
    }
    for i in 0..3 {
        assert!(!EndRunError::Stopped.to_string().is_empty(), "{i}");
    }
}

// B. Receive refusals.

#[test]
fn a_command_from_a_non_leader_is_refused() {
    let mut peers = warm(2, 70);
    let (before, after) = deliver(&mut peers, 1, 0, &command(0, 1, 60, CommandKind::Cancel));
    assert_eq!(after.refused, before.refused + 1);
    assert_eq!(after.accepted, before.accepted);
    assert_eq!(peers.sessions[0].ending(), None);
}

#[test]
fn a_command_whose_slot_is_not_its_sender_is_refused() {
    let mut peers = warm(2, 70);
    let (before, after) = deliver(&mut peers, 0, 1, &command(0, 1, 60, CommandKind::Cancel));
    assert_eq!(after.refused, before.refused + 1);
    assert_eq!(peers.sessions[1].ending(), None);
}

#[test]
fn a_restart_at_tick_59_is_refused() {
    let mut peers = warm(2, 70);
    let (before, after) = deliver(&mut peers, 0, 1, &command(0, 0, 59, CommandKind::Restart));
    assert_eq!(after.refused, before.refused + 1);
    assert_eq!(peers.sessions[1].ending(), None);
    // A cancel at 59 is not a restart: the guard is the restart's.
    let (before, after) = deliver(&mut peers, 0, 1, &command(0, 0, 59, CommandKind::Cancel));
    assert_eq!(after.accepted, before.accepted + 1);
    assert_eq!(peers.sessions[1].ending(), Some((59, CommandKind::Cancel)));
}

#[test]
fn a_command_past_the_receivers_next_tick_is_refused() {
    let mut peers = warm(2, 70);
    let far = peers.sessions[1].session().next_tick(slot(1)) + 1;
    for kind in [CommandKind::Cancel, CommandKind::Restart] {
        let (before, after) = deliver(&mut peers, 0, 1, &command(0, 0, far, kind));
        assert_eq!(after.refused, before.refused + 1, "{kind:?}");
        assert_eq!(peers.sessions[1].ending(), None, "{kind:?}");
    }
}

#[test]
fn a_valid_command_is_accepted_and_a_different_second_one_is_refused() {
    let mut peers = warm(2, 70);
    let cancel = command(0, 0, 60, CommandKind::Cancel);
    let (before, after) = deliver(&mut peers, 0, 1, &cancel);
    assert_eq!(after.accepted, before.accepted + 1);
    assert_eq!(peers.sessions[1].ending(), Some((60, CommandKind::Cancel)));
    let (before, after) = deliver(&mut peers, 0, 1, &cancel);
    assert_eq!(after.accepted, before.accepted + 1);
    assert_eq!(peers.sessions[1].ending(), Some((60, CommandKind::Cancel)));
    let (before, after) = deliver(&mut peers, 0, 1, &command(0, 0, 60, CommandKind::Restart));
    assert_eq!(after.refused, before.refused + 1);
    assert_eq!(peers.sessions[1].ending(), Some((60, CommandKind::Cancel)));
}

#[test]
fn a_command_of_another_run_is_dropped() {
    let mut peers = warm(2, 70);
    let (before, after) = deliver(&mut peers, 0, 1, &command(1, 0, 60, CommandKind::Cancel));
    assert_eq!(after.dropped, before.dropped + 1);
    assert_eq!(after.accepted, before.accepted);
    assert_eq!(peers.sessions[1].ending(), None);
}

// C. Cancel.

/// The checks every cancel case makes: every session stopped on the cancel and handed out the
/// same ended run of `t` ticks.
fn check_cancelled(peers: &mut Peers, oracle: &[u64], n: u8, t: u32, what: &str) {
    let mut bytes = Vec::new();
    for (i, s) in peers.sessions.iter_mut().enumerate() {
        assert_eq!(s.phase(), Phase::Stopped, "{what}: session {i}");
        assert_eq!(
            s.stop_reason(),
            Some(StopReason::EndedByLeader(CommandKind::Cancel)),
            "{what}: session {i}"
        );
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
            .replay(
                &support::equipment_setup(),
                stella_rain_core::run_content::RunContent::bundled(),
            )
            .expect("a replay");
        assert_eq!(
            replay.final_hash,
            format!("{:016x}", oracle[t as usize]),
            "{what}: {i}"
        );
        assert!(s.take_ended_run().is_none(), "{what}: taken once");
        bytes.push(replay.to_bytes());
    }
    assert!(
        bytes.windows(2).all(|w| w[0] == w[1]),
        "{what}: equal bytes"
    );
}

#[test]
fn a_cancel_stops_every_peer_with_its_ended_run() {
    for n in [2u8, 3] {
        let sc = script(n, 612, 2, None);
        let oracle = straight(&sc, n);
        let mut peers = Peers::new(sc, n, 7, clean(1));
        let t = stamp(&mut peers, CommandKind::Cancel, 300);
        assert!(t >= 300);
        assert_eq!(peers.sessions[0].phase(), Phase::Stopped, "n {n}");
        assert!(peers.run(&oracle), "n {n}: not stopped");
        check_cancelled(&mut peers, &oracle, n, t, &format!("n {n}"));
    }
}

#[test]
fn a_peer_behind_t_steps_to_t_and_no_further() {
    let sc = script(2, 612, 2, None);
    let oracle = straight(&sc, 2);
    let mut peers = Peers::new(sc, 2, 7, clean(1));
    while peers.sessions[0].session().confirmed() < 300 {
        peers.round();
    }
    peers.set_calls(1, 0);
    for _ in 0..10 {
        peers.round();
    }
    let t = peers.sessions[0]
        .end_run(CommandKind::Cancel)
        .expect("a cancel");
    assert!(peers.sessions[1].session().tick() < t, "slot 1 is behind T");
    peers.set_calls(1, 1);
    assert!(peers.run(&oracle));
    assert_eq!(peers.sessions[1].session().tick(), t);
    for _ in 0..20 {
        peers.round();
        assert_eq!(peers.sessions[1].session().tick(), t);
    }
}

#[test]
fn a_stopped_session_keeps_pumping() {
    let sc = script(3, 612, 2, None);
    let oracle = straight(&sc, 3);
    let mut peers = Peers::new(sc, 3, 7, clean(1));
    while peers.sessions[0].session().confirmed() < 300 {
        peers.round();
    }
    peers.set_calls(2, 0);
    for _ in 0..10 {
        peers.round();
    }
    let t = peers.sessions[0]
        .end_run(CommandKind::Cancel)
        .expect("a cancel");
    let stamped_at = peers.net.round();
    peers.set_calls(2, 1);
    assert!(peers.run(&oracle));
    let stopped_at = peers.net.round();
    for _ in 0..20 {
        peers.round();
    }
    for s in &mut peers.sessions {
        assert_eq!(
            s.advance(&Default::default()).err(),
            Some(NetError::Stopped)
        );
    }
    let sent = peers.sent();
    let in_round = |r: u32, from: usize, to: usize, f: &dyn Fn(&Packet) -> bool| {
        sent.iter()
            .any(|(round, a, b, p)| *round == r && *a == from && *b == to && f(p))
    };
    let is_cancel = |p: &Packet| matches!(p, Packet::Command(c) if c.run == 0 && c.tick == t && c.kind == CommandKind::Cancel && c.slot == slot(0));
    for r in stamped_at + 1..=stopped_at + 20 {
        for to in 1..3 {
            assert!(
                in_round(r, 0, to, &is_cancel),
                "round {r}: no cancel to {to}"
            );
        }
    }
    for r in stopped_at + 1..=stopped_at + 20 {
        for from in 0..3 {
            for to in (0..3).filter(|to| *to != from) {
                assert!(
                    in_round(r, from, to, &|p| matches!(p, Packet::Ack(_))),
                    "round {r}: no ack {from} to {to}"
                );
            }
        }
    }
    assert!(
        sent.iter().all(|(r, from, _, p)| {
            *r <= stamped_at
                || !(matches!(p, Packet::Bye(_)) || (*from != 0 && matches!(p, Packet::Command(_))))
        }),
        "no bye and no command from a peer after the stamp"
    );
    assert!(
        sent.iter().any(|(r, from, to, p)| *r > stamped_at
            && *from == 0
            && *to == 2
            && matches!(p, Packet::Input(_))),
        "the leader serves frames to a peer that is behind"
    );
    assert_eq!(peers.waits_of(Some(2), Some(NetWait::Ending)), 0);

    // An ack still counts; anything else is dropped.
    let ack = Packet::Ack(Ack {
        run: 0,
        slot: slot(1),
        next_tick: 0,
    });
    let (before, after) = deliver(&mut peers, 0, 1, &ack);
    assert_eq!(after.accepted, before.accepted + 1);
    let input = Packet::Input(InputFrame {
        run: 0,
        slot: slot(0),
        start_tick: 0,
        inputs: vec![WireInput::default()],
    });
    let (before, after) = deliver(&mut peers, 0, 1, &input);
    assert_eq!(after.dropped, before.dropped + 1);
    assert_eq!(after.accepted, before.accepted);
}
