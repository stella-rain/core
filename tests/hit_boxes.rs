//! Hit boxes (ADR-055): an active phase's hit box follows its attacker, faces the way the attack
//! faced when it started, and hits every target inside it once per active phase, or again after
//! `rehit_ticks`; wind-ups and recoveries deal nothing; an attacker killed earlier in the tick
//! still lands its hit (a trade); invulnerability stops a hit without using it up. A phase's
//! `damage` is the base damage of its hit box and of the orbs it spawns.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::stage::Stage;

/// The companions' corner of the field, far from the main character.
const CX: i32 = 20_000;
const CY: i32 = 30_000;

fn circle(forward: i32, right: i32, radius: i32) -> Value {
    json!({ "type": "circle", "offset": { "forward": forward, "right": right }, "radius": radius })
}

fn arc(radius: i32, half_angle_degrees: i32) -> Value {
    json!({ "type": "arc", "radius": radius, "half_angle": half_angle_degrees * 256 })
}

fn phase(kind: &str, ticks: u32, hit_box: Option<Value>, damage: Option<u32>) -> Value {
    let mut p = json!({ "kind": kind, "ticks": ticks });
    if let Some(h) = hit_box {
        p["hit_box"] = h;
    }
    if let Some(d) = damage {
        p["damage"] = json!(d);
    }
    p
}

fn attack(target: &str, once: bool, phases: Vec<Value>) -> Value {
    json!([{ "when": always(), "target": target, "once": once, "cooldown_ticks": 0,
             "do": { "type": "attack", "attack": { "inline": phases } } }])
}

/// An enemy holding still at `x`, `y` with `rules`.
fn still_enemy(hp: u32, x: i32, y: i32, rules: Value) -> Value {
    wave(1, 1, 0, enemy(hp, hold(x, y), rules), x, y)
}

/// A companion holding still at the corner, attacking the nearest enemy with `phases`.
fn fighter(hp: u32, phases: Vec<Value>) -> Value {
    companion(hp, hold(CX, CY), attack("nearest_enemy", false, phases))
}

/// The weapons are quiet, so that only hit boxes deal damage.
fn stage(waves: Vec<Value>, companions: Vec<Value>) -> Stage {
    quiet(stage_full(18_000, Some(far_boss()), waves, companions))
}

/// `(tick, target, damage)` of every hit by `source` over `ticks` ticks of `input`.
fn hits_by(
    stage: &Stage,
    source: EntityId,
    ticks: u32,
    input: stella_rain_core::input::Input,
) -> Vec<(u32, EntityId, u32)> {
    let events = run_until(&mut engine(stage), input, ticks, |_| false);
    events
        .into_iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::Hit {
                source: s,
                target,
                damage,
                ..
            } if s == source => Some((t, target, damage)),
            _ => None,
        })
        .collect()
}

fn player_hits(events: &[(u32, DomainEvent)]) -> Vec<(u32, u32)> {
    events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::PlayerHit { hp_left, .. } => Some((*t, *hp_left)),
            _ => None,
        })
        .collect()
}

/// The companion is entity 2 (the far boss is 1); the waves' enemies come after it, in order.
const COMPANION: EntityId = EntityId(2);

#[test]
fn only_the_active_phase_of_a_swing_deals_damage() {
    // 20 pixels above the main character, facing it: every phase covers it.
    let over = || Some(circle(20 * 256, 0, 4 * 256));
    let swing = vec![
        phase("wind_up", 3, over(), Some(5)),
        phase("active", 1, over(), Some(5)),
        phase("recovery", 3, over(), Some(5)),
    ];
    let foe = still_enemy(
        50,
        START_X,
        START_Y - 20 * 256,
        attack("player", true, swing),
    );
    let events = run_until(&mut engine(&stage(vec![foe], vec![])), idle(), 20, |_| {
        false
    });
    // Started on tick 1: the wind-up is ticks 1 to 3, the active phase tick 4.
    assert_eq!(player_hits(&events), [(4, 94)]);
}

