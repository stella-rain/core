//! The breaker kind of a style level (ADR-SIM-9 points 2 to 7, ADR-SIM-5, ADR-CONT-3): its
//! numbers are bounded by the validator, the kind is `sim_version` 2's, a path may name it at
//! several levels (its effects stack when each level is taken), it round-trips through JSON and
//! the style views carry it. Test code only: the numbers are test values, not content.

use serde_json::{Value, json};
use stella_rain_core::expertise::{Expertise, StyleKind};
use stella_rain_core::validate::Error;
use stella_rain_core::validate::limits::MAX_STUN_TICKS;
use stella_rain_core::version::Versions;

use crate::common;

const PATH: &str = "$.weapons[0].styles[0].kinds[0]";

/// The Hammer's whole breaker (test values).
fn breaker() -> Value {
    json!({
        "type": "breaker",
        "effects": [
            { "type": "stagger_pct", "pct": 200 },
            { "type": "poise_pct", "pct": 200 },
            { "type": "part_break_pct", "pct": 200 },
            { "type": "on_stagger_burst", "radius_px": 128, "effects": [
                { "type": "damage_pct", "pct": 50 },
                { "type": "stun_ticks", "ticks": 45 },
                { "type": "knock_down" }
            ] }
        ]
    })
}

/// A breaker kind holding `effects`.
fn with(effects: Value) -> Value {
    json!({ "type": "breaker", "effects": effects })
}

/// A breaker kind holding one effect.
fn one(effect: Value) -> Value {
    with(json!([effect]))
}

/// A breaker kind whose burst holds `effects` (radius 64).
fn burst(effects: Value) -> Value {
    one(json!({ "type": "on_stagger_burst", "radius_px": 64, "effects": effects }))
}

