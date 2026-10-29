//! The leash (EC2, DS80, DS106): a regular enemy outside the reach rectangle around the main
//! character (|dx| > 840 px or |dy| > 360 px) moves to the rectangle of ±484 × ±244 px on the
//! opposite side of it, scaled along its own direction and clamped to the map, at most 16 a tick,
//! in entity-ID order; and `Load` counts the relocations and the spawns the caps refused.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::fixed::{Fx, Point};
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::{Map, Stage};

const PX: i32 = 256;

/// A 4,000 by 4,000 px map with the main character starting at (`x`, 2,000) px.
fn stage_at(x: i32, waves: Vec<Value>, companions: Vec<Value>, boss: Option<Value>) -> Stage {
    stage_at_xy(x, 2000, waves, companions, boss)
}

/// The same map with the main character starting at (`x`, `y`) px.
fn stage_at_xy(
    x: i32,
    y: i32,
    waves: Vec<Value>,
    companions: Vec<Value>,
    boss: Option<Value>,
) -> Stage {
    let mut stage = quiet(stage_full(18_000, boss, waves, companions));
    stage.map = Some(Map {
        width: Fx::px(4000),
        height: Fx::px(4000),
        start: Some(Point {
            x: Fx::px(x),
            y: Fx::px(y),
        }),
        facing: None,
    });
    stage
}

/// A regular enemy that spawns at (`x`, 2,000) px and stands still: `straight` at 0, 0, since a
/// `hold` enemy is anchored and the leash leaves it where it is (EC2b).
fn still(x: i32) -> Value {
    still_at(x, 2000)
}

/// The same enemy at (`x`, `y`) px.
fn still_at(x: i32, y: i32) -> Value {
    wave(
        0,
        1,
        1,
        enemy(
            10,
            json!({ "type": "straight", "vx": 0, "vy": 0 }),
            json!([]),
        ),
        x * PX,
        y * PX,
    )
}

fn spot(e: &Engine, id: u32) -> (i32, i32) {
    let v = e
        .snapshot()
        .entities
        .into_iter()
        .find(|v| v.id == EntityId(id))
        .unwrap_or_else(|| panic!("no entity {id}"));
    (v.at.x.0, v.at.y.0)
}

fn relocated(e: &Engine) -> Vec<EntityId> {
    e.events()
        .iter()
        .filter_map(|ev| match ev {
            DomainEvent::EnemyRelocated { entity } => Some(*entity),
            _ => None,
        })
        .collect()
}

/// Steps once and returns where the lone enemy 1 is, with the main character at (`px`, `py`) px
/// and the enemy at (`ex`, `ey`) px.
fn lands(px: i32, py: i32, ex: i32, ey: i32) -> (i32, i32) {
    let mut e = engine(&stage_at_xy(px, py, vec![still_at(ex, ey)], vec![], None));
    e.step(idle());
    spot(&e, 1)
}

#[test]
fn an_enemy_841_px_to_the_side_moves_and_keeps_its_hit_points() {
    let mut e = engine(&stage_at(2000, vec![still(2841)], vec![], None));
    e.step(idle());
    let id = e
        .snapshot()
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Enemy)
        .unwrap()
        .id;
    assert_eq!(spot(&e, id.0), (1516 * PX, 2000 * PX));
    assert_eq!(
        e.snapshot()
            .entities
            .iter()
            .find(|v| v.id == id)
            .unwrap()
            .hp,
        10
    );
    assert_eq!(e.load().relocated, 1);
    assert_eq!(relocated(&e), vec![id]);
}

#[test]
fn an_enemy_840_px_to_the_side_stays() {
    let mut e = engine(&stage_at(2000, vec![still(2840)], vec![], None));
    e.step(idle());
    assert_eq!(spot(&e, 1), (2840 * PX, 2000 * PX));
    assert_eq!(e.load().relocated, 0);
    assert!(relocated(&e).is_empty());
}

#[test]
fn the_trigger_on_y_is_360_px() {
    assert_eq!(lands(2000, 2000, 2000, 2361), (2000 * PX, 1756 * PX));
    assert_eq!(lands(2000, 2000, 2000, 2360), (2000 * PX, 2360 * PX));
}

