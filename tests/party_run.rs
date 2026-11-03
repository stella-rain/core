//! A party of 2 and 3 recorded natively: the engine starts it, the recorder steps and picks per
//! player, and record, resume, `replay_at`, save and verify agree (ADR-REPL-7 point 4,
//! ADR-REPL-10, ADR-SAVE-3). Slots 1 and 2 are passive in the rules for now, so no test here pins
//! what their movement does to the hash.

use crate::common::run::trial_setup;
use stella_rain_core::engine::{ChoiceError, Engine, Outcome};
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::pack::Pack;
use stella_rain_core::run_replay::{RunRecorder, RunReplay, RunReplayError, verify_run};
use stella_rain_core::savepoint::SavepointError;
use stella_rain_core::{RangeStep, StepRange};

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).unwrap()
}

/// The scripted input of `slot` at `tick`: three distinct patterns, so the log carries each slot.
fn input_of(slot: usize, tick: u32) -> Input {
    match slot {
        0 => Input {
            move_x: if tick / 40 % 2 == 0 { 100 } else { -100 },
            main: true,
            ..Input::default()
        },
        1 => Input {
            move_y: 50,
            skill: tick % 90 < 30,
            ..Input::default()
        },
        _ => Input {
            move_x: -30,
            evade: tick % 120 < 10,
            ..Input::default()
        },
    }
}

/// Answers every open offer, each by the slot that holds it, one `choose` at a time.
fn answer_offers(rec: &mut RunRecorder, n: u8) {
    while rec.engine().offer_open() {
        let holder = (0..n)
            .find(|i| rec.engine().check_pick(slot(*i), 0).is_ok())
            .expect("an open offer belongs to a slot");
        rec.choose(slot(holder), 0).unwrap();
    }
}

fn step_party(rec: &mut RunRecorder, n: u8, tick: u32) {
    answer_offers(rec, n);
    let inputs: Vec<Input> = (0..n).map(|s| input_of(usize::from(s), tick)).collect();
    rec.step(&inputs).unwrap();
}

fn play(n: u8, ticks: u32) -> RunRecorder {
    let mut rec = RunRecorder::start_party(&trial_setup(), n).unwrap();
    for t in 0..ticks {
        if rec.engine().outcome() != Outcome::Running {
            break;
        }
        step_party(&mut rec, n, t);
    }
    answer_offers(&mut rec, n);
    rec
}

fn pack() -> &'static Pack {
    Pack::bundled()
}

// A. The engine starts a party.

#[test]
fn the_engine_starts_a_party_of_one_to_three() {
    let setup = trial_setup();
    for n in 1..=3u8 {
        let engine = Engine::start_party_with(&setup, pack().content(), n).unwrap();
        assert_eq!(engine.players(), usize::from(n));
    }
    assert!(Engine::start_party_with(&setup, pack().content(), 0).is_err());
    assert!(Engine::start_party_with(&setup, pack().content(), 4).is_err());
}

