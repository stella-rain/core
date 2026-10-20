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

/// Ticks before the measured tick: the oldest friendly bullets have flown about 260 ticks, so
/// their number has settled just under `MAX_BULLETS_ALIVE`.
pub const WARMUP_TICKS: u32 = 400;

/// The enemies hold `HOLDS` points down a vertical line 120 pixels from the left edge, from 40 to
/// 600 pixels from the top.
pub const BAND_X: i32 = 30_720;
pub const BAND_TOP: i32 = 10_240;
pub const BAND_BOTTOM: i32 = 153_600;
pub const HOLDS: i32 = 20;

/// Where the main character starts: 238 pixels from the left and 620 from the top, so the fan
/// flies up about 15 pixels clear of the band's right edge.
pub const START: (i32, i32) = (60_928, 158_720);

/// The fan's whole angle: 2 degrees, in 1/256 degree.
pub const FAN_ANGLE: i32 = 512;

/// The crowd at rest (ADR-058): 500 enemies (`MAX_ENEMIES_ALIVE`) in 20 waves of 25 that spawn
/// one a tick, each wave holding its own point of the line. By the measured tick they have pushed
/// apart into a band about 180 pixels wide down the whole height of the map.
///
/// The map is 1,024 by 640 pixels. The main character fires 90 bullets every 6 ticks at 2 pixels
/// a tick, fanned over 2 degrees round straight up, so they fly up beside the band for their
/// whole flight. All but those above the map are within one cell of the enemy grid, so
/// `Grid::first_hit` reads the cells round each, which hold enemies near the band's edge
/// (core#100), and none hits: about 3,900 are alive (`MAX_BULLETS_ALIVE` is 4,000).
///
/// Hit boxes (ADR-055): four companions (`MAX_COMPANIONS`) stand 10 pixels left of the main
/// character, level with it and so facing right, away from the band. Each is in an active phase
/// with a 256-pixel arc 10 pixels ahead of it. The enemies near them come back from the grid's
/// range query, and each is tested against the arc's angle and both its edges, and missed: all
/// of them are behind the arc.
pub fn stage() -> Stage {
    let per_hold = 500 / HOLDS;
    let waves: Vec<Value> = (0..HOLDS)
        .map(|i| {
            let y = BAND_TOP + (BAND_BOTTOM - BAND_TOP) * i / (HOLDS - 1);
            let enemy = common::enemy(100_000, common::hold(BAND_X, y), json!([]));
            common::wave(1, per_hold as u16, 1, enemy, BAND_X, y)
        })
        .collect();
    built(waves, 262_144, 163_840)
}

/// The stage with `waves` on a map of `width` by `height` (1/256 pixels), the main character at
/// `START` and the crowd's guards and fan as `stage` describes them.
fn built(waves: Vec<Value>, width: i32, height: i32) -> Stage {
    let sweep = json!([{ "kind": "active", "ticks": 30, "damage": 1, "rehit_ticks": 1,
                         "hit_box": { "type": "arc", "offset": { "forward": 2_560 },
                                      "radius": 65_536, "half_angle": 60 * 256 } }]);
    let start = Point {
        x: Fx(START.0),
        y: Fx(START.1),
    };
    let guard = common::companion(
        50,
        common::hold(START.0 - 2_560, start.y.0),
        json!([common::rule(
            common::always(),
            json!("player"),
            json!({ "type": "attack", "attack": { "inline": sweep } }),
            0
        )]),
    );
    let mut stage = common::stage_full(18_000, Some(common::far_boss()), waves, vec![guard; 4]);
    stage.map = Some(Map {
        width: Fx(width),
        height: Fx(height),
        start: Some(start),
    });
    let fan = json!({ "inline": common::spawned(json!([{
        "type": "spread", "count": 90, "angle": FAN_ANGLE, "body": [{
            "type": "fire", "speed": 512, "direction": { "type": "relative", "angle": 0 }
        }]
    }]))});
    stage.player.main = serde_json::from_value(common::main_ref(fan)).expect("the fan parses");
    stage
}

/// The largest map: 4,096 pixels a side (`MAX_MAP_SIDE`, core#87).
pub const SPREAD_SIDE: i32 = 1_048_576;

/// The spread case (core#146): the same 500 enemies, in five waves of 100 (`MAX_WAVE_COUNT`, the
/// most a wave may hold) that hold three points of the largest map: one wave at the start point,
/// two at the top right and two at the bottom right. The start wave stands 80 pixels left of the
/// start point, out of the fan's column (the fan flies up from the start point). The corners are
/// 128 pixels in from the right edge, about 3,700 and 5,000 pixels from the start.
///
/// Each enemy holds its point, as the band's do, so the crowd pushes apart round its wave's
/// point. Unlike the band, the crowds are far apart: the enemy grid spans the map (up to 128 by
/// 128 cells of 32 pixels), and its build reads the whole map.
pub fn spread_stage() -> Stage {
    let (start_x, start_y) = (START.0 - 20_480, START.1);
    let top_right = (SPREAD_SIDE - 32_768, 32_768);
    let bottom_right = (SPREAD_SIDE - 32_768, SPREAD_SIDE - 32_768);
    let far = |(x, y): (i32, i32)| {
        let enemy = common::enemy(100_000, common::hold(x, y), json!([]));
        common::wave(1, 100, 1, enemy, x, y)
    };
    let waves = vec![
        far((start_x, start_y)),
        far(top_right),
        far(top_right),
        far(bottom_right),
        far(bottom_right),
    ];
    built(waves, SPREAD_SIDE, SPREAD_SIDE)
}

/// An engine `WARMUP_TICKS` ticks into the spread case: the next `step` is the measured tick.
pub fn spread_warmed_up() -> Engine {
    let mut engine = common::engine(&spread_stage());
    for tick in 1..=WARMUP_TICKS {
        engine.step(input(tick));
    }
    engine
}

/// The input of tick `tick` (from 1): the main character stays where it starts, firing its
/// shot every 6 ticks from the first.
pub fn input(tick: u32) -> Input {
    common::firing(common::idle(), tick - 1)
}

/// An engine `WARMUP_TICKS` ticks into the run: the next `step` is the measured tick.
pub fn warmed_up() -> Engine {
    let mut engine = common::engine(&stage());
    for tick in 1..=WARMUP_TICKS {
        engine.step(input(tick));
    }
    engine
}
