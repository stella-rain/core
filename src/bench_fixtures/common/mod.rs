//! Builders for the stages of the bench cases (a copy of the subset of `tests/common/mod.rs`
//! they use).

pub mod on_hit;

use crate::engine::Engine;
use crate::input::Input;
use crate::pack::ContentFiles;
use crate::run_content::RunContent;
use crate::run_stats::RunStats;
use crate::stage::Stage;
use crate::version::Versions;
use serde_json::{Value, json};

/// The evade of every case stage (ADR-050): 48 pixels over 8 ticks, 10 ticks of invulnerability
/// and a 40-tick cooldown.
fn evade() -> Value {
    json!({ "distance": 12_288, "ticks": 8, "invulnerable_ticks": 10, "cooldown_ticks": 40 })
}

/// A combo step whose attack is `attack` (a preset or `{"inline": [...]}`), a leaf.
fn step(attack: Value) -> Value {
    json!({ "attack": attack })
}

/// An inline weapon (ADR-084) with `combo` as its root branches and no window.
pub fn weapon_combo(combo: Value) -> Value {
    json!({ "inline": { "combo": combo, "window_ticks": 0, "assist_range": 0 } })
}

/// An inline weapon with one step: `attack` on a press of `main`.
pub fn weapon_ref(attack: Value) -> Value {
    weapon_combo(json!({ "main": step(attack) }))
}

/// The Main weapon the cases share unless they set their own: a circle of 256 pixels round the
/// main character, active for two ticks and hitting every tick.
fn default_weapon() -> Value {
    weapon_ref(
        json!({ "inline": [{ "kind": "active", "ticks": 2, "damage": 1, "rehit_ticks": 1,
                                    "hit_box": { "type": "circle", "radius": 65_536 } }] }),
    )
}

/// `stage` with `weapon` as its weapon, a whole reference.
pub fn armed(mut stage: Stage, weapon: Value) -> Stage {
    stage.player.weapon = serde_json::from_value(weapon).expect("the weapon parses");
    stage
}

/// `input` with `main` pressed on the tick after a step at `tick` when `tick` is a multiple of
/// 6: the Main weapon fires every 6 ticks from the first.
pub fn firing(input: Input, tick: u32) -> Input {
    Input {
        main: tick % 6 == 0,
        ..input
    }
}

pub const START_X: i32 = 46_080;
pub const START_Y: i32 = 147_456;

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

pub fn always() -> Value {
    json!({ "type": "always" })
}

/// A stage with a 99-hp main character (so it outlives the case) and the given companions.
pub fn stage_full(
    length_ticks: u32,
    boss: Option<Value>,
    waves: Vec<Value>,
    companions: Vec<Value>,
) -> Stage {
    let mut v = json!({
        "schema_version": 1, "sim_version": crate::version::SIM_VERSION, "id": "test", "title": "Test", "seed": 7,
        "length_ticks": length_ticks,
        // The weapon is `default_weapon`; the cases replace it with `armed`.
        "player": { "base": "pilot_a", "hp": 99, "atk": 10,
                    "weapon": default_weapon(),
                    "evade": evade() },
        "companions": companions,
        "waves": waves,
    });
    if let Some(boss) = boss {
        v["boss"] = boss;
    }
    serde_json::from_value(v).expect("the case stage parses")
}

/// The bundled content with an enemy's attack at 100 stored (1 shown), as the cases were
/// measured.
pub fn design_content() -> RunContent {
    RunContent::new(design_run_stats(), RunContent::bundled().rewards().clone())
}

/// The bundled run stats with an enemy's attack at 100 stored.
pub fn design_run_stats() -> RunStats {
    let mut v: Value = serde_json::from_slice(ContentFiles::bundled_file("run_stats"))
        .expect("the bundled run stats");
    v["enemy_attack_damage"] = json!(100);
    RunStats::parse(v.to_string().as_bytes(), Versions::CURRENT).expect("the case run stats")
}

pub fn engine(stage: &Stage) -> Engine {
    Engine::new_with(stage, &design_content()).expect("the case stage is valid")
}

pub fn companion(hp: u32, movement: Value, rules: Value) -> Value {
    json!({ "base": "fairy_a", "hp": hp, "radius": 1024, "movement": movement, "rules": rules })
}

pub fn rule(when: Value, target: Value, action: Value, cooldown_ticks: u32) -> Value {
    json!({ "when": when, "target": target, "do": action, "cooldown_ticks": cooldown_ticks })
}
