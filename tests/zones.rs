//! Zones (DS10, core Z1): the pattern language lays out telegraphed zones that do not move. A
//! `zone` node places a circle, an arc or a line at an anchor, the attacker's spot or where the
//! target stood when the attack began; it warns for its wind-up ticks, hurts each target once
//! while it is active, and ends. `ring`, `spread`, `rotate` and `repeat` place and time zones as
//! they do orbs. At most 128 are alive at once (`MAX_ZONES_ALIVE`); `clear_zones` removes the
//! enemy ones. A hit box may be anchored at the target's spot too, and may be a line.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{Error, Issue, limits::*, validate};

const CX: i32 = 20_000;
const CY: i32 = 30_000;
const PX: i32 = 256;
/// The cap of `sim::MAX_ZONES_ALIVE`.
const MAX_ZONES_ALIVE: usize = 128;

/// The companion is entity 2 (the far boss is 1); the waves' enemies come after it, in order.
const COMPANION: EntityId = EntityId(2);

fn circle(radius: i32) -> Value {
    json!({ "type": "circle", "radius": radius })
}

fn line(length: i32, width: i32) -> Value {
    json!({ "type": "line", "length": length, "width": width })
}

fn zone(shape: Value, anchor: &str, wind_up: u32, active: u32) -> Value {
    json!({ "type": "zone", "shape": shape, "anchor": anchor,
            "wind_up_ticks": wind_up, "active_ticks": active })
}

fn with_offset(mut z: Value, forward: i32, right: i32) -> Value {
    z["offset"] = json!({ "forward": forward, "right": right });
    z
}

/// One attack phase that spawns `nodes` on its first tick.
fn spawning(nodes: Vec<Value>) -> Vec<Value> {
    vec![json!({ "kind": "wind_up", "ticks": 1, "damage": 5, "spawn": nodes })]
}

fn attack(target: &str, phases: Vec<Value>) -> Value {
    json!([{ "when": always(), "target": target, "once": true, "cooldown_ticks": 0,
             "do": { "type": "attack", "attack": { "inline": phases } } }])
}

fn still_enemy(hp: u32, x: i32, y: i32, rules: Value) -> Value {
    wave(1, 1, 0, enemy(hp, hold(x, y), rules), x, y)
}

/// A companion holding still at the corner, attacking the nearest enemy once with `phases`.
fn fighter(phases: Vec<Value>) -> Value {
    companion(50, hold(CX, CY), attack("nearest_enemy", phases))
}

fn stage(waves: Vec<Value>, companions: Vec<Value>) -> Stage {
    quiet(stage_full(18_000, Some(far_boss()), waves, companions))
}

/// A foe far from the main character, which sends `phases` at it once.
fn foe_attacking_player(phases: Vec<Value>) -> Value {
    still_enemy(50, START_X, START_Y - 100 * PX, attack("player", phases))
}

fn player_hit_ticks(stage: &Stage, input: Input, ticks: u32) -> Vec<u32> {
    let events = run_until(&mut engine(stage), input, ticks, |_| false);
    ticks_of(&events, |ev| matches!(ev, DomainEvent::PlayerHit { .. }))
}

/// `(tick, target)` of every hit by the companion.
fn hits_by_companion(stage: &Stage, ticks: u32) -> Vec<(u32, u32)> {
    let events = run_until(&mut engine(stage), idle(), ticks, |_| false);
    events
        .into_iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::Hit { source, target, .. } if source == COMPANION => Some((t, target.0)),
            _ => None,
        })
        .collect()
}

fn zones_of(e: &Engine) -> Vec<Value> {
    let snapshot = serde_json::to_value(e.snapshot()).unwrap();
    snapshot["zones"].as_array().cloned().unwrap_or_default()
}

// --- Anchors ---------------------------------------------------------------------------------

#[test]
fn a_target_spot_zone_hits_a_target_that_stays_and_misses_one_that_left() {
    // The zone is placed on tick 1 where the main character stands, warns for 10 ticks and is
    // active from tick 11 to 15. The main character is 3 pixels a tick away by then.
    let st = stage(
        vec![foe_attacking_player(spawning(vec![zone(
            circle(4 * PX),
            "target",
            10,
            5,
        )]))],
        vec![],
    );
    assert_eq!(player_hit_ticks(&st, idle(), 30), [11]);
    assert_eq!(
        player_hit_ticks(&st, moving(-127, 0), 30),
        Vec::<u32>::new()
    );
}

