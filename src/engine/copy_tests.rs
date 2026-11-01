//! `sim_v1` is a copy of `sim` that owns its types (ADR-REPL-9): an engine forced to version 1
//! and one forced to version 2 run the same rules, so every observable is equal after every
//! tick. Until a version 2 differs, this is the proof that the copy equals the original; the
//! frozen corpus is the proof that the copy equals what was released.

use super::*;
use crate::bench_fixtures::{self, Case};
use crate::pack::Pack;
use crate::run_replay::RunRecorder;
use crate::savepoint::{Savepoint, SavepointError, restore};

/// What a run shows after a tick, to compare two engines on.
fn observed(e: &Engine) -> (u64, Vec<DomainEvent>, crate::snapshot::Snapshot, Load) {
    (e.state_hash(), e.events().to_vec(), e.snapshot(), e.caps())
}

fn both<T>(make: impl Fn() -> T) -> (T, T) {
    (Engine::forced(1, &make), Engine::forced(2, &make))
}

fn assert_same_tick(a: &Engine, b: &Engine, what: &str, tick: u32) {
    assert_eq!(observed(a), observed(b), "{what}, tick {tick}");
    assert_eq!(a.tally().kills(), b.tally().kills(), "{what}, tick {tick}");
}

/// Steps both engines `ticks` ticks on `input`, comparing after each.
fn same_for(case: (Case, Case), ticks: u32, what: &str) {
    let (
        Case {
            engine: mut a,
            input,
        },
        Case { engine: mut b, .. },
    ) = case;
    assert_eq!((a.run.version(), b.run.version()), (1, 2), "{what}");
    assert_same_tick(&a, &b, what, 0);
    for tick in 1..=ticks {
        a.step(input);
        b.step(input);
        assert_same_tick(&a, &b, what, tick);
    }
}

#[test]
fn the_copy_equals_the_original_on_the_bench_cases() {
    same_for(both(bench_fixtures::worst_tick), 30, "worst_tick");
    same_for(both(bench_fixtures::hit_tick), 30, "hit_tick");
    // The `time_limit_tick` case builds its horde through `Engine::new_with`: it covers the
    // conversion of the horde spec and state at the `Run` boundary.
    same_for(both(bench_fixtures::time_limit_tick), 30, "time_limit_tick");
}

#[test]
fn the_copy_equals_the_original_on_the_first_ticks_of_the_worst_minute() {
    use bench_fixtures::worst_minute_case::{Run, input_at};
    let (mut a, mut b) = both(|| Run::fast_forwarded(0));
    for tick in 1..=600 {
        for run in [&mut a, &mut b] {
            while run.engine.offer_open() {
                run.engine.choose(0).unwrap();
            }
            run.engine.step(input_at(run.engine.tick()));
        }
        assert_same_tick(&a.engine, &b.engine, "worst_minute", tick);
    }
}

/// A scripted run: the main button held, a dash at tick 100 and 400, every offer taken at 0.
fn scripted(version: u32) -> (Engine, Vec<(u64, Vec<DomainEvent>)>) {
    let setup = RunSetup {
        mode: "time_limit".into(),
        seed: 7,
        ..RunSetup::default()
    };
    let mut e = Engine::forced(version, || Engine::start(&setup).unwrap());
    let mut seen = Vec::new();
    for tick in 1..=600u32 {
        while e.offer_open() {
            e.choose(0).unwrap();
            seen.push((e.state_hash(), e.events().to_vec()));
        }
        e.step(Input {
            main: true,
            move_x: if tick % 90 < 45 { 100 } else { -100 },
            evade: tick == 100 || tick == 400,
            ..Input::default()
        });
        seen.push((e.state_hash(), e.events().to_vec()));
    }
    (e, seen)
}

#[test]
fn the_copy_equals_the_original_on_a_scripted_run() {
    let ((a, seen_a), (b, seen_b)) = (scripted(1), scripted(2));
    assert_eq!(seen_a, seen_b);
    assert!(
        seen_a.iter().any(|(_, events)| !events.is_empty()),
        "the script makes something happen"
    );
    assert_eq!(a.snapshot(), b.snapshot());
}

