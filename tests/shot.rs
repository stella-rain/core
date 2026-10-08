//! The main shot (ADR-038): the stage's `shot` attack, started by the main character every
//! `SHOT_INTERVAL` ticks while `fire` is held. Its `aimed` directions point straight up the
//! screen, the way the ship faces, and `relative` ones start from there; `absolute` ones stay
//! fixed on the field (0 is down). Small stages whose every tick can be worked out by hand: the
//! main character stands at (46080, 147456) and does not move.

mod common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::fixed::Fx;
use stella_rain_core::snapshot::Snapshot;
use stella_rain_core::stage::Stage;

const DEG: i32 = 256;

// --- Builders --------------------------------------------------------------------------------

fn fire(speed: i32, direction: Value) -> Value {
    json!({ "type": "fire", "speed": speed, "direction": direction })
}

fn aimed(offset: i32) -> Value {
    json!({ "type": "aimed", "offset": offset })
}

fn wait(ticks: i32) -> Value {
    json!({ "type": "wait", "ticks": ticks })
}

/// A stage whose main shot is `shot` (a preset or `{"inline": [...]}`), against a boss that
/// cannot be reached, with `waves` of enemies and the main character's attack at `atk`.
fn shooting(shot: Value, atk: u32, waves: Vec<Value>) -> Stage {
    let mut stage = stage_with(600, Some(far_boss()), waves);
    stage.player.shot = serde_json::from_value(shot).expect("the shot parses");
    stage.player.atk = atk;
    stage
}

fn inline(nodes: Vec<Value>) -> Value {
    json!({ "inline": nodes })
}

/// Steps `ticks` ticks with fire held, or not held after the first `fire_ticks`.
fn run(e: &mut Engine, ticks: u32, fire_ticks: u32) -> Vec<Snapshot> {
    (0..ticks)
        .map(|t| {
            e.step(if t < fire_ticks { firing() } else { idle() });
            e.snapshot()
        })
        .collect()
}

/// Where the friendly bullets are, as offsets from the main character's start, in order.
fn friendly(s: &Snapshot) -> Vec<(i32, i32)> {
    s.bullets
        .iter()
        .filter(|b| b.friendly)
        .map(|b| (b.at.x.0 - START_X, b.at.y.0 - START_Y))
        .collect()
}

fn first_tick_bullets(stage: &Stage) -> Vec<(i32, i32)> {
    let mut e = engine(stage);
    e.step(firing());
    friendly(&e.snapshot())
}

// --- What the stage's shot makes -------------------------------------------------------------

#[test]
fn an_aimed_shot_goes_straight_up() {
    // The bullet is made and moves on the tick of the shot: one tick of speed 1024.
    let stage = shooting(inline(vec![fire(1024, aimed(0))]), 10, vec![]);
    assert_eq!(first_tick_bullets(&stage), [(0, -1024)]);
}

#[test]
fn the_stages_attack_decides_the_bullets_not_one_built_in_bullet() {
    // Three bullets in a fan, 10 degrees apart, around straight up. A positive turn is the
    // same turn as everywhere else: from up it goes toward the left of the screen.
    let stage = shooting(
        inline(vec![
            fire(1024, aimed(-10 * DEG)),
            fire(1024, aimed(0)),
            fire(1024, aimed(10 * DEG)),
        ]),
        10,
        vec![],
    );
    let b = first_tick_bullets(&stage);
    assert_eq!(b.len(), 3);
    assert_eq!(b[1], (0, -1024));
    assert_eq!(b[0].0, -b[2].0, "the fan is symmetric");
    assert_eq!(b[0].1, b[2].1);
    // 1024 * sin(10 degrees) = 177.8; the table is interpolated, so allow one unit.
    assert!((b[2].0.abs() - 177).abs() <= 1, "{b:?}");
    assert!(b[2].0 < 0, "a positive turn from up goes toward the left");
}

#[test]
fn the_default_twin_shot_makes_two_bullets_beside_each_other() {
    let stage = shooting(
        json!({ "preset": { "id": "twin_shot", "args": [] } }),
        10,
        vec![],
    );
    let b = first_tick_bullets(&stage);
    assert_eq!(b.len(), 2);
    assert_eq!(b[0].0, -b[1].0);
    assert_eq!(b[0].1, b[1].1);
    assert!(b[0].1 < -2000, "they fly up at speed 2048: {b:?}");
}

#[test]
fn aimed_does_not_look_for_an_enemy() {
    // An enemy stands far to the left of the line of fire; the shot still goes straight up.
    let enemy_to_the_left = wave(
        1,
        1,
        0,
        enemy(50, hold(5_000, 100_000), json!([])),
        5_000,
        100_000,
    );
    let stage = shooting(
        inline(vec![fire(1024, aimed(0))]),
        10,
        vec![enemy_to_the_left],
    );
    assert_eq!(first_tick_bullets(&stage), [(0, -1024)]);
}

#[test]
fn relative_starts_from_up_and_absolute_stays_on_the_field() {
    let one = |direction: Value| {
        first_tick_bullets(&shooting(inline(vec![fire(1024, direction)]), 10, vec![]))
    };
    assert_eq!(
        one(json!({ "type": "relative", "angle": 0 })),
        [(0, -1024)],
        "relative to the ship's heading, which is up"
    );
    assert_eq!(
        one(json!({ "type": "absolute", "angle": 0 })),
        [(0, 1024)],
        "absolute is an angle from straight down the field"
    );
    assert_eq!(
        one(json!({ "type": "absolute", "angle": 180 * DEG })),
        [(0, -1024)]
    );
}

