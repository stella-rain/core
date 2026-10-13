//! The worst case of friendly-bullet collision within the caps of ADR-020 (core#62). Shared by
//! `benches/collide_tick.rs`, which counts its instructions, and `tests/collide_case.rs`, which
//! checks that the measured tick has the load it claims.
#![allow(dead_code)]

#[path = "../../tests/common/mod.rs"]
mod common;

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::stage::Stage;

/// Ticks before the measured tick: the oldest friendly bullets have flown about 280 ticks, so
/// their number has settled just under `MAX_BULLETS_ALIVE`.
pub const WARMUP_TICKS: u32 = 400;

/// 300 enemies (`MAX_ENEMIES_ALIVE`) stand in one point at the bottom-left of the field, three
/// waves of 100 that spawn one a tick each. The main character, at its start (bottom centre),
/// fires 90 bullets every 6 ticks at 2 pixels a tick, fanned over 60 degrees around straight up:
/// none of them hits an enemy, so each is checked against the enemies on every tick it flies,
/// and about 3,900 are alive (`MAX_BULLETS_ALIVE` is 4,000).
///
/// Hit boxes (ADR-055): four companions (`MAX_COMPANIONS`) stand to the right of the enemies,
/// facing them, each in an active phase with a 256-pixel arc pushed 156 pixels past the
/// enemies. All 300 come back from the grid's range query, and each is tested against the
/// arc's angle and both its edges, and missed: the arc faces away from them.
pub fn stage() -> Stage {
    let enemy = common::enemy(100_000, common::hold(2_048, 160_000), json!([]));
    let waves: Vec<Value> = (0..3)
        .map(|_| common::wave(1, 100, 1, enemy.clone(), 2_048, 160_000))
        .collect();
    let sweep = json!([{ "kind": "active", "ticks": 30, "damage": 1, "rehit_ticks": 1,
                         "hit_box": { "type": "arc", "offset": { "forward": 40_000 },
                                      "radius": 65_536, "half_angle": 60 * 256 } }]);
    let guard = common::companion(
        50,
        common::hold(30_000, 160_000),
        json!([common::rule(
            common::always(),
            json!("nearest_enemy"),
            json!({ "type": "attack", "attack": { "inline": sweep } }),
            0
        )]),
    );
    let mut stage = common::stage_full(18_000, Some(common::far_boss()), waves, vec![guard; 4]);
    let fan = json!({ "inline": common::spawned(json!([{
        "type": "spread", "count": 90, "angle": 15_360, "body": [{
            "type": "fire", "speed": 512, "direction": { "type": "relative", "angle": 0 }
        }]
    }]))});
    stage.player.main =
        serde_json::from_value(common::main_ref(common::weapon(fan, json!("facing"), 6)))
            .expect("the fan parses");
    stage
}

/// The main character stays where it starts, its shot attacking on its own (ADR-050).
pub fn input(_tick: u32) -> Input {
    common::idle()
}

/// An engine `WARMUP_TICKS` ticks into the run: the next `step` is the measured tick.
pub fn warmed_up() -> Engine {
    let mut engine = common::engine(&stage());
    for tick in 1..=WARMUP_TICKS {
        engine.step(input(tick));
    }
    engine
}
