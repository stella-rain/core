//! `Engine::step`, `RunRecorder::step` and `RunRecorder::picks` take one entry per slot, present
//! or not (ADR-SIM-7, ADR-REPL-5): an absent slot's input and pick are ignored, and the step log
//! is still written dense, one block per present slot (ADR-REPL-5).

use crate::common::run::trial_setup;
use stella_rain_core::engine::Engine;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::{RunRecorder, verify_run};
use stella_rain_core::{RangeStep, StepRange};

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
    let setup = crate::common::equip_run::equipment_setup(trial_setup());
    let mut plain = Engine::start(&setup).unwrap();
    let mut noisy = Engine::start(&setup).unwrap();
    for t in 0..600 {
        let (a, b) = (input_of(0, t), input_of(0, t));
        plain.step(&[a, d(), d()]);
        noisy.step(&[b, garbage(), garbage()]);
        assert_eq!(plain.state_hash(), noisy.state_hash(), "tick {t}");
        assert_eq!(plain.events(), noisy.events(), "tick {t}");
    }
    assert_eq!(plain.tick(), noisy.tick());
}

// B. An absent slot's input is ignored in a party.

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
