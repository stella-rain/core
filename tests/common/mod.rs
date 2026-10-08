//! Shared by the replay, share-code, recording and `stage-verify` tests.
#![allow(dead_code)]

use serde_json::{Value, json};
use stella_rain_core::input::Input;

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
