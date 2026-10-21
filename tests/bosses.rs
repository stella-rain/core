//! The `time_limit` bosses (core#336, B1): the four mid-bosses and the final boss, each with its
//! own definition in the mode file; the draw of three mid-bosses without repeats; the elite call
//! by boss time (the 5-minute boss does not call, the 10- and 15-minute ones do at 30 seconds).

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::Boss;
use stella_rain_core::time_limit::TimeLimit;

const MINUTE: u32 = 3600;
const MID_BOSSES: [&str; 4] = ["stone_warden", "lantern_choir", "horn_charger", "weaver"];

fn setup(seed: u64, difficulty: &str) -> RunSetup {
    RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        seed,
        star_pool: None,
        extra_rerolls: 0,
        upgrades: Vec::new(),
        perk_set: "duelist".into(),
        auto_combo: false,
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
        difficulty: difficulty.into(),
    }
}

/// The bundled file with no enemies on its schedule and one elite at a time, so a run is quick to
/// play to a boss.
fn quiet_mode() -> TimeLimit {
    let mut v: Value = serde_json::from_slice(TimeLimit::BUNDLED).unwrap();
    v["waves"] = json!([]);
    v["elite_cap"] = json!(1);
    TimeLimit::parse(v.to_string().as_bytes()).unwrap_or_else(|e| panic!("{e}"))
}

fn start(mode: &TimeLimit, seed: u64, difficulty: &str) -> Engine {
    Engine::start_time_limit(&setup(seed, difficulty), RunContent::bundled(), mode).unwrap()
}

fn definition<'a>(mode: &'a TimeLimit, id: &str) -> &'a Boss {
    mode.bosses()
        .iter()
        .find(|b| b.base.0 == id)
        .unwrap_or_else(|| panic!("no definition of {id}"))
}

/// Steps to `tick` and returns the boss's sprite if one is on the field.
fn boss_at(e: &mut Engine, tick: u32) -> Option<String> {
    while e.tick() < tick {
        e.step(Input::default());
    }
    e.snapshot()
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Boss)
        .map(|v| v.asset.0.clone())
}

#[test]
fn the_pool_holds_the_four_mid_bosses_and_the_final_boss_is_the_core_pair() {
    let m = TimeLimit::bundled();
    let pool: Vec<&str> = m.mid_boss_pool().iter().map(|id| id.0.as_str()).collect();
    assert_eq!(pool, MID_BOSSES);
    assert_eq!(m.final_boss().0, "core_pair");
}

#[test]
fn every_pool_boss_and_the_final_boss_has_a_definition_with_a_profile() {
    let m = TimeLimit::bundled();
    for id in MID_BOSSES.iter().chain(&["core_pair"]) {
        let b = definition(m, id);
        assert!(b.profile.is_some(), "{id} has no match-up profile");
        assert!(b.call_elites.is_some(), "{id} has no elite call");
    }
}

#[test]
fn the_weaver_has_three_phases_and_the_other_mid_bosses_two() {
    let m = TimeLimit::bundled();
    for id in ["stone_warden", "lantern_choir", "horn_charger"] {
        assert_eq!(definition(m, id).phases.len(), 2, "{id}");
    }
    assert_eq!(definition(m, "weaver").phases.len(), 3);
}

#[test]
fn the_final_boss_has_two_cores_and_three_phases() {
    let core_pair = definition(TimeLimit::bundled(), "core_pair");
    let parts: Vec<&str> = core_pair.parts.iter().map(|p| p.id.0.as_str()).collect();
    assert_eq!(parts, ["core_left", "core_right"]);
    assert!(core_pair.parts.iter().all(|p| p.hp > 0));
    assert_eq!(core_pair.phases.len(), 3);
}

#[test]
fn three_mid_bosses_are_drawn_without_repeats_on_the_20_minute_run() {
    let m = quiet_mode();
    for seed in 0..2 {
        let mut e = start(&m, seed, "normal");
        let drawn: Vec<String> = [5, 10, 15]
            .iter()
            .map(|minute| boss_at(&mut e, minute * MINUTE).expect("a boss"))
            .collect();
        let mut unique = drawn.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 3, "seed {seed}: {drawn:?}");
        assert!(drawn.iter().all(|d| MID_BOSSES.contains(&d.as_str())));
    }
}

#[test]
fn easy_s_10_minute_run_draws_its_three_mid_bosses_without_repeats() {
    let m = quiet_mode();
    for seed in 0..2 {
        let mut e = start(&m, seed, "easy");
        let drawn: Vec<String> = [9000, 18000, 27000]
            .iter()
            .map(|&tick| boss_at(&mut e, tick).expect("a boss"))
            .collect();
        let mut unique = drawn.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 3, "seed {seed}: {drawn:?}");
    }
}

/// The calls the boss makes in `from..to`: the `ElitesCalled` events, and whether the snapshot
/// ever announced the call as pending.
fn calls_between(e: &mut Engine, from: u32, to: u32) -> (Vec<u32>, bool) {
    let mut called = Vec::new();
    let mut announced = false;
    while e.tick() < to {
        e.step(Input::default());
        if e.tick() < from {
            continue;
        }
        for ev in e.events() {
            if let DomainEvent::ElitesCalled { elites } = ev {
                called.push(*elites);
            }
        }
        announced |= e.snapshot().patterns.iter().any(|p| p.id.0 == "call_elites");
    }
    (called, announced)
}

#[test]
fn the_five_minute_boss_is_the_one_that_calls_no_elites() {
    let m = TimeLimit::bundled();
    assert_eq!(m.no_call_ticks(), [5 * MINUTE]);
    assert_eq!(m.boss_ticks()[0], 5 * MINUTE);
}

#[test]
fn the_five_minute_boss_never_calls_its_elites() {
    let m = quiet_mode();
    assert_eq!(m.no_call_ticks(), [5 * MINUTE]);
    for seed in 0..4 {
        let mut e = start(&m, seed, "normal");
        let (called, announced) = calls_between(&mut e, 5 * MINUTE, 5 * MINUTE + 2 * MINUTE);
        assert!(called.is_empty(), "seed {seed}: {called:?}");
        assert!(!announced, "seed {seed}: the call was announced");
    }
}

#[test]
fn the_ten_and_fifteen_minute_bosses_call_their_elites_after_30_seconds() {
    let m = quiet_mode();
    let mut e = start(&m, 1, "normal");
    for minute in [10, 15] {
        let from = minute * MINUTE;
        // The call comes on the tick after the 30 seconds (the boss's age 1 is its arrival).
        let (called, announced) = calls_between(&mut e, from, from + MINUTE / 2 + 2);
        assert_eq!(called.len(), 1, "minute {minute}: {called:?}");
        assert!(called[0] > 0, "minute {minute}: no elites were called");
        assert!(announced, "minute {minute}: the call was not announced first");
    }
}

#[test]
fn a_definition_the_stage_rejects_stops_the_run_from_starting() {
    let mut v: Value = serde_json::from_slice(TimeLimit::BUNDLED).unwrap();
    v["bosses"][0]["phases"][0]["timeline"][2] = json!({
        "type": "attack", "attack": { "preset": { "id": "no_such_preset", "args": [] } }
    });
    let m = TimeLimit::parse(v.to_string().as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    let result = Engine::start_time_limit(&setup(1, "normal"), RunContent::bundled(), &m);
    assert!(result.is_err());
}
