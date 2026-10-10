//! The worst-case tick (ADR-020, ADR-021). Shared by `benches/worst_tick.rs`, which counts its
//! instructions, and `tests/worst_case.rs`, which checks that it really reaches every dynamic
//! cap: a benchmark of a tick that does not would pass without measuring the worst case.
#![allow(dead_code)]

#[path = "../../tests/common/mod.rs"]
mod common;

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::stage::Stage;

/// Ticks to run before the tick that is measured, so that the caps are reached and held.
pub const WARMUP_TICKS: u32 = 200;

/// 300 enemies (`MAX_ENEMIES_ALIVE`) stand in one place, three waves of 100 that spawn one enemy
/// a tick each. Every enemy starts the same attack every 30 ticks: 100 rounds of 100 rounds of
/// one bullet. Each start is refused once 256 attacks are going (`MAX_ATTACK_TASKS`), and the
/// oldest attacks use up the 4,096 node steps of the tick (`MAX_EMITTER_STEPS_PER_TICK`) and
/// ask for far more than 200 bullets (`MAX_BULLET_SPAWNS_PER_TICK`).
///
/// The bullets fly straight down at 2,048 (8 pixels a tick) from y = 140,000. The field is
/// 163,840 high and its margin 16,384, so a bullet is gone on its 20th move: 19 batches of 200
/// are in the air when a tick starts, the tick makes the 20th, and 4,000 are alive
/// (`MAX_BULLETS_ALIVE`) until the 200 that are about to leave go. The main character waits at
/// the left wall, where no bullet flies.
pub fn stage() -> Stage {
    let burst = json!({
        "type": "repeat", "count": 100, "interval_ticks": 0, "body": [{
            "type": "repeat", "count": 100, "interval_ticks": 0, "body": [{
                "type": "fire", "speed": 2048,
                "direction": { "type": "absolute", "angle": 0 }
            }]
        }]
    });
    let attack = common::rule(
        common::always(),
        json!("player"),
        json!({ "type": "attack", "attack": { "inline": [burst] } }),
        30,
    );
    let (x, y) = (60_000, 140_000);
    let enemy = common::enemy(100_000, common::hold(x, y), json!([attack]));
    let waves: Vec<Value> = (0..3)
        .map(|_| common::wave(1, 100, 1, enemy.clone(), x, y))
        .collect();
    common::stage_full(18_000, Some(common::far_boss()), waves, vec![], vec![])
}

/// The input of tick `tick` (from 1): the main character goes to the left wall, then waits.
pub fn input(tick: u32) -> Input {
    if tick <= 80 {
        common::moving(-768, 0)
    } else {
        common::idle()
    }
}

/// An engine `WARMUP_TICKS` ticks into the run: the next `step` is the worst-case tick.
pub fn warmed_up() -> Engine {
    let mut engine = common::engine(&stage());
    for tick in 1..=WARMUP_TICKS {
        engine.step(input(tick));
    }
    engine
}
