//! The combo tempo kind of a style level (ADR-SIM-9 points 2 to 7, ADR-SIM-5, ADR-CONT-3): its
//! numbers are bounded by the validator, the kind is `sim_version` 2's, one per style path, and
//! it round-trips through JSON. Test code only: the numbers are test values, not content.

use serde_json::{Value, json};
use stella_rain_core::expertise::{Expertise, StyleKind};
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

use crate::common;

const AT: &str = "/weapons/0/styles/0/kinds";
const PATH: &str = "$.weapons[0].styles[0].kinds[0]";

/// The Hammer's tempo as the design table gives it (test values).
fn tempo() -> Value {
    json!({
        "type": "combo_tempo_pct", "max_stacks": 5,
        "effects": [{ "type": "speed_pct", "pct": 4 }, { "type": "keep_on_smash" }]
    })
}

/// A tempo kind with `max_stacks` and one `speed_pct` effect of `pct`.
fn speed(max_stacks: i64, pct: i64) -> Value {
    json!({
        "type": "combo_tempo_pct", "max_stacks": max_stacks,
        "effects": [{ "type": "speed_pct", "pct": pct }]
    })
}

/// A tempo kind holding `effects`.
fn effects(effects: Value) -> Value {
    json!({ "type": "combo_tempo_pct", "max_stacks": 5, "effects": effects })
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
fn the_tempo_kind_is_bounded() {
    valid(json!([tempo()]));
    valid(json!([speed(1, 1)]));
    valid(json!([speed(1, 800)]));
    valid(json!([speed(10, 80)]));
    for stacks in [0, 11] {
        refused_at(json!([speed(stacks, 4)]), &format!("{PATH}.max_stacks"));
    }
    for pct in [0, 801] {
        refused_at(json!([speed(1, pct)]), &format!("{PATH}.effects[0].pct"));
    }
    // The product of the stacks and the percent is past the attack speed ceiling.
    refused_at(json!([speed(10, 81)]), &format!("{PATH}.effects"));
    refused_at(json!([effects(json!([]))]), &format!("{PATH}.effects"));
    let three = json!([
        { "type": "speed_pct", "pct": 4 },
        { "type": "keep_on_smash" },
        { "type": "keep_on_smash" }
    ]);
    refused_at(json!([effects(three)]), &format!("{PATH}.effects"));
    refused_at(
        json!([effects(json!([{ "type": "keep_on_smash" }]))]),
        &format!("{PATH}.effects"),
    );
}

/// S1: `max_stacks` and `effects` are required, an unknown effect type and an unknown key are
/// parse errors.
#[test]
fn the_tempo_kind_is_strict() {
    for key in ["max_stacks", "effects"] {
        let mut t = tempo();
        t.as_object_mut().unwrap().remove(key);
        parse_error(json!([t]));
    }
    parse_error(json!([effects(json!([{ "type": "luck_pct", "pct": 5 }]))]));
    let mut t = tempo();
    t["extra"] = json!(1);
    parse_error(json!([t]));
    parse_error(json!([effects(
        json!([{ "type": "speed_pct", "pct": 5, "extra": 1 }])
    )]));
    parse_error(json!([effects(
        json!([{ "type": "speed_pct", "pct": 5 }, { "type": "keep_on_smash", "extra": 1 }])
    )]));
}

/// S1: one effect of a variant per kind, one tempo kind per style path (inside a level and
/// across levels), both refused at the later one; a level of five kinds is over the level's bound.
#[test]
fn a_kind_names_one_effect_per_variant_and_a_path_one_tempo() {
    refused_at(
        json!([effects(json!([
            { "type": "speed_pct", "pct": 4 },
            { "type": "speed_pct", "pct": 5 }
        ]))]),
        &format!("{PATH}.effects[1]"),
    );
    refused_at(json!([tempo(), tempo()]), "$.weapons[0].styles[0].kinds[1]");
    let mut v = with_kinds(json!([tempo()]));
    v["weapons"][0]["styles"][0]["upgrade_1"] = json!([{
        "id": "again", "effect": "none", "kinds": [tempo()]
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
    let mut v = with_kinds(json!([tempo()]));
    v["weapons"][0]["styles"][1]["kinds"] = json!([tempo()]);
    assert!(parse_at(&v, Versions::CURRENT).is_ok());
    let five = json!([tempo(), tempo(), tempo(), tempo(), tempo()]);
    refused_at(five, "$.weapons[0].styles[0].kinds");
}

/// S1: the kind needs `sim_version` 2, as every kind does.
#[test]
fn the_tempo_kind_needs_sim_version_2() {
    let old = Versions {
        sim_version: 1,
        ..Versions::CURRENT
    };
    match parse_at(&with_kinds(json!([tempo()])), old) {
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

/// S2: a style naming the kind loads with no gauge, no spirit and no counter on the weapon: the
/// validator checks no more.
#[test]
fn the_kind_needs_nothing_else_on_the_path() {
    valid(json!([tempo()]));
}

/// S3: the bundled pack names no tempo kind, and the kind's JSON serializes back to itself.
#[test]
fn the_kind_round_trips_and_the_bundled_file_names_none() {
    let bundled = std::str::from_utf8(common::bundled_file("expertise")).unwrap();
    assert!(!bundled.contains("combo_tempo_pct"));
    let kind: StyleKind = serde_json::from_value(tempo()).expect("the kind parses");
    assert_eq!(serde_json::to_value(&kind).unwrap(), tempo());
}

/// S4: the views carry the kind compactly and the bundled pack names none.
#[test]
fn the_views_carry_the_tempo_kind() {
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
    w["styles"][0]["kinds"] = json!([tempo()]);
    let expertise =
        Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the expertise");
    let snap = crate::style_grants::build(expertise).snapshot();
    let want = json!({ "type": "combo_tempo_pct", "max_stacks": 5, "pct_per_stack": 4 });
    let ready = serde_json::to_value(snap.players[0].tree_ready.as_ref().unwrap()).unwrap();
    assert_eq!(ready["kinds"][0], json!([want]));
    let stats = serde_json::to_value(snap.players[0].weapon_stats.as_ref().unwrap()).unwrap();
    assert_eq!(stats["trees"][0]["grants"][0]["kinds"], json!([want]));
}

/// S6: a `PlayerView` with `tempo: None` serializes without the key, and the two events use the
/// event enum's tag form (`type` and `data`, the player by name).
#[test]
fn the_view_and_the_events_serialize() {
    use stella_rain_core::event::DomainEvent;
    use stella_rain_core::id::PlayerIdx;
    use stella_rain_core::snapshot::TempoView;

    let view = TempoView {
        stacks: 3,
        max_stacks: 5,
        speed_pct: 12,
    };
    assert_eq!(
        serde_json::to_value(view).unwrap(),
        json!({ "stacks": 3, "max_stacks": 5, "speed_pct": 12 })
    );
    let raised = DomainEvent::TempoRaised {
        player: PlayerIdx::SOLO,
        stacks: 3,
    };
    assert_eq!(
        serde_json::to_value(raised).unwrap(),
        json!({ "type": "TempoRaised", "data": { "player": "first", "stacks": 3 } })
    );
    let lost = DomainEvent::TempoLost {
        player: PlayerIdx::SOLO,
    };
    assert_eq!(
        serde_json::to_value(lost).unwrap(),
        json!({ "type": "TempoLost", "data": { "player": "first" } })
    );
    let snap = crate::style_grants::build(Expertise::bundled().clone()).snapshot();
    let player = serde_json::to_value(&snap.players[0]).unwrap();
    assert!(player.get("tempo").is_none(), "{player}");
}
