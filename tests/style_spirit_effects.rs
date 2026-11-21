//! The spirit's eight extra effects and `raise_per_smash` (ADR-SIM-9 points 3 and 4, ADR-SIM-5,
//! ADR-SVC-1): typed, bounded and viewed; the pack check refuses each as unshipped until its rule
//! lands, and checks a `smash_repeat` entry's `min_charge_level` against the style's charges.
//! Test code only: the numbers are test values, not content.

use serde_json::{Value, json};
use stella_rain_core::expertise::{Expertise, StyleKind};
use stella_rain_core::pack::{Pack, PackError};
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

use crate::style_grants::build;

const PATH: &str = "$.weapons[0].styles[0].kinds[0]";

/// Three levels naming every new effect once per level, as the plan's table gives them.
fn spirit() -> Value {
    json!({
        "type": "spirit", "decay_ticks": 1200, "raise_per_smash": 2,
        "levels": [
            [{ "type": "size_stack_cap", "stacks": 1 },
             { "type": "rune_cooldown_pct", "pct": 33 }],
            [{ "type": "size_stack_ticks_pct", "pct": 50 },
             { "type": "extra_presses", "presses": 2 },
             { "type": "smash_repeat", "repeats": [
                 { "after_ticks": 30, "repeat_pct": 50 },
                 { "after_ticks": 60, "repeat_pct": 40, "min_charge_level": 2 }
             ] }],
            [{ "type": "size_stacks_per_hit", "stacks": 2 },
             { "type": "normal_pull", "distance": 2048, "ticks": 6 },
             { "type": "rune_surge", "steps": [["main", "main"], "smashes"] }]
        ]
    })
}

