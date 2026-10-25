//! Expressions never panic (ADR-036, ADR-019): every expression slot of an attack is given the
//! most extreme values the validator lets a creator write, and the engine runs on. A replay
//! proves only the creator's path, so a challenger who survives longer or meets other values
//! must not be able to crash a verified stage. The constants stay within the validator's
//! limits (±1,000,000, at most 16 nodes in an expression); the arithmetic on them does not.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::stage::Stage;

const OX: i32 = 46_080;
const OY: i32 = 50_000;
/// The length of a stage: five minutes of ticks (ADR-020).
const FULL_LENGTH: u32 = 18_000;
/// Long enough for several rounds of every attack and the orbs and zones they start.
const SHORT_RUN: u32 = 600;

/// Every place an attack takes an expression.
const SLOTS: [&str; 12] = [
    "repeat_count",
    "repeat_interval",
    "ring_count",
    "spread_count",
    "spread_angle",
    "wait_ticks",
    "rotate_angle",
    "after_ticks",
    "speed",
    "aimed_offset",
    "relative_angle",
    "sequential_step",
];

/// `v`, written as a sum: the validator range-checks a constant in a count or a wait, and the
/// engine's own clamps are what get tested here.
fn num(v: i32) -> Value {
    json!({ "add": [v, 0] })
}

/// What a slot holds when it is not under test: a small, ordinary value.
fn ordinary(slot: &str) -> Value {
    num(match slot {
        "repeat_count" => 3,
        "repeat_interval" => 5,
        "ring_count" => 4,
        "spread_count" => 3,
        "spread_angle" => 7_680,
        "wait_ticks" => 2,
        "rotate_angle" => 1_280,
        "after_ticks" => 4,
        "speed" => 600,
        "aimed_offset" => 0,
        "relative_angle" => 2_560,
        _ => 1_024,
    })
}

/// The values to try, each written within the validator's limits. The largest and smallest
/// saturate on the way; the last ones reach `i32::MIN / -1`, a division by zero and the full
/// range of `rand`.
fn extremes() -> Vec<(&'static str, Value)> {
    let largest = json!({ "mul": [1_000_000, 1_000_000] });
    let smallest = json!({ "mul": [-1_000_000, 1_000_000] });
    vec![
        ("max constant", num(1_000_000)),
        ("min constant", num(-1_000_000)),
        ("zero", num(0)),
        ("largest", largest.clone()),
        ("smallest", smallest.clone()),
        (
            "largest plus largest",
            json!({ "add": [largest.clone(), largest.clone()] }),
        ),
        (
            "smallest minus largest",
            json!({ "sub": [smallest.clone(), largest.clone()] }),
        ),
        (
            "smallest over minus one",
            json!({ "div": [smallest.clone(), -1] }),
        ),
        ("anything over zero", json!({ "div": [1_000_000, 0] })),
        (
            "rand over the legal range",
            json!({ "rand": [-1_000_000, 1_000_000] }),
        ),
        (
            "rand over the whole range",
            json!({ "rand": [smallest.clone(), largest.clone()] }),
        ),
        (
            "phase time times largest",
            json!({ "mul": ["phase_time", largest.clone()] }),
        ),
        (
            "loop index times smallest",
            json!({ "mul": ["loop_index", smallest.clone()] }),
        ),
        (
            "clamp with reversed bounds",
            json!({ "clamp": [largest, 1_000_000, smallest] }),
        ),
    ]
}

fn rule(attack: Vec<Value>) -> Value {
    json!({ "when": always(), "target": "player", "cooldown_ticks": 90,
            "do": { "type": "attack", "attack": { "inline": spawned(json!(attack)) } } })
}

