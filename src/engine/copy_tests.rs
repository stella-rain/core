//! `sim_v1` is a copy of `sim` that owns its types (ADR-REPL-2): the frozen corpus is the proof
//! that the copy equals what was released. The live rules are the equipment mode and differ from
//! version 1 by design (Starlight, the slots), so the two are no longer compared tick by tick.

use super::*;
use crate::bench_fixtures;
use crate::pack::Pack;
use crate::run_replay::RunRecorder;
use crate::savepoint::{Savepoint, SavepointError, restore};

/// The pack the frozen version 1 `easy`, `rainy`, `time_limit`, `trial` and `tutorial` run cases
/// play on: the rules and content of version 1 as they were released, not the bundled files (which
/// version 2 edits). A test that starts the version 1 rules builds from this.
pub(super) fn v1_pack() -> &'static Pack {
    static PACK: std::sync::OnceLock<Pack> = std::sync::OnceLock::new();
    PACK.get_or_init(|| {
        Pack::from_bytes(include_bytes!(
            "../../tests/corpus/v1/packs/32ec7296e6d6391476878f95571c434f7a15ed74236d26988dd8d88573bc3dd5.json"
        ))
        .expect("the version 1 pack")
    })
}

/// The version 1 pack's `time_limit` mode file, as JSON a test may edit before it parses it.
pub(super) fn v1_time_limit_json() -> serde_json::Value {
    let pack: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../tests/corpus/v1/packs/32ec7296e6d6391476878f95571c434f7a15ed74236d26988dd8d88573bc3dd5.json"
    ))
    .expect("the version 1 pack is JSON");
    pack["modes"]["time_limit"].clone()
}

#[test]
fn version_one_runs_sim_v1() {
    let frozen = RunSetup {
        schema_version: 1,
        sim_version: 1,
        ..RunSetup::default()
    };
    let (a, b) = (
        Engine::start_with(&frozen, v1_pack().content()).unwrap(),
        Engine::start(&RunSetup::default()).unwrap(),
    );
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
    // A step only the live rules have (a party's soft tether, which solo skips: ADR-SIM-7)
    // is left out; the frozen schedule never gains one. A step only the frozen rules have (the
    // star offers' `open_offers`, which the live rules no longer open: ADR-REPL-5) is left out
    // of the frozen list.
    let mut live = tick_names(include_str!("../sim/schedule.rs"));
    live.retain(|name| *name != "tether");
    let mut frozen = tick_names(include_str!("../sim_v1/schedule.rs"));
    frozen.retain(|name| *name != "open_offers");
    assert!(live.len() > 10);
    assert_eq!(live, frozen);
}

fn recorder_at(version: u32) -> RunRecorder {
    let setup = RunSetup {
        sim_version: version,
        mode: "time_limit".into(),
        seed: 3,
        ..RunSetup::default()
    };
    // Version 1 plays its own pack, the rules and content as released; version 2 the bundled one.
    let mut r = Engine::forced(version, || {
        if version == 1 {
            RunRecorder::start_with_pack(&setup, v1_pack()).unwrap()
        } else {
            RunRecorder::start(&setup).unwrap()
        }
    });
    for _ in 0..120 {
        r.step(&[
            Input {
                main: true,
                ..Input::default()
            },
            Default::default(),
            Default::default(),
        ])
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
            sim_version: version,
            mode: "time_limit".into(),
            seed: 3,
            ..RunSetup::default()
        };
        let fresh = Engine::forced(version, || {
            if version == 1 {
                Engine::start_with(&setup, v1_pack().content()).unwrap()
            } else {
                Engine::start(&setup).unwrap()
            }
        });
        let (state, ledger, counted, stats) = decode_state(&encode_state(e), &fresh).unwrap();
        let back = fresh.with_saved(state, ledger, counted, stats);
        assert_eq!(back.state_hash(), e.state_hash(), "version {version}");
        assert_eq!(back.run.version(), version);
    }
}

#[test]
fn a_savepoint_of_another_version_than_the_setup_is_refused() {
    let r = recorder_at(1);
    let bytes = r.save();
    let mut prefix = r.replay();
    assert!(restore(&bytes, &prefix, v1_pack()).is_ok());
    for other in [0, 2] {
        prefix.setup.sim_version = other;
        assert_eq!(
            restore(&bytes, &prefix, v1_pack()).err(),
            Some(SavepointError::SimVersion),
            "{other}"
        );
    }
}

#[test]
fn the_bench_cases_run_the_live_rules() {
    use bench_fixtures::{
        hit_tick, hit_tick_spread, time_limit_tick, worst_minute, worst_minute_peak, worst_tick,
    };
    let engines = [
        ("worst_tick", worst_tick().engine),
        ("hit_tick", hit_tick().engine),
        ("hit_tick_spread", hit_tick_spread().engine),
        ("time_limit_tick", time_limit_tick().engine),
        ("worst_minute", worst_minute().engine),
        ("worst_minute_peak", worst_minute_peak().engine),
    ];
    for (name, e) in engines {
        assert_eq!(e.run.module(), "sim", "{name}");
        assert_eq!(e.version(), crate::version::SIM_VERSION, "{name}");
    }
}

#[test]
fn the_frozen_v1_cases_run_sim_v1() {
    let root = super::savepoint_tests::data_root::data_root().join("tests/corpus/v1");
    let mut stages = 0;
    for dir in std::fs::read_dir(root.join("stages")).unwrap() {
        let stage = crate::validate::parse_and_validate(
            &std::fs::read(dir.unwrap().path().join("stage.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(Engine::new(&stage).unwrap().run.module(), "sim_v1");
        stages += 1;
    }
    assert!(stages > 0);
    let mut runs = 0;
    for dir in std::fs::read_dir(root.join("runs")).unwrap() {
        let replay = crate::run_replay::RunReplay::from_bytes(
            &std::fs::read(dir.unwrap().path().join("run.replay")).unwrap(),
        )
        .unwrap();
        let pack = std::fs::read_dir(root.join("packs"))
            .unwrap()
            .map(|p| Pack::from_bytes(&std::fs::read(p.unwrap().path()).unwrap()).unwrap())
            .find(|p| crate::content_hash::to_hex(&p.sha256()) == replay.content_sha256)
            .expect("the run's pack is kept");
        let e = Engine::start_with(&replay.setup, pack.content()).unwrap();
        assert_eq!(e.run.module(), "sim_v1");
        runs += 1;
    }
    assert!(runs > 0);
}
