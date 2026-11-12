//! The stage field `try_styles` (ADR-SIM-9 points 3, 5, 8 and 9, ADR-REPL-5 point 8): under it
//! every tree point of the level table is a style pick again, recorded as a `TreePick`. The field
//! is for the tutorial mode alone; the stage parser, the version gate and the run builder are
//! tested here.

use serde_json::Value;
use stella_rain_core::pack::Pack;
use stella_rain_core::run_setup::{self, RunSetup};
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{self, Error};

const V1_CLEAR: &[u8] = include_bytes!("corpus/v1/stages/clear/stage.json");

fn tutorial_file() -> Value {
    serde_json::from_slice(run_setup::mode_file("tutorial").unwrap()).unwrap()
}

fn parse(v: &Value) -> Result<Stage, Error> {
    validate::parse_and_validate(v.to_string().as_bytes())
}

fn written(stage: &Stage) -> Value {
    serde_json::to_value(stage).unwrap()
}

fn pack_with(mode: &str) -> Pack {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    v["modes"][mode]["try_styles"] = true.into();
    Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("the pack loads")
}

fn stage_of(pack: &Pack, mode: &str) -> Value {
    let setup = RunSetup {
        mode: mode.into(),
        ..RunSetup::default()
    };
    written(&run_setup::stage_in(&setup, pack.content()).expect("a stage"))
}

// T1: the field, its version gate and its parse.

#[test]
fn the_field_parses_and_is_written_back() {
    let mut v = tutorial_file();
    v["try_styles"] = true.into();
    let stage = parse(&v).expect("valid at sim_version 2");
    assert_eq!(written(&stage)["try_styles"], true);
}

#[test]
fn a_false_or_absent_field_writes_no_key() {
    for mode in ["trial", "tutorial"] {
        let v: Value = serde_json::from_slice(run_setup::mode_file(mode).unwrap()).unwrap();
        assert!(written(&parse(&v).unwrap()).get("try_styles").is_none());
    }
    let mut v = tutorial_file();
    v["try_styles"] = false.into();
    assert!(written(&parse(&v).unwrap()).get("try_styles").is_none());
}

#[test]
fn version_1_refuses_the_field_and_accepts_false() {
    let mut v: Value = serde_json::from_slice(V1_CLEAR).unwrap();
    v["try_styles"] = true.into();
    match parse(&v) {
        Err(Error::Invalid(issues)) => {
            assert_eq!(issues.len(), 1);
            assert_eq!(issues[0].path, "$.try_styles");
            assert_eq!(issues[0].message, "needs sim_version 2");
        }
        other => panic!("not refused as invalid: {other:?}"),
    }
    v["try_styles"] = false.into();
    parse(&v).expect("false is accepted at any version");
}

#[test]
fn a_non_bool_is_refused_by_the_parse() {
    for bad in [Value::from("yes"), Value::from(1)] {
        let mut v = tutorial_file();
        v["try_styles"] = bad;
        assert!(parse(&v).is_err());
    }
}

// T2: the field is inert outside the tutorial.

#[test]
fn a_trial_file_that_names_the_field_changes_no_run() {
    let pack = pack_with("trial");
    assert!(stage_of(&pack, "trial").get("try_styles").is_none());
    assert!(stage_of(&pack, "time_limit").get("try_styles").is_none());
}

#[test]
fn the_tutorial_file_that_names_the_field_gives_the_tutorial_run_alone() {
    let pack = pack_with("tutorial");
    assert_eq!(stage_of(&pack, "tutorial")["try_styles"], true);
    assert!(stage_of(&pack, "trial").get("try_styles").is_none());
}

#[test]
fn the_bundled_pack_names_the_field_nowhere_outside_the_tutorial() {
    let pack = Pack::bundled();
    assert!(stage_of(&pack, "trial").get("try_styles").is_none());
    assert!(stage_of(&pack, "time_limit").get("try_styles").is_none());
}