fn one_level(effects: Value) -> Value {
    json!({ "type": "spirit", "decay_ticks": 100, "levels": [effects] })
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

/// S1: the spirit naming every new effect parses and serializes back to the same JSON.
#[test]
fn every_new_effect_round_trips() {
    valid(json!([spirit()]));
    let kind: StyleKind = serde_json::from_value(spirit()).expect("the kind parses");
    assert_eq!(serde_json::to_value(&kind).unwrap(), spirit());
    let plain: StyleKind = serde_json::from_value(one_level(json!([]))).unwrap();
    assert!(
        serde_json::to_value(&plain)
            .unwrap()
            .get("raise_per_smash")
            .is_none()
    );
}

/// S2: each number is refused one past its bound, at its field, and loads at the top.
#[test]
fn the_new_numbers_are_bounded() {
    let raise = |v: u32| {
        json!([{ "type": "spirit", "decay_ticks": 100,
        "raise_per_smash": v, "levels": [[], [], []] }])
    };
    valid(raise(1));
    valid(raise(3));
    refused_at(raise(0), &format!("{PATH}.raise_per_smash"));
    refused_at(raise(4), &format!("{PATH}.raise_per_smash"));
    let bound = |effect: Value, key: &str, lo: u32, hi: u32| {
        let with = |v: u32| {
            let mut e = effect.clone();
            e[key] = json!(v);
            if effect["type"] == "normal_pull" && key == "distance" {
                e["ticks"] = json!(6);
            }
            if effect["type"] == "normal_pull" && key == "ticks" {
                e["distance"] = json!(2048);
            }
            json!([one_level(json!([e]))])
        };
        valid(with(hi));
        let at = format!("{PATH}.levels[0][0].{key}");
        refused_at(with(lo), &at);
        refused_at(with(hi + 1), &at);
    };
    let at_least_one = |effect: Value, key: &str, hi: u32| bound(effect, key, 0, hi);
    at_least_one(json!({ "type": "size_stack_cap" }), "stacks", 8);
    at_least_one(json!({ "type": "size_stack_ticks_pct" }), "pct", 1_000);
    at_least_one(json!({ "type": "size_stacks_per_hit" }), "stacks", 8);
    at_least_one(json!({ "type": "normal_pull" }), "distance", 16_384);
    at_least_one(json!({ "type": "normal_pull" }), "ticks", 60);
    at_least_one(json!({ "type": "rune_cooldown_pct" }), "pct", 1_000);
    at_least_one(json!({ "type": "extra_presses" }), "presses", 16);
}

/// S3: `rune_surge` names up to four steps, each a legal step, none sharing a member.
#[test]
fn rune_surge_steps_are_bounded() {
    let surge =
        |steps: Value| json!([one_level(json!([{ "type": "rune_surge", "steps": steps }]))]);
    let at = format!("{PATH}.levels[0][0].steps");
    refused_at(
        surge(json!([
            ["main"],
            ["smash"],
            ["main", "smash"],
            ["main", "main", "smash"],
            ["main", "main", "main", "smash"]
        ])),
        &at,
    );
    refused_at(surge(json!([vec!["main"; 7]])), &format!("{at}[0]"));
    refused_at(surge(json!([["main"], "normals"])), &format!("{at}[1]"));
    valid(surge(json!(["smashes", "normals"])));
    valid(surge(json!([])));
}

/// S4: `smash_repeat` holds one to four entries, each bounded; `min_charge_level` is optional.
#[test]
fn smash_repeat_entries_are_bounded() {
    let entry = json!({ "after_ticks": 30, "repeat_pct": 50 });
    let repeat = |repeats: Value| {
        json!([one_level(
            json!([{ "type": "smash_repeat", "repeats": repeats }])
        )])
    };
    let at = format!("{PATH}.levels[0][0].repeats");
    refused_at(repeat(json!([])), &at);
    refused_at(repeat(json!([entry, entry, entry, entry, entry])), &at);
    valid(repeat(json!([entry, entry, entry, entry])));
    let with = |key: &str, v: u32| {
        let mut e = entry.clone();
        e[key] = json!(v);
        repeat(json!([entry, e]))
    };
    for (key, lo, hi) in [
        ("after_ticks", 0, 120),
        ("repeat_pct", 0, 1_000),
        ("min_charge_level", 0, 8),
    ] {
        let path = format!("{at}[1].{key}");
        valid(with(key, hi));
        refused_at(with(key, lo), &path);
        refused_at(with(key, hi + 1), &path);
    }
    valid(repeat(json!([entry])));
    let mut extra = entry.clone();
    extra["anchor"] = json!("place");
    parse_error(repeat(json!([extra])));
}

/// S5: a level names an effect once, two levels may each name it; every kind needs sim_version 2.
#[test]
fn a_level_names_each_effect_once_and_the_version_is_2() {
    let surge = json!({ "type": "rune_surge", "steps": [] });
    refused_at(
        json!([one_level(json!([surge, surge]))]),
        &format!("{PATH}.levels[0][1]"),
    );
    valid(json!([json!({ "type": "spirit", "decay_ticks": 100,
        "levels": [[surge], [surge]] })]));
    let old = Versions {
        sim_version: 1,
        ..Versions::CURRENT
    };
    match parse_at(&with_kinds(json!([spirit()])), old) {
        Err(Error::Invalid(issues)) => assert!(
            issues
                .iter()
                .any(|i| i.path == "$.weapons[0].styles[0].kinds"),
            "{issues:?}"
        ),
        other => panic!("{other:?}"),
    }
}

fn circle(radius: i32) -> Value {
    json!({ "type": "circle", "offset": { "forward": 0, "right": 0 }, "radius": radius })
}

fn strike(radius: i32) -> Value {
    json!({ "inline": [
        { "kind": "active", "ticks": 3, "type": "strike", "hit_box": circle(radius) },
        { "kind": "recovery", "ticks": 4 }
    ] })
}

fn charge_kind(levels: i32) -> Value {
    let levels: Vec<Value> = (1..=levels)
        .map(|i| json!({ "at_ticks": 10 * i, "attack": strike(5_000 + 1_000 * i) }))
        .collect();
    json!({ "type": "replace_step", "step": "smashes",
        "grant": { "charge": { "levels": levels, "max_ticks": 100 } } })
}

fn gauge_kind() -> Value {
    json!({ "type": "gauge", "max": 100, "per_step": 10, "per_hit": 2, "step_cap": 14,
        "spend": { "type": "full_smash",
            "attack": { "preset": { "id": "slash", "args": [] } } } })
}

fn repeat_spirit(min: Option<u32>) -> Value {
    let mut entry = json!({ "after_ticks": 30, "repeat_pct": 50 });
    if let Some(m) = min {
        entry["min_charge_level"] = json!(m);
    }
    one_level(json!([{ "type": "smash_repeat", "repeats": [entry] }]))
}

/// The Hammer's pack with style 0 naming `kinds` at its own level and `later` at its second
/// upgrade, loaded: the whole refusal as text (empty when it loads).
fn pack_issues(kinds: Vec<Value>, later: Option<Vec<Value>>) -> String {
    let mut v: Value = serde_json::from_slice(crate::before::pack_bytes()).unwrap();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_hammer")
        .expect("the Hammer's expertise");
    w["styles"][0]["kinds"] = json!(kinds);
    w["styles"][0]["upgrade_1"][0]["effect"] = json!("attack_pct");
    if let Some(later) = later {
        w["styles"][0]["upgrade_2"][0]["kinds"] = json!(later);
    }
    match Pack::from_bytes(&serde_json::to_vec(&v).unwrap()) {
        Ok(_) => String::new(),
        Err(PackError::File {
            file: "expertise",
            error,
        }) => format!("{error:?}"),
        Err(other) => panic!("{other:?}"),
    }
}

const NO_CHARGE: &str = "no smash step of the style's tree has a charge of";

/// S6: `min_charge_level` needs a smash step whose charge has that many levels, the finished
/// path read: a charge granted after the spirit counts.
#[test]
fn min_charge_level_reads_the_styles_charges() {
    let at = "$.weapons[0].styles[0].kinds[1].levels[0][0].repeats[0].min_charge_level";
    // The Hammer's base smashes carry no charge.
    let none = pack_issues(vec![gauge_kind(), repeat_spirit(Some(1))], None);
    assert!(none.contains(at), "{none}");
    assert!(none.contains("the most is 0"), "{none}");
    // A two-level charge granted by the style: 2 is enough, 3 is not.
    let two =
        |m: Option<u32>| pack_issues(vec![gauge_kind(), repeat_spirit(m), charge_kind(2)], None);
    assert!(!two(Some(2)).contains(NO_CHARGE), "{}", two(Some(2)));
    let over = two(Some(3));
    assert!(
        over.contains(at) && over.contains("the most is 2"),
        "{over}"
    );
    // A charge granted at a later level counts.
    let later = pack_issues(
        vec![gauge_kind(), repeat_spirit(Some(2))],
        Some(vec![charge_kind(2)]),
    );
    assert!(!later.contains(NO_CHARGE), "{later}");
    // An entry without the key is not refused for this reason.
    assert!(!pack_issues(vec![gauge_kind(), repeat_spirit(None)], None).contains(NO_CHARGE));
}

/// S7: the view carries `raise_per_smash` and the new effects, each with its level; a spirit
/// naming none keeps its old form.
#[test]
fn the_view_carries_the_new_effects() {
    let view = |s: Value| {
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
        w["styles"][0]["kinds"] = json!([s]);
        let expertise = parse_at(&f, Versions::CURRENT).expect("the expertise");
        let snap = build(expertise).snapshot();
        let ready = serde_json::to_value(snap.players[0].tree_ready.as_ref().unwrap()).unwrap();
        ready["kinds"][0][0].clone()
    };
    let got = view(spirit());
    assert_eq!(
        got,
        json!({ "type": "spirit", "max_level": 3, "decay_ticks": 1200, "raise_per_smash": 2,
            "effects": [
                { "level": 1, "effect": { "type": "size_stack_cap", "stacks": 1 } },
                { "level": 1, "effect": { "type": "rune_cooldown_pct", "pct": 33 } },
                { "level": 2, "effect": { "type": "size_stack_ticks_pct", "pct": 50 } },
                { "level": 2, "effect": { "type": "extra_presses", "presses": 2 } },
                { "level": 2, "effect": { "type": "smash_repeat", "repeats": [
                    { "after_ticks": 30, "repeat_pct": 50 },
                    { "after_ticks": 60, "repeat_pct": 40, "min_charge_level": 2 }
                ] } },
                { "level": 3, "effect": { "type": "size_stacks_per_hit", "stacks": 2 } },
                { "level": 3, "effect": { "type": "normal_pull", "distance": 2048, "ticks": 6 } },
                { "level": 3, "effect": { "type": "rune_surge",
                    "steps": [["main", "main"], "smashes"] } }
            ] })
    );
    let old = view(json!({ "type": "spirit", "decay_ticks": 1200,
        "levels": [[{ "type": "damage_pct", "pct": 8 }], [], []] }));
    assert_eq!(
        old,
        json!({ "type": "spirit", "max_level": 3, "decay_ticks": 1200 })
    );
}

/// E2: the pack check refuses `raise_per_smash` and each new effect as unshipped, at its path; a
/// spirit naming only the built three loads.
#[test]
fn the_pack_refuses_each_unshipped_spirit_effect() {
    let plain = one_level(json!([
        { "type": "damage_pct", "pct": 8 },
        { "type": "range_pct", "pct": 10 }
    ]));
    let ok = pack_issues(vec![gauge_kind(), plain], None);
    assert_eq!(ok, "", "{ok}");
    let raise = json!({ "type": "spirit", "decay_ticks": 100, "raise_per_smash": 2,
        "levels": [[]] });
    let issues = pack_issues(vec![gauge_kind(), raise], None);
    assert!(
        issues.contains("$.weapons[0].styles[0].kinds[1].raise_per_smash")
            && issues.contains("the effect raise_per_smash is not shipped yet"),
        "{issues}"
    );
    let effects = [
        json!({ "type": "size_stack_cap", "stacks": 1 }),
        json!({ "type": "size_stack_ticks_pct", "pct": 50 }),
        json!({ "type": "size_stacks_per_hit", "stacks": 1 }),
        json!({ "type": "normal_pull", "distance": 2048, "ticks": 6 }),
        json!({ "type": "rune_cooldown_pct", "pct": 10 }),
        json!({ "type": "rune_surge", "steps": [] }),
        json!({ "type": "extra_presses", "presses": 1 }),
        json!({ "type": "smash_repeat", "repeats": [{ "after_ticks": 30, "repeat_pct": 50 }] }),
    ];
    for effect in effects {
        let tag = effect["type"].as_str().unwrap().to_string();
        let issues = pack_issues(vec![gauge_kind(), one_level(json!([effect]))], None);
        assert!(
            issues.contains("$.weapons[0].styles[0].kinds[1].levels[0][0]")
                && issues.contains(&format!("the effect {tag} is not shipped yet")),
            "{tag}: {issues}"
        );
    }
}
