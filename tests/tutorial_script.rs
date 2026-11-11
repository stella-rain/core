//! The tutorial mode follows the tutorial script: the run's length, its one weak golem and the
//! first weapon from the run setup are pinned, and the first weapon-tree point is reachable from
//! the bats alone, after the opening cut and before the golem arrives.

use crate::common::run::{run_input_for, trial_setup};
use crate::run_play::play_run_with;
use serde_json::Value;
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::Command;
use stella_rain_core::run_setup::{self, RunSetup};
use stella_rain_core::snapshot::EntityKind;

/// The mode file's `length_ticks`: three minutes at 60 ticks a second.
const LENGTH: u32 = 10_800;
/// The mode file's `boss.arrives_at_tick` (the same constant as `tests/tutorial.rs`).
const ARRIVAL: u32 = 4_000;
/// The length of the `tutorial_opening` corpus case (`TUTORIAL_OPENING_TICKS` of
/// `tests/corpus.rs`): it holds no tree pick, so the first point comes after it.
const TUTORIAL_OPENING_TICKS: u32 = 240;
/// The weapons the tutorial runs are played with.
const WEAPONS: [&str; 2] = ["giant_hammer", "giant_sword"];

fn mode_json() -> Value {
    serde_json::from_slice(run_setup::mode_stage("tutorial").unwrap()).unwrap()
}

fn setup(weapon: &str) -> RunSetup {
    RunSetup {
        mode: "tutorial".into(),
        weapon: weapon.into(),
        ..trial_setup()
    }
}

/// What the building bot's tutorial run does, tick by tick (the end of each tick).
struct Measured {
    first_star_drop: Option<u32>,
    first_tree_ready: Option<u32>,
    first_kill: Option<u32>,
    ended: u32,
    stardust: u32,
}

fn measure(weapon: &str) -> Measured {
    let setup = setup(weapon);
    let content = RunContent::bundled();
    let commands = play_run_with(&setup, content, LENGTH, |e, t| {
        run_input_for(e, t, &[EntityKind::Enemy, EntityKind::Boss], u32::MAX)
    });
    let mut engine = Engine::start_with(&setup, content).unwrap();
    let mut m = Measured {
        first_star_drop: None,
        first_tree_ready: None,
        first_kill: None,
        ended: 0,
        stardust: 0,
    };
    for command in &commands {
        let Command::Step(input) = *command else {
            unreachable!("a run at the live rules holds no choice")
        };
        engine.step(&[input, Default::default(), Default::default()]);
        let tick = engine.tick();
        for e in engine.events() {
            match e {
                DomainEvent::StarDropped { .. } => {
                    m.first_star_drop.get_or_insert(tick);
                }
                DomainEvent::Died { .. } => {
                    m.first_kill.get_or_insert(tick);
                }
                _ => {}
            }
        }
        if engine.snapshot().players[0].tree_ready.is_some() {
            m.first_tree_ready.get_or_insert(tick);
        }
        m.ended = tick;
    }
    m.stardust = engine.stardust();
    m
}

/// Pins what the script already has: the length, three waves of bats and one weak golem.
#[test]
fn the_mode_has_the_scripts_length_waves_and_one_weak_golem() {
    let m = mode_json();
    assert_eq!(m["length_ticks"], LENGTH);
    let waves = m["waves"].as_array().unwrap();
    let at: Vec<u64> = waves
        .iter()
        .map(|w| w["at_tick"].as_u64().unwrap())
        .collect();
    let count: Vec<u64> = waves.iter().map(|w| w["count"].as_u64().unwrap()).collect();
    assert_eq!(at, [120, 600, 1_200]);
    assert_eq!(count, [3, 3, 2]);
    assert!(waves.iter().all(|w| w["enemy"]["elite"].is_null()));
    let boss = &m["boss"];
    assert_eq!(boss["base"], "golem");
    assert!(boss["hp"].as_u64().unwrap() <= 30_000);
    assert_eq!(boss["profile"]["weak"], serde_json::json!(["strike"]));
    assert_eq!(boss["arrives_at_tick"], ARRIVAL);
    assert!(boss["parts"].as_array().unwrap().is_empty());
    assert_eq!(boss["phases"].as_array().unwrap().len(), 1);
    assert!(boss["call_elites"].is_null());
}

