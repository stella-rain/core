//! Stun (ADR-085): a smash's stun puts the `stun` status on the enemy it hits, for the smash's
//! ticks. It does not stack: a stun on a stunned enemy keeps the longer remainder. An elite takes
//! its share of the ticks (the run's `elite_pct`, 50 today), and a boss takes none. Every number
//! here is worked out by hand from the placeholders: a 30-tick stun, and 50 percent for an elite.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::StatusView;
use stella_rain_core::stage::Stage;

use crate::common;

use common::on_hit::*;
use common::*;

/// A sub step of one circle 1,024 units forward, 10 damage and 4 ticks of recovery, with the
/// stun of `ticks` as its weapon's smash.
fn stunning(ticks: u32) -> Value {
    json!({ "inline": {
        "combo": { "sub": { "attack": { "inline": [
            { "kind": "active", "ticks": 1, "damage": 10,
              "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 8_192 } },
            { "kind": "recovery", "ticks": 4 }
        ] } } },
        "window_ticks": 30,
        "assist_range": 0,
        "smash": { "type": "stun", "ticks": ticks },
    } })
}

/// A stage of `waves` and `boss`, whose main character smashes with a stun of `ticks`.
fn stage_with(ticks: u32, boss: Value, waves: Vec<Value>) -> Stage {
    armed(
        stage_full(18_000, Some(boss), waves, vec![]),
        stunning(ticks),
    )
}

/// The tough target as an elite: an `elite` with a range of 256, so it takes half a stun.
fn elite_target() -> Value {
    let mut e = enemy(100_000, hold(START_X, NEAR_Y), json!([]));
    e["elite"] = json!({ "range": 256 });
    wave(TARGET_TICK, 1, 0, e, START_X, NEAR_Y)
}

/// The `stun` statuses of `id` in the snapshot.
fn stuns(e: &Engine, id: EntityId) -> Vec<StatusView> {
    e.snapshot()
        .entities
        .into_iter()
        .find(|en| en.id == id)
        .map(|en| en.statuses)
        .unwrap_or_default()
        .into_iter()
        .filter(|st| st.id.0 == "stun")
        .collect()
}

/// Steps `limit` ticks, pressing `sub` on each tick of `presses`, and returns for each hit on
/// `id` the `stun` statuses it shows on the tick it lands.
fn landings(e: &mut Engine, id: EntityId, presses: &[u32], limit: u32) -> Vec<Vec<StatusView>> {
    let mut out = Vec::new();
    for _ in 0..limit {
        let t = e.tick();
        e.step(Input {
            sub: presses.contains(&t),
            ..idle()
        });
        let hit = e
            .events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::Hit { target, .. } if *target == id));
        if hit {
            out.push(stuns(e, id));
        }
    }
    out
}

#[test]
fn a_smash_stuns_the_enemy_it_hits_for_its_ticks() {
    let mut e = engine(&stage_with(30, far_boss(), vec![tough(0, None, None)]));
    let target = wait_for_target(&mut e);
    assert!(stuns(&e, target).is_empty());
    let start = e.tick();
    let landed = landings(&mut e, target, &[start], 8);
    let first = landed.first().expect("the smash hits the target");
    assert_eq!(first.len(), 1, "one stun status");
    assert_eq!(first[0].id.0, "stun");
    assert_eq!(first[0].total, 30);
    assert_eq!(first[0].remaining, 30);
}

#[test]
fn a_second_stun_does_not_stack_and_keeps_the_longer_remainder() {
    // The second smash lands 10 ticks into the first stun: it refreshes the stun to its own 30
    // ticks (longer than the 20 left), and still there is one stun status.
    let mut e = engine(&stage_with(30, far_boss(), vec![tough(0, None, None)]));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let landed = landings(&mut e, target, &[start, start + 10], 20);
    assert!(landed.len() >= 2, "two smashes hit: {landed:?}");
    for stuns in &landed {
        assert_eq!(stuns.len(), 1, "one stun status: {stuns:?}");
    }
    assert_eq!(landed[1][0].remaining, 30, "the longer stun is kept");
}

#[test]
fn an_elite_takes_half_the_ticks_of_a_stun() {
    let mut e = engine(&stage_with(30, far_boss(), vec![elite_target()]));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let landed = landings(&mut e, target, &[start], 8);
    let first = landed.first().expect("the smash hits the elite");
    assert_eq!(first[0].total, 15, "an elite takes 50 percent of 30 ticks");
}

#[test]
fn a_boss_takes_no_stun_from_a_smash() {
    let mut e = engine(&stage_with(30, boss_at(100_000, NEAR_Y), vec![]));
    let mut hit = None;
    for _ in 0..10 {
        let t = e.tick();
        e.step(Input {
            sub: t == 0,
            ..idle()
        });
        for ev in e.events() {
            if let DomainEvent::Hit { target, damage, .. } = ev {
                assert!(*damage > 0, "the smash hits the boss");
                hit = Some(*target);
            }
        }
    }
    let boss = hit.expect("the smash hits the boss");
    assert!(stuns(&e, boss).is_empty(), "a boss resists a stun");
}
