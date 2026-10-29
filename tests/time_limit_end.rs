//! How a `time_limit` run ends (core#212 step c): the final boss's kill clears it, the main
//! character's death or the final boss alive at the length and the overtime fails it, and a
//! replay may hold up to the length, the overtime and the Star-Hunter's extra time.

use crate::common;
use serde_json::{Value, json};
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{
    Command, RunReplay, RunReplayError, content_sha256_with, record_run, verify_run_with,
};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::version::Versions;

fn setup() -> RunSetup {
    RunSetup {
        schema_version: 1,
        sim_version: 1,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        weapon_tree: None,
        seed: 3,
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

/// A run of 100 ticks with the final boss at 100, 50 ticks of overtime and 20 of extra time.
fn short_mode() -> TimeLimit {
    let mut v: Value = serde_json::from_slice(common::bundled_file("time_limit")).unwrap();
    v["length_ticks"] = json!(100);
    v["boss_ticks"] = json!([40, 100]);
    v["overtime_ticks"] = json!(50);
    v["star_hunter_extra_ticks"] = json!(20);
    v["waves"] = json!([]);
    v["elite_table"] =
        json!([{ "start_tick": 0, "weights": [{ "elite": "elite_low", "weight": 1 }] }]);
    TimeLimit::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap()
}

fn replay_of(ticks: u32, content: &RunContent) -> Vec<u8> {
    let commands = vec![Command::Step(Input::default()); 10];
    let mut replay = record_run(&setup(), &commands).unwrap_or_else(|e| panic!("{e}"));
    replay.content_sha256 = stella_rain_core::content_hash::to_hex(
        &content_sha256_with("time_limit", content).unwrap(),
    );
    replay.ticks = ticks;
    replay.to_bytes()
}

#[test]
fn a_replay_holds_the_length_the_overtime_and_the_extra_time_and_no_more() {
    let content = RunContent::bundled().clone().with_time_limit(short_mode());
    let bound = 100 + 50 + 20;
    let refused = verify_run_with(&replay_of(bound + 1, &content), &content);
    assert!(
        matches!(
            refused,
            Err(RunReplayError::TooManyTicks { ticks, limit }) if ticks == bound + 1 && limit == bound
        ),
        "{refused:?}"
    );
    let at_the_bound = verify_run_with(&replay_of(bound, &content), &content);
    assert!(
        !matches!(at_the_bound, Err(RunReplayError::TooManyTicks { .. })),
        "{at_the_bound:?}"
    );
    let _ = RunReplay::from_bytes(&replay_of(1, &content)).unwrap();
}

#[test]
fn the_bundled_mode_bounds_a_replay_at_26_minutes() {
    let bound = 72_000 + 18_000 + 3_600;
    assert_eq!(bound, 26 * 60 * 60);
    let content = RunContent::bundled();
    let refused = verify_run_with(&replay_of(bound + 1, content), content);
    assert!(
        matches!(refused, Err(RunReplayError::TooManyTicks { limit, .. }) if limit == bound),
        "{refused:?}"
    );
}