#[test]
fn the_trigger_is_a_rectangle_not_a_circle() {
    // 689 px away, inside the rectangle; and 841 px away, outside it.
    assert_eq!(lands(2000, 2000, 2600, 2340), (2600 * PX, 2340 * PX));
    assert_eq!(lands(2000, 2000, 2841, 2000), (1516 * PX, 2000 * PX));
}

#[test]
fn landing_on_an_axis_is_484_or_244_px_on_the_other_side() {
    assert_eq!(lands(2000, 2000, 2841, 2000), (1516 * PX, 2000 * PX));
    assert_eq!(lands(2000, 2000, 1159, 2000), (2484 * PX, 2000 * PX));
    assert_eq!(lands(2000, 2000, 2000, 2361), (2000 * PX, 1756 * PX));
    assert_eq!(lands(2000, 2000, 2000, 1639), (2000 * PX, 2244 * PX));
}

#[test]
fn landing_is_scaled_along_the_enemys_own_direction() {
    // Offset (800, 400) px lands at (-484, -242) px exactly, on the vertical sides.
    assert_eq!(lands(2000, 2000, 2800, 2400), (1516 * PX, 1758 * PX));
}

#[test]
fn landing_on_the_horizontal_sides_truncates_toward_zero() {
    // Offset (300, 700) px: oy = -244 px, ox = -76,800 * 62,464 / 179,200 = -26,770.29 -> -26,770.
    assert_eq!(lands(2000, 2000, 2300, 2700), (485_230, 449_536));
    assert_eq!(lands(2000, 2000, 1700, 1300), (538_770, 574_464));
}

#[test]
fn a_relocation_near_the_map_edge_lands_on_the_map() {
    assert_eq!(lands(100, 2000, 941, 2000), (0, 2000 * PX));
    assert_eq!(lands(2000, 100, 2000, 461), (2000 * PX, 0));
}

#[test]
fn at_most_16_move_a_tick_the_lowest_ids_first() {
    let waves: Vec<Value> = (0..20).map(|_| still(3000)).collect();
    let mut e = engine(&stage_at(2000, waves, vec![], None));
    e.step(idle());
    let first = relocated(&e);
    assert_eq!(first, (1..=16).map(EntityId).collect::<Vec<_>>());
    assert_eq!(e.load().relocated, 16);
    e.step(idle());
    assert_eq!(relocated(&e), (17..=20).map(EntityId).collect::<Vec<_>>());
    assert_eq!(e.load().relocated, 4);
}

#[test]
fn a_relocation_draws_no_random_number() {
    // The same stage with the enemy 841 px away (relocated) or 840 px (not), and a wave that
    // spawns on a ring drawn from the spawn stream later: it lands in the same place.
    let mut ring = wave(
        5,
        1,
        1,
        enemy(10, hold(2000 * PX, 2000 * PX), json!([])),
        0,
        0,
    );
    ring.as_object_mut().unwrap().remove("spawn");
    ring["edge"] = json!({ "side": "any" });
    let at = |x: i32| {
        let mut e = engine(&stage_at(2000, vec![still(x), ring.clone()], vec![], None));
        for _ in 0..6 {
            e.step(idle());
        }
        let id = e
            .snapshot()
            .entities
            .iter()
            .filter(|v| v.kind == EntityKind::Enemy)
            .map(|v| v.id)
            .max()
            .unwrap();
        e.snapshot()
            .entities
            .into_iter()
            .find(|v| v.id == id)
            .unwrap()
            .at
    };
    assert_eq!(at(2841), at(2840));
}

