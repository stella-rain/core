//! The rules dispatch (ADR-REPL-7 point 3): `step` and `choose` run the rules for a run whose
//! `sim_version` is in `1..=SIM_VERSION` and fail closed on any other, changing nothing.

use super::*;
use crate::pack::ContentFiles;
use crate::run_content::RunContent;
use crate::time_limit::TimeLimit;
use crate::version::{SCHEMA_VERSION, SIM_VERSION, Versions};

fn engine() -> Engine {
    let mode = TimeLimit::parse(ContentFiles::bundled_file("time_limit"), Versions::CURRENT)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let setup = RunSetup {
        schema_version: SCHEMA_VERSION,
        sim_version: SIM_VERSION,
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
    };
    Engine::start_time_limit(&setup, RunContent::bundled(), &mode).unwrap()
}

/// The engine with its run's `sim_version` set to `v`, past what `Engine::new` checks.
fn at_version(v: u32) -> Engine {
    let mut e = engine();
    e.stage.stage.sim_version = v;
    e
}

/// The engine at `v` with a stat offer open.
fn offering_at(v: u32) -> Engine {
    let mut e = at_version(v);
    let progress = e.state.progress.as_mut().expect("a run has levels");
    progress.offers_due = 1;
    progress.offer_open = true;
    assert!(e.offer_open());
    e
}

#[test]
fn version_1_runs_the_rules() {
    let mut e = at_version(1);
    e.step(Input::default());
    assert_eq!(e.tick(), 1);
    assert_eq!(e.outcome(), Outcome::Running);
    let mut e = offering_at(1);
    assert_eq!(e.choose(0), Ok(()));
}

#[test]
fn another_version_fails_closed_without_running_the_rules() {
    for v in [0, SIM_VERSION + 1] {
        let mut e = at_version(v);
        e.step(Input::default());
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
            e.choose(0),
            Err(ChoiceError::Unsupported { sim_version: v }),
            "{v}"
        );
        assert_eq!(e.state_hash(), before, "{v}");
        assert_eq!(e.outcome(), Outcome::Running, "{v}");
        assert!(e.offer_open(), "{v}");
    }
}