/// Two shooters that start their attack again every 90 ticks, between them using every slot.
/// `value` says what each slot holds.
fn stage(value: &dyn Fn(&str) -> Value) -> Stage {
    let v = value;
    let first = vec![json!({
        "type": "repeat", "count": v("repeat_count"), "interval_ticks": v("repeat_interval"),
        "body": [
            { "type": "ring", "count": v("ring_count"), "body": [
                { "type": "spread", "count": v("spread_count"), "angle": v("spread_angle"),
                  "body": [{ "type": "orb", "speed": v("speed"), "ticks": 40,
                             "direction": { "type": "aimed", "offset": v("aimed_offset") } }] }
            ] },
            { "type": "wait", "ticks": v("wait_ticks") },
        ],
    })];
    let second = vec![json!({
        "type": "rotate", "angle": v("rotate_angle"),
        "body": [{
            "type": "orb", "speed": v("speed"), "ticks": v("after_ticks"),
            "direction": { "type": "sequential", "step": v("sequential_step") },
            "burst": [{
                "type": "repeat", "count": v("repeat_count"), "interval_ticks": v("wait_ticks"),
                "body": [{ "type": "zone", "shape": { "type": "circle", "radius": 4_096 },
                           "anchor": "target",
                           "direction": { "type": "relative", "angle": v("relative_angle") },
                           "wind_up_ticks": v("wait_ticks"), "active_ticks": 5 }],
            }],
        }],
    })];
    let waves = vec![
        wave(
            1,
            1,
            0,
            enemy(50, hold(OX, OY), json!([rule(first)])),
            OX,
            OY,
        ),
        wave(
            1,
            1,
            0,
            enemy(50, hold(OX - 20_000, OY), json!([rule(second)])),
            OX - 20_000,
            OY,
        ),
    ];
    // The weapons are quiet, so that the enemies live to fill the field.
    let mut stage = quiet(stage_full(FULL_LENGTH, Some(far_boss()), waves, vec![]));
    // 99 hit points at the design values, scaled (ADR-103): the enemies' default hit is 100.
    stage.player.hp = 9_900;
    stage
}

/// Runs `ticks` ticks without input and returns the state hash.
fn run(stage: &Stage, ticks: u32) -> (u32, u64) {
    let mut e = engine(stage);
    for _ in 0..ticks {
        e.step(idle());
    }
    (e.tick(), e.state_hash())
}

#[test]
fn every_slot_survives_every_extreme() {
    for slot in SLOTS {
        for (name, extreme) in extremes() {
            let s = stage(&|k| {
                if k == slot {
                    extreme.clone()
                } else {
                    ordinary(k)
                }
            });
            let (tick, _) = run(&s, SHORT_RUN);
            assert_eq!(tick, SHORT_RUN, "{slot} = {name}");
        }
    }
}

#[test]
fn every_slot_at_once_survives_the_whole_stage() {
    for (name, extreme) in extremes() {
        let s = stage(&|_| extreme.clone());
        assert_eq!(run(&s, FULL_LENGTH).0, FULL_LENGTH, "{name}");
    }
}

/// Everything extreme except the orbs' speed and aim, so that orbs do stay in the field
/// and the caps (ADR-020) and the emitter budget are what holds the load.
fn busy(extreme: &Value) -> Stage {
    let mut stage = stage(&|k| {
        if k == "speed" || k == "aimed_offset" {
            ordinary(k)
        } else {
            extreme.clone()
        }
    });
    // Three hits end the run, so that the zones that land are seen to.
    stage.player.hp = 300;
    stage
}

#[test]
fn busy_stages_survive_the_whole_stage() {
    // A run ends when the main character dies or the stage runs out of ticks; either way the
    // engine must get there without a panic. That some of them end early shows that the
    // zones did reach the character, so the runs are not empty.
    let ends: Vec<u32> = extremes()
        .iter()
        .map(|(_, extreme)| run(&busy(extreme), FULL_LENGTH).0)
        .collect();
    assert!(ends.iter().all(|t| *t <= FULL_LENGTH));
    assert!(ends.iter().any(|t| *t < FULL_LENGTH), "{ends:?}");
}

/// The hash of one mixed run: slot `i` takes extreme `i`, speed and aim stay ordinary. Every
/// platform must produce exactly this (ADR-019): a difference in how overflow, saturation or
/// division behaves on another architecture would show here. Under version 0 a change to the
/// rules may change it (ADR-035); then it is regenerated and the PR says so.
const MIXED_HASH: u64 = 0xdb3e_8bb1_6037_b79e;

#[test]
fn a_mixed_extreme_run_gives_the_same_hash_on_every_platform() {
    let all = extremes();
    let s = stage(&|k| {
        let i = SLOTS.iter().position(|slot| *slot == k).unwrap();
        if k == "speed" || k == "aimed_offset" {
            ordinary(k)
        } else {
            all[i % all.len()].1.clone()
        }
    });
    assert_eq!(run(&s, FULL_LENGTH), (FULL_LENGTH, MIXED_HASH));
}