#[test]
fn the_offset_is_in_the_attackers_frame_and_the_facing_is_locked_at_the_start() {
    // The enemy faces down at the main character when it starts; its right is then the
    // screen's left. The main character walks 7 ticks left (21 pixels) during the wind-up.
    let run = |right: i32| {
        let swing = vec![
            phase("wind_up", 30, None, None),
            phase("active", 1, Some(circle(20 * 256, right, 4 * 256)), Some(5)),
        ];
        let foe = still_enemy(
            50,
            START_X,
            START_Y - 20 * 256,
            attack("player", true, swing),
        );
        let mut e = engine(&stage(vec![foe], vec![]));
        let mut events = run_until(&mut e, moving(-127, 0), 7, |_| false);
        assert_eq!(player_at(&e), (START_X - 7 * SPEED, START_Y));
        events.extend(run_until(&mut e, idle(), 30, |_| false));
        player_hits(&events)
    };
    assert_eq!(
        run(7 * SPEED),
        [(31, 94)],
        "its right reaches the main character"
    );
    assert_eq!(run(-7 * SPEED), [], "its left does not");
}

#[test]
fn a_hit_box_hits_every_target_inside_it_in_id_order() {
    let wide = vec![phase("active", 1, Some(circle(0, 0, 20 * 256)), Some(3))];
    let target = |x: i32, y: i32| still_enemy(50, x, y, json!([]));
    let st = stage(
        vec![
            target(CX + 10 * 256, CY),
            target(CX, CY + 10 * 256),
            target(CX + 60 * 256, CY),
            target(CX - 10 * 256, CY),
        ],
        vec![fighter(50, wide)],
    );
    let hits = hits_by(&st, COMPANION, 1, idle());
    assert_eq!(
        hits,
        [
            (1, EntityId(3), 3),
            (1, EntityId(4), 3),
            (1, EntityId(6), 3)
        ]
    );
}

#[test]
fn an_arc_hits_what_lies_within_its_angle_or_touches_its_edges() {
    // The companion faces A, its nearest enemy, straight down. B is behind it and C to its
    // side; D's centre is 50 degrees round, outside a 45-degree arc, but its 8-pixel circle
    // reaches across the edge.
    let enemies = || {
        vec![
            still_enemy(50, CX, CY + 10 * 256, json!([])),
            still_enemy(50, CX, CY - 15 * 256, json!([])),
            still_enemy(50, CX + 15 * 256, CY, json!([])),
            still_enemy(50, CX + 3_922, CY + 3_292, json!([])),
        ]
    };
    let targets = |half: i32| {
        let sweep = vec![phase("active", 1, Some(arc(30 * 256, half)), Some(1))];
        let hits = hits_by(
            &stage(enemies(), vec![fighter(50, sweep)]),
            COMPANION,
            1,
            idle(),
        );
        hits.into_iter().map(|(_, t, _)| t.0).collect::<Vec<u32>>()
    };
    assert_eq!(targets(45), [3, 6]);
    assert_eq!(targets(180), [3, 4, 5, 6]);
}

#[test]
fn a_target_is_hit_once_per_active_phase_unless_it_may_be_hit_again() {
    let lingering = |rehit: Option<u32>| {
        let mut p = phase("active", 10, Some(circle(0, 0, 16 * 256)), Some(1));
        if let Some(r) = rehit {
            p["rehit_ticks"] = json!(r);
        }
        let st = stage(
            vec![still_enemy(1_000, CX + 10 * 256, CY, json!([]))],
            vec![fighter(50, vec![p])],
        );
        hits_by(&st, COMPANION, 10, idle())
            .into_iter()
            .map(|(t, ..)| t)
            .collect::<Vec<u32>>()
    };
    assert_eq!(lingering(None), [1]);
    assert_eq!(lingering(Some(3)), [1, 4, 7, 10]);

    // Two active phases are two chances to hit.
    let twice = vec![
        phase("active", 2, Some(circle(0, 0, 16 * 256)), Some(1)),
        phase("active", 2, Some(circle(0, 0, 16 * 256)), Some(1)),
    ];
    let st = stage(
        vec![still_enemy(1_000, CX + 10 * 256, CY, json!([]))],
        vec![fighter(50, twice)],
    );
    let ticks: Vec<u32> = hits_by(&st, COMPANION, 4, idle())
        .into_iter()
        .map(|(t, ..)| t)
        .collect();
    assert_eq!(ticks, [1, 3]);
}

