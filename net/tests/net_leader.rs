//! The leader ends a run over the net: `end_run` for slot 0, what every session refuses, the
//! Cancel that stops every peer with its ended run, and the Restart behind a barrier.

mod support;

use std::cell::Cell;
use std::rc::Rc;

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
    // An end tick this session has not confirmed yet, so the command stays pending.
    let t = peers.sessions[1].session().tick() + 1;
    assert!(t >= MIN_RESTART_TICK);
    let cancel = command(0, 0, t, CommandKind::Cancel);
    let (before, after) = deliver(&mut peers, 0, 1, &cancel);
    assert_eq!(after.accepted, before.accepted + 1);
    assert_eq!(peers.sessions[1].ending(), Some((t, CommandKind::Cancel)));
    assert_eq!(peers.sessions[1].phase(), Phase::Playing);
    let (before, after) = deliver(&mut peers, 0, 1, &cancel);
    assert_eq!(after.accepted, before.accepted + 1);
    assert_eq!(peers.sessions[1].ending(), Some((t, CommandKind::Cancel)));
    let (before, after) = deliver(&mut peers, 0, 1, &command(0, 0, t, CommandKind::Restart));
    assert_eq!(after.refused, before.refused + 1);
    assert_eq!(peers.sessions[1].ending(), Some((t, CommandKind::Cancel)));
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
    let mut peers = Peers::new(sc, 3, 7, clean(0));
    // The leader's input frames to slot 2 are lost around the stamp, so slot 2 is behind and
    // the stopped leader has to keep serving it.
    let rounds = peers.round_cell();
    let window = Rc::new(Cell::new((u32::MAX, u32::MAX)));
    let seen = Rc::clone(&window);
    peers.set_drop_if(Box::new(move |from, to, p| {
        let (lo, hi) = seen.get();
        from == 0 && to == 2 && matches!(p, Packet::Input(_)) && (lo..hi).contains(&rounds.get())
    }));
    while peers.sessions[0].session().confirmed() < 300 {
        peers.round();
    }
    peers.set_calls(2, 0);
    window.set((peers.net.round(), u32::MAX));
    for _ in 0..10 {
        peers.round();
    }
    let t = peers.sessions[0]
        .end_run(CommandKind::Cancel)
        .expect("a cancel");
    let stamped_at = peers.net.round();
    window.set((window.get().0, stamped_at + 6));
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
    // Slot 2 stepped to T and waited for the end, never past it.
    assert!(peers.waits_of(Some(2), Some(NetWait::Ending)) > 0);

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

// D. Restart.

