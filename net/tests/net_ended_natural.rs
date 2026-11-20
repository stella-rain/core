//! A party run that ends Cleared or Failed hands out one ended run at every peer, cut at the
//! tick it ended, equal across peers, and its replay verifies with core. A Cancel or Restart
//! before the end is unchanged; after it, it hands out nothing new.

mod support;

use stella_rain_core::engine::Outcome;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{verify_run_against_with, verify_run_with};
use stella_rain_net::session::party::{Phase, StopReason};
use stella_rain_net::session::report::{EndKind, EndedRun};
use stella_rain_net::wire::CommandKind;
use support::peers::{Got, Peers};
use support::simnet::Impair;
use support::{Natural, cleared_three, failed_three, failed_two, leave_three};

/// Peers on `scn` over a clean net, each stopping `after` ticks past the end.
fn peers_of(scn: &Natural, after: u32) -> Peers {
    Peers::new(scn.script.clone(), scn.n, 1, Impair::clean(1)).stop_at(scn.end + after)
}

fn books(n: u8) -> Vec<Vec<Got>> {
    vec![Vec::new(); usize::from(n)]
}

/// Plays `scn` to 60 ticks past its end, draining each round.
fn play(scn: &Natural) -> Vec<Vec<Got>> {
    let mut peers = peers_of(scn, 60);
    let mut got = books(scn.n);
    peers.play_until(&mut got, |_, _| false);
    got
}

fn kind_of(outcome: Outcome) -> EndKind {
    match outcome {
        Outcome::Cleared => EndKind::Cleared,
        Outcome::Failed => EndKind::Failed,
        Outcome::Running => panic!("a scenario ends"),
    }
}

/// Every scenario, with the oracle.
fn scenarios() -> Vec<Natural> {
    vec![failed_two(), failed_three(), cleared_three()]
}

fn replay_bytes(ended: &EndedRun, scn: &Natural) -> Vec<u8> {
    ended
        .replay(&scn.script.setup, RunContent::bundled())
        .expect("a replay")
        .to_bytes()
}

#[test]
fn the_scenarios_end_as_planned() {
    for scn in scenarios() {
        assert_ne!(scn.outcome, Outcome::Running, "{}", scn.name);
    }
    assert_eq!(failed_two().outcome, Outcome::Failed);
    assert_eq!(failed_three().outcome, Outcome::Failed);
    assert_eq!(cleared_three().outcome, Outcome::Cleared);
}

#[test]
fn a_natural_end_hands_out_one_ended_run_at_every_peer() {
    for scn in scenarios() {
        let oracle = scn.oracle();
        let got = play(&scn);
        let first = &got[0][0].ended;
        for (i, list) in got.iter().enumerate() {
            let what = format!("{}: session {i}", scn.name);
            assert_eq!(list.len(), 1, "{what}: one ended run");
            let g = &list[0];
            let e = &g.ended;
            assert_eq!(
                (e.run, e.tick, e.party),
                (0, scn.end, scn.n),
                "{what}: run, tick, party"
            );
            assert_eq!(e.ticks.len(), scn.end as usize, "{what}: ticks");
            assert_eq!(e, first, "{what}: equal across sessions");
            assert!(
                g.confirmed >= scn.end,
                "{what}: came at confirmed {}",
                g.confirmed
            );
            assert_eq!(e.kind, kind_of(scn.outcome), "{what}: kind");
            for (k, t) in e.ticks.iter().enumerate() {
                assert_eq!(t.hash, oracle[k + 1], "{what}: tick {k}");
            }
        }
    }
}

#[test]
fn a_natural_end_leaves_the_session_playing() {
    let scn = failed_two();
    let mut peers = peers_of(&scn, 60);
    let mut got = books(scn.n);
    peers.play_until(&mut got, |_, _| false);
    for s in &peers.sessions {
        assert_eq!(s.phase(), Phase::Playing);
        assert_eq!(s.stop_reason(), None);
        assert_eq!(s.run(), 0);
    }
}

