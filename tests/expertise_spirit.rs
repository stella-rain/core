//! The spirit kind of a style level (ADR-SIM-9 points 2 to 7, ADR-SIM-5, ADR-CONT-3): its
//! numbers are bounded by the validator, the kind is `sim_version` 2's, one per style path, and
//! it round-trips through JSON. Test code only: the numbers are test values, not content.

use serde_json::{Value, json};
use stella_rain_core::expertise::{Expertise, StyleKind};
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

use crate::common;

const AT: &str = "/weapons/0/styles/0/kinds";
const PATH: &str = "$.weapons[0].styles[0].kinds[0]";

/// The Sword's three levels as the design table gives them (test values).
fn spirit() -> Value {
    json!({
        "type": "spirit", "decay_ticks": 1200,
        "levels": [
            [{ "type": "damage_pct", "pct": 8 }],
            [{ "type": "damage_pct", "pct": 16 }, { "type": "range_pct", "pct": 15 }],
            [{ "type": "damage_pct", "pct": 25 }, { "type": "smash_bleed_stacks", "stacks": 2 }]
        ]
    })
}

fn spirit_with(key: &str, value: Value) -> Value {
    let mut s = spirit();
    s[key] = value;
    s
}

/// A spirit of one level holding `effects`.
fn one_level(effects: Value) -> Value {
    json!({ "type": "spirit", "decay_ticks": 100, "levels": [effects] })
}

fn file() -> Value {
    serde_json::from_slice(common::bundled_file("expertise")).unwrap()
}

fn with_kinds(kinds: Value) -> Value {
    let mut v = file();
    v.pointer_mut(AT)
        .and_then(Value::as_array_mut)
        .map(|k| *k = kinds.as_array().unwrap().clone())
        .unwrap_or_else(|| {
            v.pointer_mut("/weapons/0/styles/0")
                .and_then(Value::as_object_mut)
                .unwrap()
                .insert("kinds".into(), kinds);
        });
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

/// S1: every number loads at its bounds and is refused one past them, at its own path.
#[test]
fn the_spirit_kind_is_bounded() {
    valid(json!([spirit()]));
    for ticks in [1, 3_600] {
        valid(json!([spirit_with("decay_ticks", json!(ticks))]));
    }
    refused_at(
        json!([spirit_with("decay_ticks", json!(0))]),
        &format!("{PATH}.decay_ticks"),
    );
    refused_at(
        json!([spirit_with("decay_ticks", json!(3_601))]),
        &format!("{PATH}.decay_ticks"),
    );
    valid(json!([spirit_with("levels", json!([[]]))]));
    valid(json!([spirit_with("levels", json!([[], [], []]))]));
    refused_at(
        json!([spirit_with("levels", json!([]))]),
        &format!("{PATH}.levels"),
    );
    refused_at(
        json!([spirit_with("levels", json!([[], [], [], []]))]),
        &format!("{PATH}.levels"),
    );
    let bound = |effect: Value, key: &str, lo: u32, hi: u32, over: u32| {
        let with = |v: u32| {
            let mut e = effect.clone();
            e[key] = json!(v);
            json!([one_level(json!([e]))])
        };
        valid(with(lo));
        valid(with(hi));
        let at = format!("{PATH}.levels[0][0].{key}");
        refused_at(with(lo - 1), &at);
        refused_at(with(over), &at);
    };
    bound(json!({ "type": "damage_pct" }), "pct", 1, 1_000, 1_001);
    bound(json!({ "type": "range_pct" }), "pct", 1, 100, 101);
    bound(json!({ "type": "smash_bleed_stacks" }), "stacks", 1, 4, 5);
}

/// S1: `levels` and `decay_ticks` are required, an unknown effect type and an unknown key are
/// parse errors.
#[test]
fn the_spirit_kind_is_strict() {
    for key in ["levels", "decay_ticks"] {
        let mut s = spirit();
        s.as_object_mut().unwrap().remove(key);
        parse_error(json!([s]));
    }
    parse_error(json!([one_level(
        json!([{ "type": "luck_pct", "pct": 5 }])
    )]));
    parse_error(json!([spirit_with("extra", json!(1))]));
    parse_error(json!([one_level(
        json!([{ "type": "damage_pct", "pct": 5, "extra": 1 }])
    )]));
}

/// S1: one effect of a variant per level, one spirit per style path (inside a level and across
/// levels), both refused at the later one; a level of five kinds is over the level's bound.
#[test]
fn a_level_names_one_effect_per_variant_and_a_path_one_spirit() {
    refused_at(
        json!([one_level(json!([
            { "type": "damage_pct", "pct": 5 },
            { "type": "damage_pct", "pct": 6 }
        ]))]),
        &format!("{PATH}.levels[0][1]"),
    );
    refused_at(
        json!([spirit(), spirit()]),
        "$.weapons[0].styles[0].kinds[1]",
    );
    let mut v = with_kinds(json!([spirit()]));
    v["weapons"][0]["styles"][0]["upgrade_1"] = json!([{
        "id": "again", "effect": "none", "kinds": [spirit()]
    }]);
    match parse_at(&v, Versions::CURRENT) {
        Err(Error::Invalid(issues)) => assert!(
            issues
                .iter()
                .any(|i| i.path == "$.weapons[0].styles[0].upgrade_1[0].kinds[0]"),
            "{issues:?}"
        ),
        other => panic!("expected invalid, got {other:?}"),
    }
    // Two styles may each name one.
    let mut v = with_kinds(json!([spirit()]));
    v["weapons"][0]["styles"][1]["kinds"] = json!([spirit()]);
    assert!(parse_at(&v, Versions::CURRENT).is_ok());
    let five = json!([spirit(), spirit(), spirit(), spirit(), spirit()]);
    refused_at(five, "$.weapons[0].styles[0].kinds");
}

/// S1: the kind needs `sim_version` 2, as every kind does.
#[test]
fn the_spirit_kind_needs_sim_version_2() {
    let old = Versions {
        sim_version: 1,
        ..Versions::CURRENT
    };
    match parse_at(&with_kinds(json!([spirit()])), old) {
        Err(Error::Invalid(issues)) => assert!(
            issues
                .iter()
                .any(|i| i.path == "$.weapons[0].styles[0].kinds"
                    && i.message.contains("sim_version 2")),
            "{issues:?}"
        ),
        other => panic!("{other:?}"),
    }
}

/// S3: the bundled pack names no spirit kind, and the kind's JSON serializes back to itself.
#[test]
fn the_kind_round_trips_and_the_bundled_file_names_none() {
    let bundled = std::str::from_utf8(common::bundled_file("expertise")).unwrap();
    assert!(!bundled.contains("\"spirit\""));
    let kind: StyleKind = serde_json::from_value(spirit()).expect("the kind parses");
    assert_eq!(serde_json::to_value(&kind).unwrap(), spirit());
}
