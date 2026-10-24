//! Orbs (DS10, DS12, core Z2): an `orb` node fires a slow projectile with a required lifetime. It
//! ends when its ticks are up, and then its `burst` (zones and the nodes that place them) runs
//! where the orb ended. An orb the main character's hit box touches breaks when it is `breakable`:
//! no burst, one `OrbBroken` event. At most 8 orbs alive per owner and 32 in all
//! (`MAX_ORBS_PER_OWNER`, `MAX_ORBS_ALIVE`); one over a cap is dropped, in node order. Orbs are
//! enemy attacks only.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{Error, Issue, validate};

const PX: i32 = 256;
/// The caps of `rules_v0`.
const MAX_ORBS_PER_OWNER: usize = 8;
const MAX_ORBS_ALIVE: usize = 32;

/// The enemy appears and attacks on tick 1, and its orbs with it.
const FIRST_TICK: u32 = 1;

fn orb(ticks: u32, breakable: bool, burst: Vec<Value>) -> Value {
    json!({ "type": "orb", "speed": 64, "direction": { "type": "aimed", "offset": 0 },
            "ticks": ticks, "breakable": breakable, "burst": burst })
}

fn zone() -> Value {
    json!({ "type": "zone", "shape": { "type": "circle", "radius": 4 * PX }, "anchor": "attacker",
            "wind_up_ticks": 3, "active_ticks": 5 })
}

fn attack(phases: Value) -> Value {
    json!([{ "when": always(), "target": "player", "once": true, "cooldown_ticks": 0,
             "do": { "type": "attack", "attack": { "inline": phases } } }])
}

fn spawning(nodes: Vec<Value>) -> Value {
    json!([{ "kind": "wind_up", "ticks": 1, "damage": 5, "spawn": nodes }])
}

/// A foe that sends `nodes` at the main character once, standing `dy` pixels above it.
fn foe(x: i32, dy: i32, nodes: Vec<Value>) -> Value {
    let y = START_Y - dy * PX;
    wave(
        1,
        1,
        0,
        enemy(1000, hold(x, y), attack(spawning(nodes))),
        x,
        y,
    )
}

fn stage(waves: Vec<Value>) -> Stage {
    quiet(stage_with(600, Some(far_boss()), waves))
}

fn bullets(e: &Engine) -> Vec<Value> {
    let snapshot = serde_json::to_value(e.snapshot()).unwrap();
    snapshot["bullets"].as_array().cloned().unwrap_or_default()
}

fn orbs(e: &Engine) -> Vec<Value> {
    bullets(e)
        .into_iter()
        .filter(|b| b["orb"] == json!(true))
        .collect()
}

fn zones(e: &Engine) -> Vec<Value> {
    let snapshot = serde_json::to_value(e.snapshot()).unwrap();
    snapshot["zones"].as_array().cloned().unwrap_or_default()
}

fn broken(events: &[(u32, DomainEvent)]) -> Vec<u32> {
    events
        .iter()
        .filter(|(_, ev)| serde_json::to_value(ev).unwrap()["type"] == json!("OrbBroken"))
        .map(|(t, _)| *t)
        .collect()
}

// --- Lifetime and burst ----------------------------------------------------------------------

#[test]
fn the_lifetime_ends_the_orb() {
    let st = stage(vec![foe(START_X, 100, vec![orb(30, false, vec![])])]);
    let mut e = engine(&st);
    let mut seen = Vec::new();
    for _ in 0..60 {
        e.step(idle());
        seen.push(orbs(&e).len());
    }
    // On the map for exactly its 30 ticks, from the tick it was fired.
    let first = FIRST_TICK as usize - 1;
    assert!(seen[..first].iter().all(|n| *n == 0), "{seen:?}");
    assert!(seen[first..first + 30].iter().all(|n| *n == 1), "{seen:?}");
    assert!(seen[first + 30..].iter().all(|n| *n == 0), "{seen:?}");
}

#[test]
fn the_burst_places_its_zone_where_the_orb_ended() {
    let st = stage(vec![foe(START_X, 100, vec![orb(20, false, vec![zone()])])]);
    let mut e = engine(&st);
    let mut last_orb = None;
    let mut placed = None;
    for _ in 0..40 {
        e.step(idle());
        match (orbs(&e).first(), zones(&e).first()) {
            (Some(o), _) => last_orb = Some(o["at"].clone()),
            (None, Some(z)) => {
                placed = Some((e.tick(), z["at"].clone()));
                break;
            }
            (None, None) => {}
        }
    }
    let (tick, at) = placed.expect("the burst placed its zone");
    assert_eq!(tick, FIRST_TICK + 20);
    assert_eq!(Some(at), last_orb);
}

// --- Caps ------------------------------------------------------------------------------------

#[test]
fn the_per_owner_cap_drops_the_orbs_over_it_in_node_order() {
    // Ten orbs from one foe, each turned 10 degrees further from straight down (loop index times
    // 2,560): the eight that stay are the first eight.
    let turned = json!({ "type": "repeat", "count": 10, "interval_ticks": 0, "body": [{
        "type": "orb", "speed": 64, "ticks": 100, "breakable": false, "burst": [],
        "direction": { "type": "absolute", "angle": { "mul": ["loop_index", 2560] } } }] });
    let st = stage(vec![foe(START_X, 100, vec![turned])]);
    let mut e = engine(&st);
    e.step(idle());
    e.step(idle());
    let kept = orbs(&e);
    assert_eq!(kept.len(), MAX_ORBS_PER_OWNER);
    // 70 degrees of a speed of 64 is 60; 80 degrees is 63.
    assert!(
        kept.iter().all(|o| o["vx"].as_i64().unwrap() <= 60),
        "{kept:?}"
    );
}

