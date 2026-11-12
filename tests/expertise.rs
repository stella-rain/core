//! Expertise content (ADR-CONT-1): the thresholds of levels 1 to 10, the growth factor
//! and its maximum, each weapon's three styles and common pool, read from
//! JSON as untrusted bytes (ADR-SVC-1). Nothing reads it yet; these tests hold its bounds: the
//! thresholds rise strictly, the factor is above 1, no pool is empty, every weapon has three
//! complete styles, and every number is capped.

use crate::common;
use serde_json::{Value, json};
use stella_rain_core::expertise::{
    Expertise, ExpertiseEffect, MAX_COMMON_OPTIONS, MAX_EXPERTISE_BYTES, MAX_GROWTH_PCT,
    MAX_LEVEL_POINTS, MAX_STYLE_CHOICES, STYLES, TABLE_LEVELS,
};
use stella_rain_core::registry::{self, Kind};
use stella_rain_core::rewards::{MAX_AMOUNT, MAX_HP_AMOUNT, MAX_RANGE_PCT};
use stella_rain_core::validate::Error;
use stella_rain_core::version::{NEWEST_READ_SCHEMA_VERSION, SIM_VERSION, Versions};

fn option(id: &str, effect: &str, amount: u32) -> Value {
    json!({
        "id": id,
        "effect": effect,
        "amount": amount,
    })
}

fn style(id: &str) -> Value {
    json!({
        "id": id,
        "effect": "attack_pct",
        "amount": 10,
        "upgrade_1": [option(&format!("{id}_6"), "attack_pct", 5)],
        "upgrade_2": [option(&format!("{id}_9"), "speed_pct", 5)],
        "capstone": [option(&format!("{id}_10"), "range_pct", 20)]
    })
}

fn limited(id: &str, effect: &str, limit: u8) -> Value {
    let mut o = option(id, effect, 10);
    o["limit"] = json!(limit);
    o
}

fn weapon(id: &str) -> Value {
    json!({
        "weapon": id,
        "styles": [style("one"), style("two"), style("three")],
        "common": [option("damage", "attack_pct", 10), option("attack_speed", "attack_speed_pct", 10)]
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
        "thresholds": [10, 20, 30, 40, 50, 60, 70, 80, 90, 100],
        "growth_pct": 110,
        "max_points": 1000,
        "weapons": weapon_ids().into_iter().map(weapon).collect::<Vec<_>>()
    })
}

fn parse_at(v: &Value, versions: Versions) -> Result<Expertise, Error> {
    Expertise::parse(v.to_string().as_bytes(), versions)
}

fn parse(v: &Value) -> Result<Expertise, Error> {
    parse_at(v, Versions::CURRENT)
}

fn with(pointer: &str, value: Value) -> Value {
    let mut v = file();
    // The last key may be new (`kinds`): its parent has to exist.
    let (parent, key) = pointer.rsplit_once('/').expect("a pointer");
    match v.pointer_mut(pointer) {
        Some(slot) => *slot = value,
        None => {
            v.pointer_mut(parent)
                .and_then(Value::as_object_mut)
                .unwrap_or_else(|| panic!("{parent} exists in the test file"))
                .insert(key.to_string(), value);
        }
    }
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
        Expertise::parse(common::bundled_file("expertise"), Versions::CURRENT).as_ref(),
        Ok(bundled),
        "the bundled bytes parse to the bundled content"
    );
    assert_eq!(bundled.thresholds().len(), TABLE_LEVELS);
    assert!(bundled.growth_pct() > 100);
    let covered: Vec<&str> = bundled
        .weapons()
        .iter()
        .map(|w| w.weapon.as_str())
        .collect();
    assert_eq!(covered, weapon_ids());
    for w in bundled.weapons() {
        assert_eq!(w.styles.len(), STYLES, "{}", w.weapon.as_str());
        for s in &w.styles {
            assert!(!s.upgrade_1.is_empty() && !s.upgrade_2.is_empty() && !s.capstone.is_empty());
        }
        assert!(!w.common.is_empty(), "{}", w.weapon.as_str());
        // No option has a stack limit, attack speed included.
        assert!(
            w.common
                .iter()
                .any(|o| o.effect == ExpertiseEffect::AttackSpeedPct),
            "{}",
            w.weapon.as_str()
        );
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
        .map(|s| s.id.as_str())
        .collect();
    assert_eq!(ids, ["one", "two", "three"]);
}