#[test]
fn sequential_starts_from_the_heading_too() {
    // A step of 0 keeps the last direction, which starts as the heading: up.
    let stage = shooting(
        inline(vec![fire(1024, json!({ "type": "sequential", "step": 0 }))]),
        10,
        vec![],
    );
    assert_eq!(first_tick_bullets(&stage), [(0, -1024)]);
}

// --- When it fires ---------------------------------------------------------------------------

#[test]
fn the_shot_repeats_every_six_ticks_while_fire_is_held() {
    let stage = shooting(inline(vec![fire(256, aimed(0))]), 10, vec![]);
    let mut e = engine(&stage);
    let seen = run(&mut e, 20, 20);
    // New bullets on ticks 1, 7, 13 and 19; bullets are alive until they leave the field.
    let counts: Vec<usize> = seen.iter().map(|s| friendly(s).len()).collect();
    assert_eq!(counts[0], 1);
    assert_eq!(counts[5], 1);
    assert_eq!(counts[6], 2);
    assert_eq!(counts[12], 3);
    assert_eq!(counts[18], 4);
    assert_eq!(counts[19], 4);
}

#[test]
fn nothing_is_shot_without_fire() {
    let stage = shooting(inline(vec![fire(256, aimed(0))]), 10, vec![]);
    let mut e = engine(&stage);
    let seen = run(&mut e, 30, 0);
    assert!(seen.iter().all(|s| friendly(s).is_empty()));
}

#[test]
fn a_shot_that_has_started_carries_on_after_fire_is_released() {
    // One bullet now, one 10 ticks later, whether or not fire is still held.
    let stage = shooting(
        inline(vec![fire(256, aimed(0)), wait(10), fire(256, aimed(0))]),
        10,
        vec![],
    );
    let mut e = engine(&stage);
    let seen = run(&mut e, 14, 1);
    let counts: Vec<usize> = seen.iter().map(|s| friendly(s).len()).collect();
    assert_eq!(counts[0], 1);
    assert_eq!(counts[9], 1);
    assert_eq!(counts[10], 2, "the second bullet, on tick 11");
}

#[test]
fn a_shot_follows_the_main_character_while_it_runs() {
    // The second bullet leaves from where the main character is then, not where it was.
    let stage = shooting(
        inline(vec![fire(0, aimed(0)), wait(5), fire(0, aimed(0))]),
        10,
        vec![],
    );
    let mut e = engine(&stage);
    e.step(firing());
    for _ in 0..5 {
        e.step(moving(100, 0));
    }
    let b = e.snapshot().bullets;
    assert_eq!(b.len(), 2);
    assert_eq!(b[0].at.x.0, START_X);
    assert_eq!(b[1].at.x.0, START_X + 5 * 100);
}

// --- Damage ----------------------------------------------------------------------------------

#[test]
fn every_bullet_of_the_shot_does_the_attack_stat_in_damage() {
    // A boss in the line of fire, 8192 across: two bullets of 7 and no other hurt.
    let mut stage = shooting(
        inline(vec![fire(2048, aimed(0)), fire(2048, aimed(0))]),
        7,
        vec![],
    );
    stage.boss = Some(serde_json::from_value(boss_at(1_000, 100_000)).expect("the boss parses"));
    let mut e = engine(&stage);
    for _ in 0..60 {
        e.step(if e.tick() == 0 { firing() } else { idle() });
        if e.snapshot().entities.iter().any(|v| v.hp < 1_000) {
            break;
        }
    }
    let hp = e.snapshot().entities[0].hp;
    // Both bullets arrive on the same tick: 1000 - 7 - 7.
    assert_eq!(hp, 986);
}

#[test]
fn a_raised_attack_stat_raises_the_damage() {
    let damage_of = |atk: u32| {
        let mut stage = shooting(inline(vec![fire(2048, aimed(0))]), atk, vec![]);
        stage.boss = Some(serde_json::from_value(boss_at(1_000, 100_000)).unwrap());
        let mut e = engine(&stage);
        e.step(firing());
        for _ in 0..60 {
            e.step(idle());
        }
        1_000 - e.snapshot().entities[0].hp
    };
    assert_eq!(damage_of(5), 5);
    assert_eq!(damage_of(40), 40);
}

// --- The main shot is a task like any other --------------------------------------------------

#[test]
fn the_main_shot_is_started_on_the_tick_it_fires_and_not_before() {
    let stage = shooting(inline(vec![fire(256, aimed(0))]), 10, vec![]);
    let mut e = engine(&stage);
    e.step(idle());
    assert!(friendly(&e.snapshot()).is_empty());
    e.step(firing());
    assert_eq!(friendly(&e.snapshot()).len(), 1);
    assert_eq!(e.snapshot().player.at.x, Fx(START_X));
}

#[test]
fn a_run_with_the_main_shot_is_the_same_every_time() {
    let stage = shooting(
        json!({ "preset": { "id": "spread_5", "args": [] } }),
        10,
        vec![wave(
            1,
            2,
            30,
            enemy(40, hold(46_080, 90_000), json!([])),
            46_080,
            90_000,
        )],
    );
    let hashes = || {
        let mut e = engine(&stage);
        (0..200)
            .map(|t| {
                e.step(if t % 9 < 6 { firing() } else { idle() });
                e.state_hash()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(hashes(), hashes());
}
