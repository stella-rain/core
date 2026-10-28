//! The charge, the shrine pulse and the orb cage in the bundled `time_limit` content (Z3b, DS11
//! items 3, 4, 7 and 9): Horn Charger C (a K5 lane charge that ends in a shockwave ring), `elite_mid`
//! B (three lobs and a lane charge), Weaver C phase 3 (four orbs that stop and are joined by beams)
//! and `elite_shrine` B (a pulse during its fight). Every number is a placeholder (ADR-126). The
//! rules they use are covered by `dash.rs`, `orb_cage.rs`, `zones.rs` and `elite_shrine.rs`.

use crate::common;
use stella_rain_core::version::Versions;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::attack::ZoneShape;
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::fixed::Fx;
use stella_rain_core::input::Input;
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::snapshot::{EntityKind, ZoneView};
use stella_rain_core::stage::Stage;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::trig;

const PX: i32 = 256;

fn time_limit() -> Value {
    serde_json::from_slice(common::bundled_file("time_limit")).expect("the bundled file is JSON")
}

fn boss_def(id: &str) -> Value {
    time_limit()["bosses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["base"] == id)
        .unwrap_or_else(|| panic!("no boss {id}"))
        .clone()
}

fn elite_def(id: &str) -> Value {
    time_limit()["elite_types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .unwrap_or_else(|| panic!("no elite {id}"))
        .clone()
}

/// Every object and the path it is at, depth first.
fn objects<'a>(v: &'a Value, path: String, out: &mut Vec<(String, &'a Value)>) {
    match v {
        Value::Object(map) => {
            out.push((path.clone(), v));
            for (k, child) in map {
                objects(child, format!("{path}.{k}"), out);
            }
        }
        Value::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                objects(child, format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

fn all_objects(v: &Value) -> Vec<(String, &Value)> {
    let mut out = Vec::new();
    objects(v, "$".into(), &mut out);
    out
}

/// The attacks of a timeline, in order.
fn attacks(timeline: &Value) -> Vec<&Value> {
    timeline
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["type"] == "attack")
        .map(|s| &s["attack"]["inline"])
        .collect()
}

fn step_kinds(timeline: &Value) -> Vec<String> {
    timeline
        .as_array()
        .unwrap()
        .iter()
        .map(|s| match s["type"].as_str().unwrap() {
            "wait" => format!("wait {}", s["ticks"]),
            other => other.to_string(),
        })
        .collect()
}

// --- Content ----------------------------------------------------------------------------------

#[test]
fn the_bundled_mode_validates() {
    TimeLimit::parse(common::bundled_file("time_limit"), Versions::CURRENT)
        .unwrap_or_else(|e| panic!("{e}"));
}

/// The two `warn_only` lines of a charge: 240 px long, laid end to end along the aimed facing.
fn assert_lanes(
    spawn: &[Value],
    forwards_wanted: &[i64],
    length: i64,
    width: i64,
    wind_up: u64,
    what: &str,
) {
    let lanes: Vec<&Value> = spawn.iter().filter(|n| n["type"] == "zone").collect();
    let forwards: Vec<_> = lanes
        .iter()
        .map(|z| z["offset"]["forward"].as_i64().unwrap_or(0))
        .collect();
    assert_eq!(forwards, forwards_wanted, "{what}: the lanes' offsets");
    for z in lanes {
        assert_eq!(
            z["shape"],
            json!({ "type": "line", "length": length, "width": width })
        );
        assert_eq!(z["warn_only"], true, "{what}");
        assert_eq!(z["wind_up_ticks"], wind_up, "{what}");
        assert!(
            z.get("active_ticks").is_none(),
            "{what}: a warning has none"
        );
        assert!(
            z.get("direction")
                .is_none_or(|d| *d == json!({ "type": "aimed", "offset": 0 })),
            "{what}: aimed"
        );
        assert!(z.get("anchor").is_none(), "{what}: at the attacker");
    }
}

fn assert_charge(attack: &Value, times: [u64; 3], with_orbs: bool, what: &str) {
    let phases = attack.as_array().unwrap();
    let kinds: Vec<_> = phases.iter().map(|p| p["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["wind_up", "active", "recovery"], "{what}");
    for (p, ticks) in phases.iter().zip(times) {
        assert_eq!(p["ticks"], ticks, "{what}");
        assert_eq!(p["move_pct"], 0, "{what}: rooted");
    }
    assert_eq!(phases[0]["damage"], 7500, "{what}");
    assert_lanes(
        phases[0]["spawn"].as_array().unwrap(),
        &[0, 61_440],
        61_440,
        12_288,
        times[0],
        what,
    );
    assert_eq!(phases[1]["damage"], 7500, "{what}");
    assert_eq!(phases[1]["dash"], json!({ "distance": 122_880 }), "{what}");
    assert_eq!(phases[1]["hit_box"]["type"], "circle", "{what}");
    assert_eq!(phases[1]["hit_box"]["radius"], 8192, "{what}");
    assert_eq!(phases[2]["damage"], 5000, "{what}");
    let spawn = phases[2]["spawn"].as_array().unwrap();
    let ring = &spawn[0];
    assert_eq!(ring["type"], "ring", "{what}");
    assert_eq!(ring["count"], 10, "{what}");
    let zone = &ring["body"][0];
    assert_eq!(
        zone["shape"],
        json!({ "type": "circle", "radius": 7168 }),
        "{what}"
    );
    assert_eq!(zone["offset"]["forward"], 24_576, "{what}");
    assert_eq!(
        zone["direction"],
        json!({ "type": "absolute", "angle": 0 }),
        "{what}"
    );
    assert_eq!(
        (zone["wind_up_ticks"].clone(), zone["active_ticks"].clone()),
        (json!(40), json!(4))
    );
    if with_orbs {
        assert_eq!(spawn.len(), 2, "{what}");
        let spread = &spawn[1];
        assert_eq!(
            (spread["type"].clone(), spread["count"].clone()),
            (json!("spread"), json!(2))
        );
        assert_eq!(spread["angle"], 7680, "{what}");
        let orb = &spread["body"][0];
        assert_eq!(orb["type"], "orb", "{what}");
        assert_eq!(orb["speed"], 256, "{what}");
        assert_eq!(
            orb["direction"],
            json!({ "type": "aimed", "offset": 0 }),
            "{what}"
        );
        assert_eq!(orb["ticks"], 180, "{what}");
        assert_eq!(
            orb["burst"][0]["shape"],
            json!({ "type": "circle", "radius": 10_240 })
        );
    } else {
        assert_eq!(spawn.len(), 1, "{what}: the ring only");
    }
}

#[test]
fn horn_charger_charges_along_a_lane_and_ends_in_a_ring() {
    let b = boss_def("horn_charger");
    assert_eq!(
        (b["hp"].clone(), b["radius"].clone()),
        (json!(150_000), json!(8192))
    );
    let phases = b["phases"].as_array().unwrap();
    assert_eq!(phases.len(), 2);
    assert_eq!(
        phases[0]["until"],
        json!({ "type": "hp_below", "who": "self", "pct": 50 })
    );
    assert_eq!(
        phases[1]["transition"],
        json!([{ "type": "clear_zones" }, { "type": "invulnerable_ticks", "ticks": 60 }])
    );
    for (i, p) in phases.iter().enumerate() {
        for step in p["timeline"].as_array().unwrap() {
            assert_ne!(step["type"], "move_to", "phase {i}");
            assert_ne!(step["type"], "telegraph", "phase {i}");
        }
    }
    let one = &phases[0]["timeline"];
    assert_eq!(step_kinds(one), ["attack", "wait 170", "loop"]);
    assert_charge(attacks(one)[0], [60, 20, 60], false, "phase 1");

    let two = &phases[1]["timeline"];
    assert_eq!(
        step_kinds(two),
        [
            "attack", "wait 86", "attack", "wait 86", "attack", "wait 146", "loop"
        ]
    );
    for a in attacks(two) {
        assert_charge(a, [40, 16, 30], true, "phase 2");
    }
}

#[test]
fn elite_mid_lobs_three_and_charges() {
    let e = elite_def("elite_mid");
    let enemy = &e["enemy"];
    assert_eq!(
        enemy["movement"],
        json!({ "type": "follow", "distance": 12_288 })
    );
    assert_eq!(enemy["elite"], json!({ "range": 51_200 }));
    let rules = enemy["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 2);
    assert_eq!(
        rules[0],
        json!({
            "when": { "type": "always" }, "target": "nearest_ally",
            "do": { "type": "attack", "attack": { "inline": [
                { "kind": "wind_up", "ticks": 55, "damage": 4000, "spawn": [
                    { "type": "ring", "count": 3, "body": [
                        { "type": "zone", "shape": { "type": "circle", "radius": 7168 },
                          "anchor": "target", "offset": { "forward": 10_240 },
                          "wind_up_ticks": 55, "active_ticks": 4 } ] } ] },
                { "kind": "active", "ticks": 4 },
                { "kind": "recovery", "ticks": 20 } ] } },
            "cooldown_ticks": 120, "once": false })
    );
    let r = &rules[1];
    assert_eq!(r["when"], json!({ "type": "always" }));
    assert_eq!(r["target"], "nearest_ally");
    assert_eq!(r["cooldown_ticks"], 210);
    assert_eq!(r["once"], false);
    assert!(
        r["do"].get("ranged").is_none_or(|v| *v == false),
        "not ranged"
    );
    let phases = r["do"]["attack"]["inline"].as_array().unwrap();
    let kinds: Vec<_> = phases.iter().map(|p| p["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["wind_up", "active", "recovery"]);
    assert_eq!(
        phases
            .iter()
            .map(|p| p["ticks"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [50, 14, 50]
    );
    assert!(phases.iter().all(|p| p["move_pct"] == 0));
    assert_eq!(phases[0]["damage"], 6000);
    assert_lanes(
        phases[0]["spawn"].as_array().unwrap(),
        &[0],
        61_440,
        10_240,
        50,
        "elite_mid",
    );
    assert_eq!(
        phases[0]["spawn"].as_array().unwrap().len(),
        1,
        "one aimed line"
    );
    // The line is the dash's own length: only its first node is 240 px, so it sits at forward 0.
    assert_eq!(phases[1]["damage"], 6000);
    assert_eq!(phases[1]["dash"], json!({ "distance": 61_440 }));
    assert_eq!(phases[1]["hit_box"]["type"], "circle");
    assert_eq!(phases[1]["hit_box"]["radius"], 6144);
}

#[test]
fn weaver_phase_three_is_a_cage_of_four_halted_orbs() {
    let b = boss_def("weaver");
    let phase = &b["phases"][2];
    let orbs: Vec<_> = all_objects(phase)
        .into_iter()
        .filter(|(_, o)| o["type"] == "orb")
        .collect();
    assert_eq!(orbs.len(), 1, "one orb node");
    let cage = attacks(&phase["timeline"])[1];
    let spawn = &cage[0]["spawn"];
    assert_eq!(spawn.as_array().unwrap().len(), 1);
    assert_eq!(spawn[0]["type"], "rotate");
    assert_eq!(spawn[0]["angle"], 11_520);
    let ring = &spawn[0]["body"][0];
    assert_eq!(
        (ring["type"].clone(), ring["count"].clone()),
        (json!("ring"), json!(4))
    );
    let orb = &ring["body"][0];
    assert_eq!(orb["type"], "orb");
    assert_eq!(orb["speed"], 512);
    assert_eq!(orb["direction"], json!({ "type": "aimed", "offset": 0 }));
    assert_eq!(orb["anchor"], "target");
    // Placed 16 px out, so that none starts inside the main character and hits it at once.
    assert_eq!(orb["offset"], json!({ "forward": 4096 }));
    assert_eq!(orb["ticks"], 180);
    assert_eq!(orb["stop_after"], 60);
    assert_eq!(orb["breakable"], true);
    assert_eq!(orb["link"], json!({ "width": 5120, "rehit_ticks": 60 }));
    assert!(orb.get("burst").is_none(), "no burst");
    assert_eq!(cage[0]["damage"], 5000, "the beams use the phase's damage");
}

#[test]
fn nothing_else_halts_or_links_an_orb() {
    let v = time_limit();
    let mut found = Vec::new();
    for (path, o) in all_objects(&v) {
        if o.get("stop_after").is_some() || o.get("link").is_some() {
            found.push(path);
        }
    }
    assert_eq!(found.len(), 1, "{found:?}");
    let weaver = v["bosses"]
        .as_array()
        .unwrap()
        .iter()
        .position(|b| b["base"] == "weaver")
        .unwrap();
    let prefix = format!("$.bosses[{weaver}].phases[2].");
    assert!(found[0].starts_with(&prefix), "{found:?}");
}

#[test]
fn the_shrine_pulses_during_its_fight() {
    let e = elite_def("elite_shrine");
    let enemy = &e["enemy"];
    let rules = enemy["rules"].as_array().cloned().unwrap_or_default();
    assert_eq!(rules.len(), 1);
    let r = &rules[0];
    assert_eq!(r["when"], json!({ "type": "always" }));
    assert_eq!(r["target"], "player");
    assert_eq!(r["cooldown_ticks"], 300);
    assert_eq!(r["once"], false);
    let phases = r["do"]["attack"]["inline"].as_array().unwrap();
    assert_eq!(phases.len(), 1);
    assert_eq!(
        (phases[0]["kind"].clone(), phases[0]["ticks"].clone()),
        (json!("active"), json!(1))
    );
    assert_eq!(phases[0]["damage"], 5000);
    let spawn = phases[0]["spawn"].as_array().unwrap();
    assert_eq!(spawn.len(), 1);
    let z = &spawn[0];
    assert_eq!(z["type"], "zone");
    assert_eq!(z["shape"], json!({ "type": "circle", "radius": 16_384 }));
    assert!(z.get("anchor").is_none(), "at the attacker");
    assert_eq!(
        (z["wind_up_ticks"].clone(), z["active_ticks"].clone()),
        (json!(60), json!(4))
    );
    // What was there before is as it was.
    let m = &enemy["movement"];
    assert_eq!(m["fight_ticks"], 1800);
    assert_eq!(m["guardians"]["count"], 6);
    assert_eq!(m["guardians"]["radius"], 20_480);
    assert_eq!(m["guardians"]["enemy"]["hp"], 6000);
    assert_eq!(
        m["guardians"]["enemy"]["rules"][0]["do"]["attack"],
        json!({ "preset": { "id": "slash" } })
    );
    assert_eq!(
        enemy["elite"],
        json!({ "range": 51_200, "boss_call": "stay" })
    );
}

// --- Behaviour --------------------------------------------------------------------------------

/// A stage with the bundled boss `base`, its phase `phase` alone, at `(x, y)` pixels; the main
/// character has the hit points to outlast the test and a weapon that does nothing.
fn boss_stage(base: &str, phase: usize, at: (i32, i32), weapon: Option<Value>) -> Stage {
    let mut boss = boss_def(base);
    let mut p = boss["phases"][phase].clone();
    p.as_object_mut().unwrap().remove("until");
    p["transition"] = json!([]);
    boss["phases"] = json!([p]);
    boss["spawn"] = json!({ "x": at.0 * PX, "y": at.1 * PX });
    boss.as_object_mut().unwrap().remove("call_elites");
    let mut stage = stage_with(3_000, Some(boss), vec![]);
    stage.player.hp = 99_900;
    match weapon {
        Some(w) => armed(stage, w),
        None => quiet(stage),
    }
}

fn boss_at(e: &Engine) -> (i32, i32) {
    let s = e.snapshot();
    let b = s
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Boss)
        .expect("the boss");
    (b.at.x.0, b.at.y.0)
}

/// One tick, and the events it gave.
fn advance(e: &mut Engine, input: Input) -> Vec<(u32, DomainEvent)> {
    e.step(input);
    e.events().iter().map(|ev| (e.tick(), ev.clone())).collect()
}

fn zones(e: &Engine) -> Vec<ZoneView> {
    e.snapshot().zones
}

fn hits(events: &[(u32, DomainEvent)]) -> Vec<u32> {
    ticks_of(events, |ev| matches!(ev, DomainEvent::PlayerHit { .. }))
}

#[test]
fn horn_charger_dashes_480_px_along_a_warned_lane_and_rings_where_it_stops() {
    // 100 px from the top, straight above the main character (576 px down): room for the dash.
    let st = boss_stage("horn_charger", 0, (180, 100), None);
    let mut e = bundled_engine(&st);
    let start = boss_at(&e);
    let mut events = Vec::new();
    let mut track = Vec::new();
    let mut lane_ticks = 0;
    let mut ring_at = None;
    for _ in 0..260 {
        events.extend(advance(&mut e, idle()));
        let tick = e.tick();
        track.push(boss_at(&e));
        let zs = zones(&e);
        for z in zs.iter().filter(|z| z.warn_only) {
            lane_ticks += 1;
            assert_eq!(
                (z.direction.x, z.direction.y),
                (0, trig::SCALE),
                "the lane points at the main character, straight down"
            );
        }
        let ring: Vec<_> = zs.iter().filter(|z| !z.warn_only).collect();
        if ring_at.is_none() && ring.len() == 10 {
            let n = i64::from(ring.len() as i32);
            let cx = ring.iter().map(|z| i64::from(z.at.x.0)).sum::<i64>() / n;
            let cy = ring.iter().map(|z| i64::from(z.at.y.0)).sum::<i64>() / n;
            ring_at = Some((tick, cx as i32, cy as i32, boss_at(&e)));
        }
    }
    assert!(
        lane_ticks >= 2 * 59,
        "two lanes warn for the wind-up: {lane_ticks}"
    );
    let end = *track.last().unwrap();
    assert_eq!(end.0, start.0, "straight down");
    assert_eq!(end.1 - start.1, 480 * PX, "480 px, and no walk back");
    let (_, cx, cy, stop) = ring_at.expect("a ring of ten zones");
    assert!(
        (cx - stop.0).abs() <= PX && (cy - stop.1).abs() <= PX,
        "{cx},{cy} {stop:?}"
    );
    assert_eq!(stop, end, "the ring lies at the stop point");
    // Wind-up 60 + dash 20: the dash covers its distance over exactly the 20 active ticks.
    let moving = track.windows(2).filter(|w| w[0] != w[1]).count();
    assert_eq!(moving, 20, "the boss moves on the 20 active ticks only");
    // The main character 476 px below is on the lane: the body hurts it once, the lane never.
    assert_eq!(hits(&events).len(), 1, "{events:?}");
}

#[test]
fn horn_charger_phase_two_throws_two_orbs_with_its_ring() {
    let st = boss_stage("horn_charger", 1, (180, 100), None);
    let mut e = bundled_engine(&st);
    let mut orbs_at_ring = None;
    for _ in 0..200 {
        e.step(idle());
        let snap = e.snapshot();
        if orbs_at_ring.is_none() && snap.zones.iter().filter(|z| !z.warn_only).count() == 10 {
            orbs_at_ring = Some(snap.orbs.len());
        }
    }
    assert_eq!(orbs_at_ring, Some(2));
}

/// The bundled elite `id` alone, `(dx, dy)` pixels from the main character.
fn elite_stage(id: &str, dx: i32, dy: i32) -> Stage {
    let elite = elite_def(id)["enemy"].clone();
    let mut stage = stage_with(
        6_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, elite, START_X + dx * PX, START_Y + dy * PX)],
    );
    stage.player.hp = 99_900;
    quiet(stage)
}

#[test]
fn elite_mid_lobs_and_charges_within_420_ticks() {
    let st = elite_stage("elite_mid", 150, 0);
    let mut e = bundled_engine(&st);
    let (mut lobbed, mut warned) = (None, None);
    let mut steps: Vec<i32> = Vec::new();
    let mut last = e
        .snapshot()
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Enemy)
        .map(|v| v.at.x.0);
    for _ in 0..420 {
        e.step(idle());
        let snap = e.snapshot();
        let tick = e.tick();
        if lobbed.is_none() && snap.zones.iter().any(|z| !z.warn_only) {
            lobbed = Some(tick);
        }
        if warned.is_none() && snap.zones.iter().any(|z| z.warn_only) {
            warned = Some(tick);
        }
        let now = snap
            .entities
            .iter()
            .find(|v| v.kind == EntityKind::Enemy)
            .map(|v| v.at.x.0);
        if let (Some(a), Some(b)) = (last, now) {
            steps.push(b - a);
        }
        last = now;
    }
    assert!(lobbed.is_some(), "the lobs");
    assert!(warned.is_some(), "the charge's lane");
    // A walk is slow; the charge covers 61,440 units in 14 ticks, 4,389 a tick.
    let first = steps
        .iter()
        .position(|d| d.abs() > 3_000)
        .expect("a charge");
    let dash: Vec<i32> = steps[first..]
        .iter()
        .copied()
        .take_while(|d| d.abs() > 3_000)
        .collect();
    assert_eq!(dash.len(), 14, "{steps:?}");
    assert!(
        (dash.iter().map(|d| d.abs()).sum::<i32>() - 61_440).abs() <= 16,
        "{dash:?}"
    );
}

fn shrine_stage() -> Stage {
    let mut elite = elite_def("elite_shrine")["enemy"].clone();
    elite["radius"] = json!(3072);
    let mut stage = stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, elite, START_X + 25_600, START_Y)],
        vec![],
    );
    stage.player.hp = 99_900;
    stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).unwrap());
    stage.stars = Some(
        serde_json::from_value(json!({ "drop_per_mille": 0, "pity": 2, "cap": 16,
                    "pickup_radius": 256,
                    "tiers": [{ "from_level": 1, "weights": { "common": 1 } }] }))
        .unwrap(),
    );
    quiet(stage)
}

fn rewards() -> Rewards {
    let file = json!({ "rarities": common::rarities(), "rewards": [
        { "id": "c", "rarity": "common", "effect": "max_hp", "amount": 1, "limit": 9 } ] });
    Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT).unwrap()
}

#[test]
fn the_shrine_pulses_every_300_ticks_from_its_touch_to_the_end_of_the_fight() {
    let content = RunContent::new(RunStats::bundled().clone(), rewards());
    let mut e = Engine::new_with(&shrine_stage(), &content).unwrap();
    let shrine_at = (START_X + 25_600, START_Y);
    let mut pulses = Vec::new();
    let (mut touched, mut left) = (None, None);
    let mut go = moving(127, 0);
    for _ in 0..2_400 {
        let seen = advance(&mut e, go);
        let tick = e.tick();
        for (_, ev) in seen {
            match ev {
                DomainEvent::ShrineActivated { .. } => {
                    touched = Some(tick);
                    go = idle();
                }
                DomainEvent::EliteLeft { .. } => left = Some(tick),
                _ => {}
            }
        }
        for z in zones(&e) {
            if !z.warn_only
                && z.ticks == 60
                && z.ticks_left == 60
                && z.shape == (ZoneShape::Circle { radius: Fx(16_384) })
            {
                assert_eq!((z.at.x.0, z.at.y.0), shrine_at, "centred on the shrine");
                pulses.push(tick);
            }
        }
    }
    let touched = touched.expect("the shrine activated");
    let left = left.expect("the fight ended");
    assert!(
        pulses.iter().all(|t| *t > touched),
        "none before the touch: {pulses:?}"
    );
    assert_eq!(pulses[0], touched + 1, "{pulses:?}");
    assert!(pulses.windows(2).all(|w| w[1] - w[0] == 300), "{pulses:?}");
    assert!(pulses.len() >= 5, "{pulses:?}");
    assert!(
        pulses.iter().all(|t| *t <= left),
        "none after the fight: {pulses:?} {left}"
    );
}

/// A sword that reaches 20 px round the main character.
fn sword() -> Value {
    weapon_ref(
        json!({ "inline": [{ "kind": "active", "ticks": 3, "damage": 1,
        "hit_box": { "type": "circle", "radius": 20 * PX } }] }),
    )
}

fn orbs(e: &Engine) -> Vec<Value> {
    let snapshot = serde_json::to_value(e.snapshot()).unwrap();
    snapshot["orbs"].as_array().cloned().unwrap_or_default()
}

fn beams(orbs: &[Value]) -> usize {
    orbs.iter().filter(|o| o.get("link_to").is_some()).count()
}

fn point(o: &Value) -> (i32, i32) {
    (
        o["at"]["x"].as_i64().unwrap() as i32,
        o["at"]["y"].as_i64().unwrap() as i32,
    )
}

/// Weaver's phase 3 until its cage has formed and halted: the engine at 60 ticks after the four
/// orbs spawned, the events so far, the tick they spawned and where the main character stands.
type Events = Vec<(u32, DomainEvent)>;

fn formed_cage() -> (Engine, Events, u32, (i32, i32)) {
    let st = boss_stage("weaver", 2, (180, 40), Some(sword()));
    let mut e = bundled_engine(&st);
    let mut events: Vec<(u32, DomainEvent)> = Vec::new();
    // Up to the middle of the map first: a beam needs both its orbs on the map.
    for _ in 0..80 {
        events.extend(advance(&mut e, moving(0, -127)));
    }
    let mut spawned = None;
    for _ in 0..900 {
        events.extend(advance(&mut e, idle()));
        if orbs(&e).len() == 4 {
            spawned = Some(e.tick());
            break;
        }
    }
    let spawned = spawned.expect("the cage's orbs");
    let player = player_at(&e);
    for o in orbs(&e) {
        let (x, y) = point(&o);
        let d = i64::from(x - player.0).pow(2) + i64::from(y - player.1).pow(2);
        assert!(
            d <= i64::from(20 * PX).pow(2),
            "{o} spawns 16 px from the main character"
        );
    }
    for _ in 0..60 {
        events.extend(advance(&mut e, idle()));
    }
    assert_eq!(e.tick(), spawned + 60);
    (e, events, spawned, player)
}

#[test]
fn weaver_s_cage_halts_round_the_main_character_joined_by_four_beams() {
    let (e, _, _, player) = formed_cage();
    // Halted: 136 px out (16 + 60 ticks of 2 px) to each corner of a square.
    let cage = orbs(&e);
    assert_eq!(cage.len(), 4);
    assert!(
        cage.iter().all(|o| o["vx"] == 0 && o["vy"] == 0),
        "{cage:?}"
    );
    assert_eq!(beams(&cage), 4);
    for o in &cage {
        let (x, y) = point(o);
        let d2 = i64::from(x - player.0).pow(2) + i64::from(y - player.1).pow(2);
        let (near, far) = (i64::from(134 * PX).pow(2), i64::from(138 * PX).pow(2));
        assert!((near..=far).contains(&d2), "{o}");
    }
}

#[test]
fn a_beam_hurts_the_main_character_and_again_60_ticks_later() {
    let (mut e, mut events, _, _) = formed_cage();
    let from = e.tick();
    let after = |events: &[(u32, DomainEvent)]| -> Vec<u32> {
        hits(events).into_iter().filter(|t| *t > from).collect()
    };
    assert!(after(&events).is_empty(), "the middle is not on a beam");
    // Walk onto the side of the square, then stand on the beam.
    for _ in 0..60 {
        events.extend(advance(&mut e, moving(127, 0)));
        if !after(&events).is_empty() {
            break;
        }
    }
    let first = *after(&events).first().expect("a beam hurt");
    for _ in 0..70 {
        events.extend(advance(&mut e, idle()));
    }
    let later: Vec<_> = after(&events).into_iter().filter(|t| *t > first).collect();
    assert!(
        later
            .first()
            .is_some_and(|t| (60..=62).contains(&(t - first))),
        "{first} {later:?}"
    );
}

#[test]
fn breaking_one_orb_leaves_two_beams() {
    let (mut e, _, _, _) = formed_cage();
    let target = point(&orbs(&e)[0]);
    for _ in 0..200 {
        let p = player_at(&e);
        let (dx, dy) = (target.0 - p.0, target.1 - p.1);
        if dx.abs() < 15 * PX && dy.abs() < 15 * PX {
            break;
        }
        let sign = |d: i32| {
            if d.abs() < 3 * PX {
                0
            } else {
                d.signum() as i8 * 127
            }
        };
        advance(&mut e, moving(sign(dx), sign(dy)));
    }
    for _ in 0..30 {
        if orbs(&e).len() < 4 {
            break;
        }
        advance(&mut e, pressing_main());
    }
    let left = orbs(&e);
    assert_eq!(left.len(), 3);
    assert_eq!(beams(&left), 2, "{left:?}");
}
