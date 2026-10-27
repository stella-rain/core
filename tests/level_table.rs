//! The mode's level table (ADR-106 §4): the level of the style choice, of the two style
//! upgrades and of the capstone; every other level is a common offer. A `time_limit` mode file
//! names one and a stage may; the validator checks the style comes first, the levels rise
//! strictly, there are exactly two upgrades, and every level is within bounds (ADR-020).

use crate::common;
use serde_json::{Value, json};
use stella_rain_core::stage::{LevelTable, MAX_STYLE_LEVEL, Stage};
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::{self, Error};
use stella_rain_core::version::Versions;

fn mode(table: Value) -> Result<TimeLimit, Error> {
    let mut v: Value = serde_json::from_slice(common::bundled_file("time_limit")).unwrap();
    v["level_table"] = table;
    TimeLimit::parse(v.to_string().as_bytes(), Versions::CURRENT)
}

fn mode_paths(table: Value) -> Vec<String> {
    match mode(table) {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not a validation error: {other:?}"),
    }
}

fn stage_with(table: Value) -> Result<(), Error> {
    let mut v: Value =
        serde_json::from_slice(stella_rain_core::run_setup::mode_file("trial").unwrap()).unwrap();
    v["level_table"] = table;
    let stage: Stage = serde_json::from_value(v).unwrap();
    validate::validate(&stage)
}

#[test]
fn the_bundled_modes_name_the_normal_table() {
    let normal = LevelTable {
        style: 3,
        upgrades: [6, 9],
        capstone: 10,
    };
    assert_eq!(LevelTable::NORMAL, normal);
    assert_eq!(*TimeLimit::bundled().level_table(), normal);
    let trial = stella_rain_core::run_content::RunContent::bundled().trial();
    assert_eq!(trial.level_table(), normal);
}

#[test]
fn the_short_table_validates() {
    let t = mode(json!({ "style": 2, "upgrades": [4, 6], "capstone": 8 })).unwrap();
    assert_eq!(*t.level_table(), LevelTable::SHORT);
    assert!(stage_with(json!({ "style": 2, "upgrades": [4, 6], "capstone": 8 })).is_ok());
}

#[test]
fn a_table_that_is_not_in_order_or_out_of_bounds_is_refused() {
    for table in [
        json!({ "style": 0, "upgrades": [4, 6], "capstone": 8 }),
        json!({ "style": 4, "upgrades": [4, 6], "capstone": 8 }),
        json!({ "style": 2, "upgrades": [6, 4], "capstone": 8 }),
        json!({ "style": 2, "upgrades": [4, 4], "capstone": 8 }),
        json!({ "style": 2, "upgrades": [4, 6], "capstone": 6 }),
        json!({ "style": 2, "upgrades": [4, 6], "capstone": MAX_STYLE_LEVEL + 1 }),
    ] {
        assert_eq!(mode_paths(table.clone()), ["$.level_table"], "{table}");
        assert!(stage_with(table).is_err());
    }
    assert!(mode(json!({ "style": 2, "upgrades": [4, 6], "capstone": MAX_STYLE_LEVEL })).is_ok());
}

#[test]
fn exactly_two_upgrades_and_no_unknown_field() {
    for table in [
        json!({ "style": 2, "upgrades": [4], "capstone": 8 }),
        json!({ "style": 2, "upgrades": [4, 6, 7], "capstone": 8 }),
        json!({ "style": 2, "upgrades": [4, 6], "capstone": 8, "extra": 1 }),
        json!({ "upgrades": [4, 6], "capstone": 8 }),
    ] {
        assert!(
            matches!(mode(table.clone()), Err(Error::Parse { .. })),
            "{table}"
        );
    }
}

#[test]
fn a_mode_file_without_a_table_is_refused() {
    let mut v: Value = serde_json::from_slice(common::bundled_file("time_limit")).unwrap();
    v.as_object_mut().unwrap().remove("level_table");
    assert!(matches!(
        TimeLimit::parse(v.to_string().as_bytes(), Versions::CURRENT),
        Err(Error::Parse { .. })
    ));
}
