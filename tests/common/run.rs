//! A run of the trial mode (ADR-079), shared by the run replay, recording and corpus tests.
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::fixed::{Fx, Point};
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::Command;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::snapshot::{EntityKind, OfferOption};

/// The trial mode's length.
pub const TRIAL_LENGTH: u32 = 5400;

/// The run stat `play_run` spends every point on.
pub const SPEND_INDEX: u32 = 1;

/// The seed of the `stage_skill` run case: with it, the run kills an elite whose rare star
/// offers the black hole, and takes it.
pub const STAGE_SKILL_SEED: u64 = 74;

pub fn trial_setup() -> RunSetup {
    RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "trial".into(),
        main: "blade".into(),
        sub: "maul".into(),
        seed: 20_261_010,
        star_pool: None,
        extra_rerolls: 0,
    }
}

/// Side to side, and the `blade`'s combos in a 60-tick cycle: `main` on its ticks 0, 8 and 16
/// (slash, slash, thrust), the thrust held to tick 40 in every other cycle (a charge released at
/// level 1, inside its perfect window), and `sub` on 44 and 52 (a thrust and a slash, with the
/// `maul`'s pull); the stage skill pressed on tick 30 of every cycle.
pub fn run_script(tick: u32) -> Input {
    let c = tick % 60;
    let charging = (tick / 60) % 2 == 1 && (16..40).contains(&c);
    Input {
        move_x: if (tick / 40) % 2 == 0 { 127 } else { -127 },
        move_y: 0,
        main: matches!(c, 0 | 8 | 16) || charging,
        sub: c == 44 || c == 52,
        skill: c == 30,
        ..Input::default()
    }
}

fn points(engine: &Engine) -> u32 {
    engine.snapshot().progress.map_or(0, |p| p.points)
}

/// The tick from which `run_input` walks to the falling stars and goes for the weaker elite.
pub const STEER_FROM: u32 = 800;

/// The tick from which `run_input` stops getting out of the way of bullets, so that the run
/// ends before the trial does.
pub const BRAVE_FROM: u32 = 2400;

/// How near `run_input` walks to the enemy it fights: 32 pixels, inside the `blade`'s reach.
const NEAR: i64 = 8192;

/// `run_input` steps out of the way of a hostile bullet that would come within `HIT` in the
/// next `SEE` ticks (`LATE` while it goes for an elite), and evades it when it is `DODGE`
/// ticks away.
const SEE: i32 = 30;
const LATE: i32 = 12;
const DODGE: i32 = 3;
const HIT: i64 = 3072;

/// `run_script`'s buttons, walking to within `NEAR` of an enemy (so that the facing, the last
/// movement, points at it) and standing there: the nearest one that is not an elite, and from
/// `STEER_FROM` the elite with the fewest hit points first; from `STEER_FROM` a falling star
/// comes before any enemy. Until `BRAVE_FROM` it gets out of the way of the hostile bullets
/// about to hit: a step across their path, which overrides the rest (only `LATE` while it goes
/// for an elite), and an evade at the last moment.
pub fn run_input(engine: &Engine, tick: u32) -> Input {
    let mut input = run_script(tick);
    let s = engine.snapshot();
    let me = s.player.at;
    let d2 = |at: Point| {
        let (x, y) = (i64::from(at.x.0 - me.x.0), i64::from(at.y.0 - me.y.0));
        x * x + y * y
    };
    let toward = |d: i32| match d {
        d if d > 256 => 127,
        d if d < -256 => -127,
        _ => 0,
    };
    let hunting = tick >= STEER_FROM;
    let star = s
        .stars
        .iter()
        .map(|st| st.at)
        .filter(|_| hunting)
        .min_by_key(|at| d2(*at));
    let enemy = s
        .entities
        .iter()
        .filter(|v| v.kind == EntityKind::Enemy)
        .min_by_key(|v| {
            if hunting {
                (!v.elite, if v.elite { i64::from(v.hp) } else { d2(v.at) })
            } else {
                (v.elite, d2(v.at))
            }
        });
    let at_elite = enemy.is_some_and(|v| v.elite) && star.is_none();
    let enemy = enemy.map(|v| v.at);
    if let Some(at) = star {
        input.move_x = toward(at.x.0 - me.x.0);
        input.move_y = toward(at.y.0 - me.y.0);
    } else if let Some(at) = enemy {
        let walk = i8::from(d2(at) > NEAR * NEAR);
        input.move_x = walk * toward(at.x.0 - me.x.0);
        input.move_y = walk * toward(at.y.0 - me.y.0);
    }
    if tick >= BRAVE_FROM {
        return input;
    }
    // The first hostile bullet that would pass within `HIT` in the next `SEE` ticks.
    let threat = s
        .bullets
        .iter()
        .filter(|b| !b.friendly)
        .filter_map(|b| {
            let k = (1..=SEE).find(|&k| {
                d2(Point {
                    x: Fx(b.at.x.0 + b.vx.0 * k),
                    y: Fx(b.at.y.0 + b.vy.0 * k),
                }) < HIT * HIT
            })?;
            Some((k, b))
        })
        .min_by_key(|(k, _)| *k);
    if let Some((k, b)) = threat {
        // Across its path, to the side the main character is already on.
        let (vx, vy) = (i64::from(b.vx.0), i64::from(b.vy.0));
        let (rx, ry) = (i64::from(me.x.0 - b.at.x.0), i64::from(me.y.0 - b.at.y.0));
        let side = if rx * vy - ry * vx >= 0 { 1 } else { -1 };
        let across = |d: i64| match d.signum() * side {
            1 => 127,
            -1 => -127,
            _ => 0,
        };
        if !at_elite || k <= LATE {
            input.move_x = across(vy);
            input.move_y = across(-vx);
        }
        input.evade |= k <= DODGE;
    }
    input
}

