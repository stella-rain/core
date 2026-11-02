//! Aim assist (ADR-072): while `assist` is on, each tick picks a target among the enemies whose
//! circle reaches within the Main weapon's `assist_range` of the main character: the boss or
//! one of its parts first, then an elite, then any other enemy; within a class the nearest,
//! then the lower entity ID. A combo step faces the target as its attack starts (a charged
//! step: as it is released) and the facing turns to it; with no target it attacks along the
//! facing. The main character stands at (46080, 147456), facing up the field.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::Stage;

/// 100 pixels.
const RANGE: i32 = 25_600;
/// A bat's radius.
const BAT: i32 = 2048;
/// How far ahead a step's zone is placed, which shows where the step faced.
const SHOT: i32 = 2048;
/// `trig::SCALE`: a facing's length.
const SCALE: i32 = 65_536;

/// One zone ahead along the step's facing on its first tick, then one tick of recovery.
fn shot() -> Value {
    json!({ "inline": [
        { "kind": "active", "ticks": 1, "spawn": [{ "type": "zone",
            "shape": { "type": "circle", "radius": 512 }, "offset": { "forward": SHOT },
            "direction": { "type": "aimed", "offset": 0 }, "wind_up_ticks": 0,
            "active_ticks": 100 }] },
        { "kind": "recovery", "ticks": 1 }
    ] })
}

/// A Main weapon with `step` on a press of `main` and the test range.
fn weapon(step: Value) -> Value {
    json!({ "inline": { "combo": { "main": step }, "window_ticks": 0, "assist_range": RANGE } })
}

/// A step that charges to level 1 after 5 ticks held; each level shoots.
fn charged() -> Value {
    json!({ "attack": shot(), "charge": { "max_ticks": 60, "levels": [
        { "at_ticks": 5, "attack": shot() }] } })
}

fn bat_at(at_tick: u32, dx: i32, dy: i32) -> Value {
    let (x, y) = (START_X + dx, START_Y + dy);
    wave(at_tick, 1, 0, enemy(100_000, hold(x, y), json!([])), x, y)
}

fn elite_at(at_tick: u32, dx: i32, dy: i32) -> Value {
    let (x, y) = (START_X + dx, START_Y + dy);
    let mut e = enemy(100_000, hold(x, y), json!([]));
    e["elite"] = json!({ "range": 1 });
    wave(at_tick, 1, 0, e, x, y)
}

fn stage(boss: Value, waves: Vec<Value>, step: Value) -> Stage {
    armed(stage_with(600, Some(boss), waves), weapon(step))
}

fn plain(waves: Vec<Value>) -> Stage {
    stage(far_boss(), waves, json!({ "attack": shot() }))
}

fn assisted() -> Input {
    Input {
        assist: true,
        ..idle()
    }
}

fn pressed(assist: bool) -> Input {
    Input {
        main: true,
        assist,
        ..idle()
    }
}

/// Where the snapshot's target is, relative to the main character's start.
fn target(e: &Engine) -> Option<(i32, i32)> {
    let s = e.snapshot();
    let id = s.players[0].target?;
    let v = s.entities.iter().find(|v| v.id == id)?;
    Some((v.at.x.0 - START_X, v.at.y.0 - START_Y))
}

fn target_after_one_tick(stage: &Stage) -> Option<(i32, i32)> {
    let mut e = engine(stage);
    e.step(&[assisted()]);
    target(&e)
}

/// Where the main character's one zone points: its direction, `SHOT` long.
fn shot_velocity(e: &Engine) -> (i32, i32) {
    let s = e.snapshot();
    let mine: Vec<_> = s.zones.iter().filter(|z| z.friendly).collect();
    assert_eq!(mine.len(), 1, "one zone from the step");
    (
        mine[0].direction.x * SHOT / SCALE,
        mine[0].direction.y * SHOT / SCALE,
    )
}

fn facing(e: &Engine) -> (i32, i32) {
    let f = e.snapshot().players[0].facing;
    (f.x, f.y)
}

