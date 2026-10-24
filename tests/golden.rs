//! Golden recordings (ADR-016): the JSON Lines recording of a short run for a companion, an
//! enemy, a boss and the main shot, kept as `insta` snapshots in `tests/golden/` (ADR-021).
//! They read like a story of what each agent did and why the hash moved, and any change to the
//! rules shows up as a diff in review. After an intended change, run
//! `cargo insta test --features record --test integration --review` (or set `INSTA_UPDATE=always`)
//! and say `Corpus regenerated: <why>` in the PR (ADR-035). In CI a difference always fails.
#![cfg(feature = "record")]

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::input::Input;
use stella_rain_core::recording;

/// Compares a recording with `tests/golden/<name>.snap`. Below the header (the lines up to
/// the second `---`), the file is the recording itself, one JSON object a line, so the event
/// player can read it too.
fn check(name: &str, recording: &str) {
    insta::with_settings!({ snapshot_path => "golden", prepend_module_to_snapshot => false }, {
        insta::assert_snapshot!(name, recording);
    });
}

fn shoot_at(selector: &str, cooldown: u32) -> Value {
    rule(
        always(),
        json!(selector),
        json!({ "type": "attack", "attack": { "preset": { "id": "aimed_single", "args": [15] } } }),
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
    );
    // 80 ticks: 40 standing still, shooting up at the enemy (the buff changes the shots'
    // damage), then 40 walking right, which turns the shots away from it.
    let inputs: Vec<Input> = (0..80)
        .map(|t| firing(if t < 40 { idle() } else { moving(64, 0) }, t))
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
        { "when": { "type": "time_above", "ticks": 20 }, "target": "player", "once": true,
          "do": summon(2) },
        shoot_at("player", 12),
    ]);
    let boss_in_the_way = enemy(100, hold(46_080, 60_000), rules);
    let stage = stage_full(
        600,
        Some(far_boss()),
        vec![wave(1, 1, 0, boss_in_the_way, 46_080, 60_000)],
        vec![],
    );
    let inputs: Vec<Input> = (0..90).map(|t| firing(idle(), t)).collect();
    check("enemy", &recording::record(&stage, &inputs).unwrap());
}

/// A placeholder boss: a horn that breaks, two phases, a timeline of telegraph, attack and
/// move, a transition that clears the bullets and shields the boss, and a rule that reacts to
/// the horn. Entities: boss 1, the horn 2.
#[test]
fn a_boss_breaks_a_part_and_changes_phase() {
    let horn = json!({ "id": "horn", "asset": "horns_2", "hp": 30,
                       "offset": { "x": 0, "y": 16_000 }, "radius": 6_000 });
    let shot = json!({ "type": "attack",
                       "attack": { "preset": { "id": "aimed_single", "args": [15] } } });
    let boss = json!({
        "base": "golem", "hp": 100, "radius": 8192, "spawn": { "x": 46_080, "y": 60_000 },
        "parts": [horn],
        "phases": [
            {
                "until": { "type": "part_broken", "part": "horn" },
                "timeline": [
                    { "type": "telegraph", "ticks": 5 }, shot.clone(),
                    { "type": "move_to", "to": { "x": 49_000, "y": 60_000 }, "ticks": 10 },
                    { "type": "wait", "ticks": 5 }, { "type": "loop" }
                ],
                "rules": [rule(
                    json!({ "type": "hp_below", "who": { "part": "horn" }, "pct": 50 }),
                    json!("self"), telegraph(), 0
                )]
            },
            {
                "transition": [{ "type": "clear_bullets" },
                               { "type": "invulnerable_ticks", "ticks": 10 }],
                "timeline": [shot, { "type": "wait", "ticks": 8 }, { "type": "loop" }]
            }
        ]
    });
    let stage = stage_full(600, Some(boss), vec![], vec![]);
    let inputs: Vec<Input> = (0..160).map(|t| firing(idle(), t)).collect();
    check("boss", &recording::record(&stage, &inputs).unwrap());
}

/// The main shot is the stage's attack, along the facing: a wait-and-place attack of friendly
/// zones at a boss in front of the main character, which carries on after the main character
/// turns away.
/// Entities: boss 1.
#[test]
fn the_main_shot_is_the_stages_attack() {
    let mut stage = stage_full(600, Some(boss_at(100, 80_000)), vec![], vec![]);
    stage.player.weapon = serde_json::from_value(weapon_ref(json!({
        "inline": spawned(json!([
            { "type": "spread", "count": 2, "angle": 1280, "body": [
                { "type": "zone", "shape": { "type": "circle", "radius": 2048 },
                  "offset": { "forward": 2048 }, "direction": { "type": "aimed", "offset": 0 },
                  "wind_up_ticks": 0, "active_ticks": 8 } ] },
            { "type": "wait", "ticks": 4 },
            { "type": "zone", "shape": { "type": "circle", "radius": 2048 },
              "offset": { "forward": 2048 }, "direction": { "type": "aimed", "offset": 0 },
              "wind_up_ticks": 0, "active_ticks": 8 }
        ]))
    })))
    .unwrap();
    let inputs: Vec<Input> = (0..40)
        .map(|t| Input {
            main: t == 0 || t == 12,
            ..if t < 7 { idle() } else { moving(1, 0) }
        })
        .collect();
    check("shot", &recording::record(&stage, &inputs).unwrap());
}
