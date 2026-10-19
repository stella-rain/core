//! Shared by the replay, share-code, recording and `stage-verify` tests; `run` by the run
//! replay tests.
#![allow(dead_code)]

pub mod on_hit;
pub mod run;

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::{EntityKind, Snapshot};
use stella_rain_core::stage::Stage;

// The evade of every test stage (ADR-050): 48 pixels over 8 ticks, 10 ticks of invulnerability
// and a 40-tick cooldown.
pub const EVADE_DISTANCE: i32 = 12_288;
pub const EVADE_TICKS: u32 = 8;
pub const EVADE_INVULNERABLE: u32 = 10;
pub const EVADE_COOLDOWN: u32 = 40;

pub fn evade() -> Value {
    json!({ "distance": EVADE_DISTANCE, "ticks": EVADE_TICKS,
            "invulnerable_ticks": EVADE_INVULNERABLE, "cooldown_ticks": EVADE_COOLDOWN })
}

/// A stage whose boss (20 hp, 10 damage a hit) waits 100 pixels above the main character, out
/// of the `blade`'s reach until an evade dashes 48 pixels toward it. The Sub weapon pulls
/// nothing.
pub fn clearing_stage_json() -> Value {
    json!({
        "schema_version": 0, "sim_version": 0, "id": "test-stage", "title": "Test Stage",
        "seed": 7, "length_ticks": 600,
        "player": { "base": "pilot_a", "hp": 3, "atk": 10,
                    "main": { "preset": { "id": "blade" } },
                    "sub": quiet_sub(),
                    "evade": evade() },
        "boss": { "base": "golem", "hp": 20, "radius": 8192,
                  "spawn": { "x": START_X, "y": START_Y - 25_600 }, "phases": [{}] }
    })
}

/// A combo step whose attack is `attack` (a preset or `{"inline": [...]}`), a leaf.
pub fn step(attack: Value) -> Value {
    json!({ "attack": attack })
}

/// An inline Main weapon (ADR-070) with `combo` as its root branches and no window.
pub fn main_combo(combo: Value) -> Value {
    json!({ "inline": { "combo": combo, "window_ticks": 0, "assist_range": 0 } })
}

/// An inline Main weapon with one step: `attack` on a press of `main`.
pub fn main_ref(attack: Value) -> Value {
    main_combo(json!({ "main": step(attack) }))
}

/// A Main weapon with no steps: presses do nothing.
pub fn quiet_main() -> Value {
    main_combo(json!({}))
}

/// An inline Sub weapon whose combo-part bonus pulls by `distance` over `ticks`.
pub fn sub_pull(distance: i32, ticks: u32) -> Value {
    json!({ "inline": { "bonus": { "type": "pull", "distance": distance, "ticks": ticks } } })
}

/// A Sub weapon whose pull moves nothing.
pub fn quiet_sub() -> Value {
    sub_pull(0, 1)
}

/// An attack that does nothing: one tick of recovery.
pub fn quiet_attack() -> Value {
    json!({ "inline": [{ "kind": "recovery", "ticks": 1 }] })
}

/// `stage` with `main` and `sub` as its weapons, whole references.
pub fn armed(mut stage: Stage, main: Value, sub: Value) -> Stage {
    stage.player.main = serde_json::from_value(main).expect("the Main weapon parses");
    stage.player.sub = serde_json::from_value(sub).expect("the Sub weapon parses");
    stage
}

/// `input` with `main` pressed on the tick after a step at `tick` (the engine's tick before the
/// step) when `tick` is a multiple of 6, and let go on the others: the test stage's Main weapon
/// fires every 6 ticks from the first, as its attack did on its own before attacks came by hand
/// (ADR-070).
pub fn firing(input: Input, tick: u32) -> Input {
    Input {
        main: tick % 6 == 0,
        ..input
    }
}

/// `run_until` with `input` `firing`.
pub fn fire_until(
    e: &mut Engine,
    input: Input,
    max_ticks: u32,
    mut stop: impl FnMut(&Engine) -> bool,
) -> Vec<(u32, DomainEvent)> {
    let mut seen = Vec::new();
    for _ in 0..max_ticks {
        e.step(firing(input, e.tick()));
        seen.extend(e.events().iter().map(|ev| (e.tick(), ev.clone())));
        if stop(e) {
            break;
        }
    }
    seen
}

/// An input that holds `main` (and moves nowhere).
pub fn pressing_main() -> Input {
    Input {
        main: true,
        ..Input::default()
    }
}

/// The phases of an attack that spawns `nodes` on the tick it starts: one active phase of one
/// tick (ADR-055).
pub fn spawned(nodes: Value) -> Value {
    json!([{ "kind": "active", "ticks": 1, "spawn": nodes }])
}

pub fn stage_bytes() -> Vec<u8> {
    serde_json::to_vec(&clearing_stage_json()).unwrap()
}

pub const CLEAR_TICK: u32 = 35;

/// Inputs that clear `clearing_stage_json`: an evade toward the boss, a dash-strike (the
/// `blade`'s thrust) during the dash, then, once it has run out, a slash from the root.
pub fn clearing_inputs() -> Vec<Input> {
    (0..CLEAR_TICK)
        .map(|t| Input {
            evade: t == 0,
            main: (3..5).contains(&t) || (30..32).contains(&t),
            ..Input::default()
        })
        .collect()
}

// --- Builders for small stages whose every tick can be worked out by hand -----------------

// Playfield and speeds in 1/256 pixel (the rules' constants, worked out by hand).
pub const START_X: i32 = 46_080;
pub const START_Y: i32 = 147_456;
/// The speed cap at full stick.
pub const SPEED: i32 = 768;

pub fn idle() -> Input {
    Input::default()
}

