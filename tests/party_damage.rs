//! The pack's `party_damage_pct` (ADR-SIM-8, ADR-CONT-3): a list of three percents, one for each
//! party size, read at each hit of a player's damage. It loads, defaults and is bounded here;
//! how `land` applies it is pinned in `src/sim/damage_tests.rs`.

use crate::common;
use serde_json::{Value, json};
use stella_rain_core::rules::{MAX_PARTY_DAMAGE_PCT, Rules};
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

/// The bundled file with the table set to `table`, or without the key when `table` is `None`.
fn rules_text(table: Option<Value>) -> Vec<u8> {
    let mut v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    let map = v.as_object_mut().unwrap();
    match table {
        Some(t) => map.insert("party_damage_pct".into(), t),
        None => map.remove("party_damage_pct"),
    };
    v.to_string().into_bytes()
}

fn paths(table: Value) -> Vec<String> {
    match Rules::parse(&rules_text(Some(table)), Versions::CURRENT) {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not refused as invalid: {other:?}"),
    }
}

#[test]
fn the_bundled_table_is_the_identity() {
    assert_eq!(Rules::bundled().party_damage_pct(), [100, 100, 100]);
    assert_eq!(MAX_PARTY_DAMAGE_PCT, 100);
}

#[test]
fn a_file_without_the_key_reads_the_identity_and_a_set_key_is_read() {
    let old = Rules::parse(&rules_text(None), Versions::CURRENT).unwrap();
    assert_eq!(old.party_damage_pct(), [100, 100, 100]);
    let set = Rules::parse(&rules_text(Some(json!([100, 60, 30]))), Versions::CURRENT).unwrap();
    assert_eq!(set.party_damage_pct(), [100, 60, 30]);
}

#[test]
fn a_wrong_length_is_one_issue_at_the_key() {
    for table in [json!([]), json!([100, 100]), json!([100, 100, 100, 100])] {
        assert_eq!(paths(table.clone()), ["$.party_damage_pct"], "{table}");
    }
}

#[test]
fn an_entry_outside_1_to_100_is_one_issue_at_its_index() {
    assert_eq!(paths(json!([0, 100, 100])), ["$.party_damage_pct[0]"]);
    assert_eq!(paths(json!([100, 101, 100])), ["$.party_damage_pct[1]"]);
    assert_eq!(paths(json!([100, 100, 0])), ["$.party_damage_pct[2]"]);
}

#[test]
fn the_bounds_load() {
    for table in [json!([1, 1, 1]), json!([100, 100, 100])] {
        assert!(
            Rules::parse(&rules_text(Some(table.clone())), Versions::CURRENT).is_ok(),
            "{table}"
        );
    }
}
