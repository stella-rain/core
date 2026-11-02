//! Warning-only zones and orbs anchored at the target (Z3a-4, DS70-1, DS70-2, DS72, DS73). A
//! `warn_only` zone warns for its wind-up and ends: it never becomes active, never hurts and gives
//! no invulnerability, whatever its phase's damage. An `orb` node with `anchor: target` spawns its
//! orbs where the target is on the tick the node fires, moved by `offset`; after that they fly as
//! any orb does.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{Error, Issue, validate};

const PX: i32 = 256;
const MAX_ZONES_ALIVE: usize = 128;

fn circle(radius: i32) -> Value {
    json!({ "type": "circle", "radius": radius })
}

fn warn(wind_up: u32) -> Value {
    json!({ "type": "zone", "shape": circle(8 * PX), "anchor": "target",
            "wind_up_ticks": wind_up, "warn_only": true })
}

fn normal(wind_up: u32, active: u32) -> Value {
    json!({ "type": "zone", "shape": circle(8 * PX), "anchor": "target",
            "wind_up_ticks": wind_up, "active_ticks": active })
}

/// A foe 100 pixels above the main character, which sends `phases` at it once.
fn stage(phases: Vec<Value>) -> Stage {
    let rules = json!([{ "when": always(), "target": "player", "once": true, "cooldown_ticks": 0,
        "do": { "type": "attack", "attack": { "inline": phases } } }]);
    let e = enemy(1000, hold(START_X, START_Y - 100 * PX), rules);
    quiet(stage_with(
        900,
        Some(far_boss()),
        vec![wave(1, 1, 0, e, START_X, START_Y - 100 * PX)],
    ))
    .with_hp(99)
}

trait WithHp {
    fn with_hp(self, hp: u32) -> Self;
}

impl WithHp for Stage {
    fn with_hp(mut self, hp: u32) -> Self {
        self.player.hp = hp;
        self
    }
}

fn spawning(damage: u32, nodes: Vec<Value>) -> Vec<Value> {
    vec![json!({ "kind": "wind_up", "ticks": 30, "damage": damage, "spawn": nodes })]
}

fn snapshot(e: &Engine) -> Value {
    serde_json::to_value(e.snapshot()).unwrap()
}

fn zones(e: &Engine) -> Vec<Value> {
    snapshot(e)["zones"].as_array().cloned().unwrap_or_default()
}

fn orbs(e: &Engine) -> Vec<Value> {
    snapshot(e)["orbs"].as_array().cloned().unwrap_or_default()
}

fn player_hits(st: &Stage, input: stella_rain_core::input::Input, ticks: u32) -> Vec<u32> {
    let events = run_until(&mut engine(st), input, ticks, |_| false);
    ticks_of(&events, |ev| matches!(ev, DomainEvent::PlayerHit { .. }))
}

fn px(v: &Value) -> (i64, i64) {
    (v["x"].as_i64().unwrap(), v["y"].as_i64().unwrap())
}

// --- warn_only zones -------------------------------------------------------------------------

#[test]
fn a_warn_only_zone_warns_for_its_wind_up_and_hurts_nothing() {
    let st = stage(spawning(7500, vec![warn(30)]));
    let mut e = engine(&st);
    let hp = e.snapshot().players[0].hp;
    let mut events = Vec::new();
    for tick in 1..=120u32 {
        e.step(&[idle()]);
        events.extend(e.events().iter().map(|ev| (e.tick(), ev.clone())));
        let z = zones(&e);
        if tick <= 30 {
            assert_eq!(z.len(), 1, "tick {tick}");
            assert_eq!(z[0]["phase"], json!("wind_up"));
            assert_eq!(z[0]["ticks"], json!(30));
            assert_eq!(z[0]["ticks_left"], json!(31 - tick));
            assert_eq!(z[0]["warn_only"], json!(true));
        } else {
            assert!(z.is_empty(), "tick {tick}");
        }
        assert_eq!(e.snapshot().players[0].invulnerable, 0, "tick {tick}");
    }
    assert_eq!(e.snapshot().players[0].hp, hp);
    assert!(
        events
            .iter()
            .all(|(_, ev)| !matches!(ev, DomainEvent::PlayerHit { .. } | DomainEvent::Hit { .. })),
        "{events:?}"
    );
}

#[test]
fn the_body_hit_after_a_warn_only_zone_lands() {
    let body = json!({ "kind": "active", "ticks": 10, "damage": 5,
        "hit_box": { "type": "circle", "anchor": "target", "radius": 8 * PX } });
    let lane = json!({ "kind": "wind_up", "ticks": 30, "damage": 5, "spawn": [warn(30)] });
    let st = stage(vec![lane, body.clone()]);
    assert_eq!(player_hits(&st, idle(), 120), vec![31]);
    // A normal zone hurts at once, and its invulnerable ticks swallow the hit at 31.
    let lane = json!({ "kind": "wind_up", "ticks": 30, "damage": 5, "spawn": [normal(0, 30)] });
    let st = stage(vec![lane, body]);
    assert_eq!(player_hits(&st, idle(), 120), vec![1]);
}

