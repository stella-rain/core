//! An attack phase that dashes (DS11 items 3 and 5, core Z3a-2): an `Active` phase with
//! `dash: { distance }` moves its attacker that far along the attack's locked facing over the
//! phase, one share of the distance each tick (the whole distance is covered exactly), clamped
//! to the map, and its hit box goes with it. The phase's `move_pct` must be 0, and only an
//! enemy's or the boss's attack may dash.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{Error, Issue, validate};

const PX: i32 = 256;
/// The map's bottom edge, in units.
const MAP_H: i32 = 163_840;
const WIND_UP: u32 = 10;
const ACTIVE: u32 = 20;

/// Wind-up 10, then 20 active ticks dashing `distance` pixels; `hit` adds a circle hit box of
/// 10 pixels on the attacker.
fn phases(distance: i32, hit: bool) -> Value {
    let mut active = json!({ "kind": "active", "ticks": ACTIVE, "damage": 5, "move_pct": 0,
                              "dash": { "distance": distance * PX } });
    if hit {
        active["hit_box"] = json!({ "type": "circle", "radius": 10 * PX });
    }
    json!([{ "kind": "wind_up", "ticks": WIND_UP, "move_pct": 0 }, active])
}

fn attack_rule_of(phases: Value) -> Value {
    json!([{ "when": always(), "target": "player", "once": true, "cooldown_ticks": 0,
             "do": { "type": "attack", "attack": { "inline": phases } } }])
}

fn still() -> Value {
    json!({ "type": "straight", "vx": 0, "vy": 0 })
}

/// An enemy that stands where it spawns, at `(x, y)` in 1/256 pixel (from `near_start`, so it is
/// inside the leash's reach), and dashes once at the main character. (A `hold` would walk it
/// back to its spot after the dash.)
fn dasher((x, y): (i32, i32), phases: Value) -> Value {
    wave(1, 1, 0, enemy(1000, still(), attack_rule_of(phases)), x, y)
}

/// An enemy that only stands at `(x, y)` in 1/256 pixel.
fn bystander((x, y): (i32, i32)) -> Value {
    wave(1, 1, 0, enemy(1000, still(), json!([])), x, y)
}

fn stage(waves: Vec<Value>) -> Stage {
    quiet(stage_with(600, Some(far_boss()), waves))
}

fn enemy_positions(e: &Engine) -> Vec<(i32, i32)> {
    e.snapshot()
        .entities
        .iter()
        .filter(|v| v.kind == EntityKind::Enemy)
        .map(|v| (v.at.x.0, v.at.y.0))
        .collect()
}

/// The first enemy's place after each of `ticks` ticks.
fn track(stage: &Stage, ticks: u32) -> Vec<(i32, i32)> {
    let mut e = engine(stage);
    (0..ticks)
        .map(|_| {
            e.step(&[idle()]);
            enemy_positions(&e)[0]
        })
        .collect()
}

// --- Motion -------------------------------------------------------------------------------

#[test]
fn the_dash_covers_its_distance_exactly_a_share_each_tick() {
    // Straight above the main character, 326 pixels: the dash goes straight down.
    let (x0, y0) = near_start(0, -326);
    let path = track(&stage(vec![dasher((x0, y0), phases(200, false))]), 60);
    assert!(path.iter().all(|p| p.0 == x0), "{path:?}");
    assert_eq!(*path.last().unwrap(), (x0, y0 + 200 * PX));
    // 200 pixels over 20 ticks: 10 pixels, 2,560 units, each tick, and none before or after.
    let steps: Vec<i32> = path.windows(2).map(|w| w[1].1 - w[0].1).collect();
    let moving = steps.iter().filter(|&&d| d != 0).count();
    assert_eq!(moving, ACTIVE as usize, "{steps:?}");
    assert!(steps.iter().all(|&d| d == 0 || d == 10 * PX), "{steps:?}");
}

#[test]
fn a_dash_past_the_map_ends_at_its_edge() {
    // 326 pixels above the main character, 700 pixels down, past the bottom edge.
    let (x0, y0) = near_start(0, -326);
    let path = track(&stage(vec![dasher((x0, y0), phases(700, false))]), 60);
    assert_eq!(*path.last().unwrap(), (x0, MAP_H));
    assert!(path.iter().all(|p| p.1 <= MAP_H), "{path:?}");
}

#[test]
fn an_uneven_share_still_adds_up_to_the_distance() {
    // 7 pixels over 20 ticks does not divide: the shares differ by a unit, the sum is exact.
    let (x0, y0) = near_start(0, -326);
    let path = track(&stage(vec![dasher((x0, y0), phases(7, false))]), 60);
    assert_eq!(path.last().unwrap().1, y0 + 7 * PX);
}

#[test]
fn a_diagonal_dash_loses_no_rounding_over_its_ticks() {
    // The main character is down and to the right: the facing is not along an axis.
    let (x0, y0) = near_start(-120, -276);
    let path = track(&stage(vec![dasher((x0, y0), phases(200, false))]), 60);
    let (x, y) = *path.last().unwrap();
    let (dx, dy) = (i64::from(x - x0), i64::from(y - y0));
    assert!(dx > 0 && dy > 0, "{dx} {dy}");
    // The whole distance, less what the facing's own rounding leaves (a facing is a vector of
    // length about 256, so a unit or two in 51,200).
    let covered = (dx * dx + dy * dy).isqrt();
    let wanted = i64::from(200 * PX);
    assert!(
        (wanted - 4..=wanted).contains(&covered),
        "{covered} of {wanted}"
    );
}

