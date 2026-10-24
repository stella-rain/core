//! A hit names its damage type and a target its kind (core#D6e), so the app's Bestiary can
//! record every hit exactly: weapon hits, runes and the stage skill alike. The kind is an index
//! (`KindIndex`) into the run's `KindTable` of the content IDs that name an enemy kind (its
//! sprite), a boss, a boss part or a companion; the main character has none, and a mimic still
//! in disguise shows none.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::attack::DamageType;
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::{ContentId, EntityId};
use stella_rain_core::kinds::{KindIndex, KindTable, MAX_KINDS};
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

fn kind(e: &Engine, id: &str) -> Option<KindIndex> {
    e.kinds().index_of(&ContentId(id.into()))
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
    let stage = striker(Some("pyro"), 100);
    let bat = kind(&engine(&stage), "bat");
    assert!(bat.is_some());
    let kinds: Vec<_> = events_of(&stage, 3)
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
    assert_eq!(kinds, [bat]);
}

#[test]
fn a_kill_names_the_kind_of_the_victim() {
    let stage = striker(Some("pyro"), 10);
    let bat = kind(&engine(&stage), "bat");
    let died: Vec<_> = events_of(&stage, 3)
        .into_iter()
        .filter_map(|ev| match ev {
            DomainEvent::Died { entity, kind, .. } => Some((entity, kind)),
            _ => None,
        })
        .collect();
    assert_eq!(died, [(FOE, bat)]);
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
            .kind_index
    };
    assert_eq!(kind_of(EntityKind::Enemy), kind(&e, "bat"));
    assert_eq!(kind_of(EntityKind::Boss), kind(&e, "golem"));
    assert_eq!(kind_of(EntityKind::Companion), kind(&e, "fairy_a"));
}

#[test]
fn the_table_is_the_sorted_distinct_ids_of_the_stage() {
    let table = engine(&striker(None, 100)).kinds().clone();
    let ids: Vec<&str> = table.ids().iter().map(|k| k.0.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(ids, sorted);
    for want in ["bat", "golem", "fairy_a"] {
        assert!(ids.contains(&want), "{want} in {ids:?}");
    }
    // The same stage gives the same table, and every index maps back to its ID.
    assert_eq!(&table, engine(&striker(None, 100)).kinds());
    for (i, id) in table.ids().iter().enumerate() {
        let index = KindIndex(i as u16);
        assert_eq!(table.index_of(id), Some(index));
        assert_eq!(table.id(index), Some(id));
    }
    assert_eq!(table.id(KindIndex(table.len() as u16)), None);
}

#[test]
fn two_enemies_of_a_kind_share_an_index_and_other_kinds_differ() {
    let mut e = engine(&striker(None, 100));
    e.step(idle());
    let views = e.snapshot().entities;
    let of = |k| {
        views
            .iter()
            .find(|v| v.kind == k)
            .and_then(|v| v.kind_index)
    };
    assert_eq!(of(EntityKind::Enemy), of(EntityKind::Enemy));
    assert_ne!(of(EntityKind::Enemy), of(EntityKind::Boss));
    assert_ne!(of(EntityKind::Boss), of(EntityKind::Companion));
}

#[test]
fn a_table_past_its_limit_is_refused_with_a_path() {
    let ids: Vec<ContentId> = (0..5).map(|n| ContentId(format!("k{n}"))).collect();
    assert_eq!(KindTable::from_ids(&ids, 5).unwrap().len(), 5);
    let issue = KindTable::from_ids(&ids, 4).unwrap_err();
    assert_eq!(issue.path, "$.kinds");
    // The same ID twice is one kind; and no limit goes past what a u16 names. (A table of
    // 65,537 IDs is cheap enough to build here.)
    assert_eq!(
        KindTable::from_ids(ids.iter().chain(&ids), 5)
            .unwrap()
            .len(),
        5
    );
    let many: Vec<ContentId> = (0..=MAX_KINDS)
        .map(|n| ContentId(format!("k{n}")))
        .collect();
    assert!(KindTable::from_ids(&many, usize::MAX).is_err());
    assert!(KindTable::from_ids(&many[..MAX_KINDS], usize::MAX).is_ok());
}

#[test]
fn an_event_without_the_kind_fields_still_loads() {
    let hit: DomainEvent = serde_json::from_value(json!({ "type": "Hit", "data": {
        "source": 0, "target": 1, "damage": 3 } }))
    .unwrap();
    assert!(matches!(
        hit,
        DomainEvent::Hit {
            target_kind: None,
            damage_type: None,
            ..
        }
    ));
    let died: DomainEvent = serde_json::from_value(
        json!({ "type": "Died", "data": { "entity": 1, "class": "enemy" } }),
    )
    .unwrap();
    assert!(matches!(died, DomainEvent::Died { kind: None, .. }));
    // A kind is a bare number in the JSON.
    let with = DomainEvent::Died {
        entity: EntityId(1),
        kind: Some(KindIndex(7)),
        class: EntityKind::Enemy,
        elite: false,
    };
    assert_eq!(
        serde_json::to_value(&with).unwrap()["data"]["kind"],
        json!(7)
    );
}
