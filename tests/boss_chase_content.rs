//! The bundled bosses and the chase (ADR-SIM-4, ADR-SIM-3): which boss opts in, with what
//! values, and that a chasing boss walks as its definition says, in the trial and in a
//! `time_limit` run.

use crate::common;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::{self, RunSetup};
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

/// The default chase: 1 px a tick, 96 px stop.
const SPEED: i64 = 256;
const STOP: i64 = 24_576;
/// A Far stop, 160 px.
const FAR: i64 = 40_960;

fn time_limit_json() -> Value {
    serde_json::from_slice(common::bundled_file("time_limit")).unwrap()
}

fn trial_json() -> Value {
    serde_json::from_slice(run_setup::mode_stage("trial").unwrap()).unwrap()
}

fn boss_of(id: &str) -> Value {
    time_limit_json()["bosses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["base"] == id)
        .unwrap_or_else(|| panic!("no boss {id}"))
        .clone()
}

/// `(speed, stop_distance)` of a boss object, defaults filled in; `None` for no chase.
fn chase_of(boss: &Value) -> Option<(i64, i64)> {
    let c = boss.get("chase")?;
    Some((
        c.get("speed").and_then(Value::as_i64).unwrap_or(SPEED),
        c.get("stop_distance")
            .and_then(Value::as_i64)
            .unwrap_or(STOP),
    ))
}

#[test]
fn each_boss_opts_in_as_reviewed() {
    assert_eq!(chase_of(&trial_json()["boss"]), Some((SPEED, STOP)));
    assert_eq!(chase_of(&boss_of("stone_warden")), Some((SPEED, STOP)));
    assert_eq!(chase_of(&boss_of("lantern_choir")), Some((SPEED, FAR)));
    assert_eq!(chase_of(&boss_of("weaver")), Some((SPEED, FAR)));
    assert_eq!(chase_of(&boss_of("core_pair")), Some((192, STOP)));
}

#[test]
fn the_horn_charger_keeps_its_dash_and_does_not_chase() {
    let b = boss_of("horn_charger");
    assert!(b.get("chase").is_none());
    let text = b.to_string();
    assert_eq!(text.matches("\"dash\"").count(), 4);
    assert_eq!(text.matches("\"distance\":122880").count(), 4);
}

#[test]
fn the_bodies_and_parts_are_unchanged() {
    assert_eq!(boss_of("core_pair")["hp"], 300_000);
    let pair = boss_of("core_pair");
    let parts: Vec<&str> = pair["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(parts, ["core_left", "core_right"]);
    assert_eq!(trial_json()["boss"]["parts"][0]["id"], "horn");
    assert_eq!(trial_json()["boss"]["hp"], 300_000);
}

#[test]
fn the_trial_golem_walks_straight_at_one_pixel_a_tick() {
    let setup = RunSetup {
        mode: "trial".into(),
        weapon: "giant_sword".into(),
        sim_version: 2,
        ..RunSetup::default()
    };
    let trace = || {
        let mut e = Engine::start(&setup).unwrap();
        let mut rows = Vec::new();
        for _ in 0..120 {
            e.step(&[Input::default(), Default::default(), Default::default()]);
            let s = e.snapshot();
            let boss = s
                .entities
                .iter()
                .find(|v| v.kind == EntityKind::Boss)
                .expect("the golem is on the field");
            let me = s.players[0].at;
            rows.push((
                e.tick(),
                (boss.at.x.0, boss.at.y.0),
                (me.x.0, me.y.0),
                e.state_hash(),
            ));
        }
        rows
    };
    let rows = trace();
    for (tick, (x, y), (px, py), _) in &rows {
        assert_eq!(*x, 163_840, "tick {tick}");
        assert_eq!(
            i64::from(*y),
            35_840 + SPEED * i64::from(*tick),
            "tick {tick}"
        );
        let d = (i64::from(px - x)).pow(2) + (i64::from(py - y)).pow(2);
        assert!(d >= STOP * STOP, "tick {tick}: inside the stop");
    }
    assert_eq!(rows, trace(), "the same on a second run");
}

/// A short run with one boss of `id` at tick 100 and no enemies.
fn one_boss(id: &str) -> TimeLimit {
    let mut v = time_limit_json();
    v["length_ticks"] = json!(3000);
    v["boss_ticks"] = json!([100, 3000]);
    v["mid_boss_pool"] = json!([id]);
    v["waves"] = json!([]);
    v["elite_table"] =
        json!([{ "start_tick": 0, "weights": [{ "elite": "elite_low", "weight": 1 }] }]);
    TimeLimit::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap()
}

fn run_of(mode: &TimeLimit) -> Result<Engine, Error> {
    let setup = RunSetup {
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        sim_version: 2,
        ..RunSetup::default()
    };
    Engine::start_time_limit(&setup, RunContent::bundled(), mode)
}

/// The boss's and the player's place after each step, the player walking down from tick 90 when
/// `walk` and standing otherwise.
fn follow(id: &str, ticks: u32, walk: bool) -> Vec<(u32, Option<(i64, i64)>, (i64, i64))> {
    let mut e = run_of(&one_boss(id)).unwrap();
    let mut rows = Vec::new();
    while e.tick() < ticks {
        let input = if walk && e.tick() >= 90 {
            common::moving(0, 127)
        } else {
            Input::default()
        };
        e.step(&[input, Default::default(), Default::default()]);
        let s = e.snapshot();
        let boss = s
            .entities
            .iter()
            .find(|v| v.kind == EntityKind::Boss)
            .map(|b| (i64::from(b.at.x.0), i64::from(b.at.y.0)));
        let me = (
            i64::from(s.players[0].at.x.0),
            i64::from(s.players[0].at.y.0),
        );
        rows.push((e.tick(), boss, me));
    }
    rows
}

fn dist(a: (i64, i64), b: (i64, i64)) -> i64 {
    (((a.0 - b.0).pow(2) + (a.1 - b.1).pow(2)) as f64).sqrt() as i64
}

#[test]
fn a_mode_boss_keeps_its_distance_and_never_outruns_its_speed() {
    for id in ["stone_warden", "lantern_choir", "weaver", "core_pair"] {
        let (speed, stop) = chase_of(&boss_of(id)).unwrap();
        let rows = follow(id, 400, true);
        let mut last = None;
        // A boss that arrives inside its stop (the Far ones, 120 px out) holds that distance.
        let mut floor = stop;
        for (tick, boss, me) in &rows {
            let Some(b) = *boss else { continue };
            if last.is_none() {
                floor = floor.min(dist(b, *me));
            }
            assert!(
                dist(b, *me) + 1 >= floor,
                "{id} tick {tick}: closer than its stop"
            );
            if let Some(p) = last {
                assert!(
                    dist(p, b) <= speed + 1,
                    "{id} tick {tick}: faster than {speed}"
                );
            }
            last = Some(b);
        }
        assert!(last.is_some(), "{id} came");
    }
}

#[test]
fn an_idle_player_is_approached_to_the_stop_and_no_closer() {
    for id in ["stone_warden", "core_pair"] {
        let (_, stop) = chase_of(&boss_of(id)).unwrap();
        let rows = follow(id, 700, false);
        let (_, boss, me) = rows.last().unwrap();
        let d = dist(boss.unwrap(), *me);
        assert!((stop - 1..=stop + 400).contains(&d), "{id}: stands at {d}");
    }
}

#[test]
fn a_far_boss_stands_still_inside_its_stop_and_walks_when_the_player_leaves() {
    for id in ["lantern_choir", "weaver"] {
        let rows = follow(id, 160, false);
        let places: Vec<_> = rows.iter().filter_map(|r| r.1).collect();
        assert!(
            places.windows(2).all(|w| w[0] == w[1]),
            "{id}: moved while inside"
        );
        let walking = follow(id, 600, true);
        let first = walking.iter().find_map(|r| r.1).unwrap();
        let last = walking.last().unwrap().1.unwrap();
        assert!(last.1 > first.1, "{id}: never followed");
    }
}

#[test]
fn a_mode_boss_may_chase_but_not_arrive() {
    let with = |field: &str, value: Value| {
        let mut v = time_limit_json();
        v["bosses"][0][field] = value;
        let mode = TimeLimit::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap();
        run_of(&mode).err()
    };
    let Some(Error::Invalid(found)) = with("arrives_at_tick", json!(50)) else {
        panic!("started");
    };
    assert!(
        found
            .iter()
            .any(|i| i.path == "$.bosses[0].arrives_at_tick")
    );
    assert!(with("chase", json!({ "speed": 767 })).is_none());
    let Some(Error::Invalid(found)) = with("chase", json!({ "speed": 768 })) else {
        panic!("started");
    };
    assert!(
        found.iter().any(|i| i.path.ends_with("chase.speed")),
        "{found:?}"
    );
}
