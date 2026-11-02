//! The elite grades' traits (Kade's Q-D3, `modes/time_limit.json`): LOW is tougher than the bat
//! and lobs one zone at a time; MID throws three lobs round the main character; HIGH's slam is a
//! long wind-up, the visible tell, with an aftershock ring; an enemy's attack is never announced in the snapshot.

use serde_json::{Value, json};
use stella_rain_core::attack::PhaseKind;
use stella_rain_core::engine::Engine;
use stella_rain_core::snapshot::PatternView;
use stella_rain_core::stage::Stage;

use crate::common;

use common::*;

/// Where the elite stands: 100 pixels above the main character, so its shots come down at it.
const ELITE: (i32, i32) = (START_X, START_Y - 25_600);

/// The wind-up the big slam must have at least, in ticks (a placeholder, 1.5 seconds).
const SLAM_WIND_UP: u32 = 90;

/// The enemy of the bundled elite type `id`, as the time-limit file has it.
fn elite_type(id: &str) -> Value {
    let file: Value = serde_json::from_slice(common::bundled_file("time_limit")).expect("json");
    let types = file["elite_types"].as_array().expect("a list");
    let found = types.iter().find(|t| t["id"] == id);
    found.unwrap_or_else(|| panic!("no elite {id}"))["enemy"].clone()
}

/// The bat of the first time-limit wave: the base the low elite is tougher than.
fn base_bat() -> Value {
    let file: Value = serde_json::from_slice(common::bundled_file("time_limit")).expect("json");
    file["waves"][0]["enemy"].clone()
}

/// What each of an enemy's rules attacks with.
fn attacks(enemy: &Value) -> Vec<Value> {
    enemy["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .map(|r| r["do"]["attack"].clone())
        .collect()
}

/// The stage file of the elite standing at `ELITE`, with nothing else in the field.
fn standing_json(enemy: &Value) -> Value {
    let mut enemy = enemy.clone();
    enemy["movement"] = hold(ELITE.0, ELITE.1);
    serde_json::to_value(stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, enemy, ELITE.0, ELITE.1)],
        vec![],
    ))
    .expect("json")
}

/// The elite standing at `ELITE`, with nothing else in the field.
fn standing(enemy: &Value) -> Stage {
    serde_json::from_value(standing_json(enemy)).expect("parses")
}

fn patterns(e: &Engine) -> Vec<PatternView> {
    e.snapshot().patterns
}

/// The zones alive after each of the next `ticks` ticks.
fn zones_per_tick(e: &mut Engine, ticks: u32) -> Vec<usize> {
    (0..ticks)
        .map(|_| {
            e.step(&[idle()]);
            let snapshot = serde_json::to_value(e.snapshot()).expect("json");
            snapshot["zones"].as_array().map_or(0, Vec::len)
        })
        .collect()
}

#[test]
fn the_low_elite_is_tougher_than_the_bat_and_lobs_one_zone_at_a_time() {
    let (low, bat) = (elite_type("elite_low"), base_bat());
    assert!(low["hp"].as_u64() > bat["hp"].as_u64(), "more hit points");
    assert_eq!(attacks(&low).len(), 1, "one pattern");
    let mut e = engine(&standing(&low));
    let zones = zones_per_tick(&mut e, 400);
    assert!(zones.contains(&1), "a lob is placed");
    assert_eq!(zones.iter().max(), Some(&1), "one marker at a time");
}

#[test]
fn the_mid_elite_throws_three_lobs_round_the_main_character() {
    let mut e = engine(&standing(&elite_type("elite_mid")));
    let zones = zones_per_tick(&mut e, 300);
    assert_eq!(zones.iter().max(), Some(&3), "three lobs at once");
}

#[test]
fn the_high_elite_slam_winds_up_for_at_least_the_placeholder_and_is_not_announced() {
    let high = elite_type("elite_high");
    let slam = &attacks(&high)[0]["inline"][0];
    assert_eq!(slam["kind"], "wind_up", "the slam starts with its wind-up");
    let wind_up = u32::try_from(slam["ticks"].as_u64().expect("ticks")).unwrap();
    assert!(wind_up >= SLAM_WIND_UP, "a long wind-up");

    let mut e = engine(&standing(&high));
    let mut winding = 0;
    let mut zones = Vec::new();
    for _ in 0..(wind_up + 60) {
        e.step(&[idle()]);
        assert_eq!(
            patterns(&e),
            Vec::<PatternView>::new(),
            "an enemy's attack is never announced"
        );
        winding += e
            .snapshot()
            .entities
            .iter()
            .filter_map(|en| en.attack.as_ref())
            .filter(|a| a.kind == PhaseKind::WindUp)
            .count();
        let snapshot = serde_json::to_value(e.snapshot()).expect("json");
        zones.push(snapshot["zones"].as_array().map_or(0, Vec::len));
    }
    assert!(
        winding >= SLAM_WIND_UP as usize,
        "the wind-up is the visible tell: {winding}"
    );
    // The aftershock ring of eight is placed when the slam lands, not before.
    assert!(zones[..wind_up as usize - 1].iter().all(|n| *n == 0));
    assert_eq!(zones.iter().max(), Some(&8), "the ring of eight");
}

#[test]
fn a_big_flag_on_an_enemy_rule_is_refused_as_an_unknown_field() {
    let mut stage = standing_json(&elite_type("elite_high"));
    stage["waves"][0]["enemy"]["rules"][0]["do"]["big"] = json!(true);
    assert!(serde_json::from_value::<Stage>(stage).is_err());
}
