//! A death names what died (DC1): its class, as the snapshot's `EntityKind`, and whether it was
//! an elite (a shrine is its own class), so a reader never has to guess it from earlier snapshots.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::Stage;

const CX: i32 = 20_000;
const CY: i32 = 30_000;

/// What each `Died` in `events` says: the entity, its class and the elite mark.
pub fn deaths(events: &[(u32, DomainEvent)]) -> Vec<(EntityId, EntityKind, bool)> {
    events
        .iter()
        .filter_map(|(_, ev)| match ev {
            DomainEvent::Died {
                entity,
                class,
                elite,
                ..
            } => Some((*entity, *class, *elite)),
            _ => None,
        })
        .collect()
}

/// A companion at the corner strikes the `foe` next to it once, in a one-tick strike of 10.
fn striker(foe: Value) -> Stage {
    let strike = json!({ "type": "attack", "attack": { "inline": [{
        "kind": "active", "ticks": 1, "damage": 10,
        "hit_box": { "type": "circle", "radius": 4_096 } }] } });
    let rules = json!([{ "when": always(), "target": "nearest_enemy", "once": true,
                         "do": strike }]);
    stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, foe, CX + 2_560, CY)],
        vec![companion(50, hold(CX, CY), rules)],
    )
}

fn events_of(e: &mut Engine, ticks: u32) -> Vec<(u32, DomainEvent)> {
    run_until(e, idle(), ticks, |_| false)
}

#[test]
fn an_elite_dying_on_its_first_tick_reports_elite() {
    let mut foe = enemy(10, hold(CX + 2_560, CY), json!([]));
    foe["elite"] = json!({ "range": 256 });
    let stage = striker(foe);
    let mut e = engine(&stage);
    let events = events_of(&mut e, 3);
    // It never showed in a snapshot as anything but the elite it was born as.
    assert_eq!(
        deaths(&events),
        [(EntityId(3), EntityKind::Enemy, true)],
        "{events:?}"
    );
}

#[test]
fn an_ordinary_enemy_reports_enemy_and_not_elite() {
    let stage = striker(enemy(10, hold(CX + 2_560, CY), json!([])));
    let mut e = engine(&stage);
    let events = events_of(&mut e, 3);
    assert_eq!(
        deaths(&events),
        [(EntityId(3), EntityKind::Enemy, false)],
        "{events:?}"
    );
}

#[test]
fn a_companion_death_reports_companion() {
    let shooter = enemy(
        50,
        hold(46_080, 100_000),
        json!([rule(
            always(),
            json!("nearest_ally"),
            json!({ "type": "attack",
                    "attack": { "preset": { "id": "aimed_single", "args": [768] } } }),
            0
        )]),
    );
    let stage = stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, shooter, 46_080, 100_000)],
        vec![companion(3, hold(46_080, 130_000), json!([]))],
    );
    let mut e = engine(&stage);
    let events = events_of(&mut e, 120);
    assert_eq!(
        deaths(&events),
        [(EntityId(2), EntityKind::Companion, false)],
        "{events:?}"
    );
}

#[test]
fn a_boss_death_reports_boss() {
    // 20 hp, 10 a hit, straight above the main character.
    let boss = json!({ "base": "golem", "hp": 20, "radius": 8192,
                       "spawn": { "x": 46_080, "y": 60_000 }, "phases": [{}] });
    let stage = stage_full(18_000, Some(boss), vec![], vec![]);
    let mut e = engine(&stage);
    let events = fire_until(&mut e, idle(), 200, |_| false);
    assert_eq!(
        deaths(&events),
        [(EntityId(1), EntityKind::Boss, false)],
        "{events:?}"
    );
}

#[test]
fn the_class_is_in_the_json_and_a_plain_enemy_leaves_the_mark_out() {
    let died: DomainEvent = serde_json::from_value(json!({ "type": "Died", "data": {
        "entity": 3, "class": "enemy" } }))
    .unwrap();
    let json = serde_json::to_value(&died).unwrap();
    assert_eq!(json["data"]["class"], json!("enemy"));
    assert!(json["data"].get("elite").is_none(), "{json}");
    assert!(json["data"].get("shrine").is_none(), "{json}");
    let shrine: DomainEvent = serde_json::from_value(json!({ "type": "Died", "data": {
        "entity": 3, "class": "shrine", "elite": true } }))
    .unwrap();
    let json = serde_json::to_value(&shrine).unwrap();
    assert_eq!(json["data"]["class"], json!("shrine"));
    assert!(json["data"].get("shrine").is_none(), "{json}");
}
