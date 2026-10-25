//! The orb cage (Z3a-3, DS11 item 4): an `orb` node's `stop_after` halts the orb after that many
//! ticks, and its `link` joins the orbs of one ring with beams once they are halted. A beam is a
//! rectangle of `width` between two neighbouring live orbs; it hurts the main character, at most
//! once per `rehit_ticks`. A broken orb takes its two beams with it. `OrbView::link_to` names the
//! neighbour a beam runs to.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{Error, Issue, validate};

const PX: i32 = 256;

/// Orbs fly at 2 pixels a tick (120 px/s), and halt after 30 of them: 60 pixels from the foe.
const SPEED: i32 = 2 * PX;
const STOP_AFTER: u32 = 30;
const LIFE: u32 = 600;
const REHIT: u32 = 60;

fn orb(stop_after: Option<u32>, link: Option<Value>) -> Value {
    let mut node = json!({ "type": "orb", "speed": SPEED,
        "direction": { "type": "absolute", "angle": 0 }, "ticks": LIFE, "breakable": true });
    if let Some(n) = stop_after {
        node["stop_after"] = json!(n);
    }
    if let Some(link) = link {
        node["link"] = link;
    }
    node
}

fn link() -> Value {
    json!({ "width": 8 * PX, "rehit_ticks": REHIT })
}

/// A ring of `count` orbs that halt and are linked. Slot 0 flies straight down; the ring turns
/// each further orb a quarter turn for 4 (toward the right, which is the screen's east first).
fn cage(count: u32) -> Value {
    json!({ "type": "ring", "count": count, "body": [orb(Some(STOP_AFTER), Some(link()))] })
}

fn attack(phases: Value) -> Value {
    json!([{ "when": always(), "target": "player", "once": true, "cooldown_ticks": 0,
             "do": { "type": "attack", "attack": { "inline": phases } } }])
}

/// A foe that fires `nodes` once, at `(x, y)`.
fn foe(x: i32, y: i32, nodes: Vec<Value>) -> Value {
    let phases = json!([{ "kind": "wind_up", "ticks": 1, "damage": 1, "spawn": nodes }]);
    wave(1, 1, 0, enemy(1000, hold(x, y), attack(phases)), x, y)
}

fn stage(waves: Vec<Value>) -> Stage {
    quiet(stage_with(900, Some(far_boss()), waves))
}

/// The main character stands at `(START_X, START_Y)`; the foe `(dx, dy)` pixels from it.
fn foe_near(dx: i32, dy: i32, nodes: Vec<Value>) -> Value {
    foe(START_X + dx * PX, START_Y - dy * PX, nodes)
}

/// A sword that reaches 12 pixels round the main character.
fn sword() -> Value {
    weapon_ref(
        json!({ "inline": [{ "kind": "active", "ticks": 3, "damage": 1,
        "hit_box": { "type": "circle", "radius": 12 * PX } }] }),
    )
}

fn orbs(e: &Engine) -> Vec<Value> {
    let snapshot = serde_json::to_value(e.snapshot()).unwrap();
    snapshot["orbs"].as_array().cloned().unwrap_or_default()
}

fn beams(e: &Engine) -> usize {
    orbs(e)
        .iter()
        .filter(|o| o.get("link_to").is_some())
        .count()
}

fn hits(events: &[(u32, DomainEvent)]) -> Vec<u32> {
    events
        .iter()
        .filter(|(_, ev)| serde_json::to_value(ev).unwrap()["type"] == json!("PlayerHit"))
        .map(|(t, _)| *t)
        .collect()
}

/// Steps idle until `n` orbs are out; returns the tick they appeared on.
fn until_orbs(e: &mut Engine, n: usize) -> u32 {
    for _ in 0..20 {
        e.step(idle());
        if orbs(e).len() == n {
            return e.tick();
        }
    }
    panic!("{n} orbs never appeared");
}

/// The foe is `(-6, 54)` pixels from the main character in the geometry below: the four orbs halt
/// 60 pixels from it, at its south, east, north and west (slots 0 to 3), so the beam from slot 3
/// to slot 0 runs through the main character, 8.5 pixels from the south orb, and the
/// orbs' paths miss it by more than the touching distance of 5 pixels.
fn on_the_wrap_beam(nodes: Vec<Value>) -> Stage {
    stage(vec![foe_near(6, 54, nodes)])
}