#[test]
fn a_mixed_phase_hurts_only_through_the_normal_zone() {
    let both = stage(spawning(5, vec![warn(30), normal(0, 30)]));
    let alone = stage(spawning(5, vec![normal(0, 30)]));
    let run = |st: &Stage| {
        let mut e = engine(st);
        let hits = ticks_of(&run_until(&mut e, idle(), 60, |_| false), |ev| {
            matches!(ev, DomainEvent::PlayerHit { .. })
        });
        (hits, e.snapshot().players[0].hp)
    };
    let (hits, hp) = run(&both);
    assert_eq!(hits, vec![1]);
    assert_eq!((hits, hp), run(&alone));
}

#[test]
fn a_warn_only_zone_is_counted_in_the_cap() {
    let ring = |n: u32| {
        json!({ "type": "ring", "count": n, "body": [{ "type": "zone", "shape": circle(PX),
            "anchor": "target", "wind_up_ticks": 200, "active_ticks": 5 }] })
    };
    let full = stage(spawning(5, vec![ring(64), ring(64), warn(200)]));
    let mut e = engine(&full);
    e.step(&[idle()]);
    let z = zones(&e);
    assert_eq!(z.len(), MAX_ZONES_ALIVE);
    assert!(z.iter().all(|z| z.get("warn_only").is_none()), "dropped");
    let room = stage(spawning(5, vec![ring(64), ring(63), warn(200), warn(200)]));
    let mut e = engine(&room);
    e.step(&[idle()]);
    let z = zones(&e);
    assert_eq!(z.len(), MAX_ZONES_ALIVE);
    assert_eq!(z.iter().filter(|z| z.get("warn_only").is_some()).count(), 1);
}

#[test]
fn a_wind_up_of_zero_is_clamped_and_never_a_hit() {
    // `1 - 1` is computed, so the validator lets it through and the runtime clamps it to 1.
    let node = json!({ "type": "zone", "shape": circle(8 * PX), "anchor": "target",
        "wind_up_ticks": { "sub": [1, 1] }, "warn_only": true });
    let st = stage(spawning(5, vec![node]));
    let mut e = engine(&st);
    e.step(&[idle()]);
    let z = zones(&e);
    assert_eq!(
        (z.len(), &z[0]["ticks"], &z[0]["ticks_left"]),
        (1, &json!(1), &json!(1))
    );
    assert!(player_hits(&st, idle(), 60).is_empty());
}

#[test]
fn a_normal_zone_has_no_warn_only_key() {
    let st = stage(spawning(5, vec![normal(10, 5)]));
    let mut e = engine(&st);
    e.step(&[idle()]);
    assert!(zones(&e)[0].get("warn_only").is_none());
}

// --- orbs anchored at the target -------------------------------------------------------------

fn orb(anchor: Option<&str>, forward: i32) -> Value {
    let mut n = json!({ "type": "orb", "speed": 256,
        "direction": { "type": "aimed", "offset": 0 }, "ticks": 120 });
    if let Some(a) = anchor {
        n["anchor"] = json!(a);
    }
    if forward != 0 {
        n["offset"] = json!({ "forward": forward });
    }
    n
}

/// A 30-tick wind-up, then an active phase that fires `nodes` on its first tick (tick 31).
fn firing_late(nodes: Vec<Value>) -> Stage {
    stage(vec![
        json!({ "kind": "wind_up", "ticks": 30, "damage": 5 }),
        json!({ "kind": "active", "ticks": 10, "damage": 5, "spawn": nodes }),
    ])
}

fn first_orb_tick(e: &mut Engine, input: stella_rain_core::input::Input) -> u32 {
    for _ in 0..60 {
        e.step(&[input]);
        if !orbs(e).is_empty() {
            return e.tick();
        }
    }
    panic!("no orb");
}

#[test]
fn an_orb_spawns_at_the_target_moved_by_its_offset() {
    let st = firing_late(vec![orb(Some("target"), 16 * PX)]);
    let mut e = engine(&st);
    let t = first_orb_tick(&mut e, idle());
    assert_eq!(t, 31);
    let o = &orbs(&e)[0];
    let (vx, vy) = (o["vx"].as_i64().unwrap(), o["vy"].as_i64().unwrap());
    assert_eq!(vx, 0);
    assert!(vy > 0);
    let me = e.snapshot().players[0].at;
    assert_eq!(
        px(&o["at"]),
        (
            i64::from(me.x.0),
            i64::from(me.y.0) + i64::from(16 * PX) + vy
        )
    );
    assert!(player_hits(&st, idle(), 120).is_empty());
}

