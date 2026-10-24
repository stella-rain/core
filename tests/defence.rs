//! Defence (ADR-056): every hit, from a hit box or an orb, on any target, is the base damage
//! times the attacker's and the target's multipliers, times `K / (K + defence)` with K = 10,000 (100 at the design values, scaled by 100 with
//! the damage, ADR-103), truncated at each step and at least 1. The `Hit` event reports what is dealt.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::stage::Stage;

const CX: i32 = 20_000;
const CY: i32 = 30_000;
/// The companion is entity 2 (the far boss is 1); the first enemy is 3.
const COMPANION: EntityId = EntityId(2);

fn strike(damage: u32) -> Value {
    json!({ "type": "attack", "attack": { "inline": [
        { "kind": "active", "ticks": 1, "damage": damage,
          "hit_box": { "type": "circle", "radius": 4_096 } }] } })
}

/// An enemy with `defence` holding still next to the corner.
fn target(defence: Option<u32>) -> Value {
    let mut e = enemy(100_000, hold(CX + 2_560, CY), json!([]));
    if let Some(d) = defence {
        e["defence"] = json!(d);
    }
    wave(1, 1, 0, e, CX + 2_560, CY)
}

/// A companion at the corner that runs `rules` against the nearest enemy.
fn striker(rules: Value) -> Stage {
    stage_full(18_000, Some(far_boss()), vec![target(None)], vec![]).with_companion(companion(
        50,
        hold(CX, CY),
        rules,
    ))
}

trait WithCompanion {
    fn with_companion(self, c: Value) -> Stage;
}

impl WithCompanion for Stage {
    fn with_companion(mut self, c: Value) -> Stage {
        self.companions
            .push(serde_json::from_value(c).expect("the companion parses"));
        self
    }
}

/// The damage of every hit `source` deals over `ticks` ticks.
fn dealt(stage: &Stage, source: EntityId, ticks: u32) -> Vec<u32> {
    run_until(&mut engine(stage), idle(), ticks, |_| false)
        .into_iter()
        .filter_map(|(_, ev)| match ev {
            DomainEvent::Hit {
                source: s, damage, ..
            } if s == source => Some(damage),
            _ => None,
        })
        .collect()
}

fn once(action: Value) -> Value {
    json!([{ "when": always(), "target": "nearest_enemy", "once": true, "do": action }])
}

fn against(defence: Option<u32>, damage: u32) -> Vec<u32> {
    let mut st = striker(once(strike(damage)));
    st.waves = vec![serde_json::from_value(target(defence)).unwrap()];
    dealt(&st, COMPANION, 3)
}

#[test]
fn defence_divides_damage_by_k_plus_defence_over_k() {
    assert_eq!(against(None, 10), [10]);
    assert_eq!(against(Some(0), 10), [10]);
    assert_eq!(against(Some(10_000), 10), [5]);
    // 10 × 10,000 / 30,000 = 3.33, truncated.
    assert_eq!(against(Some(20_000), 10), [3]);
}

#[test]
fn a_hit_with_any_damage_deals_at_least_1() {
    assert_eq!(against(Some(1_000_000), 1), [1]);
    assert_eq!(against(Some(1_000_000), 50), [1]);
}

#[test]
fn defence_comes_after_the_targets_vulnerability() {
    // Tick 1 makes the enemy 50% vulnerable; tick 2 strikes for 10: 15, then 15 × 10,000 / 20,000.
    let rules = json!([
        { "when": always(), "target": "nearest_enemy", "once": true,
          "do": cast(json!({ "type": "vulnerable", "value_pct": 50, "duration_ticks": 60 })) },
        { "when": always(), "target": "nearest_enemy", "once": true, "do": strike(10) }
    ]);
    let mut st = striker(rules);
    st.waves = vec![serde_json::from_value(target(Some(10_000))).unwrap()];
    assert_eq!(dealt(&st, COMPANION, 3), [7]);
}

#[test]
fn zones_meet_the_same_defence() {
    let shot = |defence: Option<u32>| {
        let mut st = striker(once(json!({ "type": "attack", "attack": { "inline": [
            { "kind": "active", "ticks": 1, "damage": 10, "spawn": [
                { "type": "zone", "shape": { "type": "circle", "radius": 3_072 }, "anchor": "target", "wind_up_ticks": 0, "active_ticks": 2 }
            ] }] } })));
        st.waves = vec![serde_json::from_value(target(defence)).unwrap()];
        dealt(&st, COMPANION, 20)
    };
    assert_eq!(shot(None), [10]);
    assert_eq!(shot(Some(10_000)), [5]);
}

#[test]
fn the_main_character_companions_the_boss_and_its_parts_have_defence() {
    // The main character: an enemy above it swings for 10.
    let swing = json!([{ "when": always(), "target": "player", "once": true,
        "do": { "type": "attack", "attack": { "inline": [
            { "kind": "active", "ticks": 1, "damage": 10,
              "hit_box": { "type": "circle", "offset": { "forward": 5_120 }, "radius": 1_024 } }
        ] } } }]);
    let hp_left = |defence: Option<u32>| {
        let foe = wave(
            1,
            1,
            0,
            enemy(50, hold(START_X, START_Y - 5_120), swing.clone()),
            START_X,
            START_Y - 5_120,
        );
        let mut st = stage_full(18_000, Some(far_boss()), vec![foe], vec![]);
        st.player.defence = defence.unwrap_or(0);
        run_until(&mut engine(&st), idle(), 3, |_| false)
            .into_iter()
            .find_map(|(_, ev)| match ev {
                DomainEvent::PlayerHit { hp_left } => Some(hp_left),
                _ => None,
            })
    };
    assert_eq!(hp_left(None), Some(89));
    assert_eq!(hp_left(Some(10_000)), Some(94));

    // A companion: the same enemy swings at the companion beside it.
    let at_companion = |defence: u32| {
        let mut ally = companion(50, hold(CX, CY), json!([]));
        ally["defence"] = json!(defence);
        let foe = enemy(
            50,
            hold(CX + 2_560, CY),
            json!([{
            "when": always(), "target": "player", "once": true, "do": strike(10) }]),
        );
        let st = stage_full(
            18_000,
            Some(far_boss()),
            vec![wave(1, 1, 0, foe, CX + 2_560, CY)],
            vec![ally],
        );
        dealt(&st, EntityId(3), 3)
    };
    assert_eq!(at_companion(0), [10]);
    assert_eq!(at_companion(10_000), [5]);

    // The boss (defence 10,000) and its part (defence 30,000) under one hit box.
    let boss = json!({ "base": "golem", "hp": 1_000, "radius": 8_192, "defence": 10_000,
                       "spawn": { "x": CX, "y": CY + 10_240 },
                       "parts": [{ "id": "horn", "asset": "horns_2", "hp": 100, "defence": 30_000,
                                   "offset": { "x": 0, "y": -4_096 }, "radius": 2_048 }],
                       "phases": [{}] });
    let sweep = json!([{ "when": always(), "target": "boss_first", "once": true,
        "do": { "type": "attack", "attack": { "inline": [
            { "kind": "active", "ticks": 1, "damage": 12,
              "hit_box": { "type": "circle", "radius": 20_480 } }] } } }]);
    let st = stage_full(
        18_000,
        Some(boss),
        vec![],
        vec![companion(50, hold(CX, CY), sweep)],
    );
    assert_eq!(dealt(&st, COMPANION, 3), [6, 3]);
}
