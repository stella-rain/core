//! A party changes at a tick (ADR-SIM-7 point 1, ADR-SIM-8 point 5, ADR-REPL-5): the engine
//! joins and leaves a slot between steps, the step log carries the change, and record, resume,
//! `replay_at`, save and verify agree on a run whose presence mask moves.

use crate::common::run::trial_setup;
use crate::zone_bot::run_input;
use stella_rain_core::engine::{Engine, Outcome, PresenceError};
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).unwrap()
}

fn a(t: u32) -> Input {
    Input {
        move_x: if t / 40 % 2 == 0 { 100 } else { -100 },
        main: true,
        ..Input::default()
    }
}

fn d() -> Input {
    Input::default()
}

fn advance(engine: &mut Engine, from: u32, to: u32) {
    for t in from..to {
        let first = run_input(engine, t);
        engine.step(&[first, d(), d()]);
    }
}

#[test]
fn a_join_and_a_leave_move_the_mask_and_the_slots_generation() {
    let mut engine = Engine::start(&trial_setup()).unwrap();
    advance(&mut engine, 0, 120);
    let h0 = engine.state_hash();
    assert_eq!(engine.present(), 0b001);
    engine.join(slot(1)).unwrap();
    assert_eq!(engine.players(), 2);
    assert_eq!(engine.present(), 0b011);
    assert!(engine.events().is_empty());
    engine.leave(slot(1)).unwrap();
    assert_eq!(engine.present(), 0b001);
    // The leave advanced slot 1's generation, which the hash reads: the party of one is the
    // same, the state is not.
    assert_ne!(engine.state_hash(), h0);
}

#[test]
fn a_join_at_tick_zero_is_the_party_start() {
    let setup = trial_setup();
    let mut two = Engine::start(&setup).unwrap();
    two.join(slot(1)).unwrap();
    assert_eq!(
        two.state_hash(),
        Engine::start_party(&setup, 2).unwrap().state_hash()
    );
    two.join(slot(2)).unwrap();
    let three = Engine::start_party(&setup, 3).unwrap().state_hash();
    assert_eq!(two.state_hash(), three);
    let mut other = Engine::start(&setup).unwrap();
    other.join(slot(2)).unwrap();
    other.join(slot(1)).unwrap();
    assert_eq!(other.state_hash(), three);
}

#[test]
fn a_hole_steps_and_ignores_the_absent_slot() {
    let setup = trial_setup();
    let run = |junk: Input| {
        let mut e = Engine::start(&setup).unwrap();
        e.join(slot(2)).unwrap();
        assert_eq!(e.present(), 0b101);
        assert_eq!(e.players(), 2);
        for t in 0..120 {
            e.step(&[a(t), junk, d()]);
        }
        e.state_hash()
    };
    assert_eq!(
        run(d()),
        run(Input {
            move_x: 90,
            main: true,
            ..d()
        })
    );
}

#[test]
fn a_refusal_changes_nothing() {
    let mut e = Engine::start(&trial_setup()).unwrap();
    advance(&mut e, 0, 30);
    let h = e.state_hash();
    assert_eq!(
        e.join(PlayerIdx::SOLO),
        Err(PresenceError::Occupied {
            slot: PlayerIdx::SOLO
        })
    );
    assert_eq!(
        e.leave(slot(1)),
        Err(PresenceError::Absent { slot: slot(1) })
    );
    assert_eq!(e.leave(PlayerIdx::SOLO), Err(PresenceError::FirstSlot));
    e.join(slot(1)).unwrap();
    assert_eq!(
        e.join(slot(1)),
        Err(PresenceError::Occupied { slot: slot(1) })
    );
    e.leave(slot(1)).unwrap();
    // The leave advanced slot 1's generation (hashed), so the hash is not where it was.
    assert_ne!(e.state_hash(), h);
}

#[test]
fn a_call_on_an_ended_run_is_refused() {
    let mut e = Engine::start(&trial_setup()).unwrap();
    for t in 0..200_000 {
        if e.outcome() != Outcome::Running {
            break;
        }
        advance(&mut e, t, t + 1);
    }
    assert_ne!(e.outcome(), Outcome::Running, "the trial ends");
    assert_eq!(e.join(slot(1)), Err(PresenceError::Ended));
    assert_eq!(e.leave(slot(1)), Err(PresenceError::Ended));
}