#[test]
fn an_orb_samples_the_target_once_when_it_fires() {
    let st = firing_late(vec![orb(Some("target"), 16 * PX)]);
    let mut e = engine(&st);
    let start = e.snapshot().players[0].at;
    let left = moving(-127, 0);
    let t = first_orb_tick(&mut e, left);
    assert_eq!(t, 31);
    let me = e.snapshot().players[0].at;
    let o = orbs(&e)[0].clone();
    let (ox, _) = px(&o["at"]);
    assert!(
        // The 16 px offset runs along the slanted line from the foe, so the orb is some pixels off.
        (ox - i64::from(me.x.0)).abs() <= 24 * i64::from(PX),
        "near the tick-31 spot: orb {ox}, main character {}, start {}",
        me.x.0,
        start.x.0
    );
    assert!(
        i64::from(start.x.0) - ox > 40 * i64::from(PX),
        "not the tick-1 spot"
    );
    // After that it is a plain orb: velocity fixed, position advancing by exactly it.
    let (vx, vy) = (o["vx"].clone(), o["vy"].clone());
    let mut at = px(&o["at"]);
    for _ in 0..20 {
        e.step(&[left]);
        let o = &orbs(&e)[0];
        assert_eq!((&o["vx"], &o["vy"]), (&vx, &vy));
        at = (at.0 + vx.as_i64().unwrap(), at.1 + vy.as_i64().unwrap());
        assert_eq!(px(&o["at"]), at);
    }
}

#[test]
fn rounds_of_a_repeat_sample_again_when_they_fire() {
    let rep = json!({ "type": "repeat", "count": 2, "interval_ticks": 30, "body": [orb(Some("target"), 16 * PX)] });
    let st = firing_late(vec![rep]);
    let mut e = engine(&st);
    let left = moving(-127, 0);
    let mut seen = Vec::new();
    for _ in 0..70 {
        e.step(&[left]);
        let list = orbs(&e);
        if seen.len() < list.len() {
            let spawned = list.last().unwrap();
            // Undo the tick's move to get the spawn point.
            let (x, y) = px(&spawned["at"]);
            seen.push((
                x - spawned["vx"].as_i64().unwrap(),
                y - spawned["vy"].as_i64().unwrap(),
            ));
        }
    }
    assert_eq!(seen.len(), 2);
    assert!(seen[0].0 - seen[1].0 > 40 * i64::from(PX), "{seen:?}");
}

#[test]
fn without_an_anchor_the_orb_spawns_at_the_foe() {
    let st = firing_late(vec![orb(None, 0)]);
    let mut e = engine(&st);
    first_orb_tick(&mut e, idle());
    let (_, y) = px(&orbs(&e)[0]["at"]);
    assert!(y < i64::from(START_Y) - 90 * i64::from(PX), "{y}");
}

#[test]
fn an_orb_at_the_target_with_no_offset_hits_on_its_spawn_tick() {
    let st = firing_late(vec![orb(Some("target"), 0)]);
    assert_eq!(player_hits(&st, idle(), 60), vec![31]);
}

#[test]
fn a_cage_forms_round_the_target_where_it_stood_when_the_node_fired() {
    let cage = json!({ "type": "rotate", "angle": 11_520, "body": [{ "type": "ring", "count": 4,
        "body": [{ "type": "orb", "speed": 512, "ticks": 180, "stop_after": 60,
            "anchor": "target", "offset": { "forward": 4096 },
            "direction": { "type": "aimed", "offset": 0 },
            "link": { "width": 5120, "rehit_ticks": 60 } }] }] });
    let st = firing_late(vec![cage]);
    let mut e = engine(&st);
    let t0 = first_orb_tick(&mut e, idle());
    let me = e.snapshot().players[0].at;
    while e.tick() < t0 + 60 {
        e.step(&[idle()]);
    }
    let list = orbs(&e);
    assert_eq!(list.len(), 4);
    for o in &list {
        assert_eq!((o["vx"].as_i64(), o["vy"].as_i64()), (Some(0), Some(0)));
        let (x, y) = px(&o["at"]);
        // Within 1 px of 136 px, compared as squared distances in 1/256 px.
        let d2 = (x - i64::from(me.x.0)).pow(2) + (y - i64::from(me.y.0)).pow(2);
        let (lo, hi) = (135 * i64::from(PX), 137 * i64::from(PX));
        assert!(lo * lo <= d2 && d2 <= hi * hi, "{d2}");
        assert!(o.get("link_to").is_some());
    }
}

// --- Validator -------------------------------------------------------------------------------

fn issues(nodes: Vec<Value>) -> Vec<Issue> {
    match validate(&stage(spawning(5, nodes))) {
        Ok(()) => Vec::new(),
        Err(Error::Invalid(i)) => i,
        Err(other) => panic!("unexpected {other:?}"),
    }
}

#[test]
fn the_validator_judges_warn_only_and_orb_offsets() {
    let mut both = warn(30);
    both["active_ticks"] = json!(5);
    let said = |nodes| format!("{:?}", issues(nodes));
    assert!(said(vec![both]).contains("no active_ticks"));
    let mut neither = normal(5, 5);
    neither.as_object_mut().unwrap().remove("active_ticks");
    assert!(said(vec![neither]).contains("needs active_ticks"));
    assert!(!issues(vec![warn(0)]).is_empty(), "wind-up 0 is refused");
    assert!(
        issues(vec![warn(30)]).is_empty(),
        "the phase's damage is ignored"
    );
    assert!(issues(vec![orb(Some("target"), 16 * PX)]).is_empty());
    assert!(!issues(vec![orb(Some("target"), 2_000_000)]).is_empty());
}
