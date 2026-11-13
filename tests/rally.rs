//! The run rules' rally (ADR-SIM-9 points 2 to 7, ADR-CONT-3, ADR-SVC-1): three optional keys,
//! all or none, bounded by the validator, `sim_version` 2's, read once into `Rally`. Test code
//! only: the numbers are test values, not content.

use serde_json::{Value, json};
use stella_rain_core::rules::Rules;
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

use crate::common;

const KEYS: [&str; 3] = ["rally_pct", "rally_ticks", "rally_regain_pct"];

/// The three keys at `[pct, ticks, regain]` set on the bundled file.
fn with(values: [i64; 3]) -> Vec<u8> {
    let mut v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    for (key, value) in KEYS.into_iter().zip(values) {
        v[key] = json!(value);
    }
    v.to_string().into_bytes()
}

const OK: [i64; 3] = [50, 180, 100];

fn paths(result: Result<Rules, Error>) -> Vec<String> {
    match result {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not refused as invalid: {other:?}"),
    }
}

/// R1: the three keys load at their bounds.
#[test]
fn the_three_rally_keys_load_at_their_bounds() {
    for values in [OK, [1, 1, 1], [100, 600, 1_000]] {
        Rules::parse(&with(values), Versions::CURRENT)
            .unwrap_or_else(|e| panic!("{values:?} refused: {e:?}"));
    }
}

/// R1: each key is refused one past its bound, at its own path.
#[test]
fn each_rally_key_is_refused_one_past_its_bound_at_its_own_path() {
    let cases: [(usize, i64); 6] = [(0, 0), (0, 101), (1, 0), (1, 601), (2, 0), (2, 1_001)];
    for (key, value) in cases {
        let mut values = OK;
        values[key] = value;
        assert_eq!(
            paths(Rules::parse(&with(values), Versions::CURRENT)),
            [format!("$.{}", KEYS[key])],
            "{} = {value}",
            KEYS[key]
        );
    }
}

/// R1: one or two of the three is refused at each missing key.
#[test]
fn some_but_not_all_rally_keys_is_refused_naming_each_missing_key() {
    for present in 1..3 {
        let mut v: Value = serde_json::from_slice(&with(OK)).unwrap();
        for key in &KEYS[present..] {
            v.as_object_mut().unwrap().remove(*key);
        }
        let want: Vec<String> = KEYS[present..].iter().map(|k| format!("$.{k}")).collect();
        assert_eq!(
            paths(Rules::parse(v.to_string().as_bytes(), Versions::CURRENT)),
            want,
            "{present} present"
        );
    }
}

/// R1: at `sim_version` 1 any of them is refused.
#[test]
fn the_rally_keys_need_sim_version_2() {
    let v1 = Versions {
        sim_version: 1,
        ..Versions::CURRENT
    };
    for key in KEYS {
        let got = paths(Rules::parse(&with(OK), v1));
        assert!(got.contains(&format!("$.{key}")), "{key}: {got:?}");
    }
}

/// R2: the bundled file names none of the keys and the schema lists them as optional.
#[test]
fn the_bundled_file_names_no_rally_key_and_the_schema_lists_them_as_optional() {
    let v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    for key in KEYS {
        assert!(v.get(key).is_none(), "{key}");
    }
    let schema = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/rules.schema.json"),
    )
    .expect("the rules schema");
    let schema: Value = serde_json::from_str(&schema).unwrap();
    for key in KEYS {
        assert!(schema["properties"].get(key).is_some(), "{key} not listed");
        let required = schema["required"].as_array().map_or(&[][..], Vec::as_slice);
        assert!(!required.iter().any(|r| r == key), "{key} is required");
    }
}
/// R3: `Rules::rally()` reads the three numbers back.
#[test]
fn a_loaded_rally_reads_back_its_numbers() {
    let rules = Rules::parse(&with(OK), Versions::CURRENT).unwrap();
    let got = rules.rally().expect("all three keys");
    assert_eq!((got.pct, got.ticks, got.regain_pct), (50, 180, 100));
    let top = Rules::parse(&with([100, 600, 1_000]), Versions::CURRENT).unwrap();
    let got = top.rally().expect("all three keys");
    assert_eq!((got.pct, got.ticks, got.regain_pct), (100, 600, 1_000));
}

/// R3: a bundled file gives no rally.
#[test]
fn the_bundled_file_gives_no_rally() {
    assert!(Rules::bundled().rally().is_none());
}