#[test]
fn the_ledger_and_the_tallies_are_equal_for_both_versions() {
    let ((a, _), (b, _)) = (scripted(1), scripted(2));
    assert_eq!(a.ledger(), b.ledger());
    assert_eq!(a.tally().kills(), b.tally().kills());
    assert_eq!(
        a.tally().taken().collect::<Vec<_>>(),
        b.tally().taken().collect::<Vec<_>>()
    );
    assert_eq!(
        a.tally().tiers().collect::<Vec<_>>(),
        b.tally().tiers().collect::<Vec<_>>()
    );
}

#[test]
fn version_one_runs_sim_v1() {
    let (a, b) = (scripted(1).0, scripted(2).0);
    assert_eq!(a.run.module(), "sim_v1");
    assert_eq!(b.run.module(), "sim");
}

#[test]
fn the_constants_the_caps_read_are_equal_in_both_modules() {
    assert_eq!(
        (
            crate::sim::MAX_ATTACK_TASKS,
            crate::sim::MAX_ZONES_ALIVE,
            crate::sim::MAX_ORBS_ALIVE,
            crate::sim::MAX_EMITTER_STEPS_PER_TICK
        ),
        (
            crate::sim_v1::MAX_ATTACK_TASKS,
            crate::sim_v1::MAX_ZONES_ALIVE,
            crate::sim_v1::MAX_ORBS_ALIVE,
            crate::sim_v1::MAX_EMITTER_STEPS_PER_TICK
        )
    );
}

/// The names of the steps of `TICK`, in order, read from a schedule's source.
fn tick_names(src: &str) -> Vec<&str> {
    let (_, rest) = src.split_once("const TICK").expect("TICK");
    let (list, _) = rest.split_once("];").expect("the end of TICK");
    list.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect()
}

#[test]
fn the_two_schedules_name_the_same_steps_in_the_same_order() {
    let live = tick_names(include_str!("../sim/schedule.rs"));
    let frozen = tick_names(include_str!("../sim_v1/schedule.rs"));
    assert!(live.len() > 10);
    assert_eq!(live, frozen);
}

fn recorder_at(version: u32) -> RunRecorder {
    let setup = RunSetup {
        mode: "time_limit".into(),
        seed: 3,
        ..RunSetup::default()
    };
    let mut r = Engine::forced(version, || RunRecorder::start(&setup).unwrap());
    for _ in 0..120 {
        r.step(Input {
            main: true,
            ..Input::default()
        })
        .unwrap();
    }
    r
}

#[test]
fn a_savepoint_carries_the_version_of_its_engine() {
    for version in [1, 2] {
        let r = recorder_at(version);
        let file = Savepoint::from_bytes_in(&r.save(), 1, 2).unwrap();
        assert_eq!(file.header().sim_version, version);
    }
}

#[test]
fn the_state_of_either_version_round_trips_through_its_codec() {
    use crate::savepoint::state::{decode_state, encode_state};
    for version in [1, 2] {
        let r = recorder_at(version);
        let e = r.engine();
        let setup = RunSetup {
            mode: "time_limit".into(),
            seed: 3,
            ..RunSetup::default()
        };
        let fresh = Engine::forced(version, || Engine::start(&setup).unwrap());
        let (state, ledger, counted) = decode_state(&encode_state(e), &fresh).unwrap();
        let back = fresh.with_saved(state, ledger, counted);
        assert_eq!(back.state_hash(), e.state_hash(), "version {version}");
        assert_eq!(back.run.version(), version);
    }
}

#[test]
fn a_savepoint_of_another_version_than_the_setup_is_refused() {
    let r = recorder_at(1);
    let bytes = r.save();
    let mut prefix = r.replay();
    assert!(restore(&bytes, &prefix, Pack::bundled()).is_ok());
    for other in [0, 2] {
        prefix.setup.sim_version = other;
        assert_eq!(
            restore(&bytes, &prefix, Pack::bundled()).err(),
            Some(SavepointError::SimVersion),
            "{other}"
        );
    }
}

#[test]
fn a_stage_runs_the_same_under_either_version() {
    let stage = crate::validate::parse_and_validate(crate::run_setup::mode_stage("trial").unwrap())
        .unwrap();
    let mut a = Engine::new_forced(&stage, 1).unwrap();
    let mut b = Engine::new_forced(&stage, 2).unwrap();
    assert_eq!((a.run.module(), b.run.module()), ("sim_v1", "sim"));
    for tick in 1..=120 {
        let input = Input {
            main: tick % 3 == 0,
            move_x: 50,
            ..Input::default()
        };
        a.step(input);
        b.step(input);
        assert_same_tick(&a, &b, "trial", tick);
    }
}
