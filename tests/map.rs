//! The map (core#87): a stage may give its width, height and start; without one it plays on
//! the portrait field. The main character and companions are clamped to the map, and enemies
//! and bullets leave once they are more than `FIELD_MARGIN` outside it.

mod common;

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
    let stage = wide(quiet(stage_with(
        600,
        Some(far_boss()),
        vec![wave(1, 1, 0, runner, WIDTH / 2, 40_000)],
    )));
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
fn bullets_leave_past_the_maps_margin_not_the_portrait_fields() {
    // The Main weapon fires along the facing at 8 pixels a tick, every 6 ticks; one tick of
    // the stick turns it to the right.
    let stage = wide(stage_with(600, Some(far_boss()), vec![]));
    let mut e = engine(&stage);
    e.step(moving(127, 0));
    let mut most = 0;
    for _ in 0..400 {
        e.step(firing(idle(), e.tick()));
        let s = e.snapshot();
        for b in s.bullets.iter().filter(|b| b.friendly) {
            most = most.max(b.at.x.0);
        }
    }
    assert!(
        most > 92_160 + FIELD_MARGIN,
        "a shot passed the portrait field: {most}"
    );
    assert!(
        most <= WIDTH + FIELD_MARGIN,
        "no shot outlived the margin: {most}"
    );
    assert!(
        most > WIDTH + FIELD_MARGIN - 2048,
        "shots fly up to the margin: {most}"
    );
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
