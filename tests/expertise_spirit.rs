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

/// S3: the bundled pack names the spirit kind, and the kind's JSON serializes back to itself.
#[test]
fn the_kind_round_trips_and_the_bundled_file_names_it() {
    let bundled = std::str::from_utf8(common::bundled_file("expertise")).unwrap();
    assert!(bundled.contains("\"spirit\""));
    let kind: StyleKind = serde_json::from_value(spirit()).expect("the kind parses");
    assert_eq!(serde_json::to_value(&kind).unwrap(), spirit());
}

/// S6: a `PlayerView` with `spirit: None` serializes without the key, and the two events use
/// the event enum's tag form (`type` and `data`, the player by name).
#[test]
fn the_view_and_the_events_serialize() {
    use stella_rain_core::event::DomainEvent;
    use stella_rain_core::id::PlayerIdx;
    use stella_rain_core::snapshot::SpiritView;

    let view = SpiritView {
        level: 1,
        max_level: 3,
        decay_left: 100,
        decay_ticks: 100,
    };
    assert_eq!(
        serde_json::to_value(view).unwrap(),
        json!({ "level": 1, "max_level": 3, "decay_left": 100, "decay_ticks": 100 })
    );
    let raised = DomainEvent::SpiritRaised {
        player: PlayerIdx::SOLO,
        level: 1,
    };
    assert_eq!(
        serde_json::to_value(raised).unwrap(),
        json!({ "type": "SpiritRaised", "data": { "player": "first", "level": 1 } })
    );
    let fell = DomainEvent::SpiritFell {
        player: PlayerIdx::SOLO,
        level: 0,
    };
    assert_eq!(
        serde_json::to_value(fell).unwrap(),
        json!({ "type": "SpiritFell", "data": { "player": "first", "level": 0 } })
    );
    let snap = crate::style_grants::build(Expertise::bundled().clone()).snapshot();
    let player = serde_json::to_value(&snap.players[0]).unwrap();
    assert!(player.get("spirit").is_none(), "{player}");
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

/// V1: a later level of the path may name the spirit again; each number is bounded at the level
/// that names it, at its own path; a level still names one spirit.
#[test]
fn a_later_level_may_name_the_spirit_again() {
    let later = |decay: i64, levels: Value| json!([{ "type": "spirit", "decay_ticks": decay, "levels": levels }]);
    stacked_loads(json!([spirit()]), later(1800, json!([[]])));
    stacked_refused(
        json!([spirit()]),
        later(3601, json!([[]])),
        &format!("{LATER}.decay_ticks"),
    );
    stacked_refused(
        json!([spirit()]),
        later(1800, json!([])),
        &format!("{LATER}.levels"),
    );
    stacked_refused(
        json!([spirit()]),
        later(1800, json!([[], [], [], []])),
        &format!("{LATER}.levels"),
    );
    stacked_refused(
        json!([spirit()]),
        json!([spirit(), spirit()]),
        "$.weapons[0].styles[0].upgrade_1[0].kinds[1]",
    );
}
