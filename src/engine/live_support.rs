//! Helpers for the live (version 2) ports of the frozen engine tests: the twins of the helpers in
//! `horde_tests.rs`, started from the bundled files at `sim_version` 2.

#![allow(dead_code)] // read by the ports that follow

use super::*;
use crate::time_limit::TimeLimit;
use crate::version::Versions;

/// The bundled `time_limit` mode as JSON a test may edit before it parses it again.
pub(super) fn live_time_limit_json() -> serde_json::Value {
    serde_json::from_slice(RunContent::bundled().time_limit().bytes())
        .expect("the bundled mode is JSON")
}

/// A version 2 `time_limit` setup.
pub(super) fn live_setup(seed: u64, difficulty: &str) -> RunSetup {
    RunSetup {
        sim_version: 2,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        weapon_tree: None,
        seed,
        perk_set: "duelist".into(),
        difficulty: difficulty.into(),
        ..RunSetup::default()
    }
}

/// A bundled `time_limit` run at version 2 on `difficulty`.
pub(super) fn live_bundled_run(difficulty: &str, seed: u64) -> Engine {
    let content = RunContent::bundled();
    Engine::start_time_limit(&live_setup(seed, difficulty), content, content.time_limit()).unwrap()
}

/// The live twin of `horde_tests::engine_edited`: no waves, one elite, the boss times and
/// no-call times given, the mode file edited by `edit` before it is parsed.
pub(super) fn live_engine_edited(
    boss_ticks: &[u32],
    no_call_ticks: &[u32],
    edit: impl FnOnce(&mut serde_json::Value),
) -> Engine {
    let mut v = live_time_limit_json();
    v["waves"] = serde_json::json!([]);
    edit(&mut v);
    v["no_call_ticks"] = serde_json::json!(no_call_ticks);
    v["length_ticks"] = serde_json::json!(boss_ticks.last().unwrap());
    v["boss_ticks"] = serde_json::json!(boss_ticks);
    v["elite_table"] =
        serde_json::json!([{"start_tick": 0, "weights": [{"elite": "elite_low", "weight": 1}]}]);
    let mode = TimeLimit::parse(v.to_string().as_bytes(), Versions::CURRENT)
        .unwrap_or_else(|e| panic!("{e:?}"));
    Engine::start_time_limit(&live_setup(1, "normal"), RunContent::bundled(), &mode).unwrap()
}

/// The live twin of `horde_tests::run_of`: the mode edited, no waves, a main character that
/// cannot die.
pub(super) fn live_run_of(seed: u64, edit: impl FnOnce(&mut serde_json::Value)) -> Engine {
    let mut v = live_time_limit_json();
    v["waves"] = serde_json::json!([]);
    edit(&mut v);
    let mode = TimeLimit::parse(v.to_string().as_bytes(), Versions::CURRENT)
        .unwrap_or_else(|e| panic!("{e}"));
    let mut e = Engine::start_time_limit(&live_setup(seed, "normal"), RunContent::bundled(), &mode)
        .unwrap();
    e.run.solo_mut().hp = u32::MAX;
    e.run.solo_mut().max_hp = u32::MAX;
    e
}