fn file() -> Value {
    serde_json::from_slice(common::bundled_file("expertise")).unwrap()
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

/// S1: every number loads at its bounds and is refused one past them, at its own path.
#[test]
fn the_breaker_kind_is_bounded() {
    valid(json!([breaker()]));
    for key in ["stagger_pct", "poise_pct", "part_break_pct"] {
        for pct in [101, 1_000] {
            valid(json!([one(json!({ "type": key, "pct": pct }))]));
        }
        for pct in [100, 1_001] {
            refused_at(
                json!([one(json!({ "type": key, "pct": pct }))]),
                &format!("{PATH}.effects[0].pct"),
            );
        }
    }
    let at = format!("{PATH}.effects[0]");
    for radius in [1, 240] {
        valid(json!([one(json!({
            "type": "on_stagger_burst", "radius_px": radius,
            "effects": [{ "type": "knock_down" }]
        }))]));
    }
    for radius in [0, 241] {
        refused_at(
            json!([one(json!({
                "type": "on_stagger_burst", "radius_px": radius,
                "effects": [{ "type": "knock_down" }]
            }))]),
            &format!("{at}.radius_px"),
        );
    }
    for pct in [1, 1_000] {
        valid(json!([burst(
            json!([{ "type": "damage_pct", "pct": pct }])
        )]));
    }
    for pct in [0, 1_001] {
        refused_at(
            json!([burst(json!([{ "type": "damage_pct", "pct": pct }]))]),
            &format!("{at}.effects[0].pct"),
        );
    }
    for ticks in [1, i64::from(MAX_STUN_TICKS)] {
        valid(json!([burst(
            json!([{ "type": "stun_ticks", "ticks": ticks }])
        )]));
    }
    for ticks in [0, i64::from(MAX_STUN_TICKS) + 1] {
        refused_at(
            json!([burst(json!([{ "type": "stun_ticks", "ticks": ticks }]))]),
            &format!("{at}.effects[0].ticks"),
        );
    }
    refused_at(json!([with(json!([]))]), &format!("{PATH}.effects"));
    let five = json!([
        { "type": "stagger_pct", "pct": 101 },
        { "type": "poise_pct", "pct": 101 },
        { "type": "part_break_pct", "pct": 101 },
        { "type": "on_stagger_burst", "radius_px": 1, "effects": [{ "type": "knock_down" }] },
        { "type": "stagger_pct", "pct": 101 }
    ]);
    refused_at(json!([with(five)]), &format!("{PATH}.effects"));
    refused_at(json!([burst(json!([]))]), &format!("{at}.effects"));
    let four = json!([
        { "type": "damage_pct", "pct": 1 },
        { "type": "stun_ticks", "ticks": 1 },
        { "type": "knock_down" },
        { "type": "knock_down" }
    ]);
    refused_at(json!([burst(four)]), &format!("{at}.effects"));
}

/// S1: `effects` and `radius_px` are required, an unknown effect type and an unknown key are
/// parse errors.
#[test]
fn the_breaker_kind_is_strict() {
    let mut t = breaker();
    t.as_object_mut().unwrap().remove("effects");
    parse_error(json!([t]));
    parse_error(json!([one(json!({
        "type": "on_stagger_burst", "effects": [{ "type": "knock_down" }]
    }))]));
    parse_error(json!([one(json!({ "type": "luck_pct", "pct": 5 }))]));
    parse_error(json!([burst(json!([{ "type": "heal_pct", "pct": 5 }]))]));
    let mut t = breaker();
    t["extra"] = json!(1);
    parse_error(json!([t]));
    parse_error(json!([one(
        json!({ "type": "stagger_pct", "pct": 150, "extra": 1 })
    )]));
    parse_error(json!([one(json!({ "type": "stagger_pct" }))]));
    parse_error(json!([burst(
        json!([{ "type": "knock_down", "extra": 1 }])
    )]));
}

/// S1: one effect of a variant per kind and per burst, one breaker kind per level, all refused
/// at the later one; a level of five kinds is over the level's bound.
#[test]
fn a_kind_names_one_effect_per_variant_and_a_level_one_breaker() {
    refused_at(
        json!([with(json!([
            { "type": "stagger_pct", "pct": 150 },
            { "type": "stagger_pct", "pct": 160 }
        ]))]),
        &format!("{PATH}.effects[1]"),
    );
    refused_at(
        json!([burst(json!([
            { "type": "stun_ticks", "ticks": 5 },
            { "type": "stun_ticks", "ticks": 6 }
        ]))]),
        &format!("{PATH}.effects[0].effects[1]"),
    );
    refused_at(
        json!([breaker(), breaker()]),
        "$.weapons[0].styles[0].kinds[1]",
    );
    let five = json!([breaker(), breaker(), breaker(), breaker(), breaker()]);
    refused_at(five, "$.weapons[0].styles[0].kinds");
}

/// S1: the kind needs `sim_version` 2, as every kind does.
#[test]
fn the_breaker_kind_needs_sim_version_2() {
    let old = Versions {
        sim_version: 1,
        ..Versions::CURRENT
    };
    match parse_at(&with_kinds(json!([breaker()])), old) {
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

/// S2: a path may name the breaker at the style's own level and at later levels; an effect a
/// later level names again is allowed (it replaces the earlier one).
#[test]
fn a_path_may_name_the_breaker_at_several_levels() {
    let mut v = with_kinds(json!([one(json!({ "type": "stagger_pct", "pct": 200 }))]));
    v["weapons"][0]["styles"][0]["upgrade_1"] = json!([{
        "id": "second", "effect": "none",
        "kinds": [one(json!({ "type": "stagger_pct", "pct": 300 }))]
    }]);
    v["weapons"][0]["styles"][0]["capstone"] = json!([{
        "id": "third", "effect": "none", "kinds": [breaker()]
    }]);
    if let Err(e) = parse_at(&v, Versions::CURRENT) {
        panic!("several levels of one breaker: {e:?}");
    }
    // A level of two breaker kinds is still refused (the test above), and so is an unbounded
    // number inside a later level.
    v["weapons"][0]["styles"][0]["capstone"][0]["kinds"] =
        json!([one(json!({ "type": "poise_pct", "pct": 100 }))]);
    match parse_at(&v, Versions::CURRENT) {
        Err(Error::Invalid(issues)) => assert!(
            issues
                .iter()
                .any(|i| i.path == "$.weapons[0].styles[0].capstone[0].kinds[0].effects[0].pct"),
            "{issues:?}"
        ),
        other => panic!("expected invalid, got {other:?}"),
    }
    // The bundled file still loads and names none.
    assert!(parse_at(&file(), Versions::CURRENT).is_ok());
}

/// S3: the bundled pack names the breaker kind, and the kind's JSON serializes back to itself.
#[test]
fn the_kind_round_trips_and_the_bundled_file_names_it() {
    let bundled = std::str::from_utf8(common::bundled_file("expertise")).unwrap();
    assert!(bundled.contains("breaker"));
    let kind: StyleKind = serde_json::from_value(breaker()).expect("the kind parses");
    assert_eq!(serde_json::to_value(&kind).unwrap(), breaker());
}

/// S4: the views carry the kind compactly (0 and no `burst` where it names none) and the
/// bundled pack's views carry it.
#[test]
fn the_views_carry_the_breaker_kind() {
    let build = |kinds: Value| {
        let mut file = file();
        file["thresholds"] = json!([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
        let w = file["weapons"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|w| w["weapon"] == "giant_hammer")
            .unwrap();
        w["base"] = json!([]);
        for s in w["styles"].as_array_mut().unwrap() {
            s["stats"] = json!([]);
        }
        w["styles"][0]["kinds"] = kinds;
        let expertise = Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT)
            .expect("the expertise");
        let snap = crate::style_grants::build(expertise).snapshot();
        let ready = serde_json::to_value(snap.players[0].tree_ready.as_ref().unwrap()).unwrap();
        let stats = serde_json::to_value(snap.players[0].weapon_stats.as_ref().unwrap()).unwrap();
        (
            ready["kinds"][0].clone(),
            stats["trees"][0]["grants"][0]["kinds"].clone(),
        )
    };
    let want = json!({
        "type": "breaker", "stagger_pct": 200, "poise_pct": 200, "part_break_pct": 200,
        "burst": { "radius_px": 128, "damage_pct": 50, "stun_ticks": 45, "knock_down": true }
    });
    let (ready, grants) = build(json!([breaker()]));
    assert_eq!(ready, json!([want.clone()]));
    assert_eq!(grants, json!([want]));
    let bare = json!({
        "type": "breaker", "stagger_pct": 0, "poise_pct": 0, "part_break_pct": 0
    });
    let (ready, _) = build(json!([one(
        json!({ "type": "on_stagger_burst", "radius_px": 1,
        "effects": [{ "type": "damage_pct", "pct": 1 }] })
    )]));
    assert_eq!(
        ready[0]["burst"],
        json!({ "radius_px": 1, "damage_pct": 1, "stun_ticks": 0, "knock_down": false })
    );
    let (ready, _) = build(json!([one(json!({ "type": "poise_pct", "pct": 150 }))]));
    let mut want = bare;
    want["poise_pct"] = json!(150);
    assert_eq!(ready, json!([want]));
    let bundled = crate::style_grants::build(Expertise::bundled().clone()).snapshot();
    let ready = serde_json::to_string(bundled.players[0].tree_ready.as_ref().unwrap()).unwrap();
    assert!(ready.contains("breaker"));
}

/// S6: the stagger burst uses the event enum's tag form (`type` and `data`, the player by name).
#[test]
fn the_stagger_burst_event_serializes() {
    use stella_rain_core::event::DomainEvent;
    use stella_rain_core::fixed::{Fx, Point};
    use stella_rain_core::id::{EntityId, PlayerIdx};

    let event = DomainEvent::StaggerBurst {
        player: PlayerIdx::SOLO,
        target: EntityId(7),
        at: Point {
            x: Fx::px(3),
            y: Fx::px(4),
        },
        radius: 32_768,
    };
    assert_eq!(
        serde_json::to_value(event).unwrap(),
        json!({ "type": "StaggerBurst", "data": {
            "player": "first", "target": 7,
            "at": { "x": 768, "y": 1024 }, "radius": 32768
        } })
    );
}