pub fn moving(move_x: i8, move_y: i8) -> Input {
    Input {
        move_x,
        move_y,
        ..Input::default()
    }
}

/// A boss that cannot be reached or beaten, so a stage keeps running.
pub fn far_boss() -> Value {
    json!({ "base": "golem", "hp": 1_000_000, "radius": 1000, "spawn": { "x": 0, "y": 0 },
            "phases": [{}] })
}

pub fn boss_at(hp: u32, y: i32) -> Value {
    json!({ "base": "golem", "hp": hp, "radius": 8192, "spawn": { "x": START_X, "y": y },
            "phases": [{}] })
}

pub fn enemy(hp: u32, movement: Value, rules: Value) -> Value {
    json!({ "base": "bat", "hp": hp, "radius": 2048, "movement": movement, "rules": rules })
}

pub fn hold(x: i32, y: i32) -> Value {
    json!({ "type": "hold", "at": { "x": x, "y": y } })
}

pub fn wave(at_tick: u32, count: u16, every_ticks: u32, enemy: Value, x: i32, y: i32) -> Value {
    json!({ "at_tick": at_tick, "enemy": enemy, "spawn": { "x": x, "y": y },
            "count": count, "every_ticks": every_ticks })
}

pub fn attack_rule(when: Value, cooldown_ticks: u32) -> Value {
    json!({ "when": when, "target": "player",
            "do": { "type": "attack", "attack": { "preset": { "id": "aimed_single", "args": [768] } } },
            "cooldown_ticks": cooldown_ticks })
}

pub fn always() -> Value {
    json!({ "type": "always" })
}

/// A stage with a 3-hp main character and no companions.
pub fn stage_with(length_ticks: u32, boss: Option<Value>, waves: Vec<Value>) -> Stage {
    let mut stage = stage_full(length_ticks, boss, waves, vec![]);
    stage.player.hp = 3;
    stage
}

/// A stage with a 99-hp main character (so it outlives a test) and the given companions.
pub fn stage_full(
    length_ticks: u32,
    boss: Option<Value>,
    waves: Vec<Value>,
    companions: Vec<Value>,
) -> Stage {
    let mut v = json!({
        "schema_version": 0, "sim_version": 0, "id": "test", "title": "Test", "seed": 7,
        "length_ticks": length_ticks,
        // The Main weapon: one bullet along the facing (straight up until it moves) at 8 pixels
        // a tick on a press of `main`; the Sub weapon pulls nothing. So the hand-worked numbers
        // of the tests that are not about weapons stay simple (`tests/shot.rs` and
        // `tests/weapons.rs` are about them).
        "player": { "base": "pilot_a", "hp": 99, "atk": 10,
                    "main": main_ref(json!({ "preset": { "id": "aimed_single", "args": [2048] } })),
                    "sub": quiet_sub(),
                    "evade": evade() },
        "companions": companions,
        "waves": waves,
    });
    if let Some(boss) = boss {
        v["boss"] = boss;
    }
    serde_json::from_value(v).expect("the test stage parses")
}

/// Both weapons replaced by ones that do nothing, for tests whose hand-worked numbers would be
/// thrown off by an attack.
pub fn quiet(stage: Stage) -> Stage {
    armed(stage, quiet_main(), quiet_sub())
}

pub fn engine(stage: &Stage) -> Engine {
    Engine::new(stage).expect("the test stage is valid")
}

pub fn running(waves: Vec<Value>) -> Engine {
    engine(&stage_with(18_000, Some(far_boss()), waves))
}

pub fn player_at(e: &Engine) -> (i32, i32) {
    let at = e.snapshot().player.at;
    (at.x.0, at.y.0)
}

pub fn enemies(s: &Snapshot) -> usize {
    s.entities
        .iter()
        .filter(|e| e.kind == EntityKind::Enemy)
        .count()
}

/// Steps with `input` until `stop` says so, collecting `(tick, event)` for every event.
pub fn run_until(
    e: &mut Engine,
    input: Input,
    max_ticks: u32,
    mut stop: impl FnMut(&Engine) -> bool,
) -> Vec<(u32, DomainEvent)> {
    let mut seen = Vec::new();
    for _ in 0..max_ticks {
        e.step(input);
        seen.extend(e.events().iter().map(|ev| (e.tick(), ev.clone())));
        if stop(e) {
            break;
        }
    }
    seen
}

pub fn ticks_of(events: &[(u32, DomainEvent)], pick: impl Fn(&DomainEvent) -> bool) -> Vec<u32> {
    events
        .iter()
        .filter(|(_, ev)| pick(ev))
        .map(|(t, _)| *t)
        .collect()
}

// --- Agents and their rules ---------------------------------------------------------------

pub fn companion(hp: u32, movement: Value, rules: Value) -> Value {
    json!({ "base": "fairy_a", "hp": hp, "radius": 1024, "movement": movement, "rules": rules })
}

pub fn rule(when: Value, target: Value, action: Value, cooldown_ticks: u32) -> Value {
    json!({ "when": when, "target": target, "do": action, "cooldown_ticks": cooldown_ticks })
}

pub fn telegraph() -> Value {
    json!({ "type": "telegraph", "ticks": 1 })
}

pub fn cast(skill: Value) -> Value {
    json!({ "type": "cast", "skill": skill })
}

pub fn atk_up(value_pct: u32, duration_ticks: u32) -> Value {
    cast(json!({ "type": "atk_up", "value_pct": value_pct, "duration_ticks": duration_ticks }))
}

pub fn summon(count: u8) -> Value {
    json!({ "type": "summon", "count": count,
            "agent": { "base": "bat", "hp": 10, "radius": 2048, "movement": hold(1000, 1000) } })
}