#[test]
fn its_replay_verifies_with_core() {
    for scn in scenarios() {
        let oracle = scn.oracle();
        let got = play(&scn);
        let mut all = Vec::new();
        for (i, list) in got.iter().enumerate() {
            let what = format!("{}: session {i}", scn.name);
            let ended = &list[0].ended;
            let replay = ended
                .replay(&scn.script.setup, RunContent::bundled())
                .expect("a replay");
            assert_eq!(replay.ticks, scn.end, "{what}");
            assert_eq!(
                replay.final_hash,
                format!("{:016x}", oracle[scn.end as usize]),
                "{what}"
            );
            let bytes = replay.to_bytes();
            let v = verify_run_with(&bytes, RunContent::bundled()).expect("verifies");
            assert_eq!((v.ticks, v.outcome), (scn.end, scn.outcome), "{what}");
            assert_eq!(v.final_hash, oracle[scn.end as usize], "{what}");
            verify_run_against_with(&bytes, RunContent::bundled(), &oracle[1..=scn.end as usize])
                .expect("every tick's hash is the oracle's");
            all.push(bytes);
        }
        assert!(
            all.windows(2).all(|w| w[0] == w[1]),
            "{}: equal bytes",
            scn.name
        );
    }
}

#[test]
fn a_run_that_has_not_ended_hands_out_nothing() {
    let scn = failed_two();
    let mut peers = peers_of(&scn, 0).stop_at(scn.end - 1);
    let mut got = books(scn.n);
    peers.play_until(&mut got, |_, _| false);
    assert!(got.iter().all(Vec::is_empty));
}

#[test]
fn a_natural_end_with_a_leave_before_it_still_verifies() {
    let scn = leave_three();
    let mut peers = Peers::new(scn.script.clone(), 3, 1, Impair::clean(1))
        .stop_at(scn.end + 60)
        .leaver(2, 300);
    let mut got = books(3);
    peers.play_until(&mut got, |_, _| false);
    let mut bytes = Vec::new();
    for (i, list) in got.iter().enumerate().take(2) {
        assert_eq!(list.len(), 1, "session {i}");
        let ended = &list[0].ended;
        assert_eq!((ended.tick, ended.party), (scn.end, 3), "session {i}");
        let v =
            verify_run_with(&replay_bytes(ended, &scn), RunContent::bundled()).expect("verifies");
        assert_eq!(
            (v.ticks, v.outcome),
            (scn.end, Outcome::Failed),
            "session {i}"
        );
        bytes.push(ended.clone());
    }
    assert_eq!(bytes[0], bytes[1]);
}

/// Rounds until the leader has confirmed `at`, then it ends the run with `kind`; the tick.
fn stamp(peers: &mut Peers, got: &mut [Vec<Got>], kind: CommandKind, at: u32) -> u32 {
    peers.play_until(got, |p, _| p.sessions[0].session().confirmed() >= at);
    peers.sessions[0]
        .end_run(kind)
        .expect("the leader ends the run")
}

#[test]
fn a_cancel_before_the_end_is_unchanged() {
    let scn = failed_two();
    let mut peers = peers_of(&scn, 60);
    let mut got = books(2);
    let t = stamp(&mut peers, &mut got, CommandKind::Cancel, 300);
    assert!(t < scn.end);
    peers.play_until(&mut got, |_, _| false);
    for (i, list) in got.iter().enumerate() {
        assert_eq!(list.len(), 1, "session {i}");
        let e = &list[0].ended;
        assert_eq!((e.tick, e.ticks.len()), (t, t as usize), "session {i}");
        assert_eq!(e.kind, EndKind::Cancel);
        let v = verify_run_with(&replay_bytes(e, &scn), RunContent::bundled()).expect("verifies");
        assert_eq!((v.ticks, v.outcome), (t, Outcome::Running));
        assert_eq!(
            peers.sessions[i].stop_reason(),
            Some(StopReason::EndedByLeader(CommandKind::Cancel))
        );
    }
}

