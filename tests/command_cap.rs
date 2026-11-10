//! The most commands (`choose`, `spend`) the longest possible run takes, which `MAX_COMMANDS`
//! is about twice (app#336 D5b): `time_limit` at a difficulty that keeps its full length, played
//! through the final boss's time and the whole overtime by a bot that cannot die, takes every
//! offer and spends every point as soon as it has one.

use crate::common;
use serde_json::json;
use stella_rain_core::perk_sets::PerkSets;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{
    CommandKind, MAX_COMMANDS, RunRecorder, RunReplay, RunReplayError, TickCommand, pack_commands,
    verify_run_with,
};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::version::Versions;

/// The perk set the bot plays with: the most hit points and regeneration a set may hold (the
/// engine has no invulnerability switch), the most extra rerolls and banishes, and the
/// strongest attack, so that it kills, levels and takes stars as fast as a run can.
const PERK_SET: &str = "undying";

/// The run content with the `undying` perk set added to the bundled ones.
pub fn undying_content() -> RunContent {
    let mut file: serde_json::Value =
        serde_json::from_slice(common::bundled_file("perk_sets")).unwrap();
    file["perk_sets"].as_array_mut().unwrap().push(json!({
        "id": PERK_SET,
        "effects": [
            { "effect": "max_hp", "amount": 50_000 },
            { "effect": "regen", "amount": 1_000 },
            { "effect": "attack_pct", "amount": 100 },
            { "effect": "attack_speed_pct", "amount": 100 },
            { "effect": "extra_rerolls", "amount": 20 },
            { "effect": "extra_banishes", "amount": 20 }
        ]
    }));
    RunContent::bundled()
        .clone()
        .with_perk_sets(PerkSets::parse(file.to_string().as_bytes(), Versions::CURRENT).unwrap())
}

pub fn setup(seed: u64, difficulty: &str, extra_rerolls: u8) -> RunSetup {
    RunSetup {
        schema_version: 1,
        sim_version: 1,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        weapon_tree: None,
        seed,
        star_pool: None,
        extra_rerolls,
        upgrades: Vec::new(),
        perk_set: PERK_SET.into(),
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
        stat_priority: Vec::new(),
        // The most commands a run can cost: every offer answered.
        difficulty: difficulty.into(),
        ..RunSetup::default()
    }
}

/// The most commands the measurement found (`measure_the_longest_run`), pinned: `MAX_COMMANDS`
/// is at least twice it. 425 at `2ed393f` (hard, seeds 2 and 4, the `Takes` bot); 400 once the
/// setup's `pick_stats` left (the frozen rules no longer open a stat offer for each level-up).
/// Re-measure and re-pin when the deep tier reports a higher number.
const MEASURED_MOST: usize = 400;

/// The bundled mode cut to 20 seconds with a boss at 10 and at 20 and 5 of overtime, a run
/// that a debug build plays in seconds.
fn short_content() -> RunContent {
    let mut v: serde_json::Value =
        serde_json::from_slice(common::bundled_file("time_limit")).unwrap();
    v["length_ticks"] = json!(1200);
    v["boss_ticks"] = json!([600, 1200]);
    v["no_call_ticks"] = json!([]);
    v["overtime_ticks"] = json!(300);
    v["waves"]
        .as_array_mut()
        .unwrap()
        .retain(|w| w["at_tick"].as_u64().unwrap() < 1200);
    v["elite_table"] =
        json!([{ "start_tick": 0, "weights": [{ "elite": "elite_low", "weight": 1 }] }]);
    let mode = TimeLimit::parse(v.to_string().as_bytes(), Versions::CURRENT)
        .unwrap_or_else(|e| panic!("{e}"));
    undying_content().with_time_limit(mode)
}

#[test]
fn the_cap_is_twice_the_longest_run_or_more() {
    assert!(
        2 * MEASURED_MOST <= MAX_COMMANDS as usize,
        "MAX_COMMANDS {MAX_COMMANDS} is under twice {MEASURED_MOST}"
    );
}

#[test]
fn a_crafted_replay_with_one_command_over_the_cap_is_refused() {
    let content = short_content();
    // The recorder writes schema 2, which has no command list: the crafted file is schema 1.
    let mut run = RunRecorder::start_with(&setup(1, "normal", 20), &content).unwrap();
    for _ in 0..20 {
        run.step(&[Default::default(), Default::default(), Default::default()])
            .unwrap();
    }
    let mut replay = crate::common::schema_1_form(&run.replay());
    let choices = |n: u32| -> String {
        let commands: Vec<TickCommand> = (0..n)
            .map(|_| TickCommand {
                tick: 0,
                kind: CommandKind::Choose,
                index: 0,
            })
            .collect();
        pack_commands(&commands)
    };
    // At the cap the file loads (the choices are then refused by the engine for lack of an
    // offer, not for their number); one over it never gets that far.
    replay.commands = choices(MAX_COMMANDS);
    assert_eq!(
        RunReplay::from_bytes(&replay.to_bytes())
            .unwrap()
            .unpack_commands()
            .unwrap()
            .len(),
        MAX_COMMANDS as usize
    );
    replay.commands = choices(MAX_COMMANDS + 1);
    assert_eq!(
        verify_run_with(&replay.to_bytes(), &content),
        Err(RunReplayError::TooManyCommands {
            limit: MAX_COMMANDS
        })
    );
}