// --- The pick ----------------------------------------------------------------------------------

#[test]
fn with_assist_off_there_is_no_target() {
    let mut e = engine(&plain(vec![bat_at(1, RANGE / 2, 0)]));
    e.step(&[idle()]);
    assert_eq!(e.snapshot().players[0].target, None);
}

#[test]
fn the_target_is_the_nearest_enemy_in_range() {
    let st = plain(vec![bat_at(1, RANGE, 0), bat_at(1, -RANGE / 2, 0)]);
    assert_eq!(target_after_one_tick(&st), Some((-RANGE / 2, 0)));
}

#[test]
fn an_enemy_is_in_range_while_its_circle_reaches_the_range() {
    let reach = RANGE + BAT;
    assert_eq!(
        target_after_one_tick(&plain(vec![bat_at(1, reach, 0)])),
        Some((reach, 0))
    );
    assert_eq!(
        target_after_one_tick(&plain(vec![bat_at(1, reach + 1, 0)])),
        None
    );
}

#[test]
fn equal_distances_pick_the_lower_entity_id() {
    let right_first = plain(vec![bat_at(1, RANGE / 2, 0), bat_at(1, -RANGE / 2, 0)]);
    assert_eq!(target_after_one_tick(&right_first), Some((RANGE / 2, 0)));
    let left_first = plain(vec![bat_at(1, -RANGE / 2, 0), bat_at(1, RANGE / 2, 0)]);
    assert_eq!(target_after_one_tick(&left_first), Some((-RANGE / 2, 0)));
}

#[test]
fn an_elite_comes_before_a_nearer_enemy() {
    let st = plain(vec![bat_at(1, RANGE / 4, 0), elite_at(1, -RANGE, 0)]);
    assert_eq!(target_after_one_tick(&st), Some((-RANGE, 0)));
}

#[test]
fn the_boss_comes_before_a_nearer_elite() {
    let st = stage(
        boss_at(1_000_000, START_Y - RANGE),
        vec![elite_at(1, RANGE / 4, 0), bat_at(1, -RANGE / 4, 0)],
        json!({ "attack": shot() }),
    );
    let mut e = engine(&st);
    e.step(&[assisted()]);
    let s = e.snapshot();
    let boss = s
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Boss)
        .expect("the boss is on the field");
    assert_eq!(s.players[0].target, Some(boss.id));
}

// --- The step ----------------------------------------------------------------------------------

#[test]
fn a_step_faces_its_target_and_turns_the_facing() {
    let st = plain(vec![bat_at(1, RANGE / 2, 0)]);
    let mut e = engine(&st);
    e.step(&[pressed(true)]);
    assert_eq!(shot_velocity(&e), (SHOT, 0));
    assert_eq!(facing(&e), (SCALE, 0));
    // The same press with assist off shoots up the field.
    let mut e = engine(&st);
    e.step(&[pressed(false)]);
    assert_eq!(shot_velocity(&e), (0, -SHOT));
    assert_eq!(facing(&e), (0, -SCALE));
}

#[test]
fn with_no_target_in_range_a_step_attacks_along_the_facing() {
    let mut e = engine(&plain(vec![bat_at(1, RANGE * 2, 0)]));
    e.step(&[pressed(true)]);
    assert_eq!(shot_velocity(&e), (0, -SHOT));
}

#[test]
fn a_charged_step_faces_the_target_of_its_release() {
    // A bat to the right from tick 1; an elite to the left from tick 4, while the step charges.
    let st = stage(
        far_boss(),
        vec![bat_at(1, RANGE / 2, 0), elite_at(4, -RANGE / 2, 0)],
        charged(),
    );
    let mut e = engine(&st);
    for _ in 1..=8 {
        e.step(&[pressed(true)]);
    }
    assert_eq!(e.snapshot().players[0].charge.map(|c| c.level), Some(1));
    e.step(&[assisted()]);
    assert_eq!(shot_velocity(&e), (-SHOT, 0));
    assert_eq!(facing(&e), (-SCALE, 0));
}
