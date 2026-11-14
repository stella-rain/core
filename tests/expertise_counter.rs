//! The counter bonus kind of a style level (ADR-SIM-9 points 2 to 7, ADR-SIM-5, ADR-CONT-3): its
//! numbers are bounded by the validator, the kind is `sim_version` 2's, one per style path, and
//! it round-trips through JSON. Test code only: the numbers are test values, not content.

use serde_json::{Value, json};
use stella_rain_core::expertise::{Expertise, StyleKind};
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

use crate::common;

const AT: &str = "/weapons/0/styles/0/kinds";
const PATH: &str = "$.weapons[0].styles[0].kinds[0]";

/// The Sword's counter as the design table gives it (test values).
fn counter() -> Value {
    json!({
        "type": "counter_bonus", "window_ticks": 30,
        "effects": [
            { "type": "damage_pct", "pct": 100 },
            { "type": "stagger_pct", "pct": 300 },
            { "type": "stun_ticks", "ticks": 45 },
            { "type": "advance", "distance": 12288 }
        ]
    })
}

fn counter_with(key: &str, value: Value) -> Value {
    let mut c = counter();
    c[key] = value;
    c
}

/// A counter bonus holding `effects`.
fn effects(effects: Value) -> Value {
    json!({ "type": "counter_bonus", "window_ticks": 30, "effects": effects })
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
fn the_counter_kind_is_bounded() {
    valid(json!([counter()]));
    for ticks in [1, 60] {
        valid(json!([counter_with("window_ticks", json!(ticks))]));
    }
    for ticks in [0, 61] {
        refused_at(
            json!([counter_with("window_ticks", json!(ticks))]),
            &format!("{PATH}.window_ticks"),
        );
    }
    let bound = |effect: Value, key: &str, lo: i64, hi: i64, under: i64, over: i64| {
        let with = |v: i64| {
            let mut e = effect.clone();
            e[key] = json!(v);
            json!([effects(json!([e]))])
        };
        valid(with(lo));
        valid(with(hi));
        let at = format!("{PATH}.effects[0].{key}");
        refused_at(with(under), &at);
        refused_at(with(over), &at);
    };
    bound(json!({ "type": "damage_pct" }), "pct", 1, 1_000, 0, 1_001);
    bound(
        json!({ "type": "stagger_pct" }),
        "pct",
        101,
        1_000,
        100,
        1_001,
    );
    bound(json!({ "type": "stun_ticks" }), "ticks", 1, 120, 0, 121);
    bound(
        json!({ "type": "advance" }),
        "distance",
        256,
        240 * 256,
        0,
        240 * 256 + 1,
    );
    refused_at(json!([effects(json!([]))]), &format!("{PATH}.effects"));
    let five = json!([
        { "type": "damage_pct", "pct": 5 },
        { "type": "stagger_pct", "pct": 150 },
        { "type": "stun_ticks", "ticks": 5 },
        { "type": "advance", "distance": 256 },
        { "type": "damage_pct", "pct": 6 }
    ]);
    refused_at(json!([effects(five)]), &format!("{PATH}.effects"));
}

/// S1: `window_ticks` and `effects` are required, an unknown effect type and an unknown key are
/// parse errors.
#[test]
fn the_counter_kind_is_strict() {
    for key in ["window_ticks", "effects"] {
        let mut c = counter();
        c.as_object_mut().unwrap().remove(key);
        parse_error(json!([c]));
    }
    parse_error(json!([effects(json!([{ "type": "luck_pct", "pct": 5 }]))]));
    parse_error(json!([counter_with("extra", json!(1))]));
    parse_error(json!([effects(
        json!([{ "type": "damage_pct", "pct": 5, "extra": 1 }])
    )]));
}

/// S1: one effect of a variant per kind, one counter bonus per style path (inside a level and
/// across levels), both refused at the later one; a level of five kinds is over the level's bound.
#[test]
fn a_kind_names_one_effect_per_variant_and_a_path_one_counter() {
    refused_at(
        json!([effects(json!([
            { "type": "damage_pct", "pct": 5 },
            { "type": "damage_pct", "pct": 6 }
        ]))]),
        &format!("{PATH}.effects[1]"),
    );
    refused_at(
        json!([counter(), counter()]),
        "$.weapons[0].styles[0].kinds[1]",
    );
    // Two styles may each name one.
    let mut v = with_kinds(json!([counter()]));
    v["weapons"][0]["styles"][1]["kinds"] = json!([counter()]);
    assert!(parse_at(&v, Versions::CURRENT).is_ok());
    let five = json!([counter(), counter(), counter(), counter(), counter()]);
    refused_at(five, "$.weapons[0].styles[0].kinds");
}

/// S1: the kind needs `sim_version` 2, as every kind does.
#[test]
fn the_counter_kind_needs_sim_version_2() {
    let old = Versions {
        sim_version: 1,
        ..Versions::CURRENT
    };
    match parse_at(&with_kinds(json!([counter()])), old) {
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

/// S2: a style naming the kind loads with no gauge, no spirit and no dash branch step on the
/// weapon: the validator checks no more.
#[test]
fn the_kind_needs_nothing_else_on_the_path() {
    valid(json!([counter()]));
}

/// S3: the bundled pack names the counter bonus kind, and the kind's JSON serializes back to itself.
#[test]
fn the_kind_round_trips_and_the_bundled_file_names_it() {
    let bundled = std::str::from_utf8(common::bundled_file("expertise")).unwrap();
    assert!(bundled.contains("counter_bonus"));
    let kind: StyleKind = serde_json::from_value(counter()).expect("the kind parses");
    assert_eq!(serde_json::to_value(&kind).unwrap(), counter());
}

/// S4: the views carry the kind compactly and the bundled pack's views carry it.
#[test]
fn the_views_carry_the_counter_kind() {
    let mut file: Value = serde_json::from_slice(common::bundled_file("expertise")).unwrap();
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
    w["styles"][0]["kinds"] = json!([counter()]);
    let expertise =
        Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the expertise");
    let snap = crate::style_grants::build(expertise).snapshot();
    let want = json!({ "type": "counter_bonus", "window_ticks": 30 });
    let ready = serde_json::to_value(snap.players[0].tree_ready.as_ref().unwrap()).unwrap();
    assert_eq!(ready["kinds"][0], json!([want]));
    let stats = serde_json::to_value(snap.players[0].weapon_stats.as_ref().unwrap()).unwrap();
    assert_eq!(stats["trees"][0]["grants"][0]["kinds"], json!([want]));
}

/// S6: a `PlayerView` with `counter: None` serializes without the key, and the two events use
/// the event enum's tag form (`type` and `data`, the player by name).
#[test]
fn the_view_and_the_events_serialize() {
    use stella_rain_core::event::DomainEvent;
    use stella_rain_core::id::PlayerIdx;
    use stella_rain_core::snapshot::CounterView;

    let view = CounterView {
        window_left: 12,
        window_ticks: 30,
        taken: true,
    };
    assert_eq!(
        serde_json::to_value(view).unwrap(),
        json!({ "window_left": 12, "window_ticks": 30, "taken": true })
    );
    let opened = DomainEvent::CounterWindowOpened {
        player: PlayerIdx::SOLO,
    };
    assert_eq!(
        serde_json::to_value(opened).unwrap(),
        json!({ "type": "CounterWindowOpened", "data": { "player": "first" } })
    );
    let taken = DomainEvent::CounterBonusTaken {
        player: PlayerIdx::SOLO,
    };
    assert_eq!(
        serde_json::to_value(taken).unwrap(),
        json!({ "type": "CounterBonusTaken", "data": { "player": "first" } })
    );
    let snap = crate::style_grants::build(Expertise::bundled().clone()).snapshot();
    let player = serde_json::to_value(&snap.players[0]).unwrap();
    assert!(player.get("counter").is_none(), "{player}");
}

const LATER: &str = "$.weapons[0].styles[0].upgrade_1[0].kinds[0]";

/// The style's own level names `style`, its first upgrade names `later`.
fn stacked(style: Value, later: Value) -> Value {
    let mut v = with_kinds(style);
    v["weapons"][0]["styles"][0]["upgrade_1"] = json!([{
        "id": "again", "effect": "none", "kinds": later
    }]);
    v
}

fn stacked_loads(style: Value, later: Value) {
    if let Err(e) = parse_at(&stacked(style, later), Versions::CURRENT) {
        panic!("expected valid, got {e:?}");
    }
}

fn stacked_refused(style: Value, later: Value, path: &str) {
    match parse_at(&stacked(style, later), Versions::CURRENT) {
        Err(Error::Invalid(issues)) => {
            assert!(issues.iter().any(|i| i.path == path), "{path}: {issues:?}")
        }
        other => panic!("{path}: expected invalid, got {other:?}"),
    }
}

/// V2: a later level of the path may name the counter bonus again, with no effect when an
/// earlier level named it; the first naming keeps 1 to 4 effects; each number is bounded at its level.
#[test]
fn a_later_level_may_name_the_counter_bonus_again() {
    let base = json!([{
        "type": "counter_bonus", "window_ticks": 30,
        "effects": [{ "type": "damage_pct", "pct": 100 }]
    }]);
    let later = |window: i64, effects: Value| json!([{ "type": "counter_bonus", "window_ticks": window, "effects": effects }]);
    stacked_loads(base.clone(), later(45, json!([])));
    stacked_loads(
        base.clone(),
        later(45, json!([{ "type": "advance", "distance": 12288 }])),
    );
    stacked_refused(
        base.clone(),
        later(61, json!([])),
        &format!("{LATER}.window_ticks"),
    );
    refused_at(
        json!([{ "type": "counter_bonus", "window_ticks": 30, "effects": [] }]),
        &format!("{PATH}.effects"),
    );
    stacked_refused(json!([]), later(45, json!([])), &format!("{LATER}.effects"));
}