#[test]
fn a_zone_anchored_at_the_attacker_stays_where_it_was_placed() {
    // The foe stands 100 pixels above the main character and does not move: a circle of 8
    // pixels at the foe does not reach it, one of 110 pixels does.
    let st = |radius: i32| {
        stage(
            vec![foe_attacking_player(spawning(vec![zone(
                circle(radius * PX),
                "attacker",
                2,
                3,
            )]))],
            vec![],
        )
    };
    assert_eq!(player_hit_ticks(&st(8), idle(), 20), Vec::<u32>::new());
    assert_eq!(player_hit_ticks(&st(110), idle(), 20), [3]);
}

#[test]
fn a_hit_box_anchored_at_the_target_hits_where_the_target_stood() {
    let swing = |anchor: &str| {
        vec![
            json!({ "kind": "wind_up", "ticks": 10 }),
            json!({ "kind": "active", "ticks": 1, "damage": 5,
                    "hit_box": { "type": "circle", "anchor": anchor, "radius": 4 * PX } }),
        ]
    };
    let st = |anchor: &str| stage(vec![foe_attacking_player(swing(anchor))], vec![]);
    assert_eq!(player_hit_ticks(&st("target"), idle(), 30), [11]);
    assert_eq!(
        player_hit_ticks(&st("target"), moving(-127, 0), 30),
        Vec::<u32>::new()
    );
    // Anchored at the attacker, 100 pixels away, it reaches nothing.
    assert_eq!(
        player_hit_ticks(&st("attacker"), idle(), 30),
        Vec::<u32>::new()
    );
}

// --- The line --------------------------------------------------------------------------------

/// Four enemies round a companion at the corner: A below it (the nearest, so it is faced),
/// B beside A, C behind the companion and D beyond a 40-pixel line.
fn four_enemies() -> Vec<Value> {
    vec![
        still_enemy(50, CX, CY + 30 * PX, json!([])),
        still_enemy(50, CX + 20 * PX, CY + 30 * PX, json!([])),
        still_enemy(50, CX, CY - 35 * PX, json!([])),
        still_enemy(50, CX, CY + 60 * PX, json!([])),
    ]
}

#[test]
fn a_line_zone_hits_along_its_length_and_not_beside_it() {
    let st = stage(
        four_enemies(),
        vec![fighter(spawning(vec![zone(
            line(40 * PX, 6 * PX),
            "attacker",
            0,
            1,
        )]))],
    );
    // A is entity 3: only it lies along the line, in reach of its length and width.
    assert_eq!(hits_by_companion(&st, 3), [(1, 3)]);
}

#[test]
fn a_line_hit_box_hits_along_its_length_and_not_beside_it() {
    let swing = vec![json!({ "kind": "active", "ticks": 1, "damage": 5,
                              "hit_box": { "type": "line", "length": 40 * PX, "width": 6 * PX } })];
    let st = stage(four_enemies(), vec![fighter(swing)]);
    assert_eq!(hits_by_companion(&st, 3), [(1, 3)]);
}

#[test]
fn a_lines_reach_is_its_length_plus_the_targets_radius() {
    // The enemies' circles are 8 pixels: one centred 47 pixels along a 40-pixel line is touched,
    // one 49 pixels along is not.
    for (along, hit) in [(47, true), (49, false)] {
        let st = stage(
            vec![still_enemy(50, CX, CY + along * PX, json!([]))],
            vec![fighter(spawning(vec![zone(
                line(40 * PX, 6 * PX),
                "attacker",
                0,
                1,
            )]))],
        );
        assert_eq!(hits_by_companion(&st, 2).len(), usize::from(hit), "{along}");
    }
}

// --- Placing, timing and counting ------------------------------------------------------------

