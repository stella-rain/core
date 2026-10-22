//! A hit names its damage type and a target its kind id (core#D6e), so the app's Bestiary can
//! record every hit exactly: weapon hits, runes and the stage skill alike. The kind id is the
//! content ID that names an enemy kind (its sprite), a boss, a boss part or a companion; the
//! main character has none, and a mimic still in disguise shows none.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::attack::DamageType;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::{ContentId, EntityId};
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::Stage;

const CX: i32 = 20_000;
const CY: i32 = 30_000;
/// The companion is entity 2 (the far boss is 1); the first enemy is 3.
const COMPANION: EntityId = EntityId(2);
const FOE: EntityId = EntityId(3);

/// A one-tick strike of 10, typed `ty` (or neutral), over the target next to the striker.
fn strike(ty: Option<&str>) -> Value {
    let mut phase = json!({ "kind": "active", "ticks": 1, "damage": 10,
                            "hit_box": { "type": "circle", "radius": 4_096 } });
    if let Some(ty) = ty {
        phase["type"] = json!(ty);
    }
    json!({ "type": "attack", "attack": { "inline": [phase] } })
}

/// A companion at the corner strikes the 10-hp enemy next to it once.
fn striker(ty: Option<&str>, hp: u32) -> Stage {
    let rules = json!([{ "when": always(), "target": "nearest_enemy", "once": true,
                         "do": strike(ty) }]);
    let foe = wave(
        1,
        1,
        0,
        enemy(hp, hold(CX + 2_560, CY), json!([])),
        CX + 2_560,
        CY,
    );
    stage_full(
        18_000,
        Some(far_boss()),
        vec![foe],
        vec![companion(50, hold(CX, CY), rules)],
    )
}

fn events_of(stage: &Stage, ticks: u32) -> Vec<DomainEvent> {
    run_until(&mut engine(stage), idle(), ticks, |_| false)
        .into_iter()
        .map(|(_, ev)| ev)
        .collect()
}

#[test]
fn a_hit_names_the_damage_type_it_was_dealt_in() {
    for (name, ty) in [
        ("slash", DamageType::Slash),
        ("electro", DamageType::Electro),
    ] {
        let types: Vec<_> = events_of(&striker(Some(name), 100), 3)
            .into_iter()
            .filter_map(|ev| match ev {
                DomainEvent::Hit {
                    source,
                    damage_type,
                    ..
                } if source == COMPANION => Some(damage_type),
                _ => None,
            })
            .collect();
        assert_eq!(types, [Some(ty)], "{name}");
    }
}

#[test]
fn a_neutral_hit_names_no_type_and_leaves_the_field_out_of_the_json() {
    let hit = events_of(&striker(None, 100), 3)
        .into_iter()
        .find(|ev| matches!(ev, DomainEvent::Hit { source, .. } if *source == COMPANION))
        .expect("a hit");
    assert!(matches!(
        hit,
        DomainEvent::Hit {
            damage_type: None,
            ..
        }
    ));
    let json = serde_json::to_value(&hit).unwrap();
    assert!(json["data"].get("damage_type").is_none(), "{json}");
}

#[test]
fn a_hit_names_the_kind_of_its_target() {
    let kinds: Vec<_> = events_of(&striker(Some("pyro"), 100), 3)
        .into_iter()
        .filter_map(|ev| match ev {
            DomainEvent::Hit {
                target,
                target_kind,
                ..
            } if target == FOE => Some(target_kind),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, [Some(ContentId("bat".into()))]);
}

#[test]
fn a_kill_names_the_kind_of_the_victim() {
    let died: Vec<_> = events_of(&striker(Some("pyro"), 10), 3)
        .into_iter()
        .filter_map(|ev| match ev {
            DomainEvent::Died { entity, kind } => Some((entity, kind)),
            _ => None,
        })
        .collect();
    assert_eq!(died, [(FOE, Some(ContentId("bat".into())))]);
}

#[test]
fn every_entity_in_the_snapshot_carries_its_kind_id() {
    let mut e = engine(&striker(None, 100));
    e.step(idle());
    e.step(idle());
    let views = e.snapshot().entities;
    let kind_of = |kind| {
        views
            .iter()
            .find(|v| v.kind == kind)
            .unwrap_or_else(|| panic!("no {kind:?}"))
            .kind_id
            .clone()
    };
    assert_eq!(kind_of(EntityKind::Enemy), Some(ContentId("bat".into())));
    assert_eq!(kind_of(EntityKind::Boss), Some(ContentId("golem".into())));
    assert_eq!(
        kind_of(EntityKind::Companion),
        Some(ContentId("fairy_a".into()))
    );
}
