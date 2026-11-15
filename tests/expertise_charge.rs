//! The `charge_sooner` kind of a style level (ADR-SIM-9 points 2 to 7, ADR-SIM-5, ADR-SVC-1,
//! ADR-CONT-3): its number is bounded by the validator, the kind is `sim_version` 2's, one per
//! step per style path, and it round-trips through JSON. Test code only: the numbers are test
//! values, not content.

use serde_json::{Value, json};
use stella_rain_core::expertise::{Expertise, StyleKind};
use stella_rain_core::validate::Error;
use stella_rain_core::validate::limits::MAX_COMBO_DEPTH;
use stella_rain_core::version::Versions;

const STYLE: &str = "/weapons/0/styles/0";
const PATH: &str = "$.weapons[0].styles[0]";

fn sooner(step: &[&str], pct: u32) -> Value {
    json!({ "type": "charge_sooner", "step": step, "pct": pct })
}

fn aaa(pct: u32) -> Value {
    sooner(&["main", "main", "main"], pct)
}

/// The styles from before the weapons' content moved (`crate::before`): the tests patch style 0.
fn file() -> Value {
    serde_json::from_slice(crate::before::EXPERTISE).unwrap()
}

/// Style 0 with `kinds` on the style pick itself (`level` empty) or on its `level`'s first option.
fn with(level: &str, kinds: Value) -> Value {
    let mut v = file();
    let style = v.pointer_mut(STYLE).unwrap();
    let target = if level.is_empty() {
        style
    } else {
        &mut style[level][0]
    };
    target["kinds"] = kinds;
    v
}

fn parse_at(v: &Value, versions: Versions) -> Result<Expertise, Error> {
    Expertise::parse(v.to_string().as_bytes(), versions)
}

fn valid(v: &Value) {
    if let Err(e) = parse_at(v, Versions::CURRENT) {
        panic!("expected valid, got {e:?}");
    }
}

fn refused_at(v: &Value, path: &str) {
    match parse_at(v, Versions::CURRENT) {
        Err(Error::Invalid(issues)) => {
            assert!(issues.iter().any(|i| i.path == path), "{path}: {issues:?}")
        }
        other => panic!("{path}: expected invalid, got {other:?}"),
    }
}

/// V1: the kind loads, round-trips and is `sim_version` 2's.
#[test]
fn the_kind_loads_round_trips_and_is_version_two() {
    let v = with("", json!([aaa(25)]));
    valid(&v);
    let kind: StyleKind = serde_json::from_value(aaa(25)).expect("the kind parses");
    assert_eq!(serde_json::to_value(&kind).unwrap(), aaa(25));
    let old = Versions {
        sim_version: 1,
        ..Versions::CURRENT
    };
    match parse_at(&v, old) {
        Err(Error::Invalid(issues)) => assert!(
            issues
                .iter()
                .any(|i| i.path == format!("{PATH}.kinds") && i.message.contains("sim_version 2")),
            "{issues:?}"
        ),
        other => panic!("{other:?}"),
    }
}

/// V2: `pct` 1 to 90 and `step` 1 to `MAX_COMBO_DEPTH` presses.
#[test]
fn the_percent_and_the_step_are_bounded() {
    let at = format!("{PATH}.kinds[0]");
    for pct in [1, 25, 90] {
        valid(&with("", json!([aaa(pct)])));
    }
    for pct in [0, 91] {
        refused_at(&with("", json!([aaa(pct)])), &format!("{at}.pct"));
    }
    refused_at(&with("", json!([sooner(&[], 25)])), &format!("{at}.step"));
    let long = vec!["main"; MAX_COMBO_DEPTH as usize + 1];
    refused_at(&with("", json!([sooner(&long, 25)])), &format!("{at}.step"));
    let deepest = vec!["main"; MAX_COMBO_DEPTH as usize];
    valid(&with("", json!([sooner(&deepest, 25)])));
}

/// V3: one per step per style path; two on different steps are fine.
#[test]
fn a_step_is_made_sooner_once_per_style() {
    let mut v = with("", json!([aaa(25)]));
    v.pointer_mut(&format!("{STYLE}/upgrade_2/0")).unwrap()["kinds"] = json!([aaa(25)]);
    refused_at(&v, &format!("{PATH}.upgrade_2[0].kinds[0].step"));
    v.pointer_mut(&format!("{STYLE}/upgrade_2/0")).unwrap()["kinds"] =
        json!([sooner(&["main", "main"], 25)]);
    valid(&v);
    refused_at(
        &with("", json!([aaa(25), aaa(10)])),
        &format!("{PATH}.kinds[1].step"),
    );
}

/// V4: a common option names no kind (the existing rule).
#[test]
fn a_common_option_may_not_name_it() {
    let mut v = file();
    v.pointer_mut("/weapons/0/common/0").unwrap()["kinds"] = json!([aaa(25)]);
    refused_at(&v, "$.weapons[0].common[0].kinds");
}