#[test]
fn orbs_halt_after_stop_after_ticks_and_keep_their_lifetime() {
    let st = stage(vec![foe_near(0, 200, vec![cage(4)])]);
    let mut e = engine(&st);
    let t0 = until_orbs(&mut e, 4);
    let moving = |e: &Engine| {
        orbs(e)
            .iter()
            .any(|o| o["vx"] != json!(0) || o["vy"] != json!(0))
    };
    for _ in 1..STOP_AFTER - 1 {
        e.step(idle());
        assert!(moving(&e), "flying at tick {}", e.tick());
    }
    // The orb moves on the tick it is fired, so it has moved `STOP_AFTER` times, and halted,
    // on the 30th tick of its life.
    e.step(idle());
    assert_eq!(e.tick(), t0 + STOP_AFTER - 1);
    for o in orbs(&e) {
        assert_eq!((o["vx"].as_i64(), o["vy"].as_i64()), (Some(0), Some(0)));
    }
    let halted = orbs(&e);
    for _ in 0..100 {
        e.step(idle());
    }
    assert_eq!(
        orbs(&e).iter().map(|o| &o["at"]).collect::<Vec<_>>(),
        halted.iter().map(|o| &o["at"]).collect::<Vec<_>>()
    );
    // Its remaining life runs on: it is on the map for its 600 ticks all the same.
    while e.tick() < t0 + LIFE - 1 {
        e.step(idle());
    }
    assert_eq!(orbs(&e).len(), 4, "tick {}", e.tick());
    e.step(idle());
    assert!(orbs(&e).is_empty(), "tick {}", e.tick());
}

#[test]
fn a_ring_of_four_has_four_beams_once_halted() {
    let st = stage(vec![foe_near(0, 200, vec![cage(4)])]);
    let mut e = engine(&st);
    let t0 = until_orbs(&mut e, 4);
    while e.tick() < t0 + STOP_AFTER - 2 {
        e.step(idle());
        assert_eq!(beams(&e), 0, "no beam while they fly (tick {})", e.tick());
    }
    e.step(idle());
    let list = orbs(&e);
    assert_eq!(beams(&e), 4);
    // Slot j is joined to slot j + 1, and the last to the first.
    for (j, o) in list.iter().enumerate() {
        assert_eq!(o["link_to"], json!((j + 1) % 4), "{list:?}");
    }
}

#[test]
fn the_beam_hurts_the_main_character_once_per_rehit_interval() {
    let st = on_the_wrap_beam(vec![cage(4)]);
    let mut e = engine(&st);
    let t0 = until_orbs(&mut e, 4);
    let mut events = Vec::new();
    for _ in 0..200 {
        e.step(idle());
        events.extend(e.events().iter().map(|ev| (e.tick(), ev.clone())));
    }
    let landed = hits(&events);
    assert!(landed.len() >= 3, "{landed:?}");
    assert!(landed[0] >= t0 + STOP_AFTER - 1, "{landed:?} from {t0}");
    for w in landed.windows(2) {
        assert_eq!(w[1] - w[0], REHIT, "{landed:?}");
    }
}

#[test]
fn breaking_the_orb_at_slot_0_cuts_its_two_beams() {
    // The same stage, with a sword: it reaches the south orb, but not the east or west one.
    let st = armed(on_the_wrap_beam(vec![cage(4)]), sword());
    let mut e = engine(&st);
    let mut events = Vec::new();
    for tick in 0..200 {
        e.step(firing(idle(), tick));
        events.extend(e.events().iter().map(|ev| (e.tick(), ev.clone())));
    }
    let broke = events
        .iter()
        .any(|(_, ev)| serde_json::to_value(ev).unwrap()["type"] == json!("OrbBroken"));
    assert!(broke, "{events:?}");
    let list = orbs(&e);
    assert_eq!(list.len(), 3);
    // Slots 1, 2, 3 are left: beams 1-2 and 2-3, and none from 3 to the gone 0.
    assert_eq!(beams(&e), 2, "{list:?}");
    assert_eq!(list[0]["link_to"], json!(1));
    assert_eq!(list[1]["link_to"], json!(2));
    assert!(list[2].get("link_to").is_none());
    // The main character stood on the old 3-0 beam, and nothing else reaches it.
    assert!(hits(&events).is_empty(), "{events:?}");
}