/// The first weapon is the setup's, not the file's own `giant_sword`.
#[test]
fn the_first_weapon_is_the_one_the_setup_names() {
    for weapon in WEAPONS {
        let stage = run_setup::stage(&setup(weapon)).unwrap();
        let w = &serde_json::to_value(&stage.player.weapon).unwrap()["preset"];
        assert_eq!(w["id"], weapon, "the stage's first weapon");
        assert!(
            w.get("tree").is_none(),
            "no tree unless the setup names one"
        );
    }
    let tree = format!("{}_base", WEAPONS[0]);
    let with_tree = RunSetup {
        weapon_tree: Some(tree.as_str().into()),
        ..setup(WEAPONS[0])
    };
    let stage = run_setup::stage(&with_tree).unwrap();
    let w = &serde_json::to_value(&stage.player.weapon).unwrap()["preset"];
    assert_eq!(w["tree"], tree.as_str());
}

/// The bats alone give the expertise of the style point: the kills of every wave, counted at
/// their `kill` points, reach the sum of the thresholds up to the `level_table`'s style level.
#[test]
fn the_bats_kills_pay_for_the_style_point() {
    let m = mode_json();
    let kills: u64 = m["waves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["count"].as_u64().unwrap() * w["expertise"]["kill"].as_u64().unwrap())
        .sum();
    let expertise: Value =
        serde_json::from_slice(crate::common::bundled_file("expertise")).unwrap();
    let style = usize::try_from(m["level_table"]["style"].as_u64().unwrap()).unwrap();
    let need: u64 = expertise["thresholds"]
        .as_array()
        .unwrap()
        .iter()
        .take(style)
        .map(|t| t.as_u64().unwrap())
        .sum();
    assert!(
        kills >= need,
        "the bats' kills give {kills}, the style point needs {need}"
    );
}

/// The first weapon-tree point is ready after the `tutorial_opening` cut and before the golem.
#[test]
fn the_weapon_tree_point_comes_after_the_opening_and_before_the_golem() {
    for weapon in WEAPONS {
        let m = measure(weapon);
        let t = m.first_tree_ready.unwrap_or(u32::MAX);
        assert!(
            t > TUTORIAL_OPENING_TICKS && t < ARRIVAL,
            "{weapon}: the first tree point is ready at tick {t} (first kill {:?}, run ends {})",
            m.first_kill,
            m.ended
        );
    }
}

/// A star falls inside the first two waves, before tick 600.
#[test]
fn a_star_falls_before_the_second_wave() {
    for weapon in WEAPONS {
        let m = measure(weapon);
        let t = m.first_star_drop.unwrap_or(u32::MAX);
        assert!(t < 600, "{weapon}: the first star falls at tick {t}");
    }
}

/// The description says what the mode is now.
#[test]
fn the_description_says_what_the_mode_is() {
    let m = mode_json();
    let d = m["description"].as_str().unwrap();
    assert!(!d.contains("Every weapon can be switched"), "{d}");
    assert!(
        !d.contains("Placeholder content from the trial stage"),
        "{d}"
    );
    assert!(d.contains("arrives at tick 4,000"), "{d}");
}

/// The earlier guarantees hold with the new numbers: the run earns no Stardust, and the sword's
/// bot kills the golem after it arrives and before the time is up
/// (`tutorial::the_bot_kills_the_golem_in_the_tutorial` is the same check on the outcome).
#[test]
fn the_bot_still_clears_the_run_and_earns_no_stardust() {
    for weapon in WEAPONS {
        assert_eq!(measure(weapon).stardust, 0, "{weapon}");
    }
    let m = measure("giant_sword");
    assert!(m.ended > ARRIVAL && m.ended < LENGTH, "ends at {}", m.ended);
}
