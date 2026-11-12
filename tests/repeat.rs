//! The step repeat and the proc factor (ADR-SIM-9 point 3): two style kinds, `repeat` and
//! `proc_factor`, that a style level names for a step of the weapon's base tree. They are
//! runtime-only on the step, bounded here (`sim_version` 2 only) and dormant until content
//! names them.

use serde_json::{Value, json};
use stella_rain_core::expertise::{Expertise, StyleKind};
use stella_rain_core::pack::{Pack, PackError};
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

use crate::common;
use crate::style_grants;

const A: [&str; 1] = ["main"];

fn repeat_kind(step: &[&str]) -> Value {
    json!({ "type": "repeat", "step": step, "after_ticks": 10, "repeat_pct": 50,
        "anchor": "hit_list", "proc_pct": 30 })
}

fn factor_kind(step: &[&str]) -> Value {
    json!({ "type": "proc_factor", "step": step, "proc_pct": 40 })
}

fn expertise_file(kinds: Value) -> Value {
    let mut file: Value = serde_json::from_slice(common::bundled_file("expertise")).unwrap();
    file["weapons"][0]["styles"][0]["kinds"] = kinds;
    file
}

fn parse_at(file: &Value, sim_version: u32) -> Result<Expertise, Error> {
    let versions = Versions {
        sim_version,
        ..Versions::CURRENT
    };
    Expertise::parse(file.to_string().as_bytes(), versions)
}

fn invalid_at(kinds: Value, path: &str) {
    match parse_at(&expertise_file(kinds), 2) {
        Err(Error::Invalid(issues)) => {
            assert!(issues.iter().any(|i| i.path == path), "{path}: {issues:?}")
        }
        other => panic!("{path}: expected invalid, got {:?}", other.map(|_| ())),
    }
}

fn valid(kinds: Value) {
    if let Err(e) = parse_at(&expertise_file(kinds), 2) {
        panic!("expected valid, got {e:?}");
    }
}

const AT: &str = "$.weapons[0].styles[0].kinds[0]";

/// C1: the bundled pack names neither kind; the two examples round-trip.
#[test]
fn the_bundled_pack_names_neither_kind_and_the_examples_round_trip() {
    let file: Value = serde_json::from_slice(common::bundled_file("expertise")).unwrap();
    let text = file.to_string();
    assert!(!text.contains("\"repeat\"") && !text.contains("\"proc_factor\""));
    for kind in [repeat_kind(&A), factor_kind(&["main", "main"])] {
        let parsed: StyleKind = serde_json::from_value(kind.clone()).expect("the kind parses");
        assert_eq!(serde_json::to_value(&parsed).unwrap(), kind);
    }
}

/// C2: every bound: in range loads, the first out of range is invalid at the key's path.
#[test]
fn every_bound_is_checked_at_its_key() {
    valid(json!([repeat_kind(&A)]));
    valid(json!([factor_kind(&A)]));
    let edge = |key: &str, value: Value| {
        let mut k = repeat_kind(&A);
        k[key] = value;
        k
    };
    for (key, low, high) in [
        ("after_ticks", 1, 120),
        ("repeat_pct", 1, 1_000),
        ("proc_pct", 0, 100),
    ] {
        valid(json!([edge(key, json!(low))]));
        valid(json!([edge(key, json!(high))]));
    }
    invalid_at(
        json!([edge("after_ticks", json!(0))]),
        &format!("{AT}.after_ticks"),
    );
    invalid_at(
        json!([edge("after_ticks", json!(121))]),
        &format!("{AT}.after_ticks"),
    );
    invalid_at(
        json!([edge("repeat_pct", json!(0))]),
        &format!("{AT}.repeat_pct"),
    );
    invalid_at(
        json!([edge("repeat_pct", json!(1_001))]),
        &format!("{AT}.repeat_pct"),
    );
    invalid_at(
        json!([edge("proc_pct", json!(101))]),
        &format!("{AT}.proc_pct"),
    );
    let mut f = factor_kind(&A);
    f["proc_pct"] = json!(101);
    invalid_at(json!([f]), &format!("{AT}.proc_pct"));
    for make in [repeat_kind, factor_kind] {
        invalid_at(json!([make(&[])]), &format!("{AT}.step"));
        invalid_at(json!([make(&["main"; 7])]), &format!("{AT}.step"));
        valid(json!([make(&["main"; 6])]));
    }
    // An anchor of another word, and a repeat with no anchor, do not parse.
    for k in [edge("anchor", json!("target")), {
        let mut k = repeat_kind(&A);
        k.as_object_mut().unwrap().remove("anchor");
        k
    }] {
        assert!(
            matches!(
                parse_at(&expertise_file(json!([k])), 2),
                Err(Error::Parse { .. })
            ),
            "a bad anchor is a parse error"
        );
    }
    // No proc_pct is allowed on a repeat.
    let mut k = repeat_kind(&A);
    k.as_object_mut().unwrap().remove("proc_pct");
    valid(json!([k]));
}

