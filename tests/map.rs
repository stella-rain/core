//! The map (core#87): a stage may give its width, height and start; without one it plays on
//! the portrait field. The main character and companions are clamped to the map, and enemies
//! and orbs leave once they are more than `FIELD_MARGIN` outside it.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::snapshot::MapView;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::limits::FIELD_MARGIN;

/// A landscape map of 2,048 by 1,024 pixels.
const WIDTH: i32 = 524_288;
const HEIGHT: i32 = 262_144;

fn on_map(mut stage: Stage, map: Value) -> Stage {
    let mut v = serde_json::to_value(&stage).unwrap();
    v["map"] = map;
    stage = serde_json::from_value(v).expect("the test stage parses");
    stage
}

fn wide(stage: Stage) -> Stage {
    on_map(stage, json!({ "width": WIDTH, "height": HEIGHT }))
}

fn map_of(e: &Engine) -> (i32, i32) {
    let MapView { width, height } = e.snapshot().map;
    (width.0, height.0)
}

#[test]
fn without_a_map_the_run_plays_on_the_portrait_field() {
    let e = engine(&stage_with(600, Some(far_boss()), vec![]));
    assert_eq!(map_of(&e), (92_160, 163_840));
    assert_eq!(player_at(&e), (START_X, START_Y));
}

#[test]
fn a_map_sets_the_size_and_the_main_character_starts_at_its_centre() {
    let e = engine(&wide(stage_with(600, Some(far_boss()), vec![])));
    assert_eq!(map_of(&e), (WIDTH, HEIGHT));
    assert_eq!(player_at(&e), (WIDTH / 2, HEIGHT / 2));
}

#[test]
fn a_map_may_say_where_the_main_character_starts() {
    let stage = on_map(
        stage_with(600, Some(far_boss()), vec![]),
        json!({ "width": WIDTH, "height": HEIGHT, "start": { "x": 1000, "y": 2000 } }),
    );
    assert_eq!(player_at(&engine(&stage)), (1000, 2000));
}

#[test]
fn the_main_character_is_clamped_to_the_map_not_the_portrait_field() {
    let stage = on_map(
        quiet(stage_with(600, Some(far_boss()), vec![])),
        json!({ "width": WIDTH, "height": HEIGHT, "start": { "x": WIDTH - 1000, "y": 1000 } }),
    );
    let mut e = engine(&stage);
    e.step(moving(127, 0));
    assert_eq!(player_at(&e), (WIDTH - 1000 + SPEED, 1000));
    e.step(moving(127, 0));
    assert_eq!(player_at(&e), (WIDTH, 1000), "clamped at the right edge");
    for _ in 0..3 {
        e.step(moving(0, -127));
    }
    assert_eq!(player_at(&e), (WIDTH, 0), "and at the top edge");
}

/// Every tick's x of the things `pick` reads from the snapshot, until there are none left.
fn xs_until_gone(e: &mut Engine, ticks: u32, pick: impl Fn(&Engine) -> Vec<i32>) -> Vec<i32> {
    let mut seen = Vec::new();
    for _ in 0..ticks {
        e.step(idle());
        let now = pick(e);
        if now.is_empty() && !seen.is_empty() {
            break;
        }
        seen.extend(now);
    }
    seen
}

#[test]
fn enemies_leave_past_the_maps_margin_not_the_portrait_fields() {
    // 16 pixels a tick to the right, from the middle of the map.
    let runner = enemy(
        50,
        json!({ "type": "straight", "vx": 4096, "vy": 0 }),
        json!([]),
    );
    // The main character stands at the right edge, so the runner stays within the leash's reach
    // (leash_reach_x of rules.json, 640 px) until it is gone, and the leash does not bring it back (EC2).
    let stage = on_map(
        quiet(stage_with(
            600,
            Some(far_boss()),
            vec![wave(1, 1, 0, runner, WIDTH - 51_200, 40_000)],
        )),
        json!({ "width": WIDTH, "height": HEIGHT, "start": { "x": WIDTH, "y": 40_000 } }),
    );
    let mut e = engine(&stage);
    let xs = xs_until_gone(&mut e, 600, |e| {
        let s = e.snapshot();
        s.entities
            .iter()
            .filter(|v| v.kind == stella_rain_core::snapshot::EntityKind::Enemy)
            .map(|v| v.at.x.0)
            .collect()
    });
    let last = *xs.iter().max().expect("the enemy spawned");
    assert!(
        last > 92_160 + FIELD_MARGIN,
        "it passed the portrait field: {last}"
    );
    assert!(
        last <= WIDTH + FIELD_MARGIN,
        "it never outlived the margin: {last}"
    );
    assert!(
        last > WIDTH + FIELD_MARGIN - 4096,
        "it left on the first step past it: {last}"
    );
    assert_eq!(enemies(&e.snapshot()), 0, "it is gone");
}