#[test]
fn a_ring_of_eight_zones_places_eight_zones_at_equal_angles() {
    let ring = json!({ "type": "ring", "count": 8,
                       "body": [with_offset(zone(circle(8 * PX), "target", 30, 10), 50 * PX, 0)] });
    let st = stage(
        vec![still_enemy(50, CX, CY + 40 * PX, json!([]))],
        vec![fighter(spawning(vec![ring]))],
    );
    let mut e = engine(&st);
    e.step(idle());
    let zones = zones_of(&e);
    assert_eq!(zones.len(), 8);
    let spot = (i64::from(CX), i64::from(CY + 40 * PX));
    let int = |v: &Value| v.as_i64().unwrap();
    // Each zone lies 50 pixels from the target's spot along its own direction (a unit vector
    // times the table's scale, so its squared length is the scale squared).
    let dirs: Vec<(i64, i64)> = zones
        .iter()
        .map(|z| {
            let d = (int(&z["direction"]["x"]), int(&z["direction"]["y"]));
            let scale2 = d.0 * d.0 + d.1 * d.1;
            let (dx, dy) = (int(&z["at"]["x"]) - spot.0, int(&z["at"]["y"]) - spot.1);
            let along = (dx * d.0 + dy * d.1) / scale2.isqrt();
            assert!((along - i64::from(50 * PX)).abs() <= 2, "{z}");
            assert!((dx * d.1 - dy * d.0).abs() / scale2.isqrt() <= 2, "{z}");
            d
        })
        .collect();
    // Eight directions a turn apart, 45 degrees each: every pair is a multiple of 45 degrees
    // from the other, so the pairs by the cosine of their angle (in thousandths) fall into the
    // 8 of 45, 8 of 90, 8 of 135 and 4 of 180 degrees that eight equal angles make.
    let mut by_angle = [0; 4];
    for (i, a) in dirs.iter().enumerate() {
        for b in &dirs[i + 1..] {
            let cos = (a.0 * b.0 + a.1 * b.1) * 1000 / (a.0 * a.0 + a.1 * a.1);
            let bucket = match cos {
                700..=714 => 0,
                -3..=3 => 1,
                -714..=-700 => 2,
                -1000..=-997 => 3,
                other => panic!("a pair of zones at cosine {other}/1000: {dirs:?}"),
            };
            by_angle[bucket] += 1;
        }
    }
    assert_eq!(by_angle, [8, 8, 8, 4], "{dirs:?}");
}

#[test]
fn a_zone_warns_then_hurts_then_ends() {
    let st = stage(
        vec![still_enemy(50, CX, CY + 30 * PX, json!([]))],
        vec![fighter(spawning(vec![zone(
            circle(8 * PX),
            "target",
            3,
            2,
        )]))],
    );
    let mut e = engine(&st);
    let mut seen = Vec::new();
    for _ in 0..8 {
        e.step(idle());
        seen.push(zones_of(&e).first().map(|z| {
            (
                z["phase"].as_str().unwrap().to_owned(),
                z["ticks_left"].as_u64().unwrap(),
            )
        }));
    }
    let some = |p: &str, left: u64| Some((p.to_owned(), left));
    // Placed on tick 1: ticks 1 to 3 warn, ticks 4 and 5 are active, then it is gone.
    assert_eq!(
        seen,
        [
            some("wind_up", 3),
            some("wind_up", 2),
            some("wind_up", 1),
            some("active", 2),
            some("active", 1),
            None,
            None,
            None,
        ]
    );
}

#[test]
fn a_zone_hits_a_target_once_only() {
    let st = stage(
        vec![still_enemy(50, CX, CY + 30 * PX, json!([]))],
        vec![fighter(spawning(vec![zone(
            circle(8 * PX),
            "target",
            2,
            5,
        )]))],
    );
    // Active for 5 ticks, from tick 3: one hit.
    assert_eq!(hits_by_companion(&st, 12), [(3, 3)]);
}

#[test]
fn a_repeat_places_a_zone_each_round() {
    let repeat = json!({ "type": "repeat", "count": 3, "interval_ticks": 4,
                         "body": [zone(circle(8 * PX), "target", 1, 1)] });
    let st = stage(
        vec![still_enemy(50, CX, CY + 30 * PX, json!([]))],
        vec![fighter(spawning(vec![repeat]))],
    );
    let hits = hits_by_companion(&st, 20);
    assert_eq!(hits.len(), 3, "{hits:?}");
}

