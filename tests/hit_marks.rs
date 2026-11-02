//! The `Hit` event says which rune dealt it (ADR-099): the on-hit rune that fired the attack,
//! or the damage-over-time rune whose status ticked. A combo step's hit, and every other, says
//! nothing. Every rune here hits the tough target for a damage of its own, so the hit tells
//! which attack it came from: the step 10, explosion 3, thunder 4, boomerang 5, homing missiles
//! 6, bleed 7, poison 8, burn 9.

use serde_json::{Value, json};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::rune::Rune;
use stella_rain_core::runes::Runes;

use crate::common::on_hit::*;

/// An entry whose one attack is a circle of 16 pixels round the point it starts at, active for
/// 30 ticks a phase; the cooldown is over before the target arrives, and the one press fires each once.
fn near(rune: &str, damage: u32) -> Value {
    let phase = json!({ "kind": "active", "ticks": 30,
                        "hit_box": { "type": "circle", "radius": 4_096 } });
    // A boomerang flies out and back, in two phases.
    let phases = if rune == "boomerang" {
        json!([phase.clone(), phase])
    } else {
        json!([phase])
    };
    let mut e = entry(rune, 100, damage, phases);
    if rune == "homing_missiles" {
        e["missiles"] = json!(1);
        e["range"] = json!(40_960);
    }
    e
}

fn runes() -> Runes {
    let dots = vec![
        dot("bleed", 30, 90, 7, 1),
        dot("poison", 30, 90, 8, 1),
        dot("burn", 30, 90, 9, 1),
    ];
    runes_with(
        vec![
            near("explosion", 3),
            near("thunder", 4),
            near("boomerang", 5),
            near("homing_missiles", 6),
        ],
        dots,
    )
}

#[test]
fn rune_hits_carry_their_rune_and_a_combo_step_none() {
    let mut e = with(
        &stage(7, tough(0, None, None)),
        &content(vec![random_rune()], None, runes()),
    );
    take_all(&mut e);
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let mut seen: Vec<(u32, Option<Rune>)> = Vec::new();
    for _ in 0..200 {
        let t = e.tick();
        e.step(&[once(start)(t)]);
        for ev in e.events() {
            if let DomainEvent::Hit {
                target: id,
                damage,
                rune,
                ..
            } = ev
            {
                if *id == target {
                    seen.push((*damage, *rune));
                }
            }
        }
    }
    let expected = [
        (10, None),
        (3, Some(Rune::Explosion)),
        (4, Some(Rune::Thunder)),
        (5, Some(Rune::Boomerang)),
        (6, Some(Rune::HomingMissiles)),
        (7, Some(Rune::Bleed)),
        (8, Some(Rune::Poison)),
        (9, Some(Rune::Burn)),
    ];
    for (damage, rune) in expected {
        let of: Vec<_> = seen.iter().filter(|(d, _)| *d == damage).collect();
        assert!(!of.is_empty(), "a hit of {damage}: {seen:?}");
        assert!(
            of.iter().all(|(_, r)| *r == rune),
            "hits of {damage} carry {rune:?}: {seen:?}"
        );
    }
    assert_eq!(
        seen.len(),
        seen.iter().filter(|(d, _)| (3..=10).contains(d)).count()
    );
}

#[test]
fn an_unmarked_hit_leaves_the_field_out_of_the_event() {
    let hit = DomainEvent::Hit {
        source: stella_rain_core::id::EntityId(0),
        target: stella_rain_core::id::EntityId(1),
        target_kind: None,
        damage: 1,
        damage_type: None,
        match_up: Default::default(),
        rune: None,
        crit: false,
        echo: false,
    };
    assert!(!serde_json::to_string(&hit).unwrap().contains("rune"));
}