#[test]
fn companions_stay_on_the_map() {
    // A companion told to hold past the map's right edge, within the margin, is clamped to it.
    let stay = companion(10, hold(WIDTH + FIELD_MARGIN, 40_000), json!([]));
    let stage = wide(quiet(stage_full(600, Some(far_boss()), vec![], vec![stay])));
    let mut e = engine(&stage);
    e.step(idle());
    let s = e.snapshot();
    let c = s
        .entities
        .iter()
        .find(|v| v.kind == stella_rain_core::snapshot::EntityKind::Companion)
        .expect("the companion");
    assert_eq!((c.at.x.0, c.at.y.0), (WIDTH, 40_000));
}

/// `Map::holds` at every boundary, the far ends of `i32` included, on the smallest map, the
/// portrait field and the largest.
#[test]
fn holds_is_exact_at_the_margin() {
    use stella_rain_core::fixed::{Fx, Point};
    use stella_rain_core::stage::Map;
    use stella_rain_core::validate::limits::{MAX_MAP_SIDE, MIN_MAP_SIDE};
    let m = FIELD_MARGIN;
    for (w, h) in [
        (MIN_MAP_SIDE, MIN_MAP_SIDE),
        (92_160, 163_840),
        (MAX_MAP_SIDE, MAX_MAP_SIDE),
    ] {
        let map = Map {
            width: Fx(w),
            height: Fx(h),
            start: None,
            facing: None,
        };
        let holds = map.holds();
        let at = |x: i32, y: i32| holds(Point { x: Fx(x), y: Fx(y) });
        let edges = |side: i32| {
            [
                i32::MIN,
                -m - 1,
                -m,
                0,
                side,
                side + m,
                side + m + 1,
                i32::MAX - m,
                i32::MAX,
            ]
        };
        for x in edges(w) {
            assert_eq!(at(x, 0), (-m..=w + m).contains(&x), "x {x} on {w} by {h}");
        }
        for y in edges(h) {
            assert_eq!(at(0, y), (-m..=h + m).contains(&y), "y {y} on {w} by {h}");
        }
        assert!(!at(-m - 1, -m - 1) && at(w + m, h + m) && !at(w + m, h + m + 1));
    }
}

// --- An orb's lifetime and the start facing (core#147) ------------------------------------

/// An enemy at the top left of a landscape map that sends one orb straight down, 2 pixels a
/// tick, on its first tick, living `ticks` ticks. The orb is off the main character's line,
/// which is in the middle of the map, and the rest of the stage is quiet.
fn one_orb(ticks: u32) -> Stage {
    let orb = json!({ "type": "orb", "speed": 512, "ticks": ticks,
        "direction": { "type": "absolute", "angle": 0 } });
    let attack = json!({ "inline": [{ "kind": "active", "ticks": 1, "spawn": [orb] }] });
    let rules = json!([{ "when": always(), "target": "player",
        "do": { "type": "attack", "attack": attack }, "once": true }]);
    let shooter = enemy(100_000, hold(LEFT, 16_384), rules);
    wide(quiet(stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, shooter, LEFT, 16_384)],
        vec![],
    )))
}

/// A column well left of the middle, 32,768 units from the left edge.
const LEFT: i32 = 32_768;

/// Whether the stage has an orb on the map after each of `ticks` ticks.
fn on_the_map(e: &mut Engine, ticks: u32) -> Vec<bool> {
    (0..ticks)
        .map(|_| {
            e.step(idle());
            !e.snapshot().orbs.is_empty()
        })
        .collect()
}

#[test]
fn an_orb_lives_exactly_its_ticks() {
    // 30 ticks at 2 pixels a tick is 60 pixels: the margin is 64 pixels past the edge, so it
    // is not what removes the orb.
    let mut e = engine(&one_orb(30));
    let seen = on_the_map(&mut e, 60);
    assert_eq!(seen.iter().filter(|s| **s).count(), 30, "{seen:?}");
    assert!(
        seen.windows(2).filter(|w| w[0] && !w[1]).count() == 1,
        "{seen:?}"
    );
}

fn facing_of(e: &Engine) -> (i32, i32) {
    let f = e.snapshot().players[0].facing;
    (f.x, f.y)
}

#[test]
fn the_main_character_faces_up_unless_the_map_says_otherwise() {
    let up = engine(&wide(stage_with(600, Some(far_boss()), vec![])));
    assert_eq!(
        facing_of(&up),
        (0, -stella_rain_core::trig::SCALE),
        "up, as it always was"
    );
    // A quarter turn: 90 degrees, to the right of the screen.
    let angle = stella_rain_core::trig::PER_TURN / 4;
    let turned = on_map(
        stage_with(600, Some(far_boss()), vec![]),
        json!({ "width": WIDTH, "height": HEIGHT, "facing": angle }),
    );
    assert_eq!(
        facing_of(&engine(&turned)),
        stella_rain_core::trig::sin_cos(angle),
        "the map's facing"
    );
}

#[test]
fn an_orbs_lifetime_is_part_of_the_state_hash() {
    // Two stages that differ only in an orb's ticks hash differently while the orb is out,
    // so a replay that drifts in its lifetimes fails to verify (core#147, determinism rules).
    let hash_after = |ticks: u32| {
        let mut e = engine(&one_orb(ticks));
        for _ in 0..5 {
            e.step(idle());
        }
        e.state_hash()
    };
    assert_ne!(hash_after(30), hash_after(31));
    assert_eq!(hash_after(30), hash_after(30));
}
