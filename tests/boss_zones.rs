//! No bundled boss has more than 24 live zones at once (core#349, DS50 item 4): the enemy design
//! doc's limit, kept for the screen's readability and the per-tick cost. Each phase of each
//! `time_limit` boss runs alone, with its zones made harmless so the run does not end, and the
//! most zones the snapshot holds in one tick is counted.

use crate::common;
use serde_json::{Value, json};
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::time_limit::TimeLimit;

/// The enemy design doc's limit of live zones per boss.
const LIMIT: usize = 24;
/// Ticks one phase runs: many loops of the longest timeline.
const RUN_TICKS: u32 = 4000;
const BOSS_AT: u32 = 120;

fn setup() -> RunSetup {
    RunSetup {
        schema_version: 0,
        sim_version: 0,
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
        pick_stats: false,
        difficulty: "normal".into(),
    }
}

/// Every `damage` in `v` set to 1, so no zone kills the idle main character.
fn harmless(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                if k == "damage" {
                    *x = json!(1);
                } else {
                    harmless(x);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(harmless),
        _ => {}
    }
}

/// The most zones live in one tick of `phase` of boss `id`, running alone.
fn peak(id: &str, phase: usize) -> usize {
    let mut v: Value = serde_json::from_slice(common::bundled_file("time_limit")).unwrap();
    let boss = v["bosses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["base"] == id)
        .unwrap()
        .clone();
    let mut boss = boss;
    let mut only = boss["phases"][phase].clone();
    only.as_object_mut().unwrap().remove("until");
    boss["phases"] = json!([only]);
    harmless(&mut boss);
    for b in v["bosses"].as_array_mut().unwrap() {
        if b["base"] == id {
            *b = boss.clone();
        }
    }
    v["length_ticks"] = json!(100_000);
    v["boss_ticks"] = json!([BOSS_AT, 100_000]);
    v["mid_boss_pool"] = json!([id]);
    v["waves"] = json!([]);
    v["elite_table"] =
        json!([{ "start_tick": 0, "weights": [{ "elite": "elite_low", "weight": 1 }] }]);
    let mode = TimeLimit::parse(v.to_string().as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    let mut e = Engine::start_time_limit(&setup(), RunContent::bundled(), &mode).unwrap();
    let mut most = 0;
    for _ in 0..BOSS_AT + RUN_TICKS {
        e.step(Input::default());
        most = most.max(e.snapshot().zones.len());
    }
    assert_eq!(
        e.outcome(),
        Outcome::Running,
        "{id} phase {phase} ended early"
    );
    most
}

#[test]
fn no_boss_phase_has_more_than_24_live_zones() {
    let m = TimeLimit::bundled();
    let mut over = Vec::new();
    let mut table = String::new();
    for b in m.bosses() {
        let id = b.base.0.as_str();
        for phase in 0..b.phases.len() {
            let n = peak(id, phase);
            table += &format!("\n  {id} phase {}: {n}", phase + 1);
            if n > LIMIT {
                over.push(format!("{id} phase {}: {n}", phase + 1));
            }
        }
    }
    println!("peaks:{table}");
    assert!(
        over.is_empty(),
        "boss phases over {LIMIT} live zones: {over:?}\nall peaks:{table}"
    );
}
