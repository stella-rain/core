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