#[test]
fn a_party_of_one_is_the_solo_start() {
    let setup = trial_setup();
    let mut a = Engine::start_with(&setup, pack().content()).unwrap();
    let mut b = Engine::start_party_with(&setup, pack().content(), 1).unwrap();
    for t in 0..120 {
        a.step(&[input_of(0, t)]);
        b.step(&[input_of(0, t)]);
    }
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn a_party_carries_the_setup_loadout_in_every_slot() {
    let setup = stella_rain_core::run_setup::RunSetup {
        stat_priority: vec![],
        ..trial_setup()
    };
    let a = Engine::start_party_with(&setup, pack().content(), 3).unwrap();
    let b = Engine::start_party_with(&setup, pack().content(), 3).unwrap();
    assert_eq!(a.state_hash(), b.state_hash());
    let solo = Engine::start_with(&setup, pack().content()).unwrap();
    assert_ne!(a.state_hash(), solo.state_hash(), "a party is another run");
}

// B. The recorder.

#[test]
fn a_recorded_party_verifies() {
    for n in [2u8, 3] {
        let rec = play(n, 300);
        let replay = rec.replay();
        let verified = verify_run(&replay.to_bytes()).unwrap();
        assert_eq!(verified.ticks, rec.engine().tick());
        let range = replay.step_range(0, replay.ticks, u32::MAX).unwrap();
        assert_eq!(range.present, (1u8 << n) - 1);
        let first = range
            .steps
            .iter()
            .find_map(|s| match s {
                RangeStep::Ticks { inputs, .. } => Some(inputs.clone()),
                RangeStep::Picks { .. } => None,
            })
            .unwrap();
        assert_eq!(first.len(), usize::from(n));
        for (s, got) in first.iter().enumerate() {
            let want = Input {
                pick: None,
                ..input_of(s, 0)
            };
            assert_eq!(*got, want, "slot {s}");
        }
    }
}

#[test]
fn a_stick_axis_of_minus_128_is_canonical() {
    let mut rec = RunRecorder::start_party(&trial_setup(), 2).unwrap();
    let a = Input {
        move_x: -128,
        ..Input::default()
    };
    let b = Input {
        move_x: -127,
        ..Input::default()
    };
    rec.step(&[Input::default(), a]).unwrap();
    rec.step(&[Input::default(), b]).unwrap();
    verify_run(&rec.replay().to_bytes()).unwrap();
}

#[test]
fn the_recorder_refuses_what_it_cannot_record() {
    let mut rec = RunRecorder::start_party(&trial_setup(), 2).unwrap();
    assert_eq!(
        rec.step(&[Input::default()]).unwrap_err(),
        RunReplayError::BadInputs("one input per player")
    );
    assert_eq!(rec.engine().tick(), 0);
    assert_eq!(
        rec.picks(&[None, None]).unwrap_err(),
        RunReplayError::BadInputs("a step of picks holds a pick")
    );
    assert!(matches!(
        rec.choose(slot(1), 0).unwrap_err(),
        RunReplayError::ChoiceRefused {
            error: ChoiceError::NoOffer,
            ..
        }
    ));
    assert!(matches!(
        rec.choose(slot(2), 0).unwrap_err(),
        RunReplayError::ChoiceRefused {
            error: ChoiceError::NoPlayer { .. },
            ..
        }
    ));
    assert_eq!(rec.replay().ticks, 0);
}

// C. Resume, replay_at, save.

fn offer_cut(n: u8) -> (RunRecorder, u32) {
    let mut rec = RunRecorder::start_party(&trial_setup(), n).unwrap();
    for t in 0..5400 {
        step_party_no_answer(&mut rec, n, t);
        if rec.engine().offer_open() {
            return (rec, t);
        }
    }
    panic!("no offer opened");
}

fn step_party_no_answer(rec: &mut RunRecorder, n: u8, tick: u32) {
    let inputs: Vec<Input> = (0..n).map(|s| input_of(usize::from(s), tick)).collect();
    rec.step(&inputs).unwrap();
}

#[test]
fn resume_continues_a_party_run_as_the_uninterrupted_run() {
    for n in [2u8, 3] {
        let live = play(n, 400);
        let cut = live.replay_at(live.engine().tick(), live.engine().state_hash());
        let mut resumed = RunRecorder::resume_with_pack(&cut, pack()).unwrap();
        assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
        let mut live = live;
        for t in 400..460 {
            step_party(&mut live, n, t);
            step_party(&mut resumed, n, t);
        }
        assert_eq!(resumed.replay(), live.replay());
    }
}

#[test]
fn resume_works_with_an_offer_open() {
    for n in [2u8, 3] {
        let (live, _) = offer_cut(n);
        let cut = live.replay();
        let mut resumed = RunRecorder::resume_with_pack(&cut, pack()).unwrap();
        assert!(resumed.engine().offer_open());
        assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
        let mut live = live;
        answer_offers(&mut live, n);
        answer_offers(&mut resumed, n);
        assert_eq!(resumed.replay(), live.replay());
    }
}

fn clipped(replay: &RunReplay, ticks: u32) -> StepRange {
    replay
        .step_range(0, ticks.min(replay.ticks), u32::MAX)
        .unwrap()
}

#[test]
fn replay_at_cuts_a_party_log() {
    for n in [2u8, 3] {
        let rec = play(n, 200);
        let full = rec.replay();
        for t in [0, 100, full.ticks] {
            let at = rec.replay_at(t, 0);
            assert_eq!(at.ticks, t);
            assert_eq!(
                at.step_range(0, t, u32::MAX).unwrap().steps,
                clipped(&full, t).steps,
                "party {n} tick {t}"
            );
        }
        // Past the end: nothing panics and everything recorded is kept.
        let past = rec.replay_at(full.ticks + 1000, 0);
        assert_eq!(past.inputs, full.inputs);
    }
}

#[test]
fn a_party_savepoint_round_trips() {
    for n in [2u8, 3] {
        let live = play(n, 300);
        let prefix = live.replay();
        let bytes = live.save();
        let mut resumed = RunRecorder::resume_from_savepoint(&bytes, &prefix, pack()).unwrap();
        assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
        assert_eq!(
            resumed.engine().player_stats(),
            live.engine().player_stats()
        );
        assert_eq!(resumed.save(), bytes);
        let mut live = live;
        for t in 300..400 {
            step_party(&mut live, n, t);
            step_party(&mut resumed, n, t);
        }
        assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
        assert_eq!(resumed.replay(), live.replay());
    }
}

#[test]
fn a_savepoint_of_one_party_size_against_another_prefix_is_refused() {
    let two = play(2, 200);
    let three = play(3, 200);
    let solo = play(1, 200);
    for (saved, other) in [(&two, &three), (&two, &solo), (&three, &two), (&solo, &two)] {
        let err = RunRecorder::resume_from_savepoint(&saved.save(), &other.replay(), pack())
            .err()
            .expect("refused");
        assert!(matches!(
            err,
            SavepointError::Damaged | SavepointError::PrefixMismatch | SavepointError::HashMismatch
        ));
    }
}

// D. Solo views refuse a party.

#[test]
fn the_solo_views_refuse_a_party() {
    for n in [2u8, 3] {
        let replay = play(n, 100).replay();
        let want = RunReplayError::BadInputs("a party has no solo view");
        assert_eq!(replay.unpack_inputs(u32::MAX).unwrap_err(), want);
        assert_eq!(replay.unpack_commands().unwrap_err(), want);
    }
    let solo = play(1, 100).replay();
    assert_eq!(solo.unpack_inputs(u32::MAX).unwrap().len(), 100);
}
