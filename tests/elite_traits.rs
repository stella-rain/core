//! The elite grades' traits (Kade's Q-D3, `modes/time_limit.json`): LOW is tougher than the bat
//! and keeps the bat's one pattern; MID adds an aimed spread at the main character; HIGH's slam
//! is a big pattern with a long wind-up, announced in the snapshot while it winds up (ADR-103).

use serde_json::{Value, json};
use stella_rain_core::attack::PhaseKind;
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::presets;
use stella_rain_core::snapshot::PatternView;
use stella_rain_core::stage::Stage;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::{self, Error};

use crate::common;

use common::*;

/// Where the elite stands: 100 pixels above the main character, so its shots come down at it.
const ELITE: (i32, i32) = (START_X, START_Y - 25_600);

/// The wind-up the big slam must have at least, in ticks (a placeholder, 1.5 seconds).
const SLAM_WIND_UP: u32 = 90;

/// The enemy of the bundled elite type `id`, as the time-limit file has it.
fn elite_type(id: &str) -> Value {
    let file: Value = serde_json::from_slice(TimeLimit::BUNDLED).expect("json");
    let types = file["elite_types"].as_array().expect("a list");
    let found = types.iter().find(|t| t["id"] == id);
    found.unwrap_or_else(|| panic!("no elite {id}"))["enemy"].clone()
}

/// The bat of the first time-limit wave: the base the low elite is tougher than.
fn base_bat() -> Value {
    let file: Value = serde_json::from_slice(TimeLimit::BUNDLED).expect("json");
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
    let mut stage = serde_json::to_value(stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, enemy, ELITE.0, ELITE.1)],
        vec![],
    ))
    .expect("json");
    stage
}

/// The elite standing at `ELITE`, with nothing else in the field.
fn standing(enemy: &Value) -> Stage {
    serde_json::from_value(standing_json(enemy)).expect("parses")
}

fn patterns(e: &Engine) -> Vec<PatternView> {
    e.snapshot().patterns
}

/// The velocity of each bullet that appears in the next `ticks` ticks, taken from the elite's
/// spawn point, as the attack tests take it.
fn fired(e: &mut Engine, ticks: u32) -> Vec<(i32, i32)> {
    let mut seen = 0;
    let mut out = Vec::new();
    for _ in 0..ticks {
        e.step(idle());
        let bullets = e.snapshot().bullets;
        seen = seen.min(bullets.len());
        out.extend(
            bullets[seen..]
                .iter()
                .map(|b| (b.at.x.0 - ELITE.0, b.at.y.0 - ELITE.1)),
        );
        seen = bullets.len();
    }
    out
}

fn length(v: (i32, i32)) -> i32 {
    let sq = i64::from(v.0).pow(2) + i64::from(v.1).pow(2);
    i32::try_from(sq.isqrt()).expect("a bullet's length")
}

/// Whether `v` points the way of `aim`, at `speed`, within 2 units a component.
fn aimed_at(v: (i32, i32), aim: (i32, i32), speed: i32) -> bool {
    let len = length(aim);
    let want = (
        i64::from(aim.0) * i64::from(speed) / i64::from(len),
        i64::from(aim.1) * i64::from(speed) / i64::from(len),
    );
    (i64::from(v.0) - want.0).abs() <= 2 && (i64::from(v.1) - want.1).abs() <= 2
}

#[test]
fn the_low_elite_is_tougher_than_the_bat_and_keeps_its_one_pattern() {
    let (low, bat) = (elite_type("elite_low"), base_bat());
    assert!(low["hp"].as_u64() > bat["hp"].as_u64(), "more hit points");
    assert_eq!(attacks(&low), attacks(&bat), "no new pattern");
}

#[test]
fn the_mid_elite_fires_an_aimed_spread_at_the_main_character() {
    let mut e = engine(&standing(&elite_type("elite_mid")));
    let aim = {
        let (x, y) = player_at(&e);
        (x - ELITE.0, y - ELITE.1)
    };
    let spread: Vec<(i32, i32)> = fired(&mut e, 200)
        .into_iter()
        .filter(|v| (length(*v) - 768).abs() <= 2)
        .collect();
    assert_eq!(spread.len(), 7, "the seven bullets of an aimed spread");
    assert!(aimed_at(spread[3], aim, 768), "the middle one is aimed");
}

#[test]
fn the_high_elite_slam_winds_up_for_at_least_the_placeholder_and_is_announced_big() {
    let slam = presets::definition("elite_slam").expect("the preset exists");
    let wind_up = &slam[0];
    assert_eq!(
        wind_up.kind,
        PhaseKind::WindUp,
        "the slam starts with its wind-up"
    );
    assert!(wind_up.ticks >= SLAM_WIND_UP, "a long wind-up");

    let mut e = engine(&standing(&elite_type("elite_high")));
    let mut shown = Vec::new();
    for _ in 0..(wind_up.ticks + 10) {
        e.step(idle());
        shown.extend(patterns(&e));
    }
    let total = wind_up.ticks;
    let counted: Vec<u32> = shown
        .iter()
        .map(|p| {
            assert_eq!(p.id, "elite_slam".into(), "the slam is what is announced");
            assert_eq!(p.total, total, "the count-down starts at the wind-up");
            p.remaining
        })
        .collect();
    let expected: Vec<u32> = (1..=total).rev().collect();
    assert_eq!(
        counted, expected,
        "one tick a count, from the wind-up down to 1"
    );
}

#[test]
fn a_big_enemy_rule_must_name_a_preset() {
    let mut enemy = elite_type("elite_high");
    enemy["rules"][0]["do"]["attack"] = json!({ "inline": [{ "kind": "active", "ticks": 1 }] });
    enemy["rules"][0]["do"]["big"] = json!(true);
    let stage: Stage = serde_json::from_value(standing_json(&enemy)).expect("parses");
    let paths: Vec<String> = match validate::validate(&stage) {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not refused: {other:?}"),
    };
    assert!(paths.iter().any(|p| p.contains("big")), "{paths:?}");
}
