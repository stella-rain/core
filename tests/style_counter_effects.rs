//! The counter bonus's four extra effects (ADR-SIM-9 points 3 and 4, ADR-SIM-5, ADR-SVC-1):
//! `pull_round`, `fill_size_stacks`, `clear_bullets` and `fire_runes` are typed, bounded and
//! viewed; the pack check refuses each as unshipped until its rule lands. Test code only: the
//! numbers are test values, not content.

use serde_json::{Value, json};
use stella_rain_core::expertise::{Expertise, StyleKind};
use stella_rain_core::pack::{Pack, PackError};
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

use crate::style_grants::build;

const PATH: &str = "$.weapons[0].styles[0].kinds[0]";

fn counter(effects: Value) -> Value {
    json!({ "type": "counter_bonus", "window_ticks": 30, "effects": effects })
}

fn new_four() -> Value {
    json!([
        { "type": "pull_round", "radius_px": 96 },
        { "type": "fill_size_stacks" },
        { "type": "clear_bullets", "radius_px": 96 },
        { "type": "fire_runes" }
    ])
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

fn parse_at(v: &Value, versions: Versions) -> Result<Expertise, Error> {
    Expertise::parse(v.to_string().as_bytes(), versions)
}

fn valid(kinds: Value) {
    if let Err(e) = parse_at(&with_kinds(kinds), Versions::CURRENT) {
        panic!("expected valid, got {e:?}");
    }
}

fn refused_at(kinds: Value, path: &str) {
    match parse_at(&with_kinds(kinds), Versions::CURRENT) {
        Err(Error::Invalid(issues)) => {
            assert!(issues.iter().any(|i| i.path == path), "{path}: {issues:?}")
        }
        other => panic!("{path}: expected invalid, got {other:?}"),
    }
}

fn parse_error(kinds: Value) {
    assert!(
        matches!(
            parse_at(&with_kinds(kinds), Versions::CURRENT),
            Err(Error::Parse { .. })
        ),
        "expected a parse error"
    );
}

/// C1: the four new effects beside `damage_pct` load and serialize back to the same JSON; the two
/// flags are bare `{"type": ...}`.
#[test]
fn the_new_effects_round_trip() {
    let mut effects = vec![json!({ "type": "damage_pct", "pct": 20 })];
    effects.extend(new_four().as_array().unwrap().clone());
    let kind = counter(json!(effects));
    valid(json!([kind.clone()]));
    let parsed: StyleKind = serde_json::from_value(kind.clone()).expect("the kind parses");
    assert_eq!(serde_json::to_value(&parsed).unwrap(), kind);
}

/// C2: each radius is 1 to 240, the flags take no key, ten distinct effects fit and an eleventh
/// entry does not, a repeated effect is refused at the second.
#[test]
fn the_new_effects_are_bounded() {
    for tag in ["pull_round", "clear_bullets"] {
        let with = |r: u32| json!([counter(json!([{ "type": tag, "radius_px": r }]))]);
        valid(with(1));
        valid(with(240));
        let at = format!("{PATH}.effects[0].radius_px");
        refused_at(with(0), &at);
        refused_at(with(241), &at);
    }
    for tag in ["fill_size_stacks", "fire_runes"] {
        parse_error(json!([counter(json!([{ "type": tag, "extra": 1 }]))]));
    }
    let mut ten = vec![
        json!({ "type": "damage_pct", "pct": 5 }),
        json!({ "type": "stagger_pct", "pct": 110 }),
        json!({ "type": "stun_ticks", "ticks": 5 }),
        json!({ "type": "advance", "distance": 512 }),
        json!({ "type": "refund_evade" }),
        json!({ "type": "gauge_fill_pct", "pct": 10 }),
    ];
    ten.extend(new_four().as_array().unwrap().clone());
    valid(json!([counter(json!(ten))]));
    let mut eleven = ten.clone();
    eleven.push(json!({ "type": "damage_pct", "pct": 6 }));
    refused_at(json!([counter(json!(eleven))]), &format!("{PATH}.effects"));
    refused_at(
        json!([counter(json!([
            { "type": "fire_runes" },
            { "type": "fire_runes" }
        ]))]),
        &format!("{PATH}.effects[1]"),
    );
}

/// C3: a later level may name the counter with one new effect; every kind needs sim_version 2.
#[test]
fn a_later_level_may_list_one_effect_and_the_version_is_2() {
    let mut v = with_kinds(json!([counter(
        json!([{ "type": "damage_pct", "pct": 5 }])
    )]));
    let level = &mut v["weapons"][0]["styles"][0]["upgrade_1"][0];
    level["effect"] = json!("none");
    level.as_object_mut().unwrap().remove("amount");
    level["kinds"] = json!([counter(json!([{ "type": "fire_runes" }]))]);
    assert!(parse_at(&v, Versions::CURRENT).is_ok());
    let old = Versions {
        sim_version: 1,
        ..Versions::CURRENT
    };
    match parse_at(&with_kinds(json!([counter(new_four())])), old) {
        Err(Error::Invalid(issues)) => assert!(
            issues
                .iter()
                .any(|i| i.path == "$.weapons[0].styles[0].kinds"),
            "{issues:?}"
        ),
        other => panic!("{other:?}"),
    }
}

/// C4: the view lists the four new effects; a counter naming none keeps its old form.
#[test]
fn the_view_carries_the_new_effects() {
    let view = |c: Value| {
        let mut f = file();
        f["thresholds"] = json!([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
        let w = f["weapons"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|w| w["weapon"] == "giant_hammer")
            .unwrap();
        w["base"] = json!([]);
        for st in w["styles"].as_array_mut().unwrap() {
            st["stats"] = json!([]);
        }
        w["styles"][0]["kinds"] = json!([c]);
        let expertise = parse_at(&f, Versions::CURRENT).expect("the expertise");
        let snap = build(expertise).snapshot();
        let ready = serde_json::to_value(snap.players[0].tree_ready.as_ref().unwrap()).unwrap();
        ready["kinds"][0][0].clone()
    };
    assert_eq!(
        view(counter(new_four())),
        json!({ "type": "counter_bonus", "window_ticks": 30, "effects": new_four() })
    );
    let old = view(counter(json!([{ "type": "damage_pct", "pct": 5 }])));
    assert_eq!(old, json!({ "type": "counter_bonus", "window_ticks": 30 }));
}

fn pack_issues(kinds: Vec<Value>) -> String {
    let mut v: Value = serde_json::from_slice(crate::before::pack_bytes()).unwrap();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_hammer")
        .expect("the Hammer's expertise");
    w["styles"][0]["kinds"] = json!(kinds);
    w["styles"][0]["upgrade_1"][0]["effect"] = json!("attack_pct");
    match Pack::from_bytes(&serde_json::to_vec(&v).unwrap()) {
        Ok(_) => String::new(),
        Err(PackError::File {
            file: "expertise",
            error,
        }) => format!("{error:?}"),
        Err(other) => panic!("{other:?}"),
    }
}

/// E3: the pack check refuses each new effect as unshipped, at its entry; a counter naming the six
/// built effects loads.
#[test]
fn the_pack_refuses_each_unshipped_counter_effect() {
    let built = counter(json!([
        { "type": "damage_pct", "pct": 5 },
        { "type": "stagger_pct", "pct": 110 },
        { "type": "stun_ticks", "ticks": 5 },
        { "type": "advance", "distance": 512 },
        { "type": "refund_evade" },
        { "type": "gauge_fill_pct", "pct": 10 }
    ]));
    let ok = pack_issues(vec![built]);
    assert_eq!(ok, "", "{ok}");
    let four = new_four();
    for (i, effect) in four.as_array().unwrap().iter().enumerate() {
        let tag = effect["type"].as_str().unwrap();
        let issues = pack_issues(vec![counter(json!([effect]))]);
        assert!(
            issues.contains("$.weapons[0].styles[0].kinds[0].effects[0]")
                && issues.contains(&format!("the effect {tag} is not shipped yet")),
            "{i} {tag}: {issues}"
        );
    }
}
