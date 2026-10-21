//! The six regular enemy roles of the `time_limit` mode (R2, ADR-026, ADR-100): placeholder
//! content, each a wave of the bundled schedule with its own ID. The roles' rules are the ones
//! `tests/enemy_roles.rs` covers; these tests read the bundled file, so they check the content.
//! Every number is a tuning value.

use crate::common::*;
use serde_json::{Value, json};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::engine::Engine;
use stella_rain_core::fixed::Fx;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::Error;

const MINUTE: u32 = 3_600;
/// The main character's start (`tests/enemy_roles.rs`).
const PLAYER_X: i32 = 46_080;
const PLAYER_Y: i32 = 147_456;

/// The six roles, and the minute each first spawns (placeholders).
const ROLES: [(&str, u32); 6] = [
    ("swarmer", 0),
    ("chaser", 0),
    ("ranged_shooter", 3),
    ("tank", 6),
    ("exploder", 9),
    ("splitter", 12),
];

/// The bundled mode file as JSON.
fn mode() -> Value {
    serde_json::from_slice(TimeLimit::BUNDLED).expect("the bundled file is JSON")
}

/// The wave of the bundled schedule with `id`.
fn role(id: &str) -> Value {
    mode()["waves"]
        .as_array()
        .expect("a schedule")
        .iter()
        .find(|w| w["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("no wave with the id `{id}`"))
}

/// The speed of a follower in 1/256 pixel a tick: its `speed`, or 4 pixels a tick without one
/// (ADR-058).
fn follow_speed(id: &str) -> i64 {
    role(id)["enemy"]["movement"]["speed"]
        .as_i64()
        .unwrap_or(i64::from(Fx::px(4).0))
}

/// The paths of the problems a mode file has, one per problem.
fn paths_of(bytes: &[u8]) -> Vec<String> {
    match TimeLimit::parse(bytes) {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not a validation error: {other:?}"),
    }
}

/// The bundled file with `edit` applied to its JSON, as the bytes it is read from.
fn edited(edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut v = mode();
    edit(&mut v);
    serde_json::to_vec(&v).expect("JSON")
}

/// A stage whose only enemy is the bundled `id` at `(x, y)`, with its enemy edited by `edit`; the
/// main character keeps its weapon, so a splitter can be hit (`tests/enemy_roles.rs`).
fn alone(id: &str, x: i32, y: i32, edit: impl FnOnce(&mut Value)) -> Engine {
    let mut enemy = role(id)["enemy"].clone();
    edit(&mut enemy);
    engine(&stage_with(
        18_000,
        Some(far_boss()),
        vec![wave(0, 1, 1, enemy, x, y)],
    ))
}

/// The enemies alive.
fn enemies_alive(e: &Engine) -> usize {
    e.snapshot()
        .entities
        .iter()
        .filter(|v| v.kind == EntityKind::Enemy)
        .count()
}

/// The rules that fired on the last tick.
fn firings(e: &Engine) -> usize {
    e.events()
        .iter()
        .filter(|ev| matches!(ev, DomainEvent::RuleFired { .. }))
        .count()
}

// --- The schedule holds the six roles -------------------------------------------------------

#[test]
fn each_role_is_a_wave_of_the_bundled_schedule_with_its_own_id() {
    for (id, _) in ROLES {
        assert_eq!(role(id)["id"], id);
    }
    // Two bat waves and the six roles fill the schedule's eight waves.
    assert_eq!(mode()["waves"].as_array().expect("a schedule").len(), 8);
}

#[test]
fn the_roles_come_in_the_minutes_the_schedule_gives() {
    for (id, minute) in ROLES {
        assert_eq!(role(id)["at_tick"], json!(minute * MINUTE), "{id}");
    }
}

#[test]
fn each_role_has_a_match_up_profile() {
    for (id, _) in ROLES {
        let profile = &role(id)["enemy"]["profile"];
        assert!(
            profile.as_object().is_some_and(|p| !p.is_empty()),
            "{id}: {profile}"
        );
    }
}

#[test]
fn a_wave_id_is_a_well_formed_id_unique_in_the_schedule() {
    // An ID that no earlier wave has loads.
    let named = edited(|v| v["waves"][0]["id"] = json!("a_wave"));
    assert!(TimeLimit::parse(&named).is_ok());
    // An ID that is not snake_case is refused at its wave.
    let bad = edited(|v| v["waves"][0]["id"] = json!("Not Snake"));
    assert_eq!(paths_of(&bad), ["$.waves[0].id"]);
    // The same ID twice is refused at the second wave.
    let twice = edited(|v| {
        v["waves"][0]["id"] = json!("a_wave");
        v["waves"][1]["id"] = json!("a_wave");
    });
    assert_eq!(paths_of(&twice), ["$.waves[1].id"]);
}

// --- Behaviour the numbers give -------------------------------------------------------------

#[test]
fn a_tank_is_slower_than_a_chaser_and_a_swarmer_is_faster() {
    assert!(follow_speed("tank") > 0);
    assert!(
        follow_speed("tank") < follow_speed("chaser"),
        "tank {} chaser {}",
        follow_speed("tank"),
        follow_speed("chaser")
    );
    assert!(follow_speed("swarmer") > follow_speed("chaser"));
}

#[test]
fn an_exploder_bursts_once_when_near_and_removes_itself() {
    // Twenty thousand units below the main character it walks in, bursts once and is gone.
    let mut e = alone("exploder", PLAYER_X, PLAYER_Y - 20_000, |_| {});
    let mut bursts = 0;
    for _ in 0..200 {
        e.step(idle());
        bursts += firings(&e);
    }
    assert_eq!(bursts, 1);
    assert_eq!(enemies_alive(&e), 0);
}

#[test]
fn a_splitter_splits_once_when_it_is_below_half() {
    // A hundred hit points, held 3,000 units above the main character, which fires up the field.
    let at = PLAYER_Y - 3_000;
    let mut e = alone("splitter", PLAYER_X, at, |enemy| {
        enemy["hp"] = json!(100);
        enemy["movement"] = hold(PLAYER_X, at);
    });
    let mut splits = 0;
    for _ in 0..900 {
        e.step(firing(idle(), e.tick()));
        splits += firings(&e);
    }
    assert_eq!(splits, 1);
}