#[test]
fn an_elite_the_boss_a_companion_and_an_enemy_in_a_wind_up_stay() {
    let mut elite = enemy(
        10,
        json!({ "type": "hold", "at": { "x": 3000 * PX, "y": 2000 * PX } }),
        json!([]),
    );
    elite["elite"] = json!({ "range": 256 });
    // Starts its wind-up on its first tick at 700 px, and the main character then walks away.
    let winder = enemy(
        10,
        json!({ "type": "straight", "vx": 0, "vy": 0 }),
        json!([rule(
            always(),
            json!("player"),
            json!({ "type": "attack", "attack": { "inline": [{ "kind": "wind_up", "ticks": 600 }] } }),
            0
        )]),
    );
    let mut e = engine(&stage_at(
        2000,
        vec![
            wave(0, 1, 1, elite, 3000 * PX, 2000 * PX),
            wave(0, 1, 1, winder, 2700 * PX, 2000 * PX),
        ],
        vec![companion(50, hold(3000 * PX, 2000 * PX), json!([]))],
        Some(json!({ "base": "golem", "hp": 1_000_000, "radius": 1000,
                     "spawn": { "x": 3000 * PX, "y": 3000 * PX }, "phases": [{}] })),
    ));
    let before: Vec<_> = {
        e.step(idle());
        e.snapshot().entities.iter().map(|v| (v.id, v.at)).collect()
    };
    for _ in 0..80 {
        e.step(moving(-127, 0));
        assert_eq!(e.load().relocated, 0);
        assert!(relocated(&e).is_empty());
    }
    let snap = e.snapshot();
    assert!(
        snap.player.at.x.0 < (2000 - 100) * PX,
        "the main character walked far from the winder: {:?}",
        snap.player.at
    );
    for (id, at) in before {
        assert_eq!(
            snap.entities.iter().find(|v| v.id == id).unwrap().at,
            at,
            "{id:?}"
        );
    }
    let winder = snap
        .entities
        .iter()
        .find(|v| v.attack.is_some())
        .expect("the winder is in a wind-up");
    assert_eq!(winder.kind, EntityKind::Enemy);
}

#[test]
fn a_refused_spawn_is_counted_in_load() {
    // Ten waves of 26 fill the map with the 260 enemies of the cap by tick 26, 300 px from the main
    // character.
    let mut waves: Vec<Value> = (0..10)
        .map(|_| {
            wave(
                0,
                26,
                1,
                enemy(10, hold(2300 * PX, 2000 * PX), json!([])),
                2300 * PX,
                2000 * PX,
            )
        })
        .collect();
    waves.push(wave(
        60,
        1,
        1,
        enemy(10, hold(2300 * PX, 2000 * PX), json!([])),
        2300 * PX,
        2000 * PX,
    ));
    let mut e = engine(&stage_at(2000, waves, vec![], None));
    for _ in 0..10 {
        e.step(idle());
    }
    assert_eq!(e.load().refused_spawns, 0, "room, nothing due");
    for _ in 10..59 {
        e.step(idle());
    }
    assert_eq!(e.load().enemies, 260);
    assert_eq!(e.load().refused_spawns, 0, "full, but nothing due");
    e.step(idle());
    assert_eq!(e.tick(), 60);
    assert_eq!(e.load().refused_spawns, 1);
    assert_eq!(e.load().enemies, 260);
}

// --- Who the leash leaves where they are (EC2b, tasks#53) ------------------------------------

fn standing() -> Value {
    json!({ "type": "straight", "vx": 0, "vy": 0 })
}

/// Steps `ticks` ticks with `input`; the ids relocated in any of them.
fn run_relocating(e: &mut Engine, ticks: u32, input: Input) -> Vec<EntityId> {
    let mut all = Vec::new();
    for _ in 0..ticks {
        e.step(input);
        all.extend(relocated(e));
    }
    all
}

/// The enemies outside the reach rectangle around the main character.
fn far_enemies(e: &Engine) -> usize {
    let snap = e.snapshot();
    snap.entities
        .iter()
        .filter(|v| v.kind == EntityKind::Enemy)
        .filter(|v| {
            let dx = i64::from(v.at.x.0) - i64::from(snap.player.at.x.0);
            let dy = i64::from(v.at.y.0) - i64::from(snap.player.at.y.0);
            dx.abs() > i64::from(840 * PX) || dy.abs() > i64::from(360 * PX)
        })
        .count()
}

fn once_summoning(agent: Value) -> Value {
    json!({ "when": always(), "target": "player", "once": true, "cooldown_ticks": 0,
            "do": { "type": "summon", "count": 2, "agent": agent } })
}

#[test]
fn a_holding_enemy_1100_px_from_its_anchor_is_never_relocated() {
    let held = wave(
        0,
        1,
        1,
        enemy(10, hold(3100 * PX, 2000 * PX), json!([])),
        2900 * PX,
        2000 * PX,
    );
    let mut e = engine(&stage_at(2000, vec![held], vec![], None));
    assert!(run_relocating(&mut e, 300, idle()).is_empty());
    assert_eq!(e.load().relocated, 0);
    assert_eq!(spot(&e, 1), (3100 * PX, 2000 * PX));
}

