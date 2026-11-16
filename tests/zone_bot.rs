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
    let me = s.players[0].at;
    let swing = s.entities.iter().any(|v| {
        v.kind == EntityKind::Enemy
            && v.attack.as_ref().is_some_and(|a| {
                a.kind == PhaseKind::WindUp
                    && a.preset.as_ref().is_some_and(|p| p.as_str() == "slash")
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
    let me = s.players[0].at;
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

/// How near `stagger_input` stands to the elite it fights: 40 pixels, inside the `giant_hammer`'s
/// reach (1/256 pixel).
const STAGGER_NEAR: i64 = 40 * 256;
/// `stagger_input` presses `smash` on the first two ticks of every this many.
const STAGGER_EVERY: u32 = 20;

/// `run_input` until an elite that can be staggered is on the field.
pub fn stagger_input(engine: &Engine, tick: u32) -> Input {
    run_input(engine, tick)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::run::time_limit_setup;
    use crate::party_bot::PartyBot;
    use stella_rain_core::fixed::Point;
    use stella_rain_core::run_content::RunContent;
    use stella_rain_core::run_replay::RunRecorder;
    use stella_rain_core::run_setup::RunSetup;

    /// The Breaker case's setup on a world where an elite comes at tick 3,600.
    fn setup() -> RunSetup {
        RunSetup {
            seed: 3,
            difficulty: "easy".into(),
            weapon: "giant_hammer".into(),
            unlocked_styles: Some(vec!["breaker".into()]),
            ..time_limit_setup()
        }
    }

    /// Steps `rec` one tick with `input` and the building bot's action.
    fn step(rec: &mut RunRecorder, bot: &PartyBot, input: impl Fn(&Engine, u32) -> Input) {
        let t = rec.engine().tick();
        let mut step = input(rec.engine(), t);
        step.action = bot.action(rec.engine(), 0, t);
        rec.step(&[step, Input::default(), Input::default()])
            .unwrap();
    }

    /// Where the player and the nearest living elite that can be staggered stand.
    fn elite(engine: &Engine) -> Option<(Point, Point)> {
        let s = engine.snapshot();
        let me = s.players[0].at;
        s.entities
            .iter()
            .filter(|v| {
                v.kind == EntityKind::Enemy && v.elite && v.stagger_max.is_some() && v.hp > 0
            })
            .min_by_key(|v| d2(me, v.at))
            .map(|v| (me, v.at))
    }

    fn d2(a: Point, b: Point) -> i64 {
        let (x, y) = (i64::from(b.x.0 - a.x.0), i64::from(b.y.0 - a.y.0));
        x * x + y * y
    }

    /// A recorder stepped with `run_input` to the first tick an elite that can be staggered is
    /// in the snapshot.
    fn at_the_elite(bot: &PartyBot) -> RunRecorder {
        let mut rec = RunRecorder::start_with(&setup(), RunContent::bundled()).unwrap();
        while elite(rec.engine()).is_none() {
            assert!(
                rec.engine().tick() < 3_700,
                "no elite that can be staggered by tick 3,700"
            );
            step(&mut rec, bot, run_input);
        }
        rec
    }

    #[test]
    fn without_an_elite_it_is_run_input() {
        let bot = PartyBot::building(1);
        let mut rec = RunRecorder::start_with(&setup(), RunContent::bundled()).unwrap();
        for t in 0..3_600 {
            assert!(elite(rec.engine()).is_none(), "an elite at tick {t}");
            assert_eq!(
                stagger_input(rec.engine(), t),
                run_input(rec.engine(), t),
                "tick {t}"
            );
            step(&mut rec, &bot, run_input);
        }
    }

    #[test]
    fn with_an_elite_it_walks_to_it() {
        let bot = PartyBot::building(1);
        let mut rec = at_the_elite(&bot);
        let mut walked = 0;
        for _ in 0..200 {
            let t = rec.engine().tick();
            let Some((me, at)) = elite(rec.engine()) else {
                break;
            };
            let input = stagger_input(rec.engine(), t);
            assert!(!input.main, "main is pressed at tick {t}");
            let sign = |d: i32| match d {
                d if d > 256 => 127,
                d if d < -256 => -127,
                _ => 0,
            };
            if d2(me, at) > STAGGER_NEAR * STAGGER_NEAR {
                assert!(
                    !input.smash,
                    "smash is pressed far from the elite, tick {t}"
                );
                assert_eq!(
                    (input.move_x, input.move_y),
                    (sign(at.x.0 - me.x.0), sign(at.y.0 - me.y.0)),
                    "the walk at tick {t}"
                );
                walked += 1;
            } else {
                assert_eq!((input.move_x, input.move_y), (0, 0), "stands, tick {t}");
            }
            step(&mut rec, &bot, stagger_input);
        }
        assert!(walked > 0, "the elite was never farther than STAGGER_NEAR");
    }

    #[test]
    fn near_the_elite_it_stands_and_smashes() {
        let bot = PartyBot::building(1);
        let mut rec = at_the_elite(&bot);
        // Walks up with no buttons, so the setup does not depend on the function under test.
        let walk = |engine: &Engine, _t: u32| {
            let (me, at) = elite(engine).expect("the elite is on the field");
            let toward = |d: i32| match d {
                d if d > 256 => 127,
                d if d < -256 => -127,
                _ => 0,
            };
            Input {
                move_x: toward(at.x.0 - me.x.0),
                move_y: toward(at.y.0 - me.y.0),
                ..Input::default()
            }
        };
        while elite(rec.engine()).is_some_and(|(me, at)| d2(me, at) > STAGGER_NEAR * STAGGER_NEAR) {
            assert!(
                rec.engine().tick() < 3_900,
                "not near the elite by tick 3,900"
            );
            step(&mut rec, &bot, walk);
        }
        let from = rec.engine().tick();
        for t in from..from + 40 {
            let input = stagger_input(rec.engine(), t);
            assert!(
                (input.move_x, input.move_y) == (0, 0)
                    && !input.main
                    && input.smash == (t % STAGGER_EVERY < 2),
                "tick {t}: move ({}, {}), main {}, smash {}",
                input.move_x,
                input.move_y,
                input.main,
                input.smash
            );
        }
    }
}
