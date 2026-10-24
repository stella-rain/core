//! The ranged-attacker cap (DS10, DS12, core Z2): a rule marked `ranged` starts only while fewer
//! than `MAX_RANGED_ATTACKERS` (4) ranged runs hold a token, a token being held while a run is in
//! its wind-up or active phase. A start that would exceed the cap is refused and its rule keeps
//! its cooldown ready, so it retries on the next tick; starts are granted in entity-ID order.
//! A rule without `ranged` is outside the cap, and the validator refuses `ranged` on companions,
//! elites and the boss.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::attack::PhaseKind;
use stella_rain_core::engine::Engine;
use stella_rain_core::id::EntityId;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{Error, validate};

const PX: i32 = 256;
/// The cap of `rules_v0::MAX_RANGED_ATTACKERS`.
const MAX_RANGED_ATTACKERS: usize = 4;
const SHOOTERS: u32 = 6;
/// The far boss is entity 1; the shooters come after it, in wave order.
const FIRST_SHOOTER: u32 = 2;

/// Three ticks of wind-up, three of active, then recovery: a token is held for six ticks.
fn phases() -> Value {
    json!([
        { "kind": "wind_up", "ticks": 3 },
        { "kind": "active", "ticks": 3 },
        { "kind": "recovery", "ticks": 40 },
    ])
}

/// A rule that attacks the main character whenever it can: the cooldown is long, so a refused
/// start that consumed it would keep the shooter quiet for the whole test.
fn shooting(ranged: bool) -> Value {
    let mut action = json!({ "type": "attack", "attack": { "inline": phases() } });
    if ranged {
        action["ranged"] = json!(true);
    }
    json!([{ "when": always(), "target": "player", "cooldown_ticks": 1000, "do": action }])
}

fn stage(ranged: bool) -> Stage {
    let waves = (0..SHOOTERS as i32)
        .map(|i| {
            let x = START_X + (i - 3) * 20 * PX;
            wave(
                1,
                1,
                0,
                enemy(50, hold(x, START_Y - 100 * PX), shooting(ranged)),
                x,
                START_Y - 100 * PX,
            )
        })
        .collect();
    quiet(stage_with(600, Some(far_boss()), waves))
}

/// The phase kind of the attack an entity is making, if it is making one.
fn kind_of(e: &Engine, id: u32) -> Option<PhaseKind> {
    e.snapshot()
        .entities
        .iter()
        .find(|v| v.id == EntityId(id))
        .and_then(|v| v.attack.as_ref().map(|a| a.kind))
}

fn holding(e: &Engine) -> usize {
    (FIRST_SHOOTER..FIRST_SHOOTER + SHOOTERS)
        .filter(|id| matches!(kind_of(e, *id), Some(PhaseKind::WindUp | PhaseKind::Active)))
        .count()
}

/// For each tick, how many shooters hold a token.
fn held_per_tick(stage: &Stage, ticks: u32) -> Vec<usize> {
    let mut e = engine(stage);
    (0..ticks)
        .map(|_| {
            e.step(idle());
            holding(&e)
        })
        .collect()
}

/// The first tick on which each shooter is seen making an attack.
fn first_attack_ticks(stage: &Stage, ticks: u32) -> Vec<Option<u32>> {
    let mut e = engine(stage);
    let mut first = vec![None; SHOOTERS as usize];
    for _ in 0..ticks {
        e.step(idle());
        for (i, f) in first.iter_mut().enumerate() {
            if f.is_none() && kind_of(&e, FIRST_SHOOTER + i as u32).is_some() {
                *f = Some(e.tick());
            }
        }
    }
    first
}

#[test]
fn at_most_four_ranged_runs_are_in_wind_up_or_active_on_any_tick() {
    let held = held_per_tick(&stage(true), 80);
    assert_eq!(held.iter().max(), Some(&MAX_RANGED_ATTACKERS));
}

#[test]
fn the_fifth_shooter_starts_on_the_tick_a_token_frees() {
    let st = stage(true);
    let first = first_attack_ticks(&st, 80);
    let started = first[0].expect("the first shooter starts");
    assert_eq!(first[..4], [Some(started); 4]);
    // The four runs enter recovery together, six ticks after they began; the same tick the
    // tokens free, the two refused shooters start, in entity-ID order.
    let freed = started + 6;
    assert_eq!(first[4], Some(freed));
    assert_eq!(first[5], Some(freed));
}

#[test]
fn a_refused_start_does_not_consume_its_cooldown() {
    // Their cooldown is 1000 ticks: only a rule that kept its cooldown ready starts again here.
    let first = first_attack_ticks(&stage(true), 80);
    assert!(first[4].is_some_and(|t| t < 100));
}

#[test]
fn a_shooter_without_ranged_is_not_counted() {
    let held = held_per_tick(&stage(false), 80);
    assert_eq!(held.iter().max(), Some(&(SHOOTERS as usize)));
}

// --- The validator ---------------------------------------------------------------------------

fn ranged_rule() -> Value {
    shooting(true)
}

fn refused(stage: &Stage, what: &str) {
    match validate(stage) {
        Err(Error::Invalid(issues)) => assert!(
            issues
                .iter()
                .any(|i| i.path.contains("ranged") || i.message.contains("ranged")),
            "{what}: {issues:?}"
        ),
        other => panic!("{what}: expected an issue, got {other:?}"),
    }
}

#[test]
fn the_validator_allows_ranged_on_a_regular_enemy() {
    let st = quiet(stage_with(
        600,
        Some(far_boss()),
        vec![wave(1, 1, 0, enemy(5, hold(0, 0), ranged_rule()), 0, 0)],
    ));
    validate(&st).expect("a regular enemy may be ranged");
}

#[test]
fn the_validator_refuses_ranged_on_a_companion() {
    let st = quiet(stage_full(
        600,
        Some(far_boss()),
        vec![],
        vec![companion(5, hold(0, 0), ranged_rule())],
    ));
    refused(&st, "companion");
}

#[test]
fn the_validator_refuses_ranged_on_an_elite() {
    let mut elite = enemy(5, hold(0, 0), ranged_rule());
    elite["elite"] = json!({ "range": 1000, "boss_call": "stay" });
    let st = quiet(stage_with(
        600,
        Some(far_boss()),
        vec![wave(1, 1, 0, elite, 0, 0)],
    ));
    refused(&st, "elite");
}

#[test]
fn the_validator_refuses_ranged_on_the_boss() {
    let mut boss = far_boss();
    boss["phases"] = json!([{ "rules": ranged_rule() }]);
    let st = quiet(stage_with(600, Some(boss), vec![]));
    refused(&st, "boss");
}
