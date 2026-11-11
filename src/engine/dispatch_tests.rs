//! The rules dispatch (ADR-REPL-2 points 1 to 3): `step` and `choose` run the rules for a run whose
//! `sim_version` is in `1..=SIM_VERSION` and fail closed on any other, changing nothing.

use super::run::v1::{State, World};
use super::*;
use crate::version::{SCHEMA_VERSION, SIM_VERSION};

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
        difficulty: "normal".into(),
        ..RunSetup::default()
    }
}

fn engine() -> Engine {
    let content = super::copy_tests::v1_pack().content();
    Engine::start_time_limit(&setup(), content, content.time_limit()).unwrap()
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
    e.step(&[Input::default(), Default::default(), Default::default()]);
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
        e.step(&[Input::default(), Default::default(), Default::default()]);
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

const SOLO_PICK_0: [Option<u32>; crate::id::MAX_PLAYERS as usize] = [Some(0), None, None];

#[test]
fn the_seam_takes_option_0_of_an_open_offer() {
    let mut e = offering_at(1);
    let before = e.state_hash();
    assert_eq!(e.v1_pick_step(&SOLO_PICK_0), Ok(()));
    assert_ne!(e.state_hash(), before);
    assert!(!e.events().is_empty());
}

#[test]
fn the_seam_refuses_a_pick_with_no_offer() {
    let mut e = at_version(1);
    let before = e.state_hash();
    assert_eq!(e.v1_pick_step(&SOLO_PICK_0), Err(ChoiceError::NoOffer));
    assert_eq!(e.state_hash(), before);
    assert!(e.events().is_empty());
}

#[test]
fn the_seam_refuses_an_index_past_the_options() {
    let mut e = offering_at(1);
    let before = e.state_hash();
    let options = u32::try_from(
        e.run
            .offer_options(
                e.content.rewards(),
                e.content.expertise(),
                crate::id::PlayerIdx::SOLO,
            )
            .len(),
    )
    .unwrap();
    assert_eq!(
        e.v1_pick_step(&[Some(99), None, None]),
        Err(ChoiceError::OutOfRange { index: 99, options })
    );
    assert_eq!(e.state_hash(), before);
    assert!(e.offer_open());
}

#[test]
fn the_seam_checks_every_pick_before_it_takes_one() {
    let mut e = offering_at(1);
    let before = e.state_hash();
    let player = crate::id::PlayerIdx::new(1).unwrap();
    assert_eq!(
        e.v1_pick_step(&[Some(0), Some(0), None]),
        Err(ChoiceError::NoPlayer { player })
    );
    assert_eq!(e.state_hash(), before);
    assert!(e.offer_open());
}

#[test]
fn the_seam_at_another_version_is_unsupported_and_changes_nothing() {
    for v in [0, SIM_VERSION + 1] {
        let mut e = offering_at(v);
        let before = e.state_hash();
        assert_eq!(
            e.v1_pick_step(&SOLO_PICK_0),
            Err(ChoiceError::Unsupported { sim_version: v }),
            "{v}"
        );
        assert_eq!(e.state_hash(), before, "{v}");
        assert!(e.offer_open(), "{v}");
    }
}

#[test]
fn the_run_variant_is_the_stage_version() {
    let pack = super::copy_tests::v1_pack();
    let mode = pack.content().time_limit();
    let stage = pack.content().trial().clone();
    let mut recorder = crate::run_replay::RunRecorder::start_with_pack(&setup(), pack).unwrap();
    recorder
        .step(&[Input::default(), Default::default(), Default::default()])
        .unwrap();
    let prefix = recorder.replay();
    let (restored, _) = crate::savepoint::restore(&recorder.save(), &prefix, pack).unwrap();
    let engines = [
        Engine::new(&stage).unwrap(),
        Engine::start_with(&setup(), pack.content()).unwrap(),
        Engine::start_time_limit(&setup(), pack.content(), mode).unwrap(),
        engine().clone(),
        restored,
    ];
    for e in &engines {
        assert_eq!(e.run.version(), e.run.declared_version());
    }
}

#[test]
fn the_run_owns_its_world() {
    let pack = super::copy_tests::v1_pack();
    let mode = pack.content().time_limit();
    let stage = pack.content().trial().clone();
    let mut recorder = crate::run_replay::RunRecorder::start_with_pack(&setup(), pack).unwrap();
    recorder
        .step(&[Input::default(), Default::default(), Default::default()])
        .unwrap();
    let prefix = recorder.replay();
    let (restored, _) = crate::savepoint::restore(&recorder.save(), &prefix, pack).unwrap();
    let engines = [
        Engine::new(&stage).unwrap(),
        Engine::start_with(&setup(), pack.content()).unwrap(),
        Engine::start_time_limit(&setup(), pack.content(), mode).unwrap(),
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
    // Version 1 holds its world inline with its state and its frozen events list; version 2 holds
    // an `Arc` of its world.
    let v1 = size_of::<World>()
        + size_of::<State>()
        + size_of::<Vec<crate::sim_v1::event::DomainEvent>>();
    let v2 = size_of::<std::sync::Arc<crate::sim::World>>() + size_of::<crate::sim::state::State>();
    let largest = v1.max(v2);
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
            // Version 1 starts from its own released pack; version 2 from the bundled files.
            let started = if version == 1 {
                Engine::start_with(&setup, super::copy_tests::v1_pack().content())
            } else {
                Engine::start(&setup)
            };
            let e = started.unwrap_or_else(|e| panic!("{mode} {version}: {e:?}"));
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