#[test]
fn a_file_over_the_limit_is_not_parsed() {
    let big = vec![b' '; MAX_EXPERTISE_BYTES + 1];
    assert_eq!(
        Expertise::parse(&big, Versions::CURRENT),
        Err(Error::TooLarge {
            size: MAX_EXPERTISE_BYTES + 1,
            limit: MAX_EXPERTISE_BYTES
        })
    );
    let mut padded = file().to_string().into_bytes();
    padded.resize(MAX_EXPERTISE_BYTES, b' ');
    assert!(
        Expertise::parse(&padded, Versions::CURRENT).is_ok(),
        "at the limit it parses"
    );
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
        json!({ "schema_version": 1, "sim_version": 1 }),
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
    v["from_the_future"] = json!(true);
    assert_eq!(
        parse_at(
            &v,
            Versions {
                schema_version: NEWEST_READ_SCHEMA_VERSION + 1,
                sim_version: SIM_VERSION + 1
            }
        ),
        Err(Error::UnsupportedVersion {
            schema_version: NEWEST_READ_SCHEMA_VERSION + 1,
            sim_version: SIM_VERSION + 1
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
    // Each upgrade holds exactly one choice (ADR-SIM-9 point 3), and a fourth, plain style is no
    // style: the base is the plain one.
    assert_eq!(MAX_STYLE_CHOICES, 1);
    let three = json!([style("a"), style("b"), style("c")]);
    for n in [0, 2, 4] {
        let styles: Vec<Value> = (0..n).map(|i| style(&format!("s{i}"))).collect();
        invalid_at(
            &with("/weapons/0/styles", json!(styles)),
            "$.weapons[0].styles",
        );
    }
    valid(&with("/weapons/0/styles", three));
    for level in ["upgrade_1", "upgrade_2", "capstone"] {
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
fn no_expertise_option_has_a_stack_limit() {
    valid(&with(
        "/weapons/0/common/1",
        option("x", "attack_speed_pct", 10),
    ));
    // The strict parse refuses the field, wherever it appears.
    for (at, effect, limit) in [
        ("/weapons/0/common/1", "attack_speed_pct", 0),
        ("/weapons/0/common/1", "attack_speed_pct", 5),
        ("/weapons/0/common/1", "attack_speed_pct", 21),
        ("/weapons/0/common/1", "attack_pct", 3),
        ("/weapons/0/styles/0/upgrade_1/0", "attack_speed_pct", 3),
    ] {
        assert!(
            parse(&with(at, limited("x", effect, limit))).is_err(),
            "{at}: limit {limit}"
        );
    }
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
    // Hit points are scaled by 100; the bound is too.
    let hp = |amount: u32| option("x", "max_hp", amount);
    valid(&with("/weapons/0/common/0", hp(MAX_HP_AMOUNT)));
    invalid_at(&with("/weapons/0/common/0", hp(MAX_HP_AMOUNT + 1)), at);
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
        &with("/weapons/0/styles/1/upgrade_2/0/id", json!("one_6")),
        "$.weapons[0].styles[1].upgrade_2[0].id",
    );
    // Each weapon's IDs are its own: the test file repeats them across weapons.
    valid(&file());
}

#[test]
fn text_fields_are_refused_as_unknown() {
    let mut v = file();
    v["weapons"][0]["styles"][0]["title"] = json!("Style");
    assert!(parse(&v).is_err());
    let mut v = file();
    v["weapons"][0]["common"][0]["description"] = json!("desc");
    assert!(parse(&v).is_err());
}

fn charge_kind(step: Value) -> Value {
    json!({
        "type": "replace_step",
        "step": step,
        "grant": { "charge": {
            "levels": [{ "at_ticks": 20, "attack": { "preset": { "id": "slash" } } }],
            "max_ticks": 60
        } }
    })
}

fn kind_level(id: &str, kinds: Value) -> Value {
    json!({ "id": id, "effect": "none", "kinds": kinds })
}

const AAA: [&str; 3] = ["main", "main", "main"];

/// T2: each upgrade holds exactly one choice.
#[test]
fn an_upgrade_holds_exactly_one_choice() {
    for level in ["upgrade_1", "upgrade_2", "capstone"] {
        let at = format!("/weapons/0/styles/1/{level}");
        let path = format!("$.weapons[0].styles[1].{level}");
        valid(&with(&at, json!([option("c0", "attack_pct", 5)])));
        invalid_at(&with(&at, json!([])), &path);
        invalid_at(
            &with(
                &at,
                json!([option("c0", "attack_pct", 5), option("c1", "attack_pct", 5)]),
            ),
            &path,
        );
    }
    invalid_at(
        &with(
            "/weapons/0/styles",
            json!([style("a"), style("b"), style("c"), style("default")]),
        ),
        "$.weapons[0].styles",
    );
}

/// T2: a level names mechanic kinds, a closed list; a level of only a mechanic is `none`.
#[test]
fn a_level_names_its_kinds() {
    let kinds = json!([charge_kind(json!(AAA))]);
    let v = with("/weapons/0/styles/0/kinds", kinds.clone());
    let parsed = parse(&v).expect("a style with a kind parses");
    let level = serde_json::to_value(&parsed.weapons()[0].styles[0]).unwrap();
    // A preset reference serialises with its (empty) arguments.
    let mut want = kinds.clone();
    want[0]["grant"]["charge"]["levels"][0]["attack"]["preset"]["args"] = json!([]);
    assert_eq!(level["kinds"], want);
    for at in [
        "/weapons/0/styles/0/upgrade_1/0",
        "/weapons/0/styles/1/capstone/0",
    ] {
        let mut v = file();
        *v.pointer_mut(at).unwrap() = kind_level("spirit_blade", kinds.clone());
        valid(&v);
    }
    // `none` takes no amount and needs a kind.
    let level = |extra: Value| {
        let mut l = kind_level("spirit_blade", kinds.clone());
        for (k, x) in extra.as_object().unwrap() {
            l[k] = x.clone();
        }
        l
    };
    let at = "/weapons/0/styles/0/upgrade_1/0";
    valid(&with(at, level(json!({ "amount": 0 }))));
    invalid_at(
        &with(at, level(json!({ "amount": 5 }))),
        "$.weapons[0].styles[0].upgrade_1[0].amount",
    );
    invalid_at(
        &with(at, json!({ "id": "x", "effect": "none" })),
        "$.weapons[0].styles[0].upgrade_1[0].effect",
    );
    invalid_at(
        &with(at, kind_level("x", json!([]))),
        "$.weapons[0].styles[0].upgrade_1[0].effect",
    );
    // A stat level still needs its amount.
    invalid_at(
        &with("/weapons/0/styles/0/amount", json!(0)),
        "$.weapons[0].styles[0].amount",
    );
}

/// T2: the kinds of a level are bounded, and each is checked by the weapon validator's own caps.
#[test]
fn the_kinds_of_a_level_are_bounded() {
    let at = "/weapons/0/styles/0/kinds";
    let path = "$.weapons[0].styles[0].kinds";
    let steps: [Value; 5] = [
        json!(["main"]),
        json!(["smash"]),
        json!(["main", "main"]),
        json!(["main", "smash"]),
        json!(["main", "main", "smash"]),
    ];
    valid(&with(
        at,
        json!(
            steps[..4]
                .iter()
                .cloned()
                .map(charge_kind)
                .collect::<Vec<_>>()
        ),
    ));
    invalid_at(
        &with(
            at,
            json!(steps.iter().cloned().map(charge_kind).collect::<Vec<_>>()),
        ),
        path,
    );
    invalid_at(
        &with(at, json!([charge_kind(json!([]))])),
        &format!("{path}[0].step"),
    );
    invalid_at(
        &with(at, json!([charge_kind(json!(vec!["main"; 7]))])),
        &format!("{path}[0].step"),
    );
    let grant = |g: Value| {
        let mut k = charge_kind(json!(AAA));
        k["grant"] = g;
        with(at, json!([k]))
    };
    let slash = json!({ "preset": { "id": "slash" } });
    invalid_at(
        &grant(json!({ "charge": { "levels": [], "max_ticks": 60 } })),
        &format!("{path}[0].grant.charge.levels"),
    );
    invalid_at(
        &grant(
            json!({ "charge": { "levels": [{ "at_ticks": 20, "attack": slash }], "max_ticks": 0 } }),
        ),
        &format!("{path}[0].grant.charge.max_ticks"),
    );
    invalid_at(
        &grant(
            json!({ "charge": { "levels": [{ "at_ticks": 20, "attack": slash }], "max_ticks": 100_000 } }),
        ),
        &format!("{path}[0].grant.charge.max_ticks"),
    );
    invalid_at(
        &grant(json!({ "hold": { "loop": slash, "release": slash, "max_ticks": 0 } })),
        &format!("{path}[0].grant.hold.max_ticks"),
    );
    // An attack naming a preset no registry has.
    invalid_at(
        &grant(json!({ "charge": { "levels": [{ "at_ticks": 20,
            "attack": { "preset": { "id": "no_such_preset" } } }], "max_ticks": 60 } })),
        &format!("{path}[0].grant.charge.levels[0].attack.preset.id"),
    );
}

/// T2: two grants on one step inside a style are refused, one on each of two steps is not.
#[test]
fn a_style_grants_a_step_once() {
    let mut v = file();
    v["weapons"][0]["styles"][0]["kinds"] = json!([charge_kind(json!(AAA))]);
    v["weapons"][0]["styles"][0]["upgrade_1"][0] =
        kind_level("again", json!([charge_kind(json!(AAA))]));
    invalid_at(&v, "$.weapons[0].styles[0].upgrade_1[0].kinds[0].step");
    v["weapons"][0]["styles"][0]["upgrade_1"][0] =
        kind_level("other", json!([charge_kind(json!(["main", "main"]))]));
    valid(&v);
}

/// T2: `kinds` and `none` are rules of `sim_version` 2 (ADR-SIM-9 point 6).
#[test]
fn kinds_need_sim_version_2() {
    let v = with(
        "/weapons/0/styles/0/kinds",
        json!([charge_kind(json!(AAA))]),
    );
    let old = Versions {
        sim_version: 1,
        ..Versions::CURRENT
    };
    match parse_at(&v, old) {
        Err(Error::Invalid(issues)) => assert!(
            issues
                .iter()
                .any(|i| i.path == "$.weapons[0].styles[0].kinds"),
            "{issues:?}"
        ),
        other => panic!("{other:?}"),
    }
    let v = with(
        "/weapons/0/styles/0/upgrade_1/0",
        kind_level("x", json!([charge_kind(json!(AAA))])),
    );
    match parse_at(&v, old) {
        Err(Error::Invalid(issues)) => assert!(
            issues
                .iter()
                .any(|i| i.path == "$.weapons[0].styles[0].upgrade_1[0].effect"),
            "{issues:?}"
        ),
        other => panic!("{other:?}"),
    }
}

/// T2: the bundled file names no kind: every level is a stat (dormant).
#[test]
fn the_bundled_file_names_no_kind() {
    let weapons = serde_json::to_value(Expertise::bundled().weapons()).unwrap();
    for w in weapons.as_array().unwrap() {
        for s in w["styles"].as_array().unwrap() {
            let levels = ["upgrade_1", "upgrade_2", "capstone"]
                .iter()
                .flat_map(|f| s[f].as_array().unwrap().iter())
                .chain(std::iter::once(s));
            for l in levels {
                assert!(l.get("kinds").is_none(), "{l}");
                assert_ne!(l["effect"], "none", "{l}");
            }
        }
    }
}

/// A common option is a stat: it names no kind and is never `none`.
#[test]
fn a_common_option_names_no_kind() {
    let at = "/weapons/0/common/0";
    let mut v = file();
    v["weapons"][0]["common"][0] = kind_level("spirit_blade", json!([charge_kind(json!(AAA))]));
    invalid_at(&v, "$.weapons[0].common[0].kinds");
    let mut v = file();
    v["weapons"][0]["common"][0]["kinds"] = json!([charge_kind(json!(AAA))]);
    invalid_at(&v, "$.weapons[0].common[0].kinds");
    valid(&with(at, option("plain", "attack_pct", 5)));
}

fn smash_kind(effect: Value) -> Value {
    json!({ "type": "smash_effect", "effect": effect })
}

/// A style level sets its weapon's smash effect (ADR-SIM-9 point 4): the closed list, every
/// effect bounded by the weapon validator's caps, one per style path.
#[test]
fn a_level_sets_a_smash_effect() {
    let at = "/weapons/0/styles/0/kinds";
    let path = "$.weapons[0].styles[0].kinds[0].effect";
    for effect in [
        json!({ "type": "reap", "stacks": 4, "boss_pct": 30 }),
        json!({ "type": "pull", "distance": 4096, "ticks": 10 }),
        json!({ "type": "knockback", "distance": 4096, "ticks": 10 }),
        json!({ "type": "bleed" }),
        json!({ "type": "stun", "ticks": 30 }),
    ] {
        let v = with(at, json!([smash_kind(effect.clone())]));
        valid(&v);
    }
    let bad = |effect: Value, sub: &str| {
        invalid_at(
            &with(at, json!([smash_kind(effect)])),
            &format!("{path}{sub}"),
        );
    };
    bad(
        json!({ "type": "reap", "stacks": 0, "boss_pct": 30 }),
        ".stacks",
    );
    bad(
        json!({ "type": "reap", "stacks": 11, "boss_pct": 30 }),
        ".stacks",
    );
    bad(
        json!({ "type": "reap", "stacks": 4, "boss_pct": 101 }),
        ".boss_pct",
    );
    bad(
        json!({ "type": "pull", "distance": 16385, "ticks": 10 }),
        ".distance",
    );
    bad(
        json!({ "type": "pull", "distance": 10, "ticks": 0 }),
        ".ticks",
    );
    bad(json!({ "type": "stun", "ticks": 0 }), ".ticks");
}

#[test]
fn a_style_sets_the_smash_effect_once() {
    let stun = || smash_kind(json!({ "type": "stun", "ticks": 30 }));
    let mut v = with("/weapons/0/styles/0/kinds", json!([stun()]));
    v["weapons"][0]["styles"][0]["upgrade_1"] = json!([kind_level("again", json!([stun()]))]);
    invalid_at(&v, "$.weapons[0].styles[0].upgrade_1[0].kinds[0]");
    // One in each of two styles, and one beside a replace_step in a level, are fine.
    let mut v = with("/weapons/0/styles/0/kinds", json!([stun()]));
    v["weapons"][0]["styles"][1]["kinds"] = json!([stun()]);
    valid(&v);
    valid(&with(
        "/weapons/0/styles/0/kinds",
        json!([stun(), charge_kind(json!(AAA))]),
    ));
}

#[test]
fn a_smash_effect_needs_sim_version_2() {
    let v = with(
        "/weapons/0/styles/0/kinds",
        json!([smash_kind(json!({ "type": "bleed" }))]),
    );
    let old = Versions {
        sim_version: 1,
        ..Versions::CURRENT
    };
    match parse_at(&v, old) {
        Err(Error::Invalid(issues)) => assert!(
            issues
                .iter()
                .any(|i| i.path == "$.weapons[0].styles[0].kinds"),
            "{issues:?}"
        ),
        other => panic!("{other:?}"),
    }
}
