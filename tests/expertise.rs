//! Expertise content (ADR-078 §7, ADR-067): the thresholds of levels 1 to 10, the growth factor
//! and its maximum, each weapon's three styles and common pool, and the stack limits, read from
//! JSON as untrusted bytes (ADR-020). Nothing reads it yet; these tests hold its bounds: the
//! thresholds rise strictly, the factor is above 1, no pool is empty, every weapon has three
//! complete styles, and every number is capped.

use serde_json::{Value, json};
use stella_rain_core::expertise::{
    Expertise, ExpertiseEffect, MAX_COMMON_OPTIONS, MAX_EXPERTISE_BYTES, MAX_GROWTH_PCT,
    MAX_LEVEL_POINTS, MAX_STACK_LIMIT, MAX_STYLE_CHOICES, STYLES, TABLE_LEVELS,
};
use stella_rain_core::registry::{self, Kind};
use stella_rain_core::rewards::{MAX_AMOUNT, MAX_RANGE_PCT};
use stella_rain_core::validate::Error;

fn option(id: &str, effect: &str, amount: u32) -> Value {
    json!({
        "id": id,
        "effect": effect,
        "amount": amount,
        "title": "Option",
        "description": "What it does"
    })
}

fn style(id: &str) -> Value {
    json!({
        "id": id,
        "effect": "attack_pct",
        "amount": 10,
        "title": "Style",
        "description": "A path",
        "upgrade_6": [option(&format!("{id}_6"), "attack_pct", 5)],
        "upgrade_9": [option(&format!("{id}_9"), "speed_pct", 5)],
        "capstone": [option(&format!("{id}_10"), "range_pct", 20)]
    })
}

fn limited(id: &str, limit: u8) -> Value {
    let mut o = option(id, "attack_speed_pct", 10);
    o["limit"] = json!(limit);
    o
}

fn weapon(id: &str) -> Value {
    json!({
        "weapon": id,
        "styles": [style("one"), style("two"), style("three")],
        "common": [option("damage", "attack_pct", 10), limited("attack_speed", 5)]
    })
}

/// The weapon presets of the registry, in its order: every one a file must cover.
fn weapon_ids() -> Vec<&'static str> {
    registry::ENTRIES
        .iter()
        .filter(|e| e.kind == Kind::Weapon)
        .map(|e| e.id)
        .collect()
}

fn file() -> Value {
    json!({
        "schema_version": 0,
        "sim_version": 0,
        "thresholds": [10, 20, 30, 40, 50, 60, 70, 80, 90, 100],
        "growth_pct": 110,
        "max_points": 1000,
        "weapons": weapon_ids().into_iter().map(weapon).collect::<Vec<_>>()
    })
}

fn parse(v: &Value) -> Result<Expertise, Error> {
    Expertise::parse(v.to_string().as_bytes())
}

fn with(pointer: &str, value: Value) -> Value {
    let mut v = file();
    *v.pointer_mut(pointer)
        .unwrap_or_else(|| panic!("{pointer} exists in the test file")) = value;
    v
}

fn valid(v: &Value) {
    if let Err(e) = parse(v) {
        panic!("expected valid, got {e:?}");
    }
}

fn invalid_at(v: &Value, path: &str) {
    match parse(v) {
        Err(Error::Invalid(issues)) => {
            assert!(issues.iter().any(|i| i.path == path), "{path}: {issues:?}")
        }
        other => panic!("{path}: expected invalid, got {other:?}"),
    }
}

#[test]
fn the_bundled_file_covers_every_weapon_with_three_complete_styles() {
    let bundled = Expertise::bundled();
    assert_eq!(
        Expertise::parse(Expertise::BUNDLED).as_ref(),
        Ok(bundled),
        "the bundled bytes parse to the bundled content"
    );
    assert_eq!(bundled.thresholds().len(), TABLE_LEVELS);
    assert!(bundled.growth_pct() > 100);
    let covered: Vec<&str> = bundled
        .weapons()
        .iter()
        .map(|w| w.weapon.0.as_str())
        .collect();
    assert_eq!(covered, weapon_ids());
    for w in bundled.weapons() {
        assert_eq!(w.styles.len(), STYLES, "{}", w.weapon.0);
        for s in &w.styles {
            assert!(!s.upgrade_6.is_empty() && !s.upgrade_9.is_empty() && !s.capstone.is_empty());
        }
        assert!(!w.common.is_empty(), "{}", w.weapon.0);
        // Only attack speed carries a stack limit (cooldown reduction has no kind in core yet).
        for o in &w.common {
            assert_eq!(
                o.limit.is_some(),
                o.effect == ExpertiseEffect::AttackSpeedPct,
                "{}",
                o.id.0
            );
        }
        assert_eq!(bundled.weapon(&w.weapon), Some(w));
    }
}

