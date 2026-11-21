//! The gauge's three extra fills (ADR-SIM-9 points 3 and 4, ADR-SIM-5, ADR-SVC-1): `per_rune`,
//! `outer_half_pct` and the pair `per_smash_hit` / `smash_cap` are typed, bounded and viewed; the
//! pack check refuses each as unshipped until its rule lands. Test code only: the numbers are
//! test values, not content.

use serde_json::{Value, json};
use stella_rain_core::expertise::{Expertise, StyleKind};
use stella_rain_core::pack::{Pack, PackError};
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

use crate::common;
use crate::style_grants::build;

const AT: &str = "/weapons/0/styles/0/kinds";
const PATH: &str = "$.weapons[0].styles[0].kinds[0]";

fn gauge() -> Value {
    json!({
        "type": "gauge", "max": 100, "per_step": 3, "per_hit": 1, "step_cap": 14,
        "spend": { "type": "full_smash", "attack": { "preset": { "id": "slash" } } }
    })
}

fn gauge_with(fields: Value) -> Value {
    let mut g = gauge();
    for (k, v) in fields.as_object().unwrap() {
        if v.is_null() {
            g.as_object_mut().unwrap().remove(k);
        } else {
            g[k] = v.clone();
        }
    }
    g
}

fn file() -> Value {
    serde_json::from_slice(crate::before::EXPERTISE).unwrap()
}

fn with_kinds(kinds: Value) -> Value {
    let mut v = file();
    v.pointer_mut("/weapons/0/styles/0")
        .and_then(Value::as_object_mut)
        .unwrap()
        .insert("kinds".into(), kinds);
    v
}

fn parse(v: &Value) -> Result<Expertise, Error> {
    Expertise::parse(v.to_string().as_bytes(), Versions::CURRENT)
}

fn valid(g: Value) {
    if let Err(e) = parse(&with_kinds(json!([g]))) {
        panic!("expected valid, got {e:?}");
    }
}

fn refused_at(g: Value, path: &str) {
    match parse(&with_kinds(json!([g]))) {
        Err(Error::Invalid(issues)) => {
            assert!(issues.iter().any(|i| i.path == path), "{path}: {issues:?}")
        }
        other => panic!("{path}: expected invalid, got {other:?}"),
    }
}

fn parse_error(g: Value) {
    assert!(
        matches!(parse(&with_kinds(json!([g]))), Err(Error::Parse { .. })),
        "expected a parse error"
    );
}

/// G1: every fill loads and serializes back to the same JSON; a key at 0 is absent.
#[test]
fn the_fills_round_trip() {
    let g = gauge_with(json!({
        "per_rune": 2, "outer_half_pct": 50, "per_smash_hit": 1, "smash_cap": 4
    }));
    valid(g.clone());
    let kind: StyleKind = serde_json::from_value(g.clone()).expect("the kind parses");
    assert_eq!(serde_json::to_value(&kind).unwrap(), g);
    let plain: StyleKind = serde_json::from_value(gauge()).unwrap();
    let k = serde_json::to_value(&plain).unwrap();
    for key in ["per_rune", "outer_half_pct", "per_smash_hit", "smash_cap"] {
        assert!(k.get(key).is_none(), "{key} is absent at 0");
    }
}

/// G2: each fill is bounded, at its own path.
#[test]
fn the_fills_are_bounded() {
    valid(gauge_with(json!({ "per_rune": 100 })));
    refused_at(
        gauge_with(json!({ "per_rune": 101 })),
        &format!("{PATH}.per_rune"),
    );
    valid(gauge_with(json!({ "outer_half_pct": 1_000 })));
    refused_at(
        gauge_with(json!({ "outer_half_pct": 1_001 })),
        &format!("{PATH}.outer_half_pct"),
    );
    refused_at(
        gauge_with(json!({ "outer_half_pct": 50, "per_step": null })),
        &format!("{PATH}.outer_half_pct"),
    );
    valid(gauge_with(
        json!({ "per_smash_hit": 100, "smash_cap": 100 }),
    ));
    refused_at(
        gauge_with(json!({ "per_smash_hit": 1 })),
        &format!("{PATH}.smash_cap"),
    );
    refused_at(
        gauge_with(json!({ "smash_cap": 4 })),
        &format!("{PATH}.smash_cap"),
    );
    refused_at(
        gauge_with(json!({ "per_smash_hit": 5, "smash_cap": 4 })),
        &format!("{PATH}.per_smash_hit"),
    );
    refused_at(
        gauge_with(json!({ "per_smash_hit": 1, "smash_cap": 101 })),
        &format!("{PATH}.smash_cap"),
    );
    refused_at(
        gauge_with(json!({ "per_smash_hit": 101, "smash_cap": 101 })),
        &format!("{PATH}.per_smash_hit"),
    );
    parse_error(gauge_with(json!({ "per_rune": "two" })));
    parse_error(gauge_with(json!({ "per_runes": 2 })));
}

