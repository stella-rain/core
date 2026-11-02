//! The orbit reward (ADR-069): a falling star's reward whose orbs turn around the main
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
use stella_rain_core::rewards::{MAX_ORBS, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::snapshot::{EntityKind, Facing, OfferOption, OrbitView};
use stella_rain_core::stage::Stage;
use stella_rain_core::version::Versions;

use crate::common;

use common::*;

/// 32 pixels above the main character: inside the pickup circle, so a star dropped here is
/// taken at once.
const NEAR_Y: i32 = START_Y - 8_192;
/// How far out the test orbs turn: 40 pixels.
const DISTANCE: i32 = 10_240;
/// 15 degrees a tick: a quarter turn in 6 ticks.
const TURN: i32 = 15 * 256;
const DAMAGE: u32 = 7;
/// The first orbit slot: 0 is the combo and 1 the stage skill (ADR-069).
const FIRST_SLOT: u8 = 2;

/// A stage with one level and stars, where every kill drops a common star, and `waves`.
fn starry(waves: Vec<Value>) -> Stage {
    let mut stage = stage_full(18_000, Some(far_boss()), waves, vec![]);
    stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).unwrap());
    stage.stars = Some(
        serde_json::from_value(json!({
            "drop_per_mille": 1000, "pity": 1, "cap": 8, "pickup_radius": 12_288,
            "rerolls": 0, "banishes": 0,
            "tiers": [{ "from_level": 1, "weights": { "common": 1 } }],
        }))
        .unwrap(),
    );
    stage
}

/// `count` enemies of 10 hp (one shot each), one every 6 ticks from tick 1, straight above the
/// main character in the line of its shots: each drops a star it takes at once.
fn targets(count: u16) -> Value {
    wave(
        1,
        count,
        6,
        enemy(10, hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    )
}

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

fn with(stage: &Stage, rewards: Vec<Value>) -> Engine {
    let file = json!({ "rarities": common::rarities(), "rewards": rewards });
    let rewards = Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT)
        .expect("the test rewards validate");
    let content = RunContent::new(RunStats::bundled().clone(), rewards);
    Engine::new_with(stage, &content).expect("the test stage is valid")
}

/// Fires until an offer is open, then takes the candidate that is `reward`. The orbit's first
/// round starts on the next tick.
fn take(e: &mut Engine, reward: &str) {
    fire_until(e, idle(), 1200, Engine::offer_open);
    let offer = e.snapshot().progress.unwrap().offer.expect("an open offer");
    let candidate = offer
        .candidates
        .iter()
        .position(|c| c.reward.as_str() == reward)
        .unwrap_or_else(|| panic!("{reward} is not offered"));
    let option = OfferOption::Take {
        candidate: u8::try_from(candidate).unwrap(),
    };
    let index = offer.options.iter().position(|o| *o == option).unwrap();
    common::pick(e, u32::try_from(index).unwrap()).unwrap();
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
    let mut e = with(&starry(vec![targets(1)]), vec![orbit("ring", 2, 60, 120)]);
    take(&mut e, "ring");
    assert_eq!(orbits(&e), [("ring".to_string(), 2, 0)]);
    e.step(&[idle()]);
    // Orb 0 starts straight down the screen, orb 1 half a turn on.
    assert_eq!(
        attacks(&e),
        [(FIRST_SLOT, 0, DOWN), (FIRST_SLOT + 1, 0, UP)]
    );
    assert_eq!(orbits(&e), [("ring".to_string(), 2, 120)]);
}

#[test]
fn the_orbs_turn_every_tick() {
    let mut e = with(&starry(vec![targets(1)]), vec![orbit("ring", 1, 60, 120)]);
    take(&mut e, "ring");
    e.step(&[idle()]);
    assert_eq!(attacks(&e)[0].2, DOWN);
    for _ in 0..6 {
        e.step(&[idle()]);
    }
    // A quarter turn in 6 ticks, toward the screen's right.
    assert_eq!(attacks(&e), [(FIRST_SLOT, 6, RIGHT)]);
}

#[test]
fn an_orb_hits_what_it_passes_once_a_round_and_rounds_start_every_cooldown() {
    let mut e = with(
        &starry(vec![targets(1), beside()]),
        vec![orbit("ring", 1, 60, 120)],
    );
    take(&mut e, "ring");
    let enemy = the_enemy(&e);
    let first = e.tick() + 1;
    let mut hits = Vec::new();
    for _ in 0..130 {
        e.step(&[idle()]);
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
    let mut e = with(&starry(vec![targets(1)]), vec![orbit("ring", 1, 150, 120)]);
    take(&mut e, "ring");
    for _ in 0..150 {
        e.step(&[idle()]);
    }
    assert_eq!(attacks(&e)[0].1, 149, "the first round is still going");
    assert_eq!(orbits(&e)[0].2, 0, "though its cooldown has run out");
    e.step(&[idle()]);
    assert_eq!(attacks(&e)[0].1, 0, "the next round starts as it ends");
    assert_eq!(orbits(&e)[0].2, 120);
}

#[test]
fn taking_the_orbit_again_adds_orbs_from_the_next_round_up_to_the_most() {
    let mut e = with(&starry(vec![targets(2)]), vec![orbit("ring", 5, 600, 600)]);
    take(&mut e, "ring");
    e.step(&[idle()]);
    assert_eq!(slots(&e), [2, 3, 4, 5, 6]);
    take(&mut e, "ring");
    assert_eq!(orbits(&e)[0].1, MAX_ORBS, "5 and 5, held to the most");
    e.step(&[idle()]);
    assert_eq!(slots(&e).len(), 5, "the round going on keeps its orbs");
    let mut ticks = 0;
    while slots(&e).len() < usize::from(MAX_ORBS) {
        e.step(&[idle()]);
        ticks += 1;
        assert!(ticks <= 600, "no round of 8 orbs");
    }
    let started: Vec<u32> = attacks(&e).into_iter().map(|a| a.1).collect();
    assert_eq!(started, [0; 8], "all 8 start together");
    assert_eq!(slots(&e), [2, 3, 4, 5, 6, 7, 8, 9]);
}

#[test]
fn another_orbit_turns_beside_the_first_in_slots_of_its_own() {
    let mut e = with(
        &starry(vec![targets(2)]),
        vec![orbit("ring_a", 1, 600, 600), orbit("ring_b", 1, 600, 600)],
    );
    take(&mut e, "ring_a");
    take(&mut e, "ring_b");
    e.step(&[idle()]);
    let held: Vec<(String, u8)> = orbits(&e).into_iter().map(|o| (o.0, o.1)).collect();
    assert_eq!(held, [("ring_a".to_string(), 1), ("ring_b".to_string(), 1)]);
    // The second orbit's orbs start after the most a first one holds.
    assert_eq!(slots(&e), [FIRST_SLOT, FIRST_SLOT + MAX_ORBS]);
}
