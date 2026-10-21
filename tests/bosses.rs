//! The `time_limit` bosses (core#336, B1): the four mid-bosses and the final boss, each with its
//! own definition in the mode file, and the checks a run start makes of them. The draws and the
//! elite call by boss time play full timelines, so they are unit tests in `engine.rs`
//! (`horde_tests`), where the main character can be kept alive.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::stage::Boss;
use stella_rain_core::time_limit::TimeLimit;

const MINUTE: u32 = 3600;
const MID_BOSSES: [&str; 4] = ["stone_warden", "lantern_choir", "horn_charger", "weaver"];

fn setup() -> RunSetup {
    RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        seed: 1,
        star_pool: None,
        extra_rerolls: 0,
        upgrades: Vec::new(),
        perk_set: "duelist".into(),
        auto_combo: false,
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
        difficulty: "normal".into(),
    }
}

fn definition<'a>(mode: &'a TimeLimit, id: &str) -> &'a Boss {
    mode.bosses()
        .iter()
        .find(|b| b.base.0 == id)
        .unwrap_or_else(|| panic!("no definition of {id}"))
}

#[test]
fn the_pool_holds_the_four_mid_bosses_and_the_final_boss_is_the_core_pair() {
    let m = TimeLimit::bundled();
    let pool: Vec<&str> = m.mid_boss_pool().iter().map(|id| id.0.as_str()).collect();
    assert_eq!(pool, MID_BOSSES);
    assert_eq!(m.final_boss().0, "core_pair");
}

#[test]
fn every_pool_boss_and_the_final_boss_has_a_definition_with_a_profile_and_the_default_call() {
    let m = TimeLimit::bundled();
    for id in MID_BOSSES.iter().chain(&["core_pair"]) {
        let b = definition(m, id);
        assert!(b.profile.is_some(), "{id} has no match-up profile");
        let call = b.call_elites.expect("an elite call");
        assert_eq!(
            (call.after_ticks, call.below_pct),
            (Some(1800), Some(70)),
            "{id}"
        );
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
fn the_five_minute_boss_is_the_one_time_that_calls_no_elites() {
    let m = TimeLimit::bundled();
    assert_eq!(m.no_call_ticks(), [5 * MINUTE]);
    assert_eq!(m.boss_ticks()[0], 5 * MINUTE);
}

#[test]
fn a_definition_the_stage_rejects_stops_the_run_from_starting() {
    let mut v: Value = serde_json::from_slice(TimeLimit::BUNDLED).unwrap();
    v["bosses"][0]["phases"][0]["timeline"][2] = json!({
        "type": "attack", "attack": { "preset": { "id": "no_such_preset", "args": [] } }
    });
    let m = TimeLimit::parse(v.to_string().as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    let result = Engine::start_time_limit(&setup(), RunContent::bundled(), &m);
    assert!(result.is_err());
}