/// Answers the open star or boss offer, the `k`-th of the run, from 0, so that a run uses every
/// kind of option: the first with a reroll of candidate 0 and a banish of candidate 1, when
/// they are offered, then a take of candidate 0; the second with a skip; every later one with a
/// take of candidate 0. It stops when an expertise offer opens after it.
fn answer(engine: &mut Engine, k: usize, out: &mut Vec<Command>) {
    let mut step = 0;
    while let Some(offer) = engine.snapshot().progress.and_then(|p| p.offer) {
        let options = offer.options;
        let find = |o: OfferOption| options.iter().position(|x| *x == o);
        let pick = match (k, step) {
            (0, 0) => find(OfferOption::Reroll { candidate: 0 }),
            (0, 1) => find(OfferOption::Banish { candidate: 1 }),
            (1, _) => find(OfferOption::Skip),
            _ => None,
        };
        step += 1;
        let Some(pick) = pick.or(if step > 2 || k > 0 { Some(0) } else { None }) else {
            continue;
        };
        let pick = u32::try_from(pick).unwrap();
        engine.choose(pick).unwrap();
        out.push(Command::Choose(pick));
    }
}

/// Answers the open offers: an expertise offer with a take of candidate 0, a star or boss offer
/// as `answer` does, counting those in `offers`.
fn answer_all(engine: &mut Engine, offers: &mut usize, out: &mut Vec<Command>) {
    while engine.offer_open() {
        if engine.snapshot().expertise.and_then(|g| g.offer).is_some() {
            engine.choose(0).unwrap();
            out.push(Command::Choose(0));
        } else {
            answer(engine, *offers, out);
            *offers += 1;
        }
    }
}

/// The commands of a run of `run_input` for up to `ticks` ticks, spending every point on
/// `SPEND_INDEX` as it comes, between that tick and the next (and after the last), until the
/// stat is at its limit, and answering every offer as `answer_all` does.
pub fn play_run(ticks: u32) -> Vec<Command> {
    play_run_from(&trial_setup(), ticks)
}

/// `play_run` from `setup` instead of the trial's.
pub fn play_run_from(setup: &RunSetup, ticks: u32) -> Vec<Command> {
    let mut engine = Engine::start(setup).unwrap();
    let mut out = Vec::new();
    let mut offers = 0;
    let between = |engine: &mut Engine, out: &mut Vec<Command>, offers: &mut usize| {
        answer_all(engine, offers, out);
        while points(engine) > 0 && engine.spend(SPEND_INDEX).is_ok() {
            out.push(Command::Spend(SPEND_INDEX));
        }
    };
    for t in 0..ticks {
        if engine.outcome() != Outcome::Running {
            break;
        }
        between(&mut engine, &mut out, &mut offers);
        let input = run_input(&engine, t);
        engine.step(input);
        out.push(Command::Step(input));
    }
    between(&mut engine, &mut out, &mut offers);
    out
}

/// The tick at whose end the run's first point comes: the one `play_run` spends it after.
/// Offers may open before it.
pub fn first_point_tick() -> u32 {
    let commands = play_run(STEER_FROM);
    let at = commands
        .iter()
        .position(|c| matches!(c, Command::Spend(_)))
        .unwrap_or_else(|| panic!("the script gains no point before tick {STEER_FROM}"));
    let steps = commands[..at]
        .iter()
        .filter(|c| matches!(c, Command::Step(_)))
        .count();
    u32::try_from(steps).unwrap()
}