#[test]
fn the_hit_box_travels_with_the_attacker() {
    // The main character stands 326 pixels below; the dash goes 400 pixels, through it.
    let st = stage(vec![dasher(near_start(0, -326), phases(400, true))]);
    let mut e = engine(&st);
    let radius = e.snapshot().players[0].radius.0;
    let (px, py) = player_at(&e);
    assert_eq!((px, py), near_start(0, 0));
    let mut first_overlap = None;
    let mut hit_at = None;
    for _ in 0..60 {
        e.step(&[idle()]);
        let (ex, ey) = enemy_positions(&e)[0];
        let d = i64::from(ex - px).pow(2) + i64::from(ey - py).pow(2);
        let reach = i64::from(10 * PX + radius);
        if first_overlap.is_none() && d <= reach * reach {
            first_overlap = Some(e.tick());
        }
        if hit_at.is_none()
            && e.events()
                .iter()
                .any(|ev| matches!(ev, DomainEvent::PlayerHit { .. }))
        {
            hit_at = Some(e.tick());
        }
    }
    let (overlap, hit) = (first_overlap.expect("it reaches"), hit_at.expect("it hits"));
    assert!((overlap..=overlap + 1).contains(&hit), "{overlap} {hit}");
}

#[test]
fn a_dasher_is_not_moved_by_the_push_apart() {
    // A bystander stands 5 pixels off the dasher's path: their circles overlap as it passes.
    let (x0, y0) = near_start(0, -326);
    let st = stage(vec![
        dasher((x0, y0), phases(200, false)),
        bystander(near_start(5, -226)),
    ]);
    let mut e = engine(&st);
    for _ in 0..60 {
        e.step(&[idle()]);
        let at = enemy_positions(&e);
        assert_eq!(at[0].0, x0, "{at:?}");
    }
    assert_eq!(enemy_positions(&e)[0].1, y0 + 200 * PX);
}

// --- The boss -----------------------------------------------------------------------------

#[test]
fn a_boss_dash_moves_its_parts_by_the_same_offset() {
    let boss = json!({
        "base": "golem", "hp": 1_000, "radius": 8_192,
        "spawn": { "x": 180 * PX, "y": 100 * PX },
        "parts": [{ "id": "horn", "asset": "horns_2", "hp": 100,
                    "offset": { "x": 0, "y": 16 * PX }, "radius": 2_048 }],
        "phases": [{ "timeline": [
            { "type": "attack", "attack": { "inline": phases(200, false) } },
            { "type": "wait", "ticks": 100 }] }]
    });
    let st = quiet(stage_full(600, Some(boss), vec![], vec![]));
    let mut e = engine(&st);
    let at = |e: &Engine, kind: EntityKind| {
        let s = e.snapshot();
        let v = s.entities.iter().find(|v| v.kind == kind).unwrap();
        (v.at.x.0, v.at.y.0)
    };
    for _ in 0..60 {
        e.step(&[idle()]);
    }
    assert_eq!(at(&e, EntityKind::Boss), (180 * PX, 300 * PX));
    assert_eq!(at(&e, EntityKind::BossPart), (180 * PX, 316 * PX));
}

// --- The validator ------------------------------------------------------------------------

fn issues(st: &Stage) -> Vec<Issue> {
    match validate(st) {
        Ok(()) => Vec::new(),
        Err(Error::Invalid(issues)) => issues,
        Err(other) => panic!("unexpected {other:?}"),
    }
}

fn refused_for_dash(st: &Stage) {
    let found = issues(st);
    assert!(
        found
            .iter()
            .any(|i| i.message.contains("dash") || i.path.contains("dash")),
        "{found:?}"
    );
}

fn with_phase(mut phases: Value, i: usize, key: &str, value: Value) -> Value {
    phases[i][key] = value;
    phases
}

#[test]
fn an_enemy_may_dash_in_an_active_phase() {
    let st = stage(vec![dasher(near_start(0, -476), phases(200, true))]);
    assert_eq!(issues(&st), vec![]);
}

#[test]
fn a_dash_on_a_wind_up_phase_is_refused() {
    let p = with_phase(
        phases(200, false),
        0,
        "dash",
        json!({ "distance": 200 * PX }),
    );
    refused_for_dash(&stage(vec![dasher(near_start(0, -476), p)]));
}

#[test]
fn a_dash_of_no_distance_or_too_far_is_refused() {
    for distance in [0, 961] {
        let st = stage(vec![dasher(near_start(0, -476), phases(distance, false))]);
        refused_for_dash(&st);
    }
    let longest = stage(vec![dasher(near_start(0, -476), phases(960, false))]);
    assert_eq!(issues(&longest), vec![]);
}

#[test]
fn a_dashing_phase_that_also_walks_is_refused() {
    let p = with_phase(phases(200, false), 1, "move_pct", json!(50));
    refused_for_dash(&stage(vec![dasher(near_start(0, -476), p)]));
}

#[test]
fn a_weapon_attack_may_not_dash() {
    let weapon = weapon_ref(json!({ "inline": phases(200, false) }));
    let st = armed(stage(vec![]), weapon);
    refused_for_dash(&st);
}