#[test]
fn zones_over_the_cap_are_dropped() {
    let many = json!({ "type": "repeat", "count": 3, "interval_ticks": 0,
                       "body": [{ "type": "ring", "count": 100,
                                  "body": [zone(circle(PX), "attacker", 200, 50)] }] });
    let st = stage(vec![], vec![fighter(spawning(vec![many]))]);
    let mut e = engine(&st);
    e.step(idle());
    assert_eq!(zones_of(&e).len(), MAX_ZONES_ALIVE);
    // The same again, and the same state hash: a drop is not luck.
    let mut again = engine(&st);
    again.step(idle());
    assert_eq!(e.state_hash(), again.state_hash());
}

// --- clear_zones -----------------------------------------------------------------------------

#[test]
fn clear_zones_removes_the_enemy_zones_when_a_boss_phase_begins() {
    let zone_attack = json!({ "type": "attack", "attack": { "inline": spawning(vec![
        zone(circle(4 * PX), "attacker", 300, 10),
    ]) } });
    let boss = json!({ "base": "golem", "hp": 1_000_000, "radius": 1000,
                       "spawn": { "x": 0, "y": 0 },
                       "phases": [
                           { "until": { "type": "time_above", "ticks": 19 },
                             "timeline": [zone_attack] },
                           { "transition": [{ "type": "clear_zones" }] },
                       ] });
    let st = quiet(stage_full(18_000, Some(boss), vec![], vec![]));
    let mut e = engine(&st);
    for _ in 0..10 {
        e.step(idle());
    }
    assert_eq!(zones_of(&e).len(), 1, "the boss has placed a zone");
    for _ in 0..10 {
        e.step(idle());
    }
    assert_eq!(e.tick(), 20);
    assert!(zones_of(&e).is_empty());
}

// --- Validation ------------------------------------------------------------------------------

fn issues_of(phases: Vec<Value>) -> Vec<Issue> {
    let st = stage_full(
        18_000,
        Some(far_boss()),
        vec![still_enemy(50, 0, 0, attack("player", phases))],
        vec![],
    );
    match validate(&st) {
        Ok(()) => Vec::new(),
        Err(Error::Invalid(issues)) => issues,
        Err(other) => panic!("unexpected {other:?}"),
    }
}

const ZONE_AT: &str = "$.waves[0].enemy.rules[0].do.attack.inline[0].spawn[0]";

#[test]
fn zone_fields_are_bounded_at_the_limit_and_one_past_it() {
    let r = MAX_RADIUS;
    let half = 180 * 256;
    let valid = [
        zone(circle(1), "attacker", 0, 1),
        zone(circle(r), "target", MAX_ZONE_TICKS, MAX_ZONE_TICKS),
        zone(line(r, r), "target", 1, 1),
        zone(line(1, 1), "attacker", 1, 1),
        zone(
            json!({ "type": "arc", "radius": r, "half_angle": half }),
            "attacker",
            1,
            1,
        ),
        with_offset(zone(circle(1), "attacker", 1, 1), r, -r),
    ];
    for z in valid {
        let found = issues_of(spawning(vec![z.clone()]));
        assert!(found.is_empty(), "{z}: {found:?}");
    }
    let arc = |half_angle: i32| json!({ "type": "arc", "radius": 256, "half_angle": half_angle });
    let invalid = [
        (zone(circle(0), "attacker", 1, 1), "shape.radius"),
        (zone(circle(r + 1), "attacker", 1, 1), "shape.radius"),
        (zone(line(0, 256), "attacker", 1, 1), "shape.length"),
        (zone(line(r + 1, 256), "attacker", 1, 1), "shape.length"),
        (zone(line(256, 0), "attacker", 1, 1), "shape.width"),
        (zone(line(256, r + 1), "attacker", 1, 1), "shape.width"),
        (zone(arc(-1), "attacker", 1, 1), "shape.half_angle"),
        (zone(arc(half + 1), "attacker", 1, 1), "shape.half_angle"),
        (
            zone(circle(256), "attacker", MAX_ZONE_TICKS + 1, 1),
            "wind_up_ticks",
        ),
        (zone(circle(256), "attacker", 1, 0), "active_ticks"),
        (
            zone(circle(256), "attacker", 1, MAX_ZONE_TICKS + 1),
            "active_ticks",
        ),
        (
            with_offset(zone(circle(256), "attacker", 1, 1), r + 1, 0),
            "offset.forward",
        ),
        (
            with_offset(zone(circle(256), "attacker", 1, 1), 0, -r - 1),
            "offset.right",
        ),
    ];
    for (z, key) in invalid {
        let found = issues_of(spawning(vec![z.clone()]));
        let first = found.first().unwrap_or_else(|| panic!("{z} is valid"));
        assert_eq!(first.path, format!("{ZONE_AT}.{key}"), "{z}");
    }
}