#[test]
fn a_restart_before_the_end_is_unchanged() {
    let scn = failed_two();
    let mut peers = peers_of(&scn, 0).stop_at(600);
    let mut got = books(2);
    let t = stamp(&mut peers, &mut got, CommandKind::Restart, 300);
    assert!(t < scn.end);
    peers.play_until(&mut got, |_, _| false);
    for (i, list) in got.iter().enumerate() {
        assert_eq!(peers.sessions[i].run(), 1, "session {i}");
        assert_eq!(list.len(), 1, "session {i}: no second run");
        let e = &list[0].ended;
        assert_eq!((e.run, e.tick), (0, t), "session {i}");
        assert_eq!(e.kind, EndKind::Restart);
        let v = verify_run_with(&replay_bytes(e, &scn), RunContent::bundled()).expect("verifies");
        assert_eq!((v.ticks, v.outcome), (t, Outcome::Running));
    }
}

/// Plays until every session has handed out its natural end.
fn until_ended(peers: &mut Peers, got: &mut [Vec<Got>]) {
    peers.play_until(got, |_, g| g.iter().all(|l| !l.is_empty()));
}

#[test]
fn a_cancel_after_the_natural_end_hands_out_no_second_run() {
    let scn = failed_two();
    let mut peers = peers_of(&scn, 200);
    let mut got = books(2);
    until_ended(&mut peers, &mut got);
    let t = peers.sessions[0]
        .end_run(CommandKind::Cancel)
        .expect("the leader cancels");
    assert!(t >= scn.end);
    peers.play_until(&mut got, |_, _| false);
    for (i, list) in got.iter().enumerate() {
        assert_eq!(list.len(), 1, "session {i}: the natural end only");
        assert_eq!(list[0].ended.tick, scn.end);
        assert_eq!(
            peers.sessions[i].stop_reason(),
            Some(StopReason::EndedByLeader(CommandKind::Cancel)),
            "session {i}"
        );
    }
}

#[test]
fn a_cancel_before_the_natural_end_is_taken_keeps_the_natural_run() {
    let scn = failed_two();
    let mut peers = peers_of(&scn, 200);
    // No drain: the natural end waits to be taken while the Cancel finishes.
    while peers
        .sessions
        .iter()
        .any(|s| s.session().confirmed() < scn.end + 5)
    {
        peers.round();
    }
    let t = peers.sessions[0]
        .end_run(CommandKind::Cancel)
        .expect("the leader cancels");
    assert!(t >= scn.end);
    while peers.sessions.iter().any(|s| s.phase() != Phase::Stopped) {
        peers.round();
    }
    for (i, s) in peers.sessions.iter_mut().enumerate() {
        let e = s.take_ended_run().expect("the natural run");
        assert_eq!((e.tick, e.kind), (scn.end, EndKind::Failed), "session {i}");
        let v = verify_run_with(&replay_bytes(&e, &scn), RunContent::bundled()).expect("verifies");
        assert_eq!((v.ticks, v.outcome), (scn.end, Outcome::Failed));
        assert!(s.take_ended_run().is_none(), "session {i}: taken once");
    }
}

#[test]
fn a_restart_after_the_natural_end_hands_out_no_second_run_and_the_next_run_ends_again() {
    let scn = failed_two();
    let mut peers = peers_of(&scn, 120);
    let mut got = books(2);
    until_ended(&mut peers, &mut got);
    let t = peers.sessions[0]
        .end_run(CommandKind::Restart)
        .expect("the leader restarts");
    assert!(t >= scn.end);
    peers.play_until(&mut got, |p, _| p.sessions.iter().all(|s| s.run() == 1));
    assert!(
        got.iter().all(|l| l.len() == 1),
        "the restart hands out none"
    );
    // Run 1 plays the script again and ends at the same tick.
    peers.play_until(&mut got, |_, g| g.iter().all(|l| l.len() == 2));
    let mut bytes = Vec::new();
    for (i, list) in got.iter().enumerate() {
        assert_eq!(list.len(), 2, "session {i}");
        let e = &list[1].ended;
        assert_eq!(
            (e.run, e.tick, e.kind),
            (1, scn.end, EndKind::Failed),
            "session {i}"
        );
        let v = verify_run_with(&replay_bytes(e, &scn), RunContent::bundled()).expect("verifies");
        assert_eq!((v.ticks, v.outcome), (scn.end, Outcome::Failed));
        bytes.push(e.clone());
    }
    assert_eq!(bytes[0], bytes[1]);
}