#[test]
fn the_global_cap_drops_the_orbs_over_it_in_node_order() {
    // Five foes, eight orbs each: the first four fill the 32, the fifth's are all dropped.
    let ring = json!({ "type": "ring", "count": 8, "body": [orb(100, false, vec![])] });
    let xs: Vec<i32> = (0..5).map(|i| START_X + (i - 2) * 40 * PX).collect();
    let st = stage(
        xs.iter()
            .map(|x| foe(*x, 100, vec![ring.clone()]))
            .collect(),
    );
    let mut e = engine(&st);
    e.step(idle());
    e.step(idle());
    let kept = orbs(&e);
    assert_eq!(kept.len(), MAX_ORBS_ALIVE);
    let near = |o: &Value, x: i32| (o["at"]["x"].as_i64().unwrap() - i64::from(x)).abs() < 4 * 256;
    assert!(kept.iter().all(|o| !near(o, xs[4])), "{kept:?}");
    assert!(kept.iter().any(|o| near(o, xs[3])));
}

// --- Breaking --------------------------------------------------------------------------------

/// A sword swing: a 40-pixel circle round the main character, active for three ticks.
fn sword() -> Value {
    weapon_ref(
        json!({ "inline": [{ "kind": "active", "ticks": 3, "damage": 1,
        "hit_box": { "type": "circle", "radius": 40 * PX } }] }),
    )
}

/// The engine after 60 ticks, the events, and whether a zone was ever placed.
fn swung(breakable: bool) -> (Engine, Vec<(u32, DomainEvent)>, bool) {
    // The foe stands 30 pixels above the main character, inside the swing, and its burst would
    // place a zone if the orb ended by its lifetime.
    let st = armed(
        stage(vec![foe(
            START_X,
            30,
            vec![orb(40, breakable, vec![zone()])],
        )]),
        sword(),
    );
    let mut e = engine(&st);
    let mut events = Vec::new();
    let mut zoned = false;
    for tick in 0..60 {
        e.step(firing(idle(), tick));
        events.extend(e.events().iter().map(|ev| (e.tick(), ev.clone())));
        zoned |= !zones(&e).is_empty();
    }
    (e, events, zoned)
}

#[test]
fn a_sword_hit_box_breaks_a_breakable_orb_without_a_burst() {
    let (e, events, zoned) = swung(true);
    assert_eq!(broken(&events).len(), 1, "{events:?}");
    assert!(orbs(&e).is_empty());
    assert!(!zoned);
}

#[test]
fn a_non_breakable_orb_survives_the_sword() {
    let (e, events, zoned) = swung(false);
    assert!(broken(&events).is_empty());
    // It lives its 40 ticks and then bursts.
    assert!(orbs(&e).is_empty());
    assert!(zoned);
}

// --- The validator ---------------------------------------------------------------------------

fn issues(stage: &Stage) -> Vec<Issue> {
    match validate(stage) {
        Err(Error::Invalid(issues)) => issues,
        other => panic!("expected issues, got {other:?}"),
    }
}

#[test]
fn the_validator_allows_an_enemy_orb() {
    let st = stage(vec![foe(START_X, 100, vec![orb(30, true, vec![zone()])])]);
    validate(&st).expect("an enemy may fire an orb");
}

#[test]
fn the_validator_refuses_a_friendly_orb() {
    let phases = spawning(vec![orb(30, false, vec![])]);
    let st = quiet(stage_full(
        600,
        Some(far_boss()),
        vec![],
        vec![companion(5, hold(0, 0), attack(phases.clone()))],
    ));
    assert!(
        issues(&st).iter().any(|i| i.message.contains("orb")),
        "companion"
    );
    let weapon = armed(stage(vec![]), weapon_ref(json!({ "inline": phases })));
    assert!(
        issues(&weapon).iter().any(|i| i.message.contains("orb")),
        "weapon"
    );
}

#[test]
fn the_validator_refuses_a_fire_node_in_a_burst() {
    let fire =
        json!({ "type": "fire", "speed": 64, "direction": { "type": "aimed", "offset": 0 } });
    let nested = json!({ "type": "ring", "count": 2, "body": [fire.clone()] });
    for node in [fire, nested] {
        let st = stage(vec![foe(START_X, 100, vec![orb(30, false, vec![node])])]);
        assert!(
            issues(&st).iter().any(|i| i.message.contains("burst")),
            "a burst holds no fire"
        );
    }
}

#[test]
fn the_validator_bounds_an_orbs_ticks() {
    for ticks in [0, 601] {
        let st = stage(vec![foe(START_X, 100, vec![orb(ticks, false, vec![])])]);
        assert!(!issues(&st).is_empty(), "{ticks} ticks");
    }
    let st = stage(vec![foe(START_X, 100, vec![orb(600, false, vec![])])]);
    validate(&st).expect("600 ticks is the limit");
}
