//! The worst case of friendly-bullet collision within the caps of ADR-020 (core#62). Shared by
//! `benches/collide_tick.rs`, which counts its instructions, and `tests/collide_case.rs`, which
//! checks that the measured tick has the load it claims.
#![allow(dead_code)]

#[path = "../../tests/common/mod.rs"]
mod common;

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::fixed::{Fx, Point};
use stella_rain_core::input::Input;
use stella_rain_core::stage::{Map, Stage};

/// Ticks before the measured tick: the oldest friendly bullets have flown about 280 ticks, so
/// their number has settled just under `MAX_BULLETS_ALIVE`.
pub const WARMUP_TICKS: u32 = 400;

/// Where the enemies hold.
pub const ENEMIES_AT: (i32, i32) = (38_400, 102_400);

/// The crowd at rest (ADR-058): 500 enemies (`MAX_ENEMIES_ALIVE`) hold one point, five waves of
/// 100 that spawn on it one a tick each, and by the measured tick they have pushed apart into a
/// crowd about 420 pixels across round it, against the left edge of the map.
///
/// The map is 1,024 by 640 pixels, so the crowd and the bullets have room beside each other. The
/// main character starts 700 pixels from the left and 438 from the top, and fires 90 bullets
/// every 6 ticks at 2 pixels a tick, fanned over 60 degrees around straight up. The fan's left
/// edge passes more than 100 pixels to the right of the crowd, so no bullet hits an enemy, and
/// about 3,900 are alive (`MAX_BULLETS_ALIVE` is 4,000).
///
/// Hit boxes (ADR-055): four companions (`MAX_COMPANIONS`) stand at the crowd's right edge, level
/// with the main character and so facing right, away from the crowd. Each is in an active phase
/// with a 256-pixel arc 10 pixels ahead of it. The enemies of the crowd's right half come back
/// from the grid's range query, and each is tested against the arc's angle and both its edges,
/// and missed: all of them are behind the arc.
pub fn stage() -> Stage {
    let (x, y) = ENEMIES_AT;
    let enemy = common::enemy(100_000, common::hold(x, y), json!([]));
    let waves: Vec<Value> = (0..5)
        .map(|_| common::wave(1, 100, 1, enemy.clone(), x, y))
        .collect();
    let sweep = json!([{ "kind": "active", "ticks": 30, "damage": 1, "rehit_ticks": 1,
                         "hit_box": { "type": "arc", "offset": { "forward": 2_560 },
                                      "radius": 65_536, "half_angle": 60 * 256 } }]);
    let start = Point {
        x: Fx(179_200),
        y: Fx(112_128),
    };
    let guard = common::companion(
        50,
        common::hold(102_400, start.y.0),
        json!([common::rule(
            common::always(),
            json!("player"),
            json!({ "type": "attack", "attack": { "inline": sweep } }),
            0
        )]),
    );
    let mut stage = common::stage_full(18_000, Some(common::far_boss()), waves, vec![guard; 4]);
    stage.map = Some(Map {
        width: Fx(262_144),
        height: Fx(163_840),
        start: Some(start),
    });
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
