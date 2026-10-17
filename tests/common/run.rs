//! A run of the trial mode (ADR-066), shared by the run replay, recording and corpus tests.

use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::Command;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::snapshot::OfferOption;

/// The trial mode's length.
pub const TRIAL_LENGTH: u32 = 5400;

/// The run stat `play_run` spends every point on.
pub const SPEND_INDEX: u32 = 1;

pub fn trial_setup() -> RunSetup {
    RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "trial".into(),
        main: "needle".into(),
        sub: "blaster".into(),
        main_expertise: 3,
        sub_expertise: 0,
        seed: 20_261_010,
        star_pool: None,
        extra_rerolls: 0,
    }
}

/// Side to side, evading, charging and pressing the Sub skill: with `trial_setup` the first
/// level-up comes near tick 1,000 and the main character falls a few hundred ticks later.
pub fn run_script(tick: u32) -> Input {
    Input {
        move_x: if (tick / 40) % 2 == 0 { 127 } else { -127 },
        move_y: 0,
        evade: tick % 50 == 25,
        charge: (tick / 45) % 3 == 1,
        sub: tick % 9 == 0,
    }
}

fn points(engine: &Engine) -> u32 {
    engine.snapshot().progress.map_or(0, |p| p.points)
}

/// The tick from which `run_input` walks to the stars: after the first point, so the run up to
/// it is `run_script`'s alone.
pub const STEER_FROM: u32 = 1000;

/// `run_script`, and from `STEER_FROM` on a walk to the nearest falling star while there is one.
pub fn run_input(engine: &Engine, tick: u32) -> Input {
    let mut input = run_script(tick);
    if tick < STEER_FROM {
        return input;
    }
    let s = engine.snapshot();
    let me = s.player.at;
    let d2 = |x: i32, y: i32| i64::from(x) * i64::from(x) + i64::from(y) * i64::from(y);
    let toward = |d: i32| match d {
        d if d > 256 => 127,
        d if d < -256 => -127,
        _ => 0,
    };
    if let Some(star) = s
        .stars
        .iter()
        .min_by_key(|st| d2(st.at.x.0 - me.x.0, st.at.y.0 - me.y.0))
    {
        input.move_x = toward(star.at.x.0 - me.x.0);
        input.move_y = toward(star.at.y.0 - me.y.0);
    }
    input
}

/// Answers the open offer, the `k`-th of the run, from 0, so that a run uses every kind of
/// option: the first with a reroll of candidate 0 and a banish of candidate 1, when they are
/// offered, then a take of candidate 0; the second with a skip; every later one with a take of
/// candidate 0.
fn answer(engine: &mut Engine, k: usize, out: &mut Vec<Command>) {
    let mut step = 0;
    while engine.offer_open() {
        let options = engine.snapshot().progress.unwrap().offer.unwrap().options;
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

/// The commands of a run of `run_input` for up to `ticks` ticks, spending every point on
/// `SPEND_INDEX` as it comes, between that tick and the next (and after the last), until the
/// stat is at its limit, and answering every offer as `answer` does.
pub fn play_run(ticks: u32) -> Vec<Command> {
    let mut engine = Engine::start(&trial_setup()).unwrap();
    let mut out = Vec::new();
    let mut offers = 0;
    let between = |engine: &mut Engine, out: &mut Vec<Command>, offers: &mut usize| {
        if engine.offer_open() {
            answer(engine, *offers, out);
            *offers += 1;
        }
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

/// The tick at whose end the run's first point comes.
pub fn first_point_tick() -> u32 {
    let mut engine = Engine::start(&trial_setup()).unwrap();
    for t in 0..STEER_FROM {
        engine.step(run_script(t));
        assert!(!engine.offer_open(), "an offer before the first point");
        if points(&engine) > 0 {
            return engine.tick();
        }
    }
    panic!("the script gains no point before tick {STEER_FROM}");
}