#[test]
fn a_joiner_is_a_fresh_player_at_the_first_players_place() {
    let mut e = Engine::start(&trial_setup()).unwrap();
    let mut t = 0;
    while e.snapshot().progress.as_ref().is_some_and(|p| p.level < 2) {
        advance(&mut e, t, t + 1);
        t += 1;
        assert!(t < 3_000, "level 2 is reached");
    }
    let lvl0 = e.snapshot().progress.as_ref().unwrap().level;
    e.join(slot(1)).unwrap();
    let snap = e.snapshot();
    assert_eq!(snap.players.len(), 2);
    assert_eq!(
        snap.progress.as_ref().unwrap().level,
        lvl0,
        "slot 0 keeps its level"
    );
    assert_eq!(snap.players[1].at, snap.players[0].at);
    assert!(!e.offer_open());
}

// C. The recorder and the verifier.

use stella_rain_core::RangeStep;
use stella_rain_core::run_replay::RunReplay;
use stella_rain_core::run_replay::{RunRecorder, RunReplayError, verify_run};

fn rec_steps(rec: &mut RunRecorder, from: u32, to: u32, inputs: impl Fn(u32) -> [Input; 3]) {
    for t in from..to {
        rec.step(&inputs(t)).unwrap();
    }
}

fn scripted(t: u32) -> [Input; 3] {
    [
        Input {
            move_x: if t / 40 % 2 == 0 { 100 } else { -100 },
            ..d()
        },
        Input { move_y: 50, ..d() },
        Input { move_x: -30, ..d() },
    ]
}

#[test]
fn a_solo_run_that_joins_and_leaves_verifies() {
    let mut rec = RunRecorder::start(&trial_setup()).unwrap();
    rec_steps(&mut rec, 0, 60, scripted);
    rec.join(slot(1)).unwrap();
    rec_steps(&mut rec, 60, 120, scripted);
    rec.leave(slot(1)).unwrap();
    rec_steps(&mut rec, 120, 180, scripted);
    let replay = rec.replay();
    let verified = verify_run(&replay.to_bytes()).unwrap();
    assert_eq!(verified.ticks, 180);
    let range = replay.step_range(0, 180, u32::MAX).unwrap();
    assert_eq!(range.present, 0b1);
    let kinds: Vec<&str> = range
        .steps
        .iter()
        .map(|s| match s {
            RangeStep::Actions { .. } => unreachable!("no equipment action here"),
            RangeStep::Ticks { .. } => "t",
            RangeStep::Picks { .. } => "p",
            RangeStep::Join { .. } => "j",
            RangeStep::Leave { .. } => "l",
        })
        .collect();
    let changes: Vec<&&str> = kinds.iter().filter(|k| **k == "j" || **k == "l").collect();
    assert_eq!(changes, [&"j", &"l"]);
    assert_eq!(rec.engine().tick(), 180);
}

#[test]
fn a_party_that_loses_slot_one_logs_two_entries_and_verifies() {
    let mut rec = RunRecorder::start_party(&trial_setup(), 3).unwrap();
    rec_steps(&mut rec, 0, 50, scripted);
    rec.leave(slot(1)).unwrap();
    assert_eq!(rec.engine().present(), 0b101);
    rec_steps(&mut rec, 50, 150, scripted);
    rec.join(slot(1)).unwrap();
    rec_steps(&mut rec, 150, 200, scripted);
    let replay = rec.replay();
    verify_run(&replay.to_bytes()).unwrap();
    let range = replay.step_range(0, 200, u32::MAX).unwrap();
    let lens: Vec<usize> = range
        .steps
        .iter()
        .filter_map(|s| match s {
            RangeStep::Ticks { inputs, .. } => Some(inputs.len()),
            _ => None,
        })
        .collect();
    assert!(lens.contains(&2), "a hole holds two entries: {lens:?}");
    assert_eq!(
        replay.unpack_inputs(u32::MAX).unwrap_err(),
        RunReplayError::BadInputs("a party has no solo view")
    );
}

#[test]
fn the_recorder_refuses_a_bad_change_and_records_nothing() {
    let mut rec = RunRecorder::start_party(&trial_setup(), 2).unwrap();
    let before = rec.replay().to_bytes();
    for r in [rec.join(slot(1)), rec.join(PlayerIdx::SOLO)] {
        assert!(matches!(
            r,
            Err(RunReplayError::PresenceRefused {
                error: PresenceError::Occupied { .. },
                ..
            })
        ));
    }
    assert!(matches!(
        rec.leave(slot(2)),
        Err(RunReplayError::PresenceRefused {
            error: PresenceError::Absent { .. },
            ..
        })
    ));
    assert!(matches!(
        rec.leave(PlayerIdx::SOLO),
        Err(RunReplayError::PresenceRefused {
            error: PresenceError::FirstSlot,
            ..
        })
    ));
    assert_eq!(rec.replay().to_bytes(), before);
    rec.leave(slot(1)).unwrap();
    rec.join(slot(2)).unwrap();
}

