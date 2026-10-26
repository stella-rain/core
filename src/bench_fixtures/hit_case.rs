//! The worst case of melee hit boxes and zones against the crowd within the caps of ADR-020
//! (core BR1b). Shared by `benches/hit_tick.rs`, which counts its instructions, and
//! `tests/hit_case.rs`, which checks that the measured tick has the load it claims. It takes the
//! place of the main character's bullet collision case (BR1b): no orb is alive.

use super::common;
use crate::engine::Engine;
use crate::fixed::{Fx, Point};
use crate::input::Input;
use crate::stage::{Map, Stage};
use serde_json::{Value, json};

/// Ticks before the measured tick: the crowd has arrived (the last enemy on tick 100) and been
/// pushed apart, and the zones are placed (on tick 1 and 2, active for 600 ticks).
pub const WARMUP_TICKS: u32 = 400;

/// Where the main character starts: 238 pixels from the left and 620 from the top.
pub const START: (i32, i32) = (60_928, 158_720);

/// Where the crowd holds: 36,000 units (about 140 pixels) above the main character, inside its
/// step of 256 pixels with the whole clump after the push-apart.
pub const CROWD_AT: (i32, i32) = (START.0, START.1 - 36_000);

/// The radius of a friendly zone (256 pixels): it covers the clump, and each zone tests all 500
/// enemies every tick.
pub const ZONE_RADIUS: i32 = 65_536;

/// The companions' zones: 28 each, 112 in all. With the boss's 16 they fill `MAX_ZONES_ALIVE`.
/// An enemy zone hurts only the main character and the companions, so these friendly ones are
/// what loads the crowd; the boss's 16 are out of everyone's reach (placeholder, BR1b).
const ZONES_PER_COMPANION: u32 = 28;
const BOSS_ZONES: u32 = 16;

/// The hit boxes (ADR-055): the main character's Main weapon is a circle of 256 pixels round it,
/// active for two ticks and hitting every tick, tapped every other tick, so it lands on every
/// enemy within its reach, every tick. Four companions (`MAX_COMPANIONS`) stand 78 pixels right
/// of the crowd's point and cleave it with an arc of 256 pixels (`rehit_ticks` 1) and, on their
/// first tick, place their zones. Nobody dies: each enemy has 100,000 hit points.
fn built(waves: Vec<Value>, width: i32, height: i32) -> Stage {
    let zone = |radius: i32| {
        json!({ "type": "zone", "shape": { "type": "circle", "radius": radius },
                "wind_up_ticks": 0, "active_ticks": 600 })
    };
    let cleave = json!([{ "kind": "active", "ticks": 30, "damage": 1, "rehit_ticks": 1,
                          "hit_box": { "type": "arc", "offset": { "forward": 10_000 },
                                       "radius": 65_536, "half_angle": 90 * 256 },
                          "spawn": [{ "type": "ring", "count": ZONES_PER_COMPANION,
                                      "body": [zone(ZONE_RADIUS)] }] }]);
    let corner = json!([{ "kind": "active", "ticks": 1, "spawn": [{
        "type": "ring", "count": BOSS_ZONES, "body": [zone(2_048)]
    }] }]);
    let mut boss = common::far_boss();
    boss["phases"] = json!([{ "timeline": [
        { "type": "attack", "attack": { "inline": corner } },
    ] }]);
    let (x, y) = CROWD_AT;
    let guard = common::companion(
        50,
        common::hold(x + 20_000, y),
        json!([common::rule(
            common::always(),
            json!("nearest_enemy"),
            json!({ "type": "attack", "attack": { "inline": cleave } }),
            0
        )]),
    );
    let step = json!([{ "kind": "active", "ticks": 2, "damage": 1, "rehit_ticks": 1,
                        "hit_box": { "type": "circle", "radius": 65_536 } }]);
    let mut stage = common::armed(
        common::stage_full(18_000, Some(boss), waves, vec![guard; 4]),
        common::weapon_ref(json!({ "inline": step })),
    );
    stage.map = Some(Map {
        width: Fx(width),
        height: Fx(height),
        start: Some(Point {
            x: Fx(START.0),
            y: Fx(START.1),
        }),
        facing: None,
    });
    stage
}

fn crowd(x: i32, y: i32, count: u16) -> Value {
    common::wave(
        1,
        count,
        1,
        common::enemy(100_000, common::hold(x, y), json!([])),
        x,
        y,
    )
}

/// The crowd in a clump: 500 enemies (`MAX_ENEMIES_ALIVE`) in five waves of 100 that spawn one a
/// tick each at one point, and push apart (ADR-058) into a clump round it.
pub fn stage() -> Stage {
    let (x, y) = CROWD_AT;
    built((0..5).map(|_| crowd(x, y, 100)).collect(), 262_144, 163_840)
}

/// The largest map: 4,096 pixels a side (`MAX_MAP_SIDE`, core#87).
pub const SPREAD_SIDE: i32 = 1_048_576;

/// How far from `START`, on each axis, the spread case's outer crowds hold: 440 pixels, so a
/// crowd's point is about 622 pixels from the main character and the clump round it, once pushed
/// apart, stays inside the leash's reach (`LEASH_DISTANCE`, 800 pixels, EC2, DS84).
const SPREAD_OFFSET: i32 = 112_640;

/// The spread case (core#146, EC2): the same 500 enemies, in five waves of 100 on the largest
/// map: one at the clump's point, within reach of the main character's step and the companions,
/// two above and to the right of the start and two below and to the right, each inside the
/// leash's reach. The enemy grid still spans the map (up to 128 by 128 cells of 32 pixels) and its
/// build reads all of it, while the hit boxes and the zones reach the first wave only.
pub fn spread_stage() -> Stage {
    let top_right = (START.0 + SPREAD_OFFSET, START.1 - SPREAD_OFFSET);
    let bottom_right = (START.0 + SPREAD_OFFSET, START.1 + SPREAD_OFFSET);
    let waves = vec![
        crowd(CROWD_AT.0, CROWD_AT.1, 100),
        crowd(top_right.0, top_right.1, 100),
        crowd(top_right.0, top_right.1, 100),
        crowd(bottom_right.0, bottom_right.1, 100),
        crowd(bottom_right.0, bottom_right.1, 100),
    ];
    built(waves, SPREAD_SIDE, SPREAD_SIDE)
}

/// The input of tick `tick` (from 1): the main character stays where it starts and taps its
/// Main weapon every other tick.
pub fn input(tick: u32) -> Input {
    Input {
        main: tick % 2 == 0,
        ..common::idle()
    }
}

fn warmed(stage: &Stage) -> Engine {
    let mut engine = common::engine(stage);
    for tick in 1..=WARMUP_TICKS {
        engine.step(input(tick));
    }
    engine
}

/// An engine `WARMUP_TICKS` ticks into the clustered case: the next `step` is the measured tick.
pub fn warmed_up() -> Engine {
    warmed(&stage())
}

/// An engine `WARMUP_TICKS` ticks into the spread case.
pub fn spread_warmed_up() -> Engine {
    warmed(&spread_stage())
}