/// G3: `step_cap` keeps its own bound; `smash_cap` is independent of it.
#[test]
fn the_step_cap_stays() {
    refused_at(
        gauge_with(json!({ "step_cap": 0 })),
        &format!("{PATH}.step_cap"),
    );
    valid(gauge_with(
        json!({ "step_cap": 2, "per_smash_hit": 3, "smash_cap": 9 }),
    ));
}

/// G4: the view lists the fills; a gauge naming none carries no `fills` key.
#[test]
fn the_view_carries_the_fills() {
    let view = |g: Value| {
        let mut f = file();
        f["thresholds"] = json!([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
        let w = f["weapons"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|w| w["weapon"] == "giant_hammer")
            .unwrap();
        w["base"] = json!([]);
        for s in w["styles"].as_array_mut().unwrap() {
            s["stats"] = json!([]);
        }
        w["styles"][0]["kinds"] = json!([g]);
        let expertise = parse(&f).expect("the expertise");
        let snap = build(expertise).snapshot();
        let ready = serde_json::to_value(snap.players[0].tree_ready.as_ref().unwrap()).unwrap();
        ready["kinds"][0][0].clone()
    };
    let got = view(gauge_with(json!({
        "per_rune": 2, "outer_half_pct": 50, "per_smash_hit": 1, "smash_cap": 4
    })));
    assert_eq!(
        got,
        json!({ "type": "gauge", "max": 100, "spend": "full_smash", "fills": [
            { "type": "per_rune", "amount": 2 },
            { "type": "outer_half_pct", "pct": 50 },
            { "type": "per_smash_hit", "amount": 1, "cap": 4 }
        ] })
    );
    let plain = view(gauge());
    assert!(plain.get("fills").is_none(), "{plain}");
}

fn hammer_pack(kinds: Vec<Value>) -> Value {
    let mut v: Value = serde_json::from_slice(crate::before::pack_bytes()).unwrap();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_hammer")
        .expect("the Hammer's expertise");
    w["styles"][0]["kinds"] = json!(kinds);
    w["styles"][0]["upgrade_1"][0]["effect"] = json!("attack_pct");
    v
}

fn load(v: &Value) -> Result<Pack, PackError> {
    Pack::from_bytes(&serde_json::to_vec(v).unwrap())
}

/// E1: the pack check refuses each fill as unshipped, at the kind's path plus the key; the same
/// gauge with its old fields only loads, and `gauge` is not reported as an unshipped kind.
#[test]
fn the_pack_refuses_each_unshipped_fill() {
    let ok = load(&hammer_pack(vec![gauge()]));
    assert!(ok.is_ok(), "{:?}", ok.err());
    let cases = [
        (json!({ "per_rune": 2 }), "per_rune", ".per_rune"),
        (
            json!({ "outer_half_pct": 50 }),
            "outer_half_pct",
            ".outer_half_pct",
        ),
        (
            json!({ "per_smash_hit": 1, "smash_cap": 4 }),
            "per_smash_hit",
            ".per_smash_hit",
        ),
    ];
    for (fields, tag, suffix) in cases {
        match load(&hammer_pack(vec![gauge_with(fields)])) {
            Err(PackError::File {
                file: "expertise",
                error,
            }) => {
                let issues = format!("{error:?}");
                let path = format!("$.weapons[0].styles[0].kinds[0]{suffix}");
                assert!(issues.contains(&path), "{path}: {issues}");
                assert!(
                    issues.contains(&format!("the effect {tag} is not shipped yet")),
                    "{issues}"
                );
                assert!(!issues.contains("the kind gauge"), "{issues}");
            }
            other => panic!("{tag}: expected a refusal, got {:?}", other.map(|_| ())),
        }
    }
    let _ = common::bundled_file("expertise");
}