#[test]
fn a_valid_file_parses_and_keeps_its_order() {
    let e = parse(&file()).unwrap();
    assert_eq!(e.thresholds(), [10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
    assert_eq!((e.growth_pct(), e.max_points()), (110, 1000));
    let ids: Vec<&str> = e.weapons()[0]
        .styles
        .iter()
        .map(|s| s.id.0.as_str())
        .collect();
    assert_eq!(ids, ["one", "two", "three"]);
}

#[test]
fn a_file_over_the_limit_is_not_parsed() {
    let big = vec![b' '; MAX_EXPERTISE_BYTES + 1];
    assert_eq!(
        Expertise::parse(&big),
        Err(Error::TooLarge {
            size: MAX_EXPERTISE_BYTES + 1,
            limit: MAX_EXPERTISE_BYTES
        })
    );
    let mut padded = file().to_string().into_bytes();
    padded.resize(MAX_EXPERTISE_BYTES, b' ');
    assert!(Expertise::parse(&padded).is_ok(), "at the limit it parses");
}

#[test]
fn only_the_expertise_format_parses() {
    // An unknown field at the top, in a weapon, a style and an option.
    let extra = |pointer: &str| {
        let mut v = file();
        v.pointer_mut(pointer).unwrap()["extra"] = json!(1);
        v
    };
    for bad in [
        json!("not an object"),
        json!({ "schema_version": 0, "sim_version": 0 }),
        extra(""),
        extra("/weapons/0"),
        extra("/weapons/0/styles/0"),
        extra("/weapons/0/styles/0/capstone/0"),
        with("/weapons/0/common/0/effect", json!("no_such_effect")),
        with("/growth_pct", json!(-1)),
    ] {
        assert!(
            matches!(parse(&bad), Err(Error::Parse { .. })),
            "{bad}: {:?}",
            parse(&bad)
        );
    }
}

#[test]
fn newer_versions_are_refused_before_the_strict_parse() {
    let mut v = file();
    v["sim_version"] = json!(1);
    v["from_the_future"] = json!(true);
    assert_eq!(
        parse(&v),
        Err(Error::UnsupportedVersion {
            schema_version: 0,
            sim_version: 1
        })
    );
}

#[test]
fn the_table_holds_ten_thresholds_that_rise_strictly() {
    invalid_at(&with("/thresholds", json!([10, 20, 30])), "$.thresholds");
    invalid_at(
        &with("/thresholds", json!([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])),
        "$.thresholds",
    );
    invalid_at(&with("/thresholds/0", json!(0)), "$.thresholds[0]");
    invalid_at(&with("/thresholds/4", json!(40)), "$.thresholds[4]");
    invalid_at(&with("/thresholds/4", json!(39)), "$.thresholds[4]");
    // No threshold beyond the most one level needs.
    invalid_at(&with("/thresholds/9", json!(1001)), "$.thresholds[9]");
}

#[test]
fn the_maximum_is_at_least_the_last_threshold_and_capped() {
    valid(&with("/max_points", json!(100)));
    invalid_at(&with("/max_points", json!(99)), "$.max_points");
    valid(&with("/max_points", json!(MAX_LEVEL_POINTS)));
    invalid_at(
        &with("/max_points", json!(MAX_LEVEL_POINTS + 1)),
        "$.max_points",
    );
}

#[test]
fn the_growth_factor_is_above_one_and_capped() {
    invalid_at(&with("/growth_pct", json!(100)), "$.growth_pct");
    valid(&with("/growth_pct", json!(101)));
    valid(&with("/growth_pct", json!(MAX_GROWTH_PCT)));
    invalid_at(
        &with("/growth_pct", json!(MAX_GROWTH_PCT + 1)),
        "$.growth_pct",
    );
}

#[test]
fn every_weapon_preset_is_covered_once_and_nothing_else() {
    let mut missing = file();
    missing["weapons"].as_array_mut().unwrap().pop();
    invalid_at(&missing, "$.weapons");
    invalid_at(
        &with("/weapons/0/weapon", json!("no_such_weapon")),
        "$.weapons[0].weapon",
    );
    // An ID of another kind is not a weapon.
    invalid_at(
        &with("/weapons/0/weapon", json!("slash")),
        "$.weapons[0].weapon",
    );
    let second = weapon_ids()[0];
    invalid_at(
        &with("/weapons/1/weapon", json!(second)),
        "$.weapons[1].weapon",
    );
}

#[test]
fn every_weapon_has_three_complete_styles() {
    let three = json!([style("a"), style("b"), style("c")]);
    for n in [0, 2, 4] {
        let styles: Vec<Value> = (0..n).map(|i| style(&format!("s{i}"))).collect();
        invalid_at(
            &with("/weapons/0/styles", json!(styles)),
            "$.weapons[0].styles",
        );
    }
    valid(&with("/weapons/0/styles", three));
    for level in ["upgrade_6", "upgrade_9", "capstone"] {
        invalid_at(
            &with(&format!("/weapons/0/styles/1/{level}"), json!([])),
            &format!("$.weapons[0].styles[1].{level}"),
        );
        let choices: Vec<Value> = (0..=MAX_STYLE_CHOICES)
            .map(|i| option(&format!("c{i}"), "attack_pct", 5))
            .collect();
        valid(&with(
            &format!("/weapons/0/styles/1/{level}"),
            json!(choices[..MAX_STYLE_CHOICES]),
        ));
        invalid_at(
            &with(&format!("/weapons/0/styles/1/{level}"), json!(choices)),
            &format!("$.weapons[0].styles[1].{level}"),
        );
    }
}

#[test]
fn no_common_pool_is_empty_or_past_its_budget() {
    invalid_at(&with("/weapons/1/common", json!([])), "$.weapons[1].common");
    let pool = |n: usize| -> Value {
        json!(
            (0..n)
                .map(|i| option(&format!("o{i}"), "attack_pct", 5))
                .collect::<Vec<_>>()
        )
    };
    valid(&with("/weapons/1/common", pool(MAX_COMMON_OPTIONS)));
    invalid_at(
        &with("/weapons/1/common", pool(MAX_COMMON_OPTIONS + 1)),
        "$.weapons[1].common",
    );
}

#[test]
fn only_attack_speed_carries_a_stack_limit_and_it_is_bounded() {
    let at = "$.weapons[0].common[1].limit";
    invalid_at(
        &with("/weapons/0/common/1", option("x", "attack_speed_pct", 10)),
        at,
    );
    invalid_at(&with("/weapons/0/common/1", limited("x", 0)), at);
    valid(&with("/weapons/0/common/1", limited("x", MAX_STACK_LIMIT)));
    invalid_at(
        &with("/weapons/0/common/1", limited("x", MAX_STACK_LIMIT + 1)),
        at,
    );
    let mut damage = option("x", "attack_pct", 10);
    damage["limit"] = json!(3);
    invalid_at(&with("/weapons/0/common/1", damage), at);
    // A style's choices are fixed, never drawn: no limit there either.
    invalid_at(
        &with("/weapons/0/styles/0/upgrade_6/0", limited("x", 3)),
        "$.weapons[0].styles[0].upgrade_6[0].limit",
    );
}

#[test]
fn amounts_are_one_to_their_effect_s_most() {
    let at = "$.weapons[0].common[0].amount";
    invalid_at(&with("/weapons/0/common/0/amount", json!(0)), at);
    valid(&with("/weapons/0/common/0/amount", json!(MAX_AMOUNT)));
    invalid_at(
        &with("/weapons/0/common/0/amount", json!(MAX_AMOUNT + 1)),
        at,
    );
    let range = |amount: u32| option("x", "range_pct", amount);
    valid(&with("/weapons/0/common/0", range(MAX_RANGE_PCT)));
    invalid_at(&with("/weapons/0/common/0", range(MAX_RANGE_PCT + 1)), at);
    invalid_at(
        &with("/weapons/0/styles/2/amount", json!(0)),
        "$.weapons[0].styles[2].amount",
    );
    invalid_at(
        &with(
            "/weapons/0/styles/2/capstone/0/amount",
            json!(MAX_RANGE_PCT + 1),
        ),
        "$.weapons[0].styles[2].capstone[0].amount",
    );
}

#[test]
fn ids_are_well_formed_and_unique_within_a_weapon() {
    invalid_at(
        &with("/weapons/0/styles/0/id", json!("Bad ID")),
        "$.weapons[0].styles[0].id",
    );
    invalid_at(
        &with("/weapons/0/common/0/id", json!("one")),
        "$.weapons[0].common[0].id",
    );
    invalid_at(
        &with("/weapons/0/styles/1/upgrade_9/0/id", json!("one_6")),
        "$.weapons[0].styles[1].upgrade_9[0].id",
    );
    // Each weapon's IDs are its own: the test file repeats them across weapons.
    valid(&file());
}

#[test]
fn titles_and_descriptions_are_bounded_text() {
    invalid_at(
        &with("/weapons/0/styles/0/title", json!("")),
        "$.weapons[0].styles[0].title",
    );
    invalid_at(
        &with("/weapons/0/common/0/description", json!("x".repeat(161))),
        "$.weapons[0].common[0].description",
    );
    invalid_at(
        &with("/weapons/0/styles/0/capstone/0/title", json!("a\u{7}b")),
        "$.weapons[0].styles[0].capstone[0].title",
    );
}