#[test]
fn an_enemy_with_a_move_to_rule_is_never_relocated() {
    let mover = enemy(
        10,
        standing(),
        json!([rule(
            always(),
            json!("self"),
            json!({ "type": "move_to", "to": { "x": 3300 * PX, "y": 2000 * PX }, "ticks": 60 }),
            30
        )]),
    );
    let w = wave(0, 1, 1, mover, 2100 * PX, 2000 * PX);
    let mut e = engine(&stage_at(2000, vec![w], vec![], None));
    assert!(run_relocating(&mut e, 300, idle()).is_empty());
    assert!(far_enemies(&e) > 0, "it did go beyond the rectangle");
}

#[test]
fn a_shrines_guardians_are_never_relocated() {
    let mut shrine = enemy(
        10,
        json!({ "type": "shrine", "fight_ticks": 5000, "guardians": {
            "enemy": enemy(100_000, standing(), json!([])), "count": 3, "radius": 10_240 } }),
        json!([]),
    );
    shrine["radius"] = json!(3072);
    shrine["elite"] = json!({ "range": 256, "boss_call": "stay" });
    let w = wave(1, 1, 0, shrine, 2000 * PX, 2000 * PX);
    let mut e = engine(&stage_at(2000, vec![w], vec![], None));
    e.step(idle());
    assert!(run_relocating(&mut e, 400, moving(-127, 0)).is_empty());
    assert!(far_enemies(&e) >= 3, "the guardians are left far behind");
}

#[test]
fn a_pack_leaders_escort_is_never_relocated() {
    let mut leader = enemy(100_000, hold(3000 * PX, 2000 * PX), json!([]));
    leader["radius"] = json!(3072);
    leader["elite"] = json!({ "range": 256, "escort": {
        "enemy": enemy(5, standing(), json!([])), "count": 2, "radius": 10_240 } });
    let w = wave(0, 1, 1, leader, 3000 * PX, 2000 * PX);
    let mut e = engine(&stage_at(2000, vec![w], vec![], None));
    assert!(run_relocating(&mut e, 20, idle()).is_empty());
    assert!(far_enemies(&e) >= 2, "the escort");
}

#[test]
fn an_elites_summons_are_never_relocated() {
    let mut elite = enemy(
        100_000,
        hold(3000 * PX, 2000 * PX),
        json!([once_summoning(enemy(10, standing(), json!([])))]),
    );
    elite["elite"] = json!({ "range": 262_144 });
    let w = wave(0, 1, 1, elite, 3000 * PX, 2000 * PX);
    let mut e = engine(&stage_at(2000, vec![w], vec![], None));
    assert!(run_relocating(&mut e, 20, idle()).is_empty());
    assert!(far_enemies(&e) >= 2, "the two summons");
}

#[test]
fn the_bosss_summons_are_never_relocated() {
    let boss = json!({ "base": "golem", "hp": 1_000_000, "radius": 1000,
        "spawn": { "x": 3000 * PX, "y": 2000 * PX },
        "phases": [{ "rules": [once_summoning(enemy(10, standing(), json!([])))] }] });
    let mut e = engine(&stage_at(2000, vec![], vec![], Some(boss)));
    assert!(run_relocating(&mut e, 20, idle()).is_empty());
    assert!(far_enemies(&e) >= 2, "the two summons");
}

#[test]
fn a_regular_enemys_summons_and_a_still_enemy_are_relocated() {
    // The summoner stands 839 px away; its summons drift outward at 1 px a tick.
    let drifting = json!({ "type": "straight", "vx": PX, "vy": 0 });
    let summoner = enemy(
        100_000,
        standing(),
        json!([once_summoning(enemy(10, drifting, json!([])))]),
    );
    let w = wave(0, 1, 1, summoner, 2839 * PX, 2000 * PX);
    let mut e = engine(&stage_at(2000, vec![w], vec![], None));
    let moved = run_relocating(&mut e, 10, idle());
    assert!(moved.iter().any(|id| id.0 > 1), "a summon moved: {moved:?}");

    let mut e = engine(&stage_at(2000, vec![still(3000)], vec![], None));
    assert_eq!(run_relocating(&mut e, 1, idle()), vec![EntityId(1)]);
}
