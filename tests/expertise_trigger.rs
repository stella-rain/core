//! The giant trigger kind of a style level (ADR-SIM-9 points 2 to 7, ADR-SIM-5, ADR-CONT-3): its
//! numbers are bounded by the validator, the kind is `sim_version` 2's, one per style path, a
//! style naming it also names a full-smash gauge, and it round-trips through JSON. Test code
//! only: the numbers are test values, not content.

use serde_json::{Value, json};
use stella_rain_core::expertise::{Expertise, StyleKind};
use stella_rain_core::pack::{Pack, PackError};
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

use crate::common;

const AT: &str = "/weapons/0/styles/0/kinds";
const PATH: &str = "$.weapons[0].styles[0].kinds[0]";

/// The Hammer's trigger as the design table gives it (test values).
fn trigger() -> Value {
    json!({
        "type": "giant_trigger", "ticks": 480,
        "effects": [
            { "type": "range_pct", "pct": 40 },
            { "type": "smash_pct", "pct": 20 },
            { "type": "super_armour" }
        ]
    })
}

/// A trigger kind of `ticks` with `effects`.
fn with(ticks: i64, effects: Value) -> Value {
    json!({ "type": "giant_trigger", "ticks": ticks, "effects": effects })
}

/// A trigger kind holding one effect.
fn one(effect: Value) -> Value {
    with(480, json!([effect]))
}