#[test]
fn an_attacker_killed_earlier_in_the_tick_still_lands_its_hit() {
    // The companion (entity 2) resolves first and kills the enemy (3); the enemy's swing lands
    // too, and kills the companion.
    let blow = || vec![phase("active", 1, Some(circle(0, 0, 16 * 256)), Some(5))];
    let st = stage(
        vec![still_enemy(
            5,
            CX + 10 * 256,
            CY,
            attack("player", true, blow()),
        )],
        vec![fighter(5, blow())],
    );
    let events = run_until(&mut engine(&st), idle(), 1, |_| false);
    let died: Vec<EntityId> = events
        .iter()
        .filter_map(|(_, ev)| match ev {
            DomainEvent::Died { entity, .. } => Some(*entity),
            _ => None,
        })
        .collect();
    assert_eq!(died, [EntityId(3), COMPANION]);
}

#[test]
fn invulnerability_stops_a_hit_without_using_it_up() {
    // Both enemies swing on tick 1, one from above the main character and one from below (on one
    // point they would push each other away). The first hits; the second's hit box stays over
    // the main character for 100 ticks and lands as soon as the invulnerability after the first
    // hit ends.
    let over = || Some(circle(20 * 256, 0, 4 * 256));
    let quick = vec![phase("active", 1, over(), Some(1))];
    let lingering = vec![phase("active", 100, over(), Some(1))];
    let foes = vec![
        still_enemy(
            50,
            START_X,
            START_Y - 20 * 256,
            attack("player", true, quick),
        ),
        still_enemy(
            50,
            START_X,
            START_Y + 20 * 256,
            attack("player", true, lingering),
        ),
    ];
    let events = run_until(&mut engine(&stage(foes, vec![])), idle(), 100, |_| false);
    assert_eq!(player_hits(&events), [(1, 98), (61, 97)]);
}

#[test]
fn a_phases_damage_is_the_base_damage_of_the_zones_it_spawns() {
    let shot = |damage: Option<u32>| {
        let mut p = phase("active", 1, None, damage);
        p["spawn"] = json!([{ "type": "zone", "shape": { "type": "circle", "radius": 3_072 },
                              "anchor": "target", "wind_up_ticks": 0, "active_ticks": 2 }]);
        let foe = still_enemy(
            50,
            START_X,
            START_Y - 40 * 256,
            attack("player", true, vec![p]),
        );
        let mut st = stage(vec![foe], vec![]);
        st.player.hp = 9_900;
        let events = run_until(&mut engine(&st), idle(), 30, |_| false);
        player_hits(&events)
            .into_iter()
            .map(|(_, hp)| hp)
            .collect::<Vec<u32>>()
    };
    assert_eq!(shot(Some(7)), [9_893]);
    // Without one, an enemy's attack does 100 (1 before the scale of ADR-103).
    assert_eq!(shot(None), [9_800]);
}

#[test]
fn a_hit_box_over_the_boss_hits_the_body_and_each_part_once() {
    let boss = json!({ "base": "golem", "hp": 1_000, "radius": 8192,
                       "spawn": { "x": CX, "y": CY + 40 * 256 },
                       "parts": [
                           { "id": "horn", "asset": "horns_2", "hp": 100,
                             "offset": { "x": 0, "y": -4_096 }, "radius": 2_048 },
                           { "id": "tail", "asset": "horns_2", "hp": 100,
                             "offset": { "x": 0, "y": 40_000 }, "radius": 2_048 }
                       ],
                       "phases": [{}] });
    let sweep = vec![phase("active", 3, Some(circle(0, 0, 50 * 256)), Some(2))];
    let companion = companion(50, hold(CX, CY), attack("boss_first", true, sweep));
    let st = stage_full(18_000, Some(boss), vec![], vec![companion]);
    // The boss is 1, the companion 2, the parts 3 and 4; the tail is out of reach.
    assert_eq!(
        hits_by(&st, COMPANION, 3, idle()),
        [(1, EntityId(1), 2), (1, EntityId(3), 2)]
    );
}
