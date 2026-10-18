//! The match-up table (ADR-100, ADR-067): weak, strong and resisted percents, read from JSON as
//! untrusted bytes (ADR-020), each at most `MAX_MATCH_UP_PCT`, and the most the type bonuses
//! add to one attack, 1 to `MAX_TYPE_BONUS_PCT` (ADR-100 point 7). The bundled file is the
//! default; other bytes run the same way and change the run's content hash.

use serde_json::{Value, json};
use stella_rain_core::attack::MatchUp;
use stella_rain_core::match_ups::{MAX_MATCH_UP_PCT, MAX_MATCH_UPS_BYTES, MatchUps};
use stella_rain_core::rewards::MAX_TYPE_BONUS_PCT;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{
    RunRecorder, RunReplayError, content_sha256, content_sha256_with, verify_run, verify_run_with,
};
use stella_rain_core::validate::Error;

mod common;

use common::run::{run_script, trial_setup};

fn file(weak: Value, strong: Value, resisted: Value) -> Value {
    json!({ "schema_version": 0, "sim_version": 0,
            "weak_pct": weak, "strong_pct": strong, "resisted_pct": resisted,
            "max_type_bonus_pct": 1000 })
}

fn parse(v: &Value) -> Result<MatchUps, Error> {
    MatchUps::parse(v.to_string().as_bytes())
}

fn paths(e: Error) -> Vec<String> {
    match e {
        Error::Invalid(issues) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not a validation error: {other:?}"),
    }
}

#[test]
fn the_bundled_table_is_20_percent_each_way() {
    let t = MatchUps::bundled();
    assert_eq!(t.pct(MatchUp::Weak), 20);
    assert_eq!(t.pct(MatchUp::Strong), -20);
    assert_eq!(t.pct(MatchUp::Resisted), -20);
    assert_eq!(t.pct(MatchUp::None), 0);
}

#[test]
fn the_bundled_type_bonus_limit_is_the_hard_cap() {
    assert_eq!(MatchUps::bundled().max_type_bonus_pct(), MAX_TYPE_BONUS_PCT);
}

#[test]
fn the_type_bonus_limit_is_1_to_the_hard_cap() {
    let with = |pct: Value| {
        let mut v = file(json!(20), json!(20), json!(20));
        v["max_type_bonus_pct"] = pct;
        v
    };
    assert_eq!(parse(&with(json!(1))).unwrap().max_type_bonus_pct(), 1);
    assert_eq!(parse(&with(json!(250))).unwrap().max_type_bonus_pct(), 250);
    assert_eq!(
        paths(parse(&with(json!(0))).unwrap_err()),
        ["$.max_type_bonus_pct"]
    );
    let over = parse(&with(json!(MAX_TYPE_BONUS_PCT + 1))).unwrap_err();
    assert_eq!(paths(over), ["$.max_type_bonus_pct"]);
    assert!(parse(&with(json!(-1))).is_err());
    let mut missing = file(json!(20), json!(20), json!(20));
    missing
        .as_object_mut()
        .unwrap()
        .remove("max_type_bonus_pct");
    assert!(matches!(parse(&missing), Err(Error::Parse { .. })));
}

#[test]
fn each_percent_is_held_to_the_cap() {
    let cap = MAX_MATCH_UP_PCT;
    assert!(parse(&file(json!(cap), json!(cap), json!(0))).is_ok());
    let over = parse(&file(json!(cap + 1), json!(20), json!(cap + 1))).unwrap_err();
    assert_eq!(paths(over), ["$.weak_pct", "$.resisted_pct"]);
    assert!(parse(&file(json!(20), json!(-1), json!(20))).is_err());
    assert!(parse(&file(json!(20), json!(20), json!(256))).is_err());
}

#[test]
fn the_file_is_strict_and_size_limited() {
    let mut v = file(json!(20), json!(20), json!(20));
    v["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let mut missing = file(json!(20), json!(20), json!(20));
    missing.as_object_mut().unwrap().remove("strong_pct");
    assert!(matches!(parse(&missing), Err(Error::Parse { .. })));
    let mut newer = file(json!(20), json!(20), json!(20));
    newer["sim_version"] = json!(1);
    assert!(matches!(
        parse(&newer),
        Err(Error::UnsupportedVersion { .. })
    ));
    let mut big = MatchUps::BUNDLED.to_vec();
    big.resize(MAX_MATCH_UPS_BYTES + 1, b' ');
    assert!(matches!(MatchUps::parse(&big), Err(Error::TooLarge { .. })));
}

fn other() -> MatchUps {
    MatchUps::parse(file(json!(50), json!(20), json!(20)).to_string().as_bytes()).unwrap()
}

#[test]
fn the_content_hash_covers_the_match_ups() {
    let content = RunContent::bundled().clone().with_match_ups(other());
    assert_eq!(content.match_ups(), &other());
    assert_eq!(RunContent::bundled().match_ups(), MatchUps::bundled());
    assert_ne!(
        content_sha256_with("trial", &content),
        content_sha256("trial")
    );
}

#[test]
fn a_run_with_another_table_verifies_only_with_that_table() {
    let content = RunContent::bundled().clone().with_match_ups(other());
    let mut recorder = RunRecorder::start_with(&trial_setup(), &content).unwrap();
    for t in 0..50 {
        recorder.step(run_script(t)).unwrap();
    }
    let bytes = recorder.replay().to_bytes();
    assert!(verify_run_with(&bytes, &content).is_ok());
    assert_eq!(verify_run(&bytes), Err(RunReplayError::ContentChanged));
}
