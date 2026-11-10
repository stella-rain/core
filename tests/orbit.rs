//! The orbit reward: a falling star's reward whose orbs turn around the main
//! character. Each orb runs the orbit's phased attack in a slot of its own, its hit box turned
//! `turn_per_tick` further every tick, the orbs spaced evenly around the turn. A round of attacks
//! starts when the orbit arrives and again every `cooldown_ticks`, counted from a round's start;
//! a round that outlasts the cooldown makes the next one wait for its end. Taking the same orbit
//! again adds its orbs, at most `MAX_ORBS`, from the next round; another orbit turns beside it.
//! Every number here is worked out by hand from the rules' constants and the sine table.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::snapshot::{EntityKind, Facing, OrbitView};

use crate::common;

use common::*;

/// How far out the test orbs turn: 40 pixels.
const DISTANCE: i32 = 10_240;
/// 15 degrees a tick: a quarter turn in 6 ticks.
const TURN: i32 = 15 * 256;
const DAMAGE: u32 = 7;
/// The first orbit slot: 0 is the combo and 1 the stage skill.
const FIRST_SLOT: u8 = 2;

/// An enemy of 50 hp that never moves or attacks, `DISTANCE` to the right of the main
/// character: an orb's hit box reaches it from 75 to 105 degrees round the turn.
fn beside() -> Value {
    wave(
        1,
        1,
        0,
        enemy(
            50,
            json!({ "type": "straight", "vx": 0, "vy": 0 }),
            json!([]),
        ),
        START_X + DISTANCE,
        START_Y,
    )
}

/// An orbit reward `id` of `amount` orbs, five times a run: each orb one active phase of
/// `active` ticks, a circle of 4 pixels `DISTANCE` out, turning `TURN` a tick, every `cooldown`
/// ticks.
fn orbit(id: &str, amount: u32, active: u32, cooldown: u32) -> Value {
    json!({
        "id": id, "rarity": "common", "effect": "orbit", "amount": amount, "limit": 5,
        "orbit": {
            "attack": { "inline": [
                { "kind": "active", "ticks": active, "damage": DAMAGE,
                  "hit_box": { "type": "circle", "offset": { "forward": DISTANCE },
                               "radius": 1_024 } }
            ] },
            "turn_per_tick": TURN,
            "cooldown_ticks": cooldown
        }
    })
}

/// A run with the orbit `ring` equipped at Lv1 in the first attack slot, on a stage of `waves`
/// with no stars: no drop and no offer (ADR-SIM-7). The orbit arrives ready.
fn ring_equipped(waves: Vec<Value>, ring: Value) -> Engine {
    let stage = stage_full(18_000, Some(far_boss()), waves, vec![]);
    let file = json!({ "rarities": common::rarities(), "rewards": [ring] });
    common::equip::equipped(&stage, file, &[("ring", 0)], 1)
}

/// The orbit's attacks, not the combo's: slot, tick in its phase and facing.
fn attacks(e: &Engine) -> Vec<(u8, u32, Facing)> {
    e.snapshot().players[0]
        .attacks
        .iter()
        .filter(|a| a.slot >= FIRST_SLOT)
        .map(|a| (a.slot, a.tick, a.facing))
        .collect()
}

fn slots(e: &Engine) -> Vec<u8> {
    attacks(e).into_iter().map(|a| a.0).collect()
}

fn orbits(e: &Engine) -> Vec<(String, u8, u32)> {
    e.snapshot().players[0]
        .orbits
        .iter()
        .map(
            |OrbitView {
                 reward,
                 orbs,
                 cooldown_left,
             }| { (reward.to_string(), *orbs, *cooldown_left) },
        )
        .collect()
}

fn the_enemy(e: &Engine) -> EntityId {
    e.snapshot()
        .entities
        .iter()
        .find(|x| x.kind == EntityKind::Enemy)
        .expect("the enemy beside")
        .id
}

const DOWN: Facing = Facing { x: 0, y: 65_536 };
const UP: Facing = Facing { x: 0, y: -65_536 };
const RIGHT: Facing = Facing { x: 65_536, y: 0 };

#[test]
fn an_orbit_arrives_ready_its_orbs_spaced_evenly_round_the_turn() {
    let mut e = ring_equipped(vec![], orbit("ring", 2, 60, 120));
    assert_eq!(orbits(&e), [("ring".to_string(), 2, 0)]);
    e.step(&[idle(), Default::default(), Default::default()]);
    // Orb 0 starts straight down the screen, orb 1 half a turn on.
    assert_eq!(
        attacks(&e),
        [(FIRST_SLOT, 0, DOWN), (FIRST_SLOT + 1, 0, UP)]
    );
    assert_eq!(orbits(&e), [("ring".to_string(), 2, 120)]);
}

#[test]
fn the_orbs_turn_every_tick() {
    let mut e = ring_equipped(vec![], orbit("ring", 1, 60, 120));
    e.step(&[idle(), Default::default(), Default::default()]);
    assert_eq!(attacks(&e)[0].2, DOWN);
    for _ in 0..6 {
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    // A quarter turn in 6 ticks, toward the screen's right.
    assert_eq!(attacks(&e), [(FIRST_SLOT, 6, RIGHT)]);
}

#[test]
fn an_orb_hits_what_it_passes_once_a_round_and_rounds_start_every_cooldown() {
    let mut e = ring_equipped(vec![beside()], orbit("ring", 1, 60, 120));
    // The first step starts the round (its tick 0) and spawns the enemy.
    e.step(&[idle(), Default::default(), Default::default()]);
    let enemy = the_enemy(&e);
    let first = e.tick();
    let mut hits = Vec::new();
    for _ in 0..130 {
        e.step(&[idle(), Default::default(), Default::default()]);
        for ev in e.events() {
            if let DomainEvent::Hit { target, damage, .. } = ev {
                if *target == enemy {
                    hits.push((e.tick() - first, *damage));
                }
            }
        }
        if e.tick() - first == 60 {
            assert_eq!(slots(&e), [] as [u8; 0], "the round is over");
            assert_eq!(orbits(&e)[0].2, 60, "the cooldown counts from its start");
        }
    }
    // 75 degrees round on its fifth tick, once in the round, though it turns past twice more;
    // the next round starts 120 ticks after the first.
    assert_eq!(hits, [(5, DAMAGE), (125, DAMAGE)]);
}

#[test]
fn a_round_longer_than_the_cooldown_makes_the_next_wait_for_its_end() {
    let mut e = ring_equipped(vec![], orbit("ring", 1, 150, 120));
    for _ in 0..150 {
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    assert_eq!(attacks(&e)[0].1, 149, "the first round is still going");
    assert_eq!(orbits(&e)[0].2, 0, "though its cooldown has run out");
    e.step(&[idle(), Default::default(), Default::default()]);
    assert_eq!(attacks(&e)[0].1, 0, "the next round starts as it ends");
    assert_eq!(orbits(&e)[0].2, 120);
}
