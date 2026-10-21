//! The perk sets file (ADR-105 point 3, ADR-107, ADR-067): each set a content ID, a title and a
//! short list of effects from a closed list, read from JSON as untrusted bytes (ADR-020).
//! The bundled file is the default; other bytes run the same way and change the run's content
//! hash.

use serde_json::{Value, json};
use stella_rain_core::perk_sets::{
    MAX_PERK_EFFECTS, MAX_PERK_MAX_HP, MAX_PERK_PCT, MAX_PERK_REGEN, MAX_PERK_SETS,
    MAX_PERK_SETS_BYTES, PerkEffect, PerkSets,
};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{content_sha256, content_sha256_with};
use stella_rain_core::validate::Error;
use stella_rain_core::validate::limits::DAMAGE_SCALE;

fn set(id: &str, effects: Value) -> Value {
    json!({ "id": id, "title": "Set", "description": "A set", "effects": effects })
}

fn file(sets: Vec<Value>) -> Value {
    json!({ "schema_version": 0, "sim_version": 0, "perk_sets": sets })
}

fn one(effects: Value) -> Value {
    file(vec![set("duelist", effects)])
}

fn parse(v: &Value) -> Result<PerkSets, Error> {
    PerkSets::parse(v.to_string().as_bytes())
}

fn paths(e: Error) -> Vec<String> {
    match e {
        Error::Invalid(issues) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not a validation error: {other:?}"),
    }
}

fn effects_of(id: &str) -> Vec<(PerkEffect, u32)> {
    let sets = PerkSets::bundled();
    let at = sets.index_of(&id.into()).expect("a bundled set");
    sets.sets()[at]
        .effects
        .iter()
        .map(|e| (e.effect, e.amount))
        .collect()
}

#[test]
fn the_bundled_file_holds_the_four_sets_with_kades_numbers() {
    let ids: Vec<&str> = PerkSets::bundled()
        .sets()
        .iter()
        .map(|s| s.id.0.as_str())
        .collect();
    assert_eq!(ids, ["vanguard", "duelist", "scout", "scholar"]);
    assert_eq!(
        effects_of("vanguard"),
        // Kade's numbers are shown hit points; the file holds them scaled (ADR-103).
        [
            (PerkEffect::MaxHp, 300 * DAMAGE_SCALE),
            (PerkEffect::Regen, 10 * DAMAGE_SCALE)
        ]
    );
    assert_eq!(
        effects_of("duelist"),
        [(PerkEffect::AttackPct, 15), (PerkEffect::AttackSpeedPct, 8)]
    );
    assert_eq!(
        effects_of("scout"),
        [(PerkEffect::SpeedPct, 12), (PerkEffect::EvadePct, 20)]
    );
    assert_eq!(
        effects_of("scholar"),
        [
            (PerkEffect::ExtraRerolls, 2),
            (PerkEffect::ExtraBanishes, 1)
        ]
    );
}

#[test]
fn a_valid_file_loads_and_keeps_its_hash() {
    let v = one(json!([{ "effect": "attack_pct", "amount": 5 }]));
    let bytes = v.to_string();
    let sets = PerkSets::parse(bytes.as_bytes()).unwrap();
    assert_eq!(sets.sets().len(), 1);
    assert_ne!(sets.sha256(), PerkSets::bundled().sha256());
}

#[test]
fn a_file_that_is_too_large_is_refused_before_parsing() {
    let big = vec![b' '; MAX_PERK_SETS_BYTES + 1];
    assert!(matches!(PerkSets::parse(&big), Err(Error::TooLarge { .. })));
}

