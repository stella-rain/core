//! Shared by the replay, share-code, recording and `stage-verify` tests.
#![allow(dead_code)]

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::{EntityKind, Snapshot};
use stella_rain_core::stage::Stage;

/// A stage whose boss (20 hp, 10 damage a hit) is beaten on tick 62 by holding fire: the first
/// shot touches it on tick 56 and the second, fired on tick 7, on tick 62.
pub fn clearing_stage_json() -> Value {
    json!({
        "schema_version": 0, "sim_version": 0, "id": "test-stage", "title": "Test Stage",
        "seed": 7, "length_ticks": 600,
        "player": { "base": "pilot_a", "hp": 3, "atk": 10,
                    "shot": { "preset": { "id": "twin_shot", "args": [] } } },
        "boss": { "base": "golem", "hp": 20, "radius": 8192,
                  "spawn": { "x": 46080, "y": 25600 }, "phases": [{}] }
    })
}

pub fn stage_bytes() -> Vec<u8> {
    serde_json::to_vec(&clearing_stage_json()).unwrap()
}

pub const CLEAR_TICK: u32 = 62;

pub fn firing() -> Input {
    Input {
        fire: true,
        ..Input::default()
    }
}

/// Inputs that clear `clearing_stage_json` on tick 62, with some movement along the way.
pub fn clearing_inputs() -> Vec<Input> {
    (0..CLEAR_TICK)
        .map(|t| Input {
            dx: if t < 10 { 40 } else { 0 },
            dy: 0,
            focus: false,
            fire: true,
            held: u8::from(t % 20 == 0) * 2,
        })
        .collect()
}

// --- Builders for small stages whose every tick can be worked out by hand -----------------

// Playfield and speeds in 1/256 pixel (the rules' constants, worked out by hand).
pub const START_X: i32 = 46_080;
pub const START_Y: i32 = 147_456;
pub const SPEED: i32 = 768;
pub const FOCUS_SPEED: i32 = 256;

pub fn idle() -> Input {
    Input::default()
}

pub fn moving(dx: i16, dy: i16) -> Input {
    Input {
        dx,
        dy,
        ..Input::default()
    }
}

/// A boss that cannot be reached or beaten, so a stage keeps running.
pub fn far_boss() -> Value {
    json!({ "base": "golem", "hp": 1_000_000, "radius": 1000, "spawn": { "x": 0, "y": 0 },
            "parts": [{ "id": "horn_left", "asset": "horns_2", "hp": 10,
                        "offset": { "x": 0, "y": 0 }, "radius": 100 }],
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

/// A stage with a 3-hp main character, no companions and no skills.
pub fn stage_with(length_ticks: u32, boss: Option<Value>, waves: Vec<Value>) -> Stage {
    let mut stage = stage_full(length_ticks, boss, waves, vec![], vec![]);
    stage.player.hp = 3;
    stage
}

/// A stage with a 99-hp main character (so it outlives a test), the given companions and
/// skill slots.
pub fn stage_full(
    length_ticks: u32,
    boss: Option<Value>,
    waves: Vec<Value>,
    companions: Vec<Value>,
    skills: Vec<Value>,
) -> Stage {
    let mut v = json!({
        "schema_version": 0, "sim_version": 0, "id": "test", "title": "Test", "seed": 7,
        "length_ticks": length_ticks,
        "player": { "base": "pilot_a", "hp": 99, "atk": 10,
                    "shot": { "preset": { "id": "twin_shot", "args": [] } }, "skills": skills },
        "companions": companions,
        "waves": waves,
    });
    if let Some(boss) = boss {
        v["boss"] = boss;
    }
    serde_json::from_value(v).expect("the test stage parses")
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
