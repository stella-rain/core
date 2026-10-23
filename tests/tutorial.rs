//! The tutorial mode (ADR-086): a three-minute run of gentle bats and one weak golem, with no
//! elites and no mid-boss, that every bundled weapon can be played in, and whose file is in the
//! pack the run's content hash covers.

use crate::common::run::{run_input_for, trial_setup};
use crate::run_play::play_run_with;
use serde_json::Value;
use sha2::{Digest, Sha256};
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::pack::Pack;
use stella_rain_core::registry::{self, Kind};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay;
use stella_rain_core::run_replay::Command;
use stella_rain_core::run_setup::{self, RunSetup};
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::validate;

/// Three minutes at 60 ticks a second.
const LENGTH: u32 = 3 * 60 * 60;

fn setup(weapon: &str) -> RunSetup {
    RunSetup {
        mode: "tutorial".into(),
        weapon: weapon.into(),
        ..trial_setup()
    }
}

/// The bundled mode file, as JSON.
fn mode_json() -> Value {
    serde_json::from_slice(run_setup::mode_stage("tutorial").unwrap()).unwrap()
}

#[test]
fn the_tutorial_mode_is_registered_and_its_file_validates() {
    assert!(
        registry::ENTRIES
            .iter()
            .any(|e| e.kind == Kind::Mode && e.id == "tutorial"),
        "tutorial is a mode of the registry"
    );
    let stage = validate::parse_and_validate(run_setup::mode_stage("tutorial").unwrap()).unwrap();
    assert_eq!(stage.length_ticks, LENGTH);
    let run = run_setup::stage(&setup("giant_sword")).unwrap();
    assert_eq!(run.length_ticks, LENGTH);
}

#[test]
fn the_tutorial_run_is_three_minutes() {
    assert_eq!(LENGTH, 10_800);
    assert_eq!(mode_json()["length_ticks"], LENGTH);
}

#[test]
fn the_tutorial_has_one_weak_boss_and_no_elites_or_mid_boss() {
    let m = mode_json();
    let waves = m["waves"].as_array().unwrap();
    assert!(!waves.is_empty(), "the tutorial has bats");
    assert!(
        waves.iter().all(|w| w["enemy"]["elite"].is_null()),
        "no elite in any wave"
    );
    assert_eq!(m["boss"]["base"], "golem");
    assert!(
        m["boss"]["call_elites"].is_null(),
        "the boss calls no elites"
    );
    assert!(m["boss"]["parts"].as_array().unwrap().is_empty());
    assert_eq!(m["boss"]["phases"].as_array().unwrap().len(), 1);
    assert!(
        m["boss"]["hp"].as_u64().unwrap() <= 30_000,
        "a weak boss: its hit points stay low"
    );
    assert!(m.get("boss_ticks").is_none() && m.get("mid_boss_pool").is_none());
}

#[test]
fn every_bundled_weapon_runs_the_tutorial_and_its_replay_verifies() {
    let weapons: Vec<&str> = registry::ENTRIES
        .iter()
        .filter(|e| e.kind == Kind::Weapon)
        .map(|e| e.id)
        .collect();
    assert!(weapons.len() >= 2, "more than one weapon is bundled");
    for weapon in weapons {
        let setup = setup(weapon);
        let commands = play_run_with(&setup, RunContent::bundled(), LENGTH, |e, t| {
            run_input_for(e, t, &[EntityKind::Enemy, EntityKind::Boss], u32::MAX)
        });
        let recorded =
            run_replay::record_run(&setup, &commands).unwrap_or_else(|e| panic!("{weapon}: {e:?}"));
        let verified = run_replay::verify_run(&recorded.to_bytes())
            .unwrap_or_else(|e| panic!("{weapon}: {e:?}"));
        assert_ne!(verified.outcome, Outcome::Running, "{weapon} ends the run");
        assert_eq!(
            verified.ticks, recorded.ticks,
            "{weapon} replays every tick"
        );
    }
}

#[test]
fn the_bot_kills_the_golem_in_the_tutorial() {
    let setup = setup("giant_sword");
    let commands = play_run_with(&setup, RunContent::bundled(), LENGTH, |e, t| {
        run_input_for(e, t, &[EntityKind::Enemy, EntityKind::Boss], u32::MAX)
    });
    let recorded = run_replay::record_run(&setup, &commands).unwrap();
    let verified = run_replay::verify_run(&recorded.to_bytes()).unwrap();
    assert_eq!(verified.outcome, Outcome::Cleared);
    assert!(
        verified.ticks < LENGTH,
        "the golem dies before the time is up"
    );
}

#[test]
fn the_bundled_pack_holds_the_tutorial_file_and_the_run_hash_covers_it() {
    let pack: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let file: Value = serde_json::from_slice(include_bytes!("../modes/tutorial.json")).unwrap();
    assert_eq!(pack["modes"]["tutorial"], file);
    let sha: [u8; 32] = Sha256::digest(Pack::BUNDLED).into();
    assert_eq!(run_replay::content_sha256("tutorial"), Some(sha));
    let mut edited = pack.clone();
    edited["modes"]["tutorial"]["length_ticks"] = 0.into();
    let edited = serde_json::to_vec(&edited).unwrap();
    assert!(
        Pack::from_bytes(&edited).is_err(),
        "a pack whose tutorial file is changed is refused"
    );
}

/// The tutorial earns no Stardust (ADR-086, ADR-109): a stage with no Stardust factor counts
/// nothing, and the tutorial's stage has none, so the bot's run, which kills what it reaches,
/// leaves the counter at zero.
#[test]
fn the_tutorial_earns_no_stardust() {
    let setup = setup("giant_sword");
    let bundled = RunContent::bundled();
    let commands = play_run_with(&setup, bundled, LENGTH, |e, t| {
        run_input_for(e, t, &[EntityKind::Enemy, EntityKind::Boss], u32::MAX)
    });
    let mut engine = Engine::start_with(&setup, bundled).unwrap();
    let mut kills = 0;
    for command in &commands {
        match *command {
            Command::Step(input) => {
                engine.step(input);
                kills += engine
                    .events()
                    .iter()
                    .filter(|e| matches!(e, DomainEvent::Died { .. }))
                    .count();
            }
            Command::Choose(index) => engine.choose(index).unwrap(),
        }
    }
    assert!(
        kills > 0,
        "the bot kills something, so the counter is tested"
    );
    assert_eq!(engine.stardust(), 0);
}
