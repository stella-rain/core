//! The run bot of `common::run` with the steps the zone content asks for (BR1a, DS10, DS11): it
//! steps out of the hostile zone it stands in and evades a `slash` about to land within reach.
//! Beside `common`, not in it: `tests/common/` is a bench input, and a change to it together with
//! a change to `src/` cannot be compared by the bench job.

use crate::common::run::run_input_for as base_input_for;
use stella_rain_core::attack::{PhaseKind, ZoneShape};
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::trig::SCALE;

/// The seed of the enemy-role bot run: the first seed (checked 1 to 40) where this bot clears the
/// `time_limit` run on Normal with the zone content of BR1a; seeds 6 and 12 still do
/// after BZ1 (5, 6, 9, 12 and 13 did before it).
pub const ENEMY_ROLE_BOT_SEED: u64 = 6;

/// Ticks before a zone hurts at which the bot starts to step out of it.
const ZONE_SEE: u32 = 30;
/// A step out of a zone is made with this many ticks left or fewer: the evade.
const DODGE: u32 = 3;
/// How far a `slash` reaches from its enemy, with the radii (1/256 pixel): 48 pixels.
const SWING_REACH: i64 = 48 * 256;
/// Pixels past a zone's edge the bot steps to (1/256 pixel).
const ZONE_MARGIN: i64 = 12 * 256;

/// `common::run::run_input` that also gets out of the way of zones and swings.
pub fn run_input(engine: &Engine, tick: u32) -> Input {
    run_input_for(
        engine,
        tick,
        &[EntityKind::Enemy],
        crate::common::run::BRAVE_FROM,
    )
}

/// `common::run::run_input_for` that also gets out of the way of zones and swings until
/// `brave_from`.
pub fn run_input_for(engine: &Engine, tick: u32, kinds: &[EntityKind], brave_from: u32) -> Input {
    let input = base_input_for(engine, tick, kinds, brave_from);
    if tick >= brave_from {
        return input;
    }
    avoid(engine, input)
}

/// `input` with the evade of a swing about to land and the step out of the hostile zone the main
/// character stands in: away from a circle's or an arc's centre, across a line. It overrides the
/// orb and the chase steering.
pub fn avoid(engine: &Engine, input: Input) -> Input {
    avoid_zones(engine, evade_swings(engine, input))
}

/// `input` with the evade of a `slash` about to land within reach.
pub fn evade_swings(engine: &Engine, mut input: Input) -> Input {
    let s = engine.snapshot();
    let me = s.player.at;
    let swing = s.entities.iter().any(|v| {
        v.kind == EntityKind::Enemy
            && v.attack.as_ref().is_some_and(|a| {
                a.kind == PhaseKind::WindUp && a.preset.as_ref().is_some_and(|p| p.0 == "slash")
            })
            && {
                let (dx, dy) = (i64::from(me.x.0 - v.at.x.0), i64::from(me.y.0 - v.at.y.0));
                dx * dx + dy * dy < SWING_REACH * SWING_REACH
            }
    });
    input.evade |= swing;
    input
}

/// `input` with the step out of the hostile zone the main character stands in, if it hurts within
/// `ZONE_SEE` ticks.
pub fn avoid_zones(engine: &Engine, mut input: Input) -> Input {
    let s = engine.snapshot();
    let me = s.player.at;
    let threat = s
        .zones
        .iter()
        .filter(|z| !z.friendly)
        .filter(|z| z.phase == PhaseKind::Active || z.ticks_left <= ZONE_SEE)
        .filter_map(|z| {
            let (dx, dy) = (i64::from(me.x.0 - z.at.x.0), i64::from(me.y.0 - z.at.y.0));
            let away = match z.shape {
                ZoneShape::Circle { radius } | ZoneShape::Arc { radius, .. } => (dx * dx + dy * dy
                    < (i64::from(radius.0) + ZONE_MARGIN).pow(2))
                .then_some((dx, dy))?,
                ZoneShape::Line { length, width } => {
                    let (fx, fy) = (i64::from(z.direction.x), i64::from(z.direction.y));
                    let scale = i64::from(SCALE);
                    let along = (dx * fx + dy * fy) / scale;
                    let across = (dx * -fy + dy * fx) / scale;
                    let inside = (0..=i64::from(length.0)).contains(&along)
                        && across.abs() < i64::from(width.0) / 2 + ZONE_MARGIN;
                    inside.then_some((-fy * across.signum(), fx * across.signum()))?
                }
            };
            Some((z.ticks_left, away))
        })
        .min_by_key(|(k, _)| *k);
    if let Some((k, (ax, ay))) = threat {
        let toward = |d: i64| match d {
            d if d > 0 => 127,
            d if d < 0 => -127,
            _ => 0,
        };
        // Standing on the centre, any way out will do.
        let (ax, ay) = if ax == 0 && ay == 0 { (1, 1) } else { (ax, ay) };
        input.move_x = toward(ax);
        input.move_y = toward(ay);
        input.evade |= k <= DODGE;
    }
    input
}