/// The styles from before the weapons' content moved (`crate::before`): the tests patch style 0.
fn file() -> Value {
    serde_json::from_slice(crate::before::EXPERTISE).unwrap()
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
fn the_trigger_kind_is_bounded() {
    valid(json!([trigger()]));
    let max_range = i64::from(stella_rain_core::rewards::MAX_RANGE_PCT);
    for ticks in [1, 3_600] {
        valid(json!([with(ticks, json!([{ "type": "super_armour" }]))]));
    }
    for ticks in [0, 3_601] {
        refused_at(
            json!([with(ticks, json!([{ "type": "super_armour" }]))]),
            &format!("{PATH}.ticks"),
        );
    }
    for pct in [1, max_range] {
        valid(json!([one(json!({ "type": "range_pct", "pct": pct }))]));
    }
    for pct in [0, max_range + 1] {
        refused_at(
            json!([one(json!({ "type": "range_pct", "pct": pct }))]),
            &format!("{PATH}.effects[0].pct"),
        );
    }
    for pct in [1, 1_000] {
        valid(json!([one(json!({ "type": "smash_pct", "pct": pct }))]));
    }
    for pct in [0, 1_001] {
        refused_at(
            json!([one(json!({ "type": "smash_pct", "pct": pct }))]),
            &format!("{PATH}.effects[0].pct"),
        );
    }
    for pct in [101, 1_000] {
        valid(json!([one(json!({ "type": "rally_pct", "pct": pct }))]));
    }
    for pct in [100, 1_001] {
        refused_at(
            json!([one(json!({ "type": "rally_pct", "pct": pct }))]),
            &format!("{PATH}.effects[0].pct"),
        );
    }
    refused_at(json!([with(480, json!([]))]), &format!("{PATH}.effects"));
    let five = json!([
        { "type": "range_pct", "pct": 1 },
        { "type": "smash_pct", "pct": 1 },
        { "type": "super_armour" },
        { "type": "rally_pct", "pct": 101 },
        { "type": "super_armour" }
    ]);
    refused_at(json!([with(480, five)]), &format!("{PATH}.effects"));
}

/// S1: `ticks` and `effects` are required, an unknown effect type and an unknown key are parse
/// errors.
#[test]
fn the_trigger_kind_is_strict() {
    for key in ["ticks", "effects"] {
        let mut t = trigger();
        t.as_object_mut().unwrap().remove(key);
        parse_error(json!([t]));
    }
    parse_error(json!([one(json!({ "type": "luck_pct", "pct": 5 }))]));
    let mut t = trigger();
    t["extra"] = json!(1);
    parse_error(json!([t]));
    parse_error(json!([one(
        json!({ "type": "range_pct", "pct": 5, "extra": 1 })
    )]));
    parse_error(json!([one(json!({ "type": "super_armour", "extra": 1 }))]));
}

/// S1: one effect of a variant per kind, one trigger kind per style path (inside a level and
/// across levels), both refused at the later one; a level of five kinds is over the level's bound.
#[test]
fn a_kind_names_one_effect_per_variant_and_a_path_one_trigger() {
    refused_at(
        json!([with(
            480,
            json!([
                { "type": "range_pct", "pct": 4 },
                { "type": "range_pct", "pct": 5 }
            ])
        )]),
        &format!("{PATH}.effects[1]"),
    );
    refused_at(
        json!([trigger(), trigger()]),
        "$.weapons[0].styles[0].kinds[1]",
    );
    // Two styles may each name one.
    let mut v = with_kinds(json!([trigger()]));
    v["weapons"][0]["styles"][1]["kinds"] = json!([trigger()]);
    assert!(parse_at(&v, Versions::CURRENT).is_ok());
    let five = json!([trigger(), trigger(), trigger(), trigger(), trigger()]);
    refused_at(five, "$.weapons[0].styles[0].kinds");
}

/// S1: the kind needs `sim_version` 2, as every kind does.
#[test]
fn the_trigger_kind_needs_sim_version_2() {
    let old = Versions {
        sim_version: 1,
        ..Versions::CURRENT
    };
    match parse_at(&with_kinds(json!([trigger()])), old) {
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

fn gauge() -> Value {
    json!({
        "type": "gauge", "max": 100, "per_step": 10, "per_hit": 2, "step_cap": 14,
        "spend": { "type": "full_smash", "attack": { "preset": { "id": "slash" } } }
    })
}

fn hammer_pack(kinds: Vec<Value>) -> Value {
    let mut v = crate::before::pack_value();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_hammer")
        .expect("the Hammer's expertise");
    w["styles"][0]["kinds"] = json!(kinds);
    v
}

/// S2: a style naming the trigger needs a full-smash gauge on its path, refused at the kind
/// ("nothing starts it"); a gauge at a later upgrade is enough; the bundled pack still loads.
#[test]
fn a_trigger_needs_a_full_smash_gauge_on_its_path() {
    match Pack::from_bytes(&serde_json::to_vec(&hammer_pack(vec![trigger()])).unwrap()) {
        Err(PackError::File {
            file: "expertise",
            error,
        }) => {
            let issues = format!("{error:?}");
            assert!(
                issues.contains("$.weapons[0].styles[0].kinds[0]")
                    && issues.contains("nothing starts it: the style names no full-smash gauge"),
                "{issues}"
            );
        }
        other => panic!("expected a refusal, got {:?}", other.map(|_| ())),
    }
    let both = hammer_pack(vec![trigger(), gauge()]);
    let loaded = Pack::from_bytes(&serde_json::to_vec(&both).unwrap());
    assert!(loaded.is_ok(), "{:?}", loaded.err());
    let mut later = hammer_pack(vec![trigger()]);
    later["expertise"]["weapons"][0]["styles"][0]["upgrade_2"][0]["kinds"] = json!([gauge()]);
    let loaded = Pack::from_bytes(&serde_json::to_vec(&later).unwrap());
    assert!(loaded.is_ok(), "{:?}", loaded.err());
    assert!(Pack::from_bytes(Pack::BUNDLED).is_ok());
}

/// S3: the bundled pack names the trigger kind, and the kind's JSON serializes back to itself.
#[test]
fn the_kind_round_trips_and_the_bundled_file_names_it() {
    let bundled = std::str::from_utf8(common::bundled_file("expertise")).unwrap();
    assert!(bundled.contains("giant_trigger"));
    let kind: StyleKind = serde_json::from_value(trigger()).expect("the kind parses");
    assert_eq!(serde_json::to_value(&kind).unwrap(), trigger());
}

/// S4: the views carry the kind compactly and the bundled pack's views carry it.
#[test]
fn the_views_carry_the_trigger_kind() {
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
    w["styles"][0]["kinds"] = json!([trigger()]);
    let expertise =
        Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the expertise");
    let snap = crate::style_grants::build(expertise).snapshot();
    let want = json!({
        "type": "giant_trigger", "ticks": 480, "range_pct": 40, "smash_pct": 20,
        "rally_pct": 0, "super_armour": true
    });
    let ready = serde_json::to_value(snap.players[0].tree_ready.as_ref().unwrap()).unwrap();
    assert_eq!(ready["kinds"][0], json!([want]));
    let stats = serde_json::to_value(snap.players[0].weapon_stats.as_ref().unwrap()).unwrap();
    assert_eq!(stats["trees"][0]["grants"][0]["kinds"], json!([want]));
    let bundled = crate::style_grants::build(Expertise::bundled().clone()).snapshot();
    let ready = serde_json::to_string(bundled.players[0].tree_ready.as_ref().unwrap()).unwrap();
    assert!(ready.contains("giant_trigger"));
}
/// S6: a `PlayerView` with `trigger: None, rally: None` serializes without either key, and the
/// five events use the event enum's tag form (`type` and `data`, the player by name).
#[test]
fn the_views_and_the_events_serialize() {
    use stella_rain_core::event::DomainEvent;
    use stella_rain_core::id::PlayerIdx;
    use stella_rain_core::snapshot::{RallyView, TriggerView};

    let view = TriggerView {
        ticks_left: 200,
        ticks: 480,
    };
    assert_eq!(
        serde_json::to_value(view).unwrap(),
        json!({ "ticks_left": 200, "ticks": 480 })
    );
    let rally = RallyView {
        hp: 50,
        ticks_left: 90,
    };
    assert_eq!(
        serde_json::to_value(rally).unwrap(),
        json!({ "hp": 50, "ticks_left": 90 })
    );
    let p = PlayerIdx::SOLO;
    let cases = [
        (
            DomainEvent::TriggerStarted {
                player: p,
                ticks: 480,
            },
            json!({ "type": "TriggerStarted", "data": { "player": "first", "ticks": 480 } }),
        ),
        (
            DomainEvent::TriggerEnded { player: p },
            json!({ "type": "TriggerEnded", "data": { "player": "first" } }),
        ),
        (
            DomainEvent::RallyGained {
                player: p,
                amount: 50,
                rally: 50,
            },
            json!({ "type": "RallyGained", "data": { "player": "first", "amount": 50, "rally": 50 } }),
        ),
        (
            DomainEvent::RallyWon {
                player: p,
                amount: 20,
                rally: 30,
            },
            json!({ "type": "RallyWon", "data": { "player": "first", "amount": 20, "rally": 30 } }),
        ),
        (
            DomainEvent::RallyLapsed { player: p },
            json!({ "type": "RallyLapsed", "data": { "player": "first" } }),
        ),
    ];
    for (event, want) in cases {
        assert_eq!(serde_json::to_value(event).unwrap(), want);
    }
    let snap = crate::style_grants::build(Expertise::bundled().clone()).snapshot();
    let player = serde_json::to_value(&snap.players[0]).unwrap();
    assert!(player.get("trigger").is_none(), "{player}");
    assert!(player.get("rally").is_none(), "{player}");
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

/// V4: a later level of the path may name the giant trigger again, with no effect when an
/// earlier level named it; the first naming keeps 1 to 4 effects; each number is bounded at its level.
#[test]
fn a_later_level_may_name_the_giant_trigger_again() {
    let base = json!([with(480, json!([{ "type": "range_pct", "pct": 40 }]))]);
    let later = |ticks: i64, effects: Value| json!([with(ticks, effects)]);
    stacked_loads(base.clone(), later(720, json!([])));
    stacked_refused(
        base.clone(),
        later(3601, json!([])),
        &format!("{LATER}.ticks"),
    );
    refused_at(json!([with(480, json!([]))]), &format!("{PATH}.effects"));
    stacked_refused(
        json!([]),
        later(720, json!([])),
        &format!("{LATER}.effects"),
    );
}