#[test]
fn the_versions_and_unknown_fields_fail_closed() {
    let mut v = one(json!([]));
    v["schema_version"] = json!(9);
    assert!(matches!(parse(&v), Err(Error::UnsupportedVersion { .. })));
    let mut v = one(json!([]));
    v["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let v = one(json!([{ "effect": "attack_pct", "amount": 5, "extra": 1 }]));
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let v = one(json!([{ "effect": "gold_pct", "amount": 5 }]));
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
}

#[test]
fn the_number_of_sets_is_1_to_the_cap() {
    assert_eq!(paths(parse(&file(vec![])).unwrap_err()), ["$.perk_sets"]);
    let many = |n: usize| {
        file(
            (0..n)
                .map(|i| set(&format!("set_{i}"), json!([])))
                .collect(),
        )
    };
    assert!(parse(&many(MAX_PERK_SETS)).is_ok());
    assert_eq!(
        paths(parse(&many(MAX_PERK_SETS + 1)).unwrap_err()),
        ["$.perk_sets"]
    );
}

#[test]
fn ids_are_well_formed_and_unique() {
    let v = file(vec![set("Bad Id", json!([])), set("ok", json!([]))]);
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.perk_sets[0].id"]);
    let v = file(vec![set("same", json!([])), set("same", json!([]))]);
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.perk_sets[1].id"]);
}

#[test]
fn a_set_holds_up_to_the_cap_of_effects_each_kind_once() {
    let all = [
        "max_hp",
        "regen",
        "attack_pct",
        "attack_speed_pct",
        "evade_pct",
        "speed_pct",
        "extra_rerolls",
        "extra_banishes",
    ];
    assert_eq!(all.len(), MAX_PERK_EFFECTS);
    let effects: Vec<Value> = all
        .iter()
        .map(|e| json!({ "effect": e, "amount": 1 }))
        .collect();
    assert!(parse(&one(Value::Array(effects.clone()))).is_ok());
    let mut more = effects;
    more.push(json!({ "effect": "max_hp", "amount": 1 }));
    let found = paths(parse(&one(Value::Array(more))).unwrap_err());
    assert!(found.contains(&"$.perk_sets[0].effects".to_owned()));
    let twice = one(json!([
        { "effect": "max_hp", "amount": 1 },
        { "effect": "max_hp", "amount": 2 }
    ]));
    assert_eq!(
        paths(parse(&twice).unwrap_err()),
        ["$.perk_sets[0].effects[1].effect"]
    );
}

#[test]
fn an_amount_is_1_to_its_effects_cap() {
    let at =
        |effect: &str, amount: u32| parse(&one(json!([{ "effect": effect, "amount": amount }])));
    for effect in ["attack_pct", "attack_speed_pct", "evade_pct", "speed_pct"] {
        assert!(at(effect, MAX_PERK_PCT).is_ok(), "{effect} at the cap");
        assert_eq!(
            paths(at(effect, MAX_PERK_PCT + 1).unwrap_err()),
            ["$.perk_sets[0].effects[0].amount"]
        );
        assert!(at(effect, 0).is_err(), "{effect} at 0");
    }
    for (effect, cap) in [("max_hp", MAX_PERK_MAX_HP), ("regen", MAX_PERK_REGEN)] {
        assert!(at(effect, cap).is_ok(), "{effect} at the cap");
        assert_eq!(
            paths(at(effect, cap + 1).unwrap_err()),
            ["$.perk_sets[0].effects[0].amount"]
        );
    }
    for effect in ["max_hp", "regen", "extra_rerolls", "extra_banishes"] {
        assert!(at(effect, 0).is_err(), "{effect} at 0");
        assert!(at(effect, 1_000_000).is_err(), "{effect} far past the cap");
    }
}

#[test]
fn titles_and_descriptions_are_bounded_text() {
    let mut v = file(vec![set("a", json!([]))]);
    v["perk_sets"][0]["title"] = json!("");
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.perk_sets[0].title"]);
    let mut v = file(vec![set("a", json!([]))]);
    v["perk_sets"][0]["description"] = json!("x".repeat(500));
    assert_eq!(
        paths(parse(&v).unwrap_err()),
        ["$.perk_sets[0].description"]
    );
}

#[test]
fn other_bytes_change_the_content_hash() {
    let other = parse(&one(json!([{ "effect": "attack_pct", "amount": 5 }]))).unwrap();
    let content = RunContent::bundled().clone().with_perk_sets(other);
    assert_ne!(
        content_sha256_with("trial", &content),
        content_sha256("trial")
    );
    assert!(content.pack_sha256().is_none());
}
