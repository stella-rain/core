//! Golden recordings (ADR-016): the JSON Lines recording of a short run for a companion and
//! for an enemy, compared byte for byte with the files in `tests/golden/`. They read like a
//! story of what each agent did and why the hash moved, and any change to the rules shows up
//! as a diff in review. After an intended change, regenerate them with
//! `UPDATE_GOLDEN=1 cargo test --features record --test golden` and say
//! `Corpus regenerated: <why>` in the PR (ADR-035).
#![cfg(feature = "record")]

mod common;

use std::path::Path;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::input::Input;
use stella_rain_core::recording;

fn check(name: &str, recording: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.jsonl"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, recording).unwrap();
    }
    let golden = std::fs::read_to_string(&path).unwrap_or_default();
    if golden != recording {
        let first = golden
            .lines()
            .zip(recording.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| golden.lines().count().min(recording.lines().count()));
        panic!(
            "{name}.jsonl differs from the recording, first at line {}:\n  golden:    {}\n  recording: {}\n\
             after an intended change run `UPDATE_GOLDEN=1 cargo test --features record --test golden`",
            first + 1,
            golden.lines().nth(first).unwrap_or("(end of file)"),
            recording.lines().nth(first).unwrap_or("(end of file)"),
        );
    }
}

fn shoot_at(selector: &str, cooldown: u32) -> Value {
    rule(
        always(),
        json!(selector),
        json!({ "type": "attack", "attack": { "preset": { "id": "twin_shot", "args": [] } } }),
        cooldown,
    )
}

/// A companion that is shot at, healed by a second one, and that buffs the main character.
/// Entities: boss 1, K 2, healer 3, shooter 4.
#[test]
fn a_companion_buffs_is_shot_at_and_is_healed() {
    let k = companion(
        10,
        hold(46_080, 130_000),
        json!([rule(always(), json!("player"), atk_up(25, 40), 30)]),
    );
    let healer = companion(
        10,
        hold(80_000, 147_456),
        json!([rule(
            json!({ "type": "hp_below", "who": "lowest_hp_ally", "pct": 100 }),
            json!("lowest_hp_ally"),
            cast(json!({ "type": "heal", "amount": 5 })),
            0
        )]),
    );
    let shooter = enemy(
        50,
        hold(46_080, 100_000),
        json!([shoot_at("nearest_ally", 5)]),
    );
    let stage = stage_full(
        600,
        Some(far_boss()),
        vec![wave(1, 1, 0, shooter, 46_080, 100_000)],
        vec![k, healer],
        vec![],
    );
    // 80 ticks, firing for the first 40: the buff changes the shots' damage.
    let inputs: Vec<Input> = (0..80)
        .map(|t| if t < 40 { firing() } else { idle() })
        .collect();
    check("companion", &recording::record(&stage, &inputs).unwrap());
}

/// An enemy that waits, telegraphs, shoots at the main character and summons two helpers
/// while the main character shoots it. Entities: boss 1, the enemy 2, the helpers 3 and 4.
#[test]
fn an_enemy_waits_telegraphs_shoots_and_summons() {
    let rules = json!([
        {
            "when": { "type": "time_above", "ticks": 10 }, "target": "player", "once": true,
            "do": telegraph()
        },
        { "when": { "type": "player_firing", "min_ticks": 20 }, "target": "player", "once": true,
          "do": summon(2) },
        shoot_at("player", 12),
    ]);
    let boss_in_the_way = enemy(100, hold(46_080, 60_000), rules);
    let stage = stage_full(
        600,
        Some(far_boss()),
        vec![wave(1, 1, 0, boss_in_the_way, 46_080, 60_000)],
        vec![],
        vec![],
    );
    let inputs: Vec<Input> = (0..90).map(|_| firing()).collect();
    check("enemy", &recording::record(&stage, &inputs).unwrap());
}
