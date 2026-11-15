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
    let four = json!([
        { "type": "speed_pct", "pct": 4 },
        { "type": "keep_on_smash" },
        { "type": "super_armour_at_full" },
        { "type": "keep_on_smash" }
    ]);
    // Four entries are within the bound; the fourth is a duplicate, refused at its own path.
    refused_at(json!([effects(four)]), &format!("{PATH}.effects[3]"));
    let five = json!([
        { "type": "speed_pct", "pct": 4 },
        { "type": "keep_on_smash" },
        { "type": "super_armour_at_full" },
        { "type": "range_pct_at_full", "pct": 25 },
        { "type": "keep_on_smash" }
    ]);
    refused_at(json!([effects(five)]), &format!("{PATH}.effects"));
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

/// S3: the bundled pack names the tempo kind, and the kind's JSON serializes back to itself.
#[test]
fn the_kind_round_trips_and_the_bundled_file_names_it() {
    let bundled = std::str::from_utf8(common::bundled_file("expertise")).unwrap();
    assert!(bundled.contains("combo_tempo_pct"));
    let kind: StyleKind = serde_json::from_value(tempo()).expect("the kind parses");
    assert_eq!(serde_json::to_value(&kind).unwrap(), tempo());
}

/// S4: the views carry the kind compactly and the bundled pack's views carry it.
#[test]
fn the_views_carry_the_tempo_kind() {
    let mut file: Value = serde_json::from_slice(crate::before::EXPERTISE).unwrap();
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
    let want = json!({ "type": "combo_tempo_pct", "max_stacks": 5, "pct_per_stack": 4, "armour_at_full": false, "range_pct_at_full": 0 });
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

/// V3: a later level of the path may name the combo tempo again, with no effect when an earlier
/// level named it; `max_stacks` never falls and the folded product stays within the ceiling.
#[test]
fn a_later_level_may_name_the_combo_tempo_again() {
    let base = json!([{
        "type": "combo_tempo_pct", "max_stacks": 5,
        "effects": [{ "type": "speed_pct", "pct": 4 }]
    }]);
    let later = |max: i64, effects: Value| json!([{ "type": "combo_tempo_pct", "max_stacks": max, "effects": effects }]);
    stacked_loads(base.clone(), later(6, json!([])));
    stacked_refused(
        base.clone(),
        later(4, json!([])),
        &format!("{LATER}.max_stacks"),
    );
    stacked_refused(
        json!([speed(5, 90)]),
        later(10, json!([])),
        &format!("{LATER}.max_stacks"),
    );
    stacked_refused(
        base.clone(),
        later(5, json!([{ "type": "speed_pct", "pct": 0 }])),
        &format!("{LATER}.effects[0].pct"),
    );
    stacked_refused(json!([]), later(5, json!([])), &format!("{LATER}.effects"));
    stacked_refused(
        base,
        later(
            5,
            json!([{ "type": "keep_on_smash" }, { "type": "keep_on_smash" }]),
        ),
        &format!("{LATER}.effects[1]"),
    );
}

/// The four different effects of the kind.
fn all_four() -> Value {
    json!([
        { "type": "speed_pct", "pct": 4 },
        { "type": "keep_on_smash" },
        { "type": "super_armour_at_full" },
        { "type": "range_pct_at_full", "pct": 25 }
    ])
}

/// V1: the kind with `range_pct_at_full` loads under the current versions, serialises back to
/// its input and is refused under `sim_version` 1.
#[test]
fn range_pct_at_full_loads_and_round_trips() {
    let kind = effects(all_four());
    valid(json!([kind.clone()]));
    let parsed: StyleKind = serde_json::from_value(kind.clone()).expect("the kind parses");
    assert_eq!(serde_json::to_value(&parsed).unwrap(), kind);
    let old = Versions {
        sim_version: 1,
        ..Versions::CURRENT
    };
    match parse_at(&with_kinds(json!([kind])), old) {
        Err(Error::Invalid(issues)) => assert!(
            issues
                .iter()
                .any(|i| i.path == "$.weapons[0].styles[0].kinds"),
            "{issues:?}"
        ),
        other => panic!("{other:?}"),
    }
}

/// V2: `pct` is 1 to 100; `pct` is required and the effect is strict.
#[test]
fn range_pct_at_full_is_bounded_and_strict() {
    let with = |pct: i64| {
        json!([effects(json!([
            { "type": "speed_pct", "pct": 4 },
            { "type": "range_pct_at_full", "pct": pct }
        ]))])
    };
    valid(with(1));
    valid(with(100));
    for pct in [0, 101] {
        refused_at(with(pct), &format!("{PATH}.effects[1].pct"));
    }
    parse_error(json!([effects(json!([
        { "type": "speed_pct", "pct": 4 },
        { "type": "range_pct_at_full" }
    ]))]));
    parse_error(json!([effects(json!([
        { "type": "speed_pct", "pct": 4 },
        { "type": "range_pct_at_full", "pct": 25, "extra": 1 }
    ]))]));
}

/// V3: four different effects load, a repeated one is refused at its own path.
#[test]
fn a_kind_may_name_four_effects() {
    valid(json!([effects(all_four())]));
    refused_at(
        json!([effects(json!([
            { "type": "speed_pct", "pct": 4 },
            { "type": "range_pct_at_full", "pct": 25 },
            { "type": "range_pct_at_full", "pct": 30 }
        ]))]),
        &format!("{PATH}.effects[2]"),
    );
}

/// V3b: the speed checks still run at four effects.
#[test]
fn the_speed_checks_run_at_four_effects() {
    let first = json!([{
        "type": "combo_tempo_pct", "max_stacks": 10,
        "effects": [
            { "type": "speed_pct", "pct": 81 },
            { "type": "keep_on_smash" },
            { "type": "super_armour_at_full" },
            { "type": "range_pct_at_full", "pct": 25 }
        ]
    }]);
    match parse_at(&with_kinds(first), Versions::CURRENT) {
        Err(Error::Invalid(issues)) => assert!(
            issues
                .iter()
                .any(|i| i.path == format!("{PATH}.effects") && i.message.contains("at most 800")),
            "{issues:?}"
        ),
        other => panic!("{other:?}"),
    }
    let no_speed = json!([effects(json!([
        { "type": "keep_on_smash" },
        { "type": "super_armour_at_full" },
        { "type": "range_pct_at_full", "pct": 25 },
        { "type": "keep_on_smash" }
    ]))]);
    match parse_at(&with_kinds(no_speed), Versions::CURRENT) {
        Err(Error::Invalid(issues)) => {
            assert!(
                issues
                    .iter()
                    .any(|i| i.path == format!("{PATH}.effects[3]")),
                "{issues:?}"
            );
            assert!(
                issues.iter().any(|i| i.path == format!("{PATH}.effects")
                    && i.message.contains("names no speed_pct")),
                "{issues:?}"
            );
        }
        other => panic!("{other:?}"),
    }
}

/// V4: a later level lists only the range, or nothing; its bounds hold there too.
#[test]
fn a_later_level_may_name_range_pct_at_full() {
    let base = json!([{
        "type": "combo_tempo_pct", "max_stacks": 5,
        "effects": [{ "type": "speed_pct", "pct": 4 }]
    }]);
    let later = |max: i64, effects: Value| json!([{ "type": "combo_tempo_pct", "max_stacks": max, "effects": effects }]);
    let range = |pct: i64| json!([{ "type": "range_pct_at_full", "pct": pct }]);
    stacked_loads(base.clone(), later(5, range(25)));
    stacked_loads(base.clone(), later(5, json!([])));
    stacked_loads(
        base.clone(),
        later(
            5,
            json!([{ "type": "speed_pct", "pct": 4 }, { "type": "range_pct_at_full", "pct": 25 }]),
        ),
    );
    stacked_refused(
        base.clone(),
        later(5, range(101)),
        &format!("{LATER}.effects[0].pct"),
    );
    stacked_refused(base, later(4, range(25)), &format!("{LATER}.max_stacks"));
    // A first naming with only the range has no speed.
    valid_refused_no_speed();
}

fn valid_refused_no_speed() {
    refused_at(
        json!([effects(json!([{ "type": "range_pct_at_full", "pct": 25 }]))]),
        &format!("{PATH}.effects"),
    );
}