#[test]
fn the_recorder_stops_at_sixty_four_changes() {
    let mut rec = RunRecorder::start(&trial_setup()).unwrap();
    for _ in 0..32 {
        rec.join(slot(1)).unwrap();
        rec.leave(slot(1)).unwrap();
    }
    assert_eq!(
        rec.join(slot(1)).unwrap_err(),
        RunReplayError::BadInputs("more joins and leaves than the cap")
    );
}

// D. Resume, replay_at and save across a presence change.

use stella_rain_core::pack::Pack;

/// A party of 3 that loses slot 1 at 50, slot 2 at 100 and takes slot 2 back at 150.
fn churn(from: u32, to: u32, rec: Option<RunRecorder>) -> RunRecorder {
    let mut rec = rec.unwrap_or_else(|| RunRecorder::start_party(&trial_setup(), 3).unwrap());
    for t in from..to {
        match t {
            50 => rec.leave(slot(1)).unwrap(),
            100 => rec.leave(slot(2)).unwrap(),
            150 => rec.join(slot(2)).unwrap(),
            _ => {}
        }
        rec_steps(&mut rec, t, t + 1, scripted);
    }
    rec
}

const END: u32 = 220;
const CUTS: [u32; 5] = [25, 75, 125, 180, END];

#[test]
fn resume_continues_a_run_with_changes_at_every_cut() {
    let whole = churn(0, END, None);
    verify_run(&whole.replay().to_bytes()).unwrap();
    assert_eq!(whole.engine().present(), 0b101);
    for cut in CUTS {
        let live = churn(0, cut, None);
        let prefix = live.replay();
        let mut resumed = RunRecorder::resume_with_pack(&prefix, Pack::bundled()).unwrap();
        assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
        assert_eq!(resumed.engine().present(), live.engine().present());
        resumed = churn(cut, END, Some(resumed));
        assert_eq!(resumed.replay(), whole.replay(), "resume at {cut}");
    }
}

#[test]
fn a_savepoint_in_any_window_round_trips() {
    let whole = churn(0, END, None);
    for cut in CUTS {
        let live = churn(0, cut, None);
        let (prefix, bytes) = (live.replay(), live.save());
        let mut resumed =
            RunRecorder::resume_from_savepoint(&bytes, &prefix, Pack::bundled()).unwrap();
        assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
        assert_eq!(
            resumed.engine().player_stats(),
            live.engine().player_stats()
        );
        assert_eq!(resumed.save(), bytes, "re-saving at {cut}");
        resumed = churn(cut, END, Some(resumed));
        assert_eq!(resumed.replay(), whole.replay(), "save at {cut}");
    }
}

#[test]
fn replay_at_cuts_a_log_with_changes() {
    let whole = churn(0, END, None);
    let full = whole.replay();
    for t in [0, 50, 75, 100, 150, 200, END] {
        let at = whole.replay_at(t, 0);
        assert_eq!(at.ticks, t);
        // A cut keeps the picks and changes that follow its last tick, as the range reader
        // does at its end; the world steps are the same.
        let worlds = |r: &RunReplay| -> Vec<RangeStep> {
            r.step_range(0, t, u32::MAX)
                .unwrap()
                .steps
                .into_iter()
                .filter(|s| matches!(s, RangeStep::Ticks { .. }))
                .collect()
        };
        assert_eq!(worlds(&at), worlds(&full), "tick {t}");
    }
    let past = whole.replay_at(END + 1000, 0);
    assert_eq!(past.inputs, full.inputs);
}

#[test]
fn a_savepoint_against_a_prefix_with_another_mask_is_refused() {
    let live = churn(0, 75, None);
    let bytes = live.save();
    let before_the_leave = churn(0, 25, None).replay();
    assert!(
        RunRecorder::resume_from_savepoint(&bytes, &before_the_leave, Pack::bundled()).is_err()
    );
    let solo = RunRecorder::start(&trial_setup()).unwrap().replay();
    assert!(RunRecorder::resume_from_savepoint(&bytes, &solo, Pack::bundled()).is_err());
}
