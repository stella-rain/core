//! The leash (EC2, DS80): a regular enemy left more than `LEASH_DISTANCE` (800 px) behind the main
//! character moves to the ring of `LEASH_RING` (400 px) on the opposite side of it, at most 16 a
//! tick, in entity-ID order; and `Load` counts the relocations and the spawns the caps refused.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::fixed::{Fx, Point};
use stella_rain_core::id::EntityId;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::{Map, Stage};

const PX: i32 = 256;

/// A 4,000 by 4,000 px map with the main character starting at (`x`, 2,000) px.
fn stage_at(x: i32, waves: Vec<Value>, companions: Vec<Value>, boss: Option<Value>) -> Stage {
    let mut stage = quiet(stage_full(18_000, boss, waves, companions));
    stage.map = Some(Map {
        width: Fx::px(4000),
        height: Fx::px(4000),
        start: Some(Point {
            x: Fx::px(x),
            y: Fx::px(2000),
        }),
        facing: None,
    });
    stage
}

/// A regular enemy that spawns at (`x`, 2,000) px and stands still: `straight` at 0, 0, since a
/// `hold` enemy is anchored and the leash leaves it where it is (EC2b).
fn still(x: i32) -> Value {
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
        2000 * PX,
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

#[test]
fn an_enemy_801_px_behind_moves_400_px_to_the_other_side() {
    let mut e = engine(&stage_at(2000, vec![still(2801)], vec![], None));
    e.step(idle());
    let id = e
        .snapshot()
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Enemy)
        .unwrap()
        .id;
    assert_eq!(spot(&e, id.0), (1600 * PX, 2000 * PX));
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
fn an_enemy_799_px_away_stays() {
    let mut e = engine(&stage_at(2000, vec![still(2799)], vec![], None));
    e.step(idle());
    assert_eq!(spot(&e, 1), (2799 * PX, 2000 * PX));
    assert_eq!(e.load().relocated, 0);
    assert!(relocated(&e).is_empty());
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
fn a_relocation_near_the_map_edge_lands_on_the_map() {
    let mut e = engine(&stage_at(100, vec![still(901)], vec![], None));
    e.step(idle());
    assert_eq!(spot(&e, 1), (0, 2000 * PX));
}

#[test]
fn a_relocation_draws_no_random_number() {
    // The same stage with the enemy 801 px away (relocated) or 799 px (not), and a wave that
    // spawns on a ring drawn from the spawn stream later: it lands in the same place.
    let mut ring = wave(
        5,
        1,
        1,
        enemy(10, hold(2000 * PX, 2000 * PX), json!([])),
        0,
        0,
    );
    ring["around"] = json!(102_400);
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
    assert_eq!(at(2801), at(2799));
}

#[test]
fn a_refused_spawn_is_counted_in_load() {
    // Ten waves of 50 fill the map with 500 enemies by tick 50, 300 px from the main character.
    let mut waves: Vec<Value> = (0..10)
        .map(|_| {
            wave(
                0,
                50,
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
    assert_eq!(e.load().enemies, 500);
    assert_eq!(e.load().refused_spawns, 0, "full, but nothing due");
    e.step(idle());
    assert_eq!(e.tick(), 60);
    assert_eq!(e.load().refused_spawns, 1);
    assert_eq!(e.load().enemies, 500);
}
