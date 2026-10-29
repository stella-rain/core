//! A run of the trial mode (ADR-079), shared by the run replay, recording and corpus tests.
use crate::common;
use stella_rain_core::engine::Engine;
use stella_rain_core::fixed::{Fx, Point};
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::Command;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::snapshot::{EntityKind, OfferOption};
use stella_rain_core::version::Versions;

/// The trial mode's length.
pub const TRIAL_LENGTH: u32 = 5400;

/// The seed of the `stage_skill` run case: a seed where the run kills an elite whose rare star
/// offers the black hole, takes it, and uses it twice and nothing else. The first one from 11 up
/// under the ranged tokens (RA1); 11 took the hex as well once the tokens moved the run, and 15
/// the aegis under the rectangle leash (LS1), where 11 takes the black hole again.
pub const STAGE_SKILL_SEED: u64 = 11;

/// The seed of the `time_limit_kill` run case: with it the hand-played fight against the final
/// boss ends in its kill, as the case's name says. Under the Vindictus shape (ADR-084) seed
/// 20261010 ends with the main character falling. Of seeds 1 to 10, seeds 1, 2, 4, 5 and 8 end in
/// a kill; this one is the lowest.
pub const KILL_SEED: u64 = 1;

/// The seed of the `elites` run case: the first seed under R1's streams where the scripted run
/// still leaves a shrine and a mimic on the map when the boss calls (checked 1 to 40).
pub const ELITES_SEED: u64 = 5;

/// The seed of the runes reroll test: the first seed under R1's streams where the first offer
/// after the reroll holds the three other rewards.
pub const RUNES_REROLL_SEED: u32 = 9;

/// The seed of the constellations trio test: the first seed under R1's streams where the first
/// offer holds Alpha (n1).
pub const CONSTELLATIONS_TRIO_SEED: u32 = 1;

/// The seed of the enemy-role bot run: the first seed under R1's streams where the run on Normal
/// ends in the boss's kill.
pub const ENEMY_ROLE_BOT_SEED: u64 = 3;

pub fn trial_setup() -> RunSetup {
    RunSetup {
        seed: 20_261_010,
        ..RunSetup::default()
    }
}

/// A run of the `time_limit` mode with the trial's weapons and seed.
pub fn time_limit_setup() -> RunSetup {
    RunSetup {
        mode: "time_limit".into(),
        ..trial_setup()
    }
}

/// Side to side, and the `giant_sword`'s combos in a 60-tick cycle: `main` on its ticks 0, 8 and 16
/// (slash, slash, thrust), the thrust held to tick 40 in every other cycle (a charge released at
/// level 1, inside its perfect window), and `smash` on 44 and 52 (two smashes from the root, each a
/// slash that bleeds what it hits); the stage skill pressed on tick 30 of every cycle.
pub fn run_script(tick: u32) -> Input {
    let c = tick % 60;
    let charging = (tick / 60) % 2 == 1 && (16..40).contains(&c);
    Input {
        move_x: if (tick / 40) % 2 == 0 { 127 } else { -127 },
        move_y: 0,
        main: matches!(c, 0 | 8 | 16) || charging,
        smash: c == 44 || c == 52,
        skill: c == 30,
        ..Input::default()
    }
}

/// The tick from which `run_input` walks to the falling stars and goes for the weaker elite.
pub const STEER_FROM: u32 = 800;

/// The tick from which `run_input` stops getting out of the way of orbs, so that the run
/// ends before the trial does.
pub const BRAVE_FROM: u32 = 2400;

/// How near `run_input` walks to the enemy it fights: 32 pixels, inside the `giant_sword`'s reach.
const NEAR: i64 = 8192;

/// `run_input` steps out of the way of a hostile orb that would come within `HIT` in the
/// next `SEE` ticks (`LATE` while it goes for an elite), and evades it when it is `DODGE`
/// ticks away.
const SEE: i32 = 30;
const LATE: i32 = 12;
const DODGE: i32 = 3;
const HIT: i64 = 3072;

/// `run_script`'s buttons, walking to within `NEAR` of an enemy (so that the facing, the last
/// movement, points at it) and standing there: the nearest one that is not an elite, and from
/// `STEER_FROM` the elite with the fewest hit points first; from `STEER_FROM` a falling star
/// comes before any enemy. Until `BRAVE_FROM` it gets out of the way of the hostile orbs
/// about to hit: a step across their path, which overrides the rest (only `LATE` while it goes
/// for an elite), and an evade at the last moment.
pub fn run_input(engine: &Engine, tick: u32) -> Input {
    run_input_for(engine, tick, &[EntityKind::Enemy], BRAVE_FROM)
}

/// `run_input` going for the entities of `kinds` and getting out of the way of orbs until
/// tick `brave_from`.
pub fn run_input_for(engine: &Engine, tick: u32, kinds: &[EntityKind], brave_from: u32) -> Input {
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
        .filter(|v| kinds.contains(&v.kind))
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
    if tick >= brave_from {
        return input;
    }
    avoid_orbs(engine, input, at_elite)
}

/// `input` with the step across the path of the first hostile orb about to hit, and the evade
/// at the last moment, that `run_input_for` makes (`LATE` while it goes for an elite, `at_elite`).
pub fn avoid_orbs(engine: &Engine, mut input: Input, at_elite: bool) -> Input {
    let s = engine.snapshot();
    let me = s.player.at;
    let d2 = |at: Point| {
        let (x, y) = (i64::from(at.x.0 - me.x.0), i64::from(at.y.0 - me.y.0));
        x * x + y * y
    };
    // The first hostile orb that would pass within `HIT` in the next `SEE` ticks.
    let threat = s
        .orbs
        .iter()
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
pub fn answer_all(engine: &mut Engine, offers: &mut usize, out: &mut Vec<Command>) {
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

/// The bundled perk sets and a `plain` one with no effects: a run that differs from the same
/// stage played without a setup only by what its setup holds.
pub fn plain_content() -> RunContent {
    let mut file: serde_json::Value =
        serde_json::from_slice(common::bundled_file("perk_sets")).unwrap();
    file["perk_sets"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id": "plain", "effects": []
        }));
    super::design_content().with_perk_sets(
        stella_rain_core::perk_sets::PerkSets::parse(
            file.to_string().as_bytes(),
            Versions::CURRENT,
        )
        .unwrap(),
    )
}

/// The trial setup under the `plain` perk set (`plain_content`).
pub fn plain_setup() -> RunSetup {
    RunSetup {
        perk_set: "plain".into(),
        ..trial_setup()
    }
}