/// C3: at `sim_version` 1 each kind is refused.
#[test]
fn the_kinds_need_sim_version_2() {
    for kind in [repeat_kind(&A), factor_kind(&A)] {
        match parse_at(&expertise_file(json!([kind])), 1) {
            Err(Error::Invalid(issues)) => assert!(
                issues
                    .iter()
                    .any(|i| i.path == "$.weapons[0].styles[0].kinds"
                        && i.message.contains("sim_version 2")),
                "{issues:?}"
            ),
            other => panic!("expected invalid, got {:?}", other.map(|_| ())),
        }
    }
}

fn pack_with_hammer_kinds(kinds: Value) -> Value {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_hammer")
        .expect("the Hammer's expertise");
    w["styles"][0]["kinds"] = kinds;
    w["styles"][0]["upgrade_1"][0]["effect"] = json!("attack_pct");
    v
}

fn load(v: &Value) -> Result<Pack, PackError> {
    Pack::from_bytes(&serde_json::to_vec(v).unwrap())
}

fn refused_in_expertise(v: &Value) -> Vec<(String, String)> {
    match load(v) {
        Err(PackError::File {
            file: "expertise",
            error: Error::Invalid(issues),
        }) => issues.into_iter().map(|i| (i.path, i.message)).collect(),
        other => panic!("expected an expertise refusal, got {:?}", other.map(|_| ())),
    }
}

fn hold_grant() -> Value {
    let active = json!({ "kind": "active", "ticks": 4, "type": "strike",
        "hit_box": { "type": "circle", "offset": { "forward": 0, "right": 0 }, "radius": 8192 } });
    json!({ "hold": { "loop": { "inline": [active.clone()] },
        "release": { "inline": [active] }, "max_ticks": 60 } })
}

/// C4: the kinds are checked against the base tree, in the pack.
#[test]
fn the_pack_checks_the_kinds_against_the_base_tree() {
    assert!(
        load(&pack_with_hammer_kinds(json!([
            repeat_kind(&A),
            factor_kind(&A)
        ])))
        .is_ok()
    );
    // Not a step of the tree.
    for kind in [
        repeat_kind(&["smash"; 7][..]),
        factor_kind(&["smash", "main"]),
    ] {
        let paths = refused_in_expertise(&pack_with_hammer_kinds(json!([kind])));
        assert!(
            paths.iter().any(|(p, _)| p == &format!("{AT}.step")),
            "{paths:?}"
        );
    }
    // A repeat on a held step.
    let hold = json!({ "type": "replace_step", "step": A, "grant": hold_grant() });
    let paths = refused_in_expertise(&pack_with_hammer_kinds(json!([hold, repeat_kind(&A)])));
    assert!(
        paths
            .iter()
            .any(|(p, m)| p.starts_with("$.weapons[0].styles[0].kinds[1]")
                && m.contains("held step does not repeat")),
        "{paths:?}"
    );
    // The fifth repeat on one step of one style path is refused; four load. The style's own
    // level holds at most four kinds, so the rest come from its upgrades.
    let four = json!([
        repeat_kind(&A),
        repeat_kind(&A),
        repeat_kind(&A),
        repeat_kind(&A)
    ]);
    assert!(load(&pack_with_hammer_kinds(four.clone())).is_ok());
    let mut v = pack_with_hammer_kinds(four);
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_hammer")
        .unwrap();
    w["styles"][0]["upgrade_1"][0]["kinds"] = json!([repeat_kind(&A)]);
    let paths = refused_in_expertise(&v);
    assert!(
        paths
            .iter()
            .any(|(p, _)| p == "$.weapons[0].styles[0].upgrade_1[0].kinds[0]"),
        "{paths:?}"
    );
    // Two factors on one step in one style are refused; on two steps they load.
    let paths = refused_in_expertise(&pack_with_hammer_kinds(json!([
        factor_kind(&A),
        factor_kind(&A)
    ])));
    assert!(
        paths
            .iter()
            .any(|(p, _)| p == "$.weapons[0].styles[0].kinds[1].step"),
        "{paths:?}"
    );
    assert!(
        load(&pack_with_hammer_kinds(json!([
            factor_kind(&A),
            factor_kind(&["main", "main"])
        ])))
        .is_ok()
    );
}

