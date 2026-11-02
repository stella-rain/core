//! The rules dispatch (ADR-REPL-7 point 3): `step` and `choose` run the rules for a run whose
//! `sim_version` is in `1..=SIM_VERSION` and fail closed on any other, changing nothing.

use super::run::v1::{State, World};
use super::*;
use crate::pack::ContentFiles;
use crate::run_content::RunContent;
use crate::time_limit::TimeLimit;
use crate::version::{SCHEMA_VERSION, SIM_VERSION, Versions};

fn setup() -> RunSetup {
    RunSetup {
        schema_version: SCHEMA_VERSION,
        sim_version: 1,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        weapon_tree: None,
        seed: 1,
        star_pool: None,
        extra_rerolls: 0,
        upgrades: Vec::new(),
        perk_set: "duelist".into(),
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
        stat_priority: Vec::new(),
        pick_stats: true,
        difficulty: "normal".into(),
    }
}

fn engine() -> Engine {
    let mode = TimeLimit::parse(ContentFiles::bundled_file("time_limit"), Versions::CURRENT)
        .unwrap_or_else(|e| panic!("{e:?}"));
    Engine::start_time_limit(&setup(), RunContent::bundled(), &mode).unwrap()
}

/// The engine with its run's `sim_version` set to `v`, past what `Engine::new` checks.
fn at_version(v: u32) -> Engine {
    let mut e = engine();
    e.run.world_mut().stage.sim_version = v;
    e
}

/// The engine at `v` with a stat offer open.
fn offering_at(v: u32) -> Engine {
    let mut e = at_version(v);
    let progress = e.run.progress.as_mut().expect("a run has levels");
    progress.offers_due = 1;
    progress.offer_open = true;
    assert!(e.offer_open());
    e
}

#[test]
fn version_1_runs_the_rules() {
    let mut e = at_version(1);
    e.step(&[Input::default()]);
    assert_eq!(e.tick(), 1);
    assert_eq!(e.outcome(), Outcome::Running);
    let mut e = offering_at(1);
    assert_eq!(
        crate::engine::pick(&mut e, crate::id::PlayerIdx::SOLO, 0),
        Ok(())
    );
}

#[test]
fn another_version_fails_closed_without_running_the_rules() {
    for v in [0, SIM_VERSION + 1] {
        let mut e = at_version(v);
        e.step(&[Input::default()]);
        assert_eq!(e.tick(), 0, "{v}");
        assert_eq!(e.outcome(), Outcome::Failed, "{v}");
    }
}

#[test]
fn a_choice_at_another_version_is_unsupported_and_changes_nothing() {
    for v in [0, SIM_VERSION + 1] {
        let mut e = offering_at(v);
        let before = e.state_hash();
        assert_eq!(
            crate::engine::pick(&mut e, crate::id::PlayerIdx::SOLO, 0),
            Err(ChoiceError::Unsupported { sim_version: v }),
            "{v}"
        );
        assert_eq!(e.state_hash(), before, "{v}");
        assert_eq!(e.outcome(), Outcome::Running, "{v}");
        assert!(e.offer_open(), "{v}");
    }
}

#[test]
fn the_run_variant_is_the_stage_version() {
    let mode = TimeLimit::parse(ContentFiles::bundled_file("time_limit"), Versions::CURRENT)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let stage = validate::parse_and_validate(run_setup::mode_stage("trial").unwrap()).unwrap();
    let mut recorder = crate::run_replay::RunRecorder::start(&setup()).unwrap();
    recorder.step(Input::default()).unwrap();
    let prefix = recorder.replay();
    let (restored, _) =
        crate::savepoint::restore(&recorder.save(), &prefix, crate::pack::Pack::bundled()).unwrap();
    let engines = [
        Engine::new(&stage).unwrap(),
        Engine::start(&setup()).unwrap(),
        Engine::start_time_limit(&setup(), RunContent::bundled(), &mode).unwrap(),
        engine().clone(),
        restored,
    ];
    for e in &engines {
        assert_eq!(e.run.version(), e.run.declared_version());
    }
}

#[test]
fn the_run_owns_its_world() {
    let mode = TimeLimit::parse(ContentFiles::bundled_file("time_limit"), Versions::CURRENT)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let stage = validate::parse_and_validate(run_setup::mode_stage("trial").unwrap()).unwrap();
    let mut recorder = crate::run_replay::RunRecorder::start(&setup()).unwrap();
    recorder.step(Input::default()).unwrap();
    let prefix = recorder.replay();
    let (restored, _) =
        crate::savepoint::restore(&recorder.save(), &prefix, crate::pack::Pack::bundled()).unwrap();
    let engines = [
        Engine::new(&stage).unwrap(),
        Engine::start(&setup()).unwrap(),
        Engine::start_time_limit(&setup(), RunContent::bundled(), &mode).unwrap(),
        engine().clone(),
        restored,
    ];
    for e in &engines {
        assert_eq!(e.run.world().sim_version, e.run.declared_version());
        assert_eq!(e.run.world().sim_version, e.run.version());
    }
}

#[test]
fn the_run_is_its_world_and_state_and_a_tag_alone() {
    // No box: the enum holds the larger variant's world and state, and its tag.
    let largest = (size_of::<World>() + size_of::<State>())
        .max(size_of::<crate::sim::World>() + size_of::<crate::sim::state::State>());
    assert!(size_of::<Run>() >= largest);
    assert!(size_of::<Run>() <= largest + align_of::<Run>());
}

#[test]
fn a_setup_runs_the_rules_of_its_own_version() {
    for (version, module) in [(1, "sim_v1"), (2, "sim")] {
        for mode in ["trial", "time_limit"] {
            let setup = RunSetup {
                sim_version: version,
                mode: mode.into(),
                ..setup()
            };
            let e = Engine::start(&setup).unwrap_or_else(|e| panic!("{mode} {version}: {e:?}"));
            assert_eq!(e.version(), version, "{mode} {version}");
            assert_eq!(e.run.module(), module, "{mode} {version}");
        }
    }
}

#[test]
fn a_setup_of_another_version_fails_closed() {
    for version in [0, SIM_VERSION + 1] {
        for mode in ["trial", "time_limit"] {
            let setup = RunSetup {
                sim_version: version,
                mode: mode.into(),
                ..setup()
            };
            assert!(Engine::start(&setup).is_err(), "{mode} {version}");
        }
    }
}