/// The run every session ended: the same, `t` ticks, on the oracle's hash.
fn check_ended(peers: &mut Peers, oracle: &[u64], n: u8, t: u32, run: u8, what: &str) {
    let mut bytes = Vec::new();
    for (i, s) in peers.sessions.iter_mut().enumerate() {
        let ended = s.take_ended_run().expect("an ended run");
        assert_eq!(
            (ended.run, ended.tick, ended.party),
            (run, t, n),
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
        bytes.push(replay.to_bytes());
    }
    assert!(
        bytes.windows(2).all(|w| w[0] == w[1]),
        "{what}: equal bytes"
    );
}

/// Every session plays run 1 to tick 612 on the oracle's hash.
fn check_next_run(peers: &mut Peers, oracle: &[u64], what: &str) {
    assert!(peers.run(oracle), "{what}: the next run did not finish");
    for _ in 0..30 {
        peers.round();
    }
    for (i, s) in peers.sessions.iter().enumerate() {
        assert_eq!(s.run(), 1, "{what}: session {i}");
        assert_eq!(s.phase(), Phase::Playing, "{what}: session {i}");
        assert_eq!(s.ending(), None, "{what}: session {i}");
        assert_eq!(s.session().confirmed(), 612, "{what}: session {i}");
        assert_eq!(
            s.session().hash_after(612),
            Some(oracle[612]),
            "{what}: session {i}"
        );
    }
}

#[test]
fn a_restart_plays_the_next_run_from_tick_0() {
    for n in [2u8, 3] {
        let sc = script(n, 612, 2, None);
        let oracle = straight(&sc, n);
        let mut peers = Peers::new(sc, n, 7, clean(1));
        let t = stamp(&mut peers, CommandKind::Restart, 300);
        assert!(t >= 300);
        // Run until every session has restarted, then the new run plays to 612.
        while peers.sessions.iter().any(|s| s.run() == 0) {
            assert!(peers.net.round() < ROUND_LIMIT, "n {n}: never restarted");
            peers.round();
            peers.check(&oracle);
        }
        check_ended(&mut peers, &oracle, n, t, 0, &format!("n {n}"));
        check_next_run(&mut peers, &oracle, &format!("n {n}"));
    }
}

#[test]
fn a_silent_peer_holds_the_barrier() {
    let sc = script(2, 612, 2, None);
    let oracle = straight(&sc, 2);
    let mut peers = Peers::new(sc, 2, 7, clean(1));
    // From 10 rounds before the stamp slot 1's acks are lost, so the leader's `acked` of it is
    // below T; from the stamp for 40 rounds every packet from slot 1 is lost.
    let rounds = peers.round_cell();
    let window = Rc::new(Cell::new((u32::MAX, u32::MAX, u32::MAX)));
    let seen = Rc::clone(&window);
    peers.set_drop_if(Box::new(move |from, to, p| {
        let (acks, lo, hi) = seen.get();
        let r = rounds.get();
        from == 1
            && to == 0
            && ((r >= acks && matches!(p, Packet::Ack(_))) || (lo..hi).contains(&r))
    }));
    while peers.sessions[0].session().confirmed() < 290 {
        peers.round();
    }
    window.set((peers.net.round(), u32::MAX, u32::MAX));
    for _ in 0..10 {
        peers.round();
    }
    let t = peers.sessions[0]
        .end_run(CommandKind::Restart)
        .expect("a restart");
    let from = peers.net.round() + 1;
    window.set((window.get().0, from, from + 40));
    while peers.net.round() < from + 40 {
        peers.round();
        assert_eq!(peers.sessions[0].run(), 0, "round {}", peers.net.round());
        assert_eq!(peers.sessions[0].ending(), Some((t, CommandKind::Restart)));
    }
    assert!(peers.waits_of(Some(0), Some(NetWait::Ending)) > 0);
    assert_eq!(
        peers.sessions[1].run(),
        1,
        "slot 1 restarted behind the silence"
    );
    window.set((u32::MAX, u32::MAX, u32::MAX));
    for _ in 0..10 {
        peers.round();
    }
    assert_eq!(
        peers.sessions[0].run(),
        1,
        "the packets of run 1 pass the barrier"
    );
    check_ended(&mut peers, &oracle, 2, t, 0, "silent peer");
    check_next_run(&mut peers, &oracle, "silent peer");
}

#[test]
fn the_acked_arm_passes_the_barrier() {
    let sc = script(2, 612, 2, None);
    let mut peers = Peers::new(sc, 2, 7, clean(1));
    // Slot 1 never hears the command (so it never sends a packet of run 1), and its acks are lost
    // until `release`: the leader's `acked` of it stays below T.
    let rounds = peers.round_cell();
    let release = Rc::new(Cell::new((u32::MAX, u32::MAX)));
    let seen = Rc::clone(&release);
    peers.set_drop_if(Box::new(move |from, to, p| {
        let (acks, release) = seen.get();
        let r = rounds.get();
        (from == 0 && to == 1 && matches!(p, Packet::Command(_)))
            || (from == 1 && to == 0 && matches!(p, Packet::Ack(_)) && (acks..release).contains(&r))
    }));
    while peers.sessions[0].session().confirmed() < 290 {
        peers.round();
    }
    release.set((peers.net.round(), u32::MAX));
    for _ in 0..10 {
        peers.round();
    }
    let t = peers.sessions[0]
        .end_run(CommandKind::Restart)
        .expect("a restart");
    assert_eq!(peers.sessions[0].run(), 0, "acked is below T at the stamp");
    for _ in 0..20 {
        peers.round();
        assert_eq!(peers.sessions[0].run(), 0, "round {}", peers.net.round());
    }
    release.set((release.get().0, peers.net.round() + 1));
    let mut waited = 0;
    while peers.sessions[0].run() == 0 {
        peers.round();
        waited += 1;
        assert!(waited <= 3, "the leader did not restart on slot 1's acks");
    }
    assert_eq!(peers.sessions[1].run(), 0, "slot 1 never heard the command");
    let ended = peers.sessions[0].take_ended_run().expect("an ended run");
    assert_eq!((ended.run, ended.tick), (0, t));
}

#[test]
fn the_old_command_is_resent_until_the_peer_catches_up() {
    let sc = script(3, 612, 2, None);
    let oracle = straight(&sc, 3);
    let mut peers = Peers::new(sc, 3, 7, clean(1));
    let rounds = peers.round_cell();
    let window = Rc::new(Cell::new((u32::MAX, u32::MAX)));
    let seen = Rc::clone(&window);
    peers.set_drop_if(Box::new(move |from, to, p| {
        let (lo, hi) = seen.get();
        from == 0 && to == 2 && matches!(p, Packet::Command(_)) && (lo..hi).contains(&rounds.get())
    }));
    while peers.sessions[0].session().confirmed() < 300 {
        peers.round();
    }
    let t = peers.sessions[0]
        .end_run(CommandKind::Restart)
        .expect("a restart");
    let from = peers.net.round() + 1;
    window.set((from, from + 40));
    // Per round: the runs of slots 0, 1 and 2 before and after it.
    let mut books: Vec<(u32, [u8; 3], [u8; 3])> = Vec::new();
    let mut leader_restart = None;
    let mut slot1_restart = None;
    while peers.sessions.iter().any(|s| s.run() == 0) || peers.net.round() < from + 40 {
        assert!(peers.net.round() < ROUND_LIMIT, "never restarted");
        let before = [0, 1, 2].map(|i| peers.sessions[i].run());
        peers.round();
        let after = [0, 1, 2].map(|i| peers.sessions[i].run());
        let r = peers.net.round();
        if after[0] == 1 && leader_restart.is_none() {
            leader_restart = Some(r);
        }
        if after[1] == 1 && slot1_restart.is_none() {
            slot1_restart = Some(r);
        }
        books.push((r, before, after));
    }
    let leader_restart = leader_restart.expect("the leader restarted");
    let slot1_restart = slot1_restart.expect("slot 1 restarted");
    assert!(
        books.iter().any(|(_, _, a)| a[0] == 1 && a[2] == 0),
        "the leader reaches run 1 while slot 2 is at run 0"
    );
    let sent = peers.sent();
    let old_command = |r: u32, to: usize| {
        sent.iter().any(|(round, a, b, p)| {
            *round == r
                && *a == 0
                && *b == to
                && matches!(p, Packet::Command(c) if c.run == 0 && c.tick == t && c.kind == CommandKind::Restart)
        })
    };
    for (r, before, after) in &books {
        if after[0] == 1 && before[2] == 0 && after[2] == 0 {
            assert!(old_command(*r, 2), "round {r}: no old command to slot 2");
        }
        if *r >= leader_restart.max(slot1_restart) + 2 {
            assert!(
                !old_command(*r, 1),
                "round {r}: old command to caught-up slot 1"
            );
        }
    }
    check_ended(&mut peers, &oracle, 3, t, 0, "resend");
    check_next_run(&mut peers, &oracle, "resend");
}
