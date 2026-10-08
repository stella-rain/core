//! The JSON Lines recording (ADR-016), behind the `record` feature.
#![cfg(feature = "record")]

mod common;

use common::{CLEAR_TICK, clearing_inputs, stage_bytes};
use serde_json::Value;
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::recording::{hashes_from_jsonl, inputs_from_jsonl, record};
use stella_rain_core::replay::{self, hash_to_hex};
use stella_rain_core::validate::parse_and_validate;

fn recording() -> String {
    let stage = parse_and_validate(&stage_bytes()).unwrap();
    record(&stage, &clearing_inputs()).unwrap()
}

#[test]
fn every_tick_has_an_input_its_events_and_a_hash() {
    let text = recording();
    let lines: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let kinds = |tick: u64| -> Vec<&str> {
        lines
            .iter()
            .filter(|l| l["tick"] == tick)
            .map(|l| l["type"].as_str().unwrap())
            .collect()
    };
    assert_eq!(kinds(1), ["Input", "StateHash"]);
    // The last tick: both bullets of the shot land, the boss dies, the stage is cleared.
    assert_eq!(
        kinds(u64::from(CLEAR_TICK)),
        ["Input", "Hit", "Hit", "Died", "StageCleared", "StateHash"]
    );
    let directions: Vec<(&str, &str)> = lines
        .iter()
        .filter(|l| l["tick"] == 1)
        .map(|l| (l["type"].as_str().unwrap(), l["dir"].as_str().unwrap()))
        .collect();
    assert_eq!(directions, [("Input", "fe>be"), ("StateHash", "be>fe")]);
    // The lines of the doc example's shape: tick first is not required, but every field is there.
    assert!(
        lines
            .iter()
            .all(|l| l["tick"].is_u64() && l["dir"].is_string())
    );
}

#[test]
fn inputs_go_to_a_recording_and_back_unchanged() {
    let inputs = clearing_inputs();
    assert_eq!(inputs_from_jsonl(&recording()).unwrap(), inputs);
    // And through the compact encoding of a replay and back.
    let replay = replay::record(&stage_bytes(), &inputs).unwrap();
    let again = replay.unpack_inputs(600).unwrap();
    assert_eq!(again, inputs);
    assert_eq!(inputs_from_jsonl(&recording()).unwrap(), again);
}

#[test]
fn the_hash_lines_are_the_engines_and_end_on_the_replays_final_hash() {
    let hashes = hashes_from_jsonl(&recording()).unwrap();
    assert_eq!(hashes.len(), CLEAR_TICK as usize);
    let stage = parse_and_validate(&stage_bytes()).unwrap();
    let mut engine = Engine::new(&stage).unwrap();
    for (input, expected) in clearing_inputs().iter().zip(&hashes) {
        engine.step(*input);
        assert_eq!(engine.state_hash(), *expected);
    }
    let replay = replay::record(&stage_bytes(), &clearing_inputs()).unwrap();
    assert_eq!(replay.final_hash, hash_to_hex(*hashes.last().unwrap()));
    // Those hashes are what a corpus check compares against.
    assert!(replay::verify_against(&stage_bytes(), &replay.to_bytes(), &hashes).is_ok());
}

#[test]
fn events_in_a_recording_are_domain_events() {
    for line in recording().lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        if v["dir"] == "be>fe" && v["type"] != "StateHash" {
            let mut event = v.clone();
            let object = event.as_object_mut().unwrap();
            object.remove("tick");
            object.remove("dir");
            serde_json::from_value::<DomainEvent>(event).unwrap();
        }
    }
}

#[test]
fn a_damaged_recording_is_refused_with_its_line() {
    let good = recording();
    let line = |n: usize, replacement: &str| {
        let mut lines: Vec<&str> = good.lines().collect();
        lines[n] = replacement;
        lines.join("\n")
    };
    let err = inputs_from_jsonl(&line(2, "not json")).unwrap_err();
    assert_eq!((err.line, err.reason), (3, "not a recording line"));
    let err = inputs_from_jsonl(&line(0, r#"{"tick":5,"dir":"fe>be","type":"Input","data":{"dx":0,"dy":0,"focus":false,"fire":false,"held":0}}"#)).unwrap_err();
    assert_eq!((err.line, err.reason), (1, "ticks are not in order"));
    let err = inputs_from_jsonl(&line(
        0,
        r#"{"tick":1,"dir":"fe>be","type":"Input","data":{"dx":"x"}}"#,
    ))
    .unwrap_err();
    assert_eq!(err.reason, "not an input");
    let err = hashes_from_jsonl(&line(
        1,
        r#"{"tick":1,"dir":"be>fe","type":"StateHash","data":"nope"}"#,
    ))
    .unwrap_err();
    assert_eq!((err.line, err.reason), (2, "not a state hash"));
    assert_eq!(inputs_from_jsonl("").unwrap(), vec![]);
    assert_eq!(inputs_from_jsonl("\n\n").unwrap(), vec![]);
}