#[test]
fn two_cages_make_two_groups_and_no_beam_joins_them() {
    let st = stage(vec![
        foe_near(-150, 200, vec![cage(4)]),
        foe_near(150, 200, vec![cage(4)]),
    ]);
    let mut e = engine(&st);
    let t0 = until_orbs(&mut e, 8);
    while e.tick() < t0 + STOP_AFTER {
        e.step(idle());
    }
    let list = orbs(&e);
    assert_eq!(beams(&e), 8);
    let x = |i: usize| list[i]["at"]["x"].as_i64().unwrap();
    for (i, o) in list.iter().enumerate() {
        let j = usize::try_from(o["link_to"].as_u64().unwrap()).unwrap();
        // A cage is 120 pixels across; the two are 300 pixels apart.
        assert!((x(i) - x(j)).abs() <= 120 * i64::from(PX), "{list:?}");
    }
}

#[test]
fn a_ring_of_two_has_one_beam() {
    let st = stage(vec![foe_near(0, 200, vec![cage(2)])]);
    let mut e = engine(&st);
    let t0 = until_orbs(&mut e, 2);
    while e.tick() < t0 + STOP_AFTER {
        e.step(idle());
    }
    assert_eq!(beams(&e), 1);
}

#[test]
fn a_ring_cut_short_by_the_cap_links_the_orbs_that_were_made() {
    // A ring of 5, then a ring of 4 of which the cap of 8 per owner leaves 3: 5 + 3 beams.
    let st = stage(vec![foe_near(0, 200, vec![cage(5), cage(4)])]);
    let mut e = engine(&st);
    let t0 = until_orbs(&mut e, 8);
    while e.tick() < t0 + STOP_AFTER {
        e.step(idle());
    }
    assert_eq!(beams(&e), 8);
    let list = orbs(&e);
    // The last three are a ring of their own.
    assert_eq!(list[5]["link_to"], json!(6));
    assert_eq!(list[6]["link_to"], json!(7));
    assert_eq!(list[7]["link_to"], json!(5));
}

// --- The validator ---------------------------------------------------------------------------

fn issues(stage: &Stage) -> Vec<Issue> {
    match validate(stage) {
        Err(Error::Invalid(issues)) => issues,
        other => panic!("expected issues, got {other:?}"),
    }
}

fn refused(node: Value, what: &str) {
    let st = stage(vec![foe_near(0, 200, vec![node])]);
    let found = issues(&st);
    assert!(
        found.iter().any(|i| format!("{i:?}").contains(what)),
        "{what}: {found:?}"
    );
}

fn ring_of(count: u32, orb: Value) -> Value {
    json!({ "type": "ring", "count": count, "body": [orb] })
}

#[test]
fn the_validator_allows_a_cage() {
    validate(&stage(vec![foe_near(0, 200, vec![cage(4)])])).expect("a cage is valid");
    let widest = json!({ "width": 64 * PX, "rehit_ticks": 600 });
    let node = ring_of(2, orb(Some(LIFE), Some(widest)));
    validate(&stage(vec![foe_near(0, 200, vec![node])])).expect("the limits are valid");
}

#[test]
fn the_validator_bounds_a_links_width_and_rehit_interval() {
    for width in [0, 64 * PX + 1] {
        let l = json!({ "width": width, "rehit_ticks": REHIT });
        refused(ring_of(4, orb(Some(30), Some(l))), "width");
    }
    for rehit in [0, 601] {
        let l = json!({ "width": 8 * PX, "rehit_ticks": rehit });
        refused(ring_of(4, orb(Some(30), Some(l))), "rehit_ticks");
    }
}

#[test]
fn the_validator_bounds_stop_after_by_ticks() {
    refused(ring_of(4, orb(Some(LIFE + 1), Some(link()))), "stop_after");
}

#[test]
fn the_validator_refuses_a_link_with_a_single_orb() {
    refused(orb(Some(30), Some(link())), "link");
    refused(ring_of(1, orb(Some(30), Some(link()))), "link");
}
