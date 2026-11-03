//! `Engine::step`, `RunRecorder::step` and `RunRecorder::picks` take one entry per slot, present
//! or not (ADR-SIM-7, ADR-REPL-5): an absent slot's input and pick are ignored, and the step log
//! is still written dense, one block per present slot (ADR-REPL-5).

use crate::common::run::trial_setup;
use crate::zone_bot::run_input;
use stella_rain_core::engine::Engine;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::{RunRecorder, RunReplayError, verify_run};
use stella_rain_core::{RangeStep, StepRange};

/// The state hash after 600 ticks of the scripted solo run in `a_solo_run_is_the_same_run`,
/// read from the engine before the step took one entry per slot, and re-pinned for the slot
/// generations.
const H600: u64 = 0x56a0_02f0_dbab_94ea;

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).unwrap()
}

fn d() -> Input {
    Input::default()
}

/// Everything set, a pick included: what a caller must not be able to leak through an absent slot.
fn garbage() -> Input {
    Input {
        main: true,
        move_x: 127,
        move_y: -127,
        evade: true,
        skill: true,
        smash: true,
        pick: Some(0),
        ..Input::default()
    }
}

/// The scripted input of `slot` at `tick`.
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

fn holder(engine: &Engine, n: u8) -> u8 {
    (0..n)
        .find(|i| engine.check_pick(slot(*i), 0).is_ok())
        .expect("an open offer belongs to a slot")
}

fn steps(rec: &RunRecorder) -> StepRange {
    let replay = rec.replay();
    replay.step_range(0, replay.ticks, u32::MAX).unwrap()
}

// A. A solo run is the same run.

#[test]
fn a_solo_run_is_the_same_run() {
    let mut plain = Engine::start(&trial_setup()).unwrap();
    let mut noisy = Engine::start(&trial_setup()).unwrap();
    for t in 0..600 {
        let (a, b) = if plain.offer_open() {
            (Input::picking(0), Input::picking(0))
        } else {
            (input_of(0, t), input_of(0, t))
        };
        plain.step(&[a, d(), d()]);
        noisy.step(&[b, garbage(), garbage()]);
        assert_eq!(plain.state_hash(), noisy.state_hash(), "tick {t}");
        assert_eq!(plain.events(), noisy.events(), "tick {t}");
    }
    assert_eq!(plain.tick(), noisy.tick());
    assert_eq!(plain.state_hash(), H600);
}

// B. An absent slot's input is ignored in a party.

#[test]
fn an_absent_slots_input_is_ignored_in_a_party() {
    let mut plain = Engine::start_party(&trial_setup(), 2).unwrap();
    let mut noisy = Engine::start_party(&trial_setup(), 2).unwrap();
    let mut t = 0;
    while !plain.offer_open() && t < 5400 {
        let (a, b) = (run_input(&plain, t), input_of(1, t));
        plain.step(&[a, b, d()]);
        noisy.step(&[a, b, garbage()]);
        assert_eq!(plain.state_hash(), noisy.state_hash(), "tick {t}");
        t += 1;
    }
    assert!(t >= 300, "the party plays at least 300 ticks, {t}");
    assert!(plain.offer_open(), "the run reaches an offer");
    let h = holder(&plain, 2);

    let (hash, tick) = (plain.state_hash(), plain.tick());
    plain.step(&[d(), d(), Input::picking(0)]);
    assert_eq!(plain.state_hash(), hash);
    assert_eq!(plain.tick(), tick);
    assert!(plain.events().is_empty());
    assert!(plain.offer_open());

    let mut inputs = [d(); 3];
    inputs[usize::from(h)] = Input::picking(0);
    plain.step(&inputs);
    assert!(!plain.events().is_empty(), "the holder's pick is taken");
}

// C. The recorder writes the step log dense.

fn record(n: u8, ticks: u32, fill: Input) -> RunRecorder {
    let mut rec = RunRecorder::start_party(&trial_setup(), n).unwrap();
    for t in 0..ticks {
        while rec.engine().offer_open() {
            let h = holder(rec.engine(), n);
            rec.choose(slot(h), 0).unwrap();
        }
        let mut inputs = [fill; 3];
        for (s, input) in inputs.iter_mut().enumerate().take(usize::from(n)) {
            *input = input_of(s, t);
        }
        rec.step(&inputs).unwrap();
    }
    rec
}

#[test]
fn the_step_log_is_dense_for_a_solo_run_and_a_party() {
    for n in [1u8, 2] {
        let plain = record(n, 300, d());
        let noisy = record(n, 300, garbage());
        let bytes = plain.replay().to_bytes();
        assert_eq!(bytes, noisy.replay().to_bytes(), "party of {n}");
        verify_run(&bytes).unwrap();
        for step in &steps(&noisy).steps {
            if let RangeStep::Ticks { inputs, .. } = step {
                assert_eq!(inputs.len(), usize::from(n));
            }
        }
    }
}

// D. `picks` ignores an absent slot's pick.

#[test]
fn picks_ignores_an_absent_slots_pick() {
    let reach = || {
        let mut rec = RunRecorder::start_party(&trial_setup(), 2).unwrap();
        for t in 0..5400 {
            if rec.engine().offer_open() {
                return rec;
            }
            let a = run_input(rec.engine(), t);
            rec.step(&[a, input_of(1, t), d()]).unwrap();
        }
        panic!("no offer within 5400 ticks");
    };
    let mut rec = reach();
    let h = usize::from(holder(rec.engine(), 2));

    let (bytes, tick) = (rec.replay().to_bytes(), rec.engine().tick());
    let mut only_absent = [None; 3];
    only_absent[2] = Some(0);
    assert_eq!(
        rec.picks(&only_absent).unwrap_err(),
        RunReplayError::BadInputs("a step of picks holds a pick")
    );
    assert_eq!(rec.replay().to_bytes(), bytes);
    assert_eq!(rec.engine().tick(), tick);

    let mut noisy = reach();
    let mut with = [None; 3];
    with[h] = Some(0);
    let without = with;
    with[2] = Some(7);
    noisy.picks(&with).unwrap();
    rec.picks(&without).unwrap();
    assert_eq!(noisy.replay().to_bytes(), rec.replay().to_bytes());
    match steps(&noisy).steps.last().unwrap() {
        RangeStep::Picks { picks, .. } => assert_eq!(picks.len(), 2),
        other => panic!("the last step is not picks: {other:?}"),
    }

    let mut party = reach();
    assert_eq!(
        party.choose(slot(2), 0).unwrap_err(),
        RunReplayError::BadInputs("a player outside the party")
    );
}