#[test]
fn a_line_hit_box_is_bounded_like_a_line_zone() {
    let at = |key: &str| format!("$.waves[0].enemy.rules[0].do.attack.inline[0].hit_box.{key}");
    let boxed = |length: i32, width: i32| {
        vec![json!({ "kind": "active", "ticks": 1,
                     "hit_box": { "type": "line", "length": length, "width": width } })]
    };
    assert!(issues_of(boxed(MAX_RADIUS, MAX_RADIUS)).is_empty());
    for (length, width, key) in [
        (0, 256, "length"),
        (MAX_RADIUS + 1, 256, "length"),
        (256, 0, "width"),
        (256, MAX_RADIUS + 1, "width"),
    ] {
        let found = issues_of(boxed(length, width));
        assert_eq!(found.first().map(|i| i.path.clone()), Some(at(key)));
    }
}

// --- The snapshot carries the phase length ---------------------------------------------------

#[test]
fn a_zone_view_carries_the_length_of_its_current_phase() {
    let s = stage(
        vec![foe_attacking_player(spawning(vec![zone(
            circle(20 * PX),
            "attacker",
            30,
            12,
        )]))],
        vec![],
    );
    let mut e = engine(&s);
    // The zone is placed on the first step; 10 more ticks of its 30-tick warning pass.
    e.step(idle());
    for _ in 0..10 {
        e.step(idle());
    }
    let z = e.snapshot().zones.remove(0);
    assert_eq!(z.phase, stella_rain_core::attack::PhaseKind::WindUp);
    assert_eq!(z.ticks, 30);
    assert_eq!(z.ticks_left, 20);
    assert!(z.ticks >= z.ticks_left);
    // Once active, `ticks` is the active length.
    for _ in 0..25 {
        e.step(idle());
    }
    let z = e.snapshot().zones.remove(0);
    assert_eq!(z.phase, stella_rain_core::attack::PhaseKind::Active);
    assert_eq!(z.ticks, 12);
    assert!(z.ticks >= z.ticks_left);
}

// --- The source of a hit (PHIT) --------------------------------------------------------------

/// The `source` of the first `PlayerHit` over `ticks` idle ticks, as JSON.
fn first_hit_source(stage: &Stage, ticks: u32) -> Value {
    let events = run_until(&mut engine(stage), idle(), ticks, |_| false);
    events
        .into_iter()
        .find_map(|(_, ev)| match ev {
            DomainEvent::PlayerHit { source, .. } => Some(serde_json::to_value(source).unwrap()),
            _ => None,
        })
        .expect("the main character is hit")
}

#[test]
fn a_zone_hit_names_the_zone_kind_and_its_owner() {
    let st = stage(
        vec![foe_attacking_player(spawning(vec![zone(
            circle(4 * PX),
            "target",
            10,
            5,
        )]))],
        vec![],
    );
    // The foe is entity 2 (the far boss is 1); an inline attack has no preset.
    assert_eq!(
        first_hit_source(&st, 30),
        json!({ "kind": "zone", "owner": 2 })
    );
}

#[test]
fn a_zone_a_preset_placed_names_the_preset() {
    let rules = json!([attack_rule(always(), 0)]);
    let st = stage(
        vec![still_enemy(50, START_X, START_Y - 100 * PX, rules)],
        vec![],
    );
    assert_eq!(
        first_hit_source(&st, 60),
        json!({ "kind": "zone", "owner": 2, "attack": "aimed_single" })
    );
}