/// C5: `repeats` and `proc_pct` are runtime-only: a weapons file naming either is refused.
#[test]
fn a_weapons_file_cannot_name_a_repeat_or_a_factor() {
    for (key, value) in [("repeats", json!([])), ("proc_pct", json!(10))] {
        let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
        v["weapons"]["weapons"][0]["trees"][0]["weapon"]["combo"]["main"][key] = value;
        match load(&v) {
            Err(PackError::File {
                file: "weapons",
                error: Error::Parse { .. },
            }) => {}
            other => panic!("{key}: expected a refusal, got {:?}", other.map(|_| ())),
        }
    }
}

/// C6: the views carry both kinds with their numbers; the bundled pack's views have none.
#[test]
fn the_views_carry_the_kinds() {
    let file = {
        let mut file: Value = serde_json::from_slice(common::bundled_file("expertise")).unwrap();
        file["thresholds"] = json!([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
        for w in file["weapons"].as_array_mut().unwrap() {
            w["base"] = json!([]);
            for s in w["styles"].as_array_mut().unwrap() {
                s["stats"] = json!([]);
            }
            if w["weapon"] == "giant_hammer" {
                w["styles"][0]["kinds"] = json!([repeat_kind(&A), factor_kind(&["main", "main"])]);
            }
        }
        file
    };
    let e = style_grants::build(
        Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the expertise"),
    );
    let snap = e.snapshot();
    let ready =
        serde_json::to_value(snap.players[0].tree_ready.as_ref().expect("a point")).unwrap();
    let repeat = json!({ "type": "repeat", "step": ["main"], "after_ticks": 10, "repeat_pct": 50,
        "anchor": "hit_list", "proc_pct": 30 });
    let factor = json!({ "type": "proc_factor", "step": ["main", "main"], "proc_pct": 40 });
    assert_eq!(ready["kinds"][0], json!([repeat, factor]));
    let stats = serde_json::to_value(snap.players[0].weapon_stats.as_ref().unwrap()).unwrap();
    assert_eq!(
        stats["trees"][0]["grants"][0]["kinds"],
        json!([repeat, factor])
    );
    // The bundled pack names none.
    let e = style_grants::build(Expertise::bundled().clone());
    let snap = e.snapshot();
    let ready = serde_json::to_value(snap.players[0].tree_ready.as_ref().unwrap()).unwrap();
    assert!(ready.get("kinds").is_none());
}

/// R12 (the event): a `Hit` that is no repeat serializes without the key; a repeat's carries it.
#[test]
fn only_a_repeat_s_hit_carries_the_repeat_key() {
    use stella_rain_core::event::DomainEvent;
    let hit = |repeat: bool| {
        let v = json!({ "type": "Hit", "data": {
            "source": 1, "target": 2, "damage": 5, "repeat": repeat } });
        let e: DomainEvent = serde_json::from_value(v).expect("a hit with a repeat flag");
        serde_json::to_value(&e).unwrap()
    };
    assert_eq!(hit(true)["data"]["repeat"], json!(true));
    assert!(hit(false)["data"].get("repeat").is_none());
}

// ---- The rules, over whole runs (the Hammer's A step: 6 ticks of wind-up, 4 active with a
// circle of 32 pixels at the player, 14 of recovery). ----

use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::{EntityId, PlayerIdx};
use stella_rain_core::input::{EquipAction, Input};

use crate::common::equip::{Parts, equipped_in};
use crate::common::{START_X, START_Y, enemy, hold, idle, stage_full, wave};

/// Style 0 names `kinds` on its first level; every other style names none.
fn expertise_with(kinds: Vec<Value>) -> Expertise {
    let mut file: Value = serde_json::from_slice(common::bundled_file("expertise")).unwrap();
    file["thresholds"] = json!([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
    for w in file["weapons"].as_array_mut().unwrap() {
        w["base"] = json!([]);
        for s in w["styles"].as_array_mut().unwrap() {
            s["stats"] = json!([]);
        }
        if w["weapon"] == "giant_hammer" {
            w["styles"][0]["kinds"] = json!(kinds);
        }
    }
    Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the expertise")
}

/// A Hammer run with one enemy of a million hit points, holding still, at each `(dx, dy)` in
/// pixels from the character (spawned at tick 1, far from anything else).
fn rig(kinds: Vec<Value>, at: &[(i32, i32)], pick_style: Option<u32>) -> Engine {
    let waves = at
        .iter()
        .map(|&(dx, dy)| {
            let (x, y) = (START_X + dx * 256, START_Y + dy * 256);
            let mut w = wave(1, 1, 0, enemy(1_000_000, hold(x, y), json!([])), x, y);
            w["expertise"] = json!({ "hit": 0, "kill": 10 });
            w
        })
        .collect();
    let mut stage = common::armed(
        stage_full(18_000, Some(common::far_boss()), waves, vec![]),
        json!({ "preset": { "id": "giant_hammer" } }),
    );
    stage.sim_version = 2;
    let parts = Parts {
        expertise: Some(expertise_with(kinds)),
        expertise_points: 550,
        ..Parts::default()
    };
    let file = json!({ "rarities": common::rarities(), "rewards": [
        { "id": "reach", "rarity": "common", "effect": "range_pct", "amount": 30, "limit": 2 }
    ] });
    let mut e = equipped_in(&stage, file, parts, &[], 1);
    e.step(&[idle(), Default::default(), Default::default()]);
    if let Some(option) = pick_style {
        e.act(PlayerIdx::SOLO, EquipAction::TreePick { option })
            .expect("a pick");
    }
    e
}

fn tap() -> Input {
    Input {
        main: true,
        ..Input::default()
    }
}

/// `(tick, target, damage, repeat)` of every enemy `Hit` of the player, over `ticks` ticks
/// starting with one press of A.
fn play(e: &mut Engine, ticks: u32) -> Vec<(u32, EntityId, u32, bool)> {
    let mut seen = Vec::new();
    for t in 0..ticks {
        let input = if t == 0 { tap() } else { idle() };
        e.step(&[input, Default::default(), Default::default()]);
        for ev in e.events() {
            if let DomainEvent::Hit {
                target,
                damage,
                repeat,
                player: Some(_),
                ..
            } = ev
            {
                seen.push((e.tick(), *target, *damage, *repeat));
            }
        }
    }
    seen
}

/// R1: the repeat lands on the enemies the step hit, `after_ticks` after the step's last
/// active phase ends, at its percent, in ID order; nothing more is dealt.
#[test]
fn a_hit_list_repeat_lands_on_what_the_step_hit_after_its_delay() {
    let kinds = vec![
        json!({ "type": "repeat", "step": ["main"], "after_ticks": 10,
        "repeat_pct": 50, "anchor": "hit_list" }),
    ];
    // Two enemies in the box (32 px round the character), one outside it.
    let mut e = rig(kinds, &[(0, -20), (15, -10), (0, -100)], Some(0));
    let seen = play(&mut e, 80);
    let step: Vec<_> = seen.iter().filter(|h| !h.3).collect();
    let repeats: Vec<_> = seen.iter().filter(|h| h.3).collect();
    assert_eq!(step.len(), 2, "{seen:?}");
    let d = step[0].2;
    assert!(d > 1 && step.iter().all(|h| h.2 == d && h.0 == step[0].0));
    assert_eq!(repeats.len(), 2, "{seen:?}");
    // The 4 active ticks end 4 ticks after the hit; the delay is 10 more.
    assert!(
        repeats
            .iter()
            .all(|h| h.0 == step[0].0 + 14 && h.2 == d * 50 / 100)
    );
    assert_eq!(
        repeats.iter().map(|h| h.1).collect::<Vec<_>>(),
        step.iter().map(|h| h.1).collect::<Vec<_>>(),
        "the same targets, in ID order"
    );
    assert!(repeats[0].1.0 < repeats[1].1.0);
}

/// R5: a step with two repeats queues both, and each lands on its own tick.
#[test]
fn two_repeats_of_a_step_land_on_their_own_ticks() {
    let at = |after: u32, pct: u32| {
        json!({ "type": "repeat", "step": ["main"], "after_ticks": after,
            "repeat_pct": pct, "anchor": "hit_list" })
    };
    let mut e = rig(vec![at(5, 50), at(12, 100)], &[(0, -20)], Some(0));
    let seen = play(&mut e, 80);
    let hit = seen.iter().find(|h| !h.3).expect("the step's hit");
    let repeats: Vec<_> = seen.iter().filter(|h| h.3).map(|h| (h.0, h.2)).collect();
    assert_eq!(
        repeats,
        vec![(hit.0 + 4 + 5, hit.2 / 2), (hit.0 + 4 + 12, hit.2)]
    );
}

/// A repeat of another step is not queued by this one, and a step that is not hit by anything
/// has an empty hit list: nothing lands.
#[test]
fn a_repeat_on_another_step_or_with_no_target_deals_nothing() {
    let on = |step: &[&str]| {
        vec![
            json!({ "type": "repeat", "step": step, "after_ticks": 5, "repeat_pct": 50,
            "anchor": "hit_list" }),
        ]
    };
    let mut e = rig(on(&["main", "main"]), &[(0, -20)], Some(0));
    assert!(
        play(&mut e, 60).iter().all(|h| !h.3),
        "AA's repeat, A pressed"
    );
    let mut e = rig(on(&A), &[(0, -100)], Some(0));
    assert!(play(&mut e, 60).is_empty(), "nothing in the box");
}

/// D2: a repeat on a style no one picks changes no tick of the run.
#[test]
fn a_style_that_is_never_picked_changes_no_hash() {
    let kinds = vec![
        json!({ "type": "repeat", "step": ["main"], "after_ticks": 5, "repeat_pct": 50,
            "anchor": "hit_list", "proc_pct": 40 }),
        json!({ "type": "proc_factor", "step": ["main"], "proc_pct": 40 }),
    ];
    let hashes = |kinds: Vec<Value>| {
        let mut e = rig(kinds, &[(0, -20)], Some(1));
        let mut out = Vec::new();
        for t in 0..200 {
            let input = if t % 12 == 0 { tap() } else { idle() };
            e.step(&[input, Default::default(), Default::default()]);
            out.push(e.state_hash());
        }
        out
    };
    // Style 0 holds the kinds in one run and none in the other; style 1 is the one picked.
    assert_eq!(hashes(kinds), hashes(vec![]));
}

/// D3: with the style picked and nothing to hit, the `place` repeat is queued (the hash differs
/// from a style that names none, while it waits) and no `Hit` is ever produced.
#[test]
fn a_place_repeat_with_nothing_to_hit_is_queued_and_hits_nothing() {
    let kinds = vec![
        json!({ "type": "repeat", "step": ["main"], "after_ticks": 10,
        "repeat_pct": 50, "anchor": "place" }),
    ];
    let run = |kinds: Vec<Value>| {
        let mut e = rig(kinds, &[(0, -100)], Some(0));
        let mut hashes = Vec::new();
        let mut hits = 0;
        for t in 0..50 {
            let input = if t == 0 { tap() } else { idle() };
            e.step(&[input, Default::default(), Default::default()]);
            hits += e
                .events()
                .iter()
                .filter(|ev| matches!(ev, DomainEvent::Hit { .. }))
                .count();
            hashes.push(e.state_hash());
        }
        (hashes, hits)
    };
    let (with, hits) = run(kinds);
    let (without, _) = run(vec![]);
    assert_eq!(hits, 0);
    let differ: Vec<bool> = with.iter().zip(&without).map(|(a, b)| a != b).collect();
    assert!(differ.iter().any(|d| *d), "the queued entry moves the hash");
    assert!(
        !differ[0] && !differ[49],
        "before it is queued and after it is dealt"
    );
}
