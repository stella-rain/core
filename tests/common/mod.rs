//! Shared by the replay, share-code, recording and `stage-verify` tests; `run` by the run
//! replay tests.
#![allow(dead_code)]

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

/// A stage whose boss (20 hp, 10 damage a bullet) is beaten on tick 57 by the Main weapon, which
/// attacks on its own straight up: the `blaster` preset's Main role, the twin shot, both of whose
/// bullets touch the boss on tick 57. The Sub weapon does nothing.
pub fn clearing_stage_json() -> Value {
    json!({
        "schema_version": 0, "sim_version": 0, "id": "test-stage", "title": "Test Stage",
        "seed": 7, "length_ticks": 600,
        "player": { "base": "pilot_a", "hp": 3, "atk": 10,
                    "main": { "preset": { "id": "blaster" } },
                    "sub": sub_ref(quiet_weapon()),
                    "evade": evade() },
        "boss": { "base": "golem", "hp": 20, "radius": 8192,
                  "spawn": { "x": 46080, "y": 25600 }, "phases": [{}] }
    })
}

/// A weapon's attack (core#83): `attack` (a preset or `{"inline": [...]}`) at `targeting`,
/// every `cooldown_ticks` ticks.
pub fn weapon(attack: Value, targeting: Value, cooldown_ticks: u32) -> Value {
    json!({ "attack": attack, "targeting": targeting, "cooldown_ticks": cooldown_ticks })
}

/// An attack that does nothing: one tick of recovery.
pub fn quiet_attack() -> Value {
    json!({ "inline": [{ "kind": "recovery", "ticks": 1 }] })
}

/// A weapon attack that does nothing, along the facing.
pub fn quiet_weapon() -> Value {
    weapon(quiet_attack(), json!("facing"), 1)
}

/// A charge that does nothing: one level, whose release is quiet.
pub fn quiet_charge() -> Value {
    json!({ "levels": [{ "at_ticks": 0, "attack": quiet_attack() }],
            "max_ticks": 600, "cooldown_ticks": 0 })
}

/// An inline Main weapon whose automatic attack is `auto` and whose charge does nothing.
pub fn main_ref(auto: Value) -> Value {
    json!({ "inline": { "auto": auto, "charge": quiet_charge() } })
}

/// An inline Sub weapon whose automatic attack is `auto` and whose skill does nothing.
pub fn sub_ref(auto: Value) -> Value {
    json!({ "inline": { "auto": auto, "skill": quiet_weapon() } })
}

/// `stage` with weapons whose automatic attacks are `main` and `sub` (`weapon` values, with
/// skills that do nothing), or presets (`{"preset": ...}`).
pub fn armed(stage: Stage, main: Value, sub: Value) -> Stage {
    let main = if main.get("preset").is_some() {
        main
    } else {
        main_ref(main)
    };
    let sub = if sub.get("preset").is_some() {
        sub
    } else {
        sub_ref(sub)
    };
    armed_full(stage, main, sub)
}

/// `stage` with `main` and `sub` as its weapons, whole references.
pub fn armed_full(mut stage: Stage, main: Value, sub: Value) -> Stage {
    stage.player.main = serde_json::from_value(main).expect("the Main weapon parses");
    stage.player.sub = serde_json::from_value(sub).expect("the Sub weapon parses");
    stage
}

/// The phases of an attack that spawns `nodes` on the tick it starts: one active phase of one
/// tick (ADR-055).
pub fn spawned(nodes: Value) -> Value {
    json!([{ "kind": "active", "ticks": 1, "spawn": nodes }])
}

pub fn stage_bytes() -> Vec<u8> {
    serde_json::to_vec(&clearing_stage_json()).unwrap()
}

pub const CLEAR_TICK: u32 = 57;

/// Inputs that clear `clearing_stage_json` on tick 57: a nudge up the field on tick 1, so that
/// the first shot goes up and beats the boss, then some movement to the right and an evade.
pub fn clearing_inputs() -> Vec<Input> {
    (0..CLEAR_TICK)
        .map(|t| Input {
            move_x: if (1..10).contains(&t) { 7 } else { 0 },
            move_y: if t == 0 { -1 } else { 0 },
            evade: t == 20,
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
        // a tick, every 6 ticks; the Sub weapon does nothing. So the hand-worked numbers of the
        // tests that are not about weapons stay simple (`tests/shot.rs` and `tests/weapons.rs`
        // are about them).
        "player": { "base": "pilot_a", "hp": 99, "atk": 10,
                    "main": main_ref(weapon(
                        json!({ "preset": { "id": "aimed_single", "args": [2048] } }),
                        json!("facing"), 6)),
                    "sub": sub_ref(quiet_weapon()),
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
/// thrown off by the weapons that attack on their own (ADR-050).
pub fn quiet(stage: Stage) -> Stage {
    armed(stage, quiet_weapon(), quiet_weapon())
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
