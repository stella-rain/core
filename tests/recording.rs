//! The JSON Lines recording (ADR-016), behind the `record` feature.
#![cfg(feature = "record")]

use crate::common;

use common::run::{first_point_tick, play_run, trial_setup};
use common::{CLEAR_TICK, clearing_inputs, stage_bytes};
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::recording::{
    commands_from_jsonl, hashes_from_jsonl, inputs_from_jsonl, record, record_run,
};
use stella_rain_core::replay::{self, hash_to_hex};
use stella_rain_core::run_replay::{Command, RunReplayError};
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
    assert_eq!(kinds(1), ["Input", "Evaded", "StateHash"]);
    // The last tick: the slash lands, the boss dies, the stage is cleared.
    assert_eq!(
        kinds(u64::from(CLEAR_TICK)),
        ["Input", "Hit", "Died", "StageCleared", "StateHash"]
    );
    let directions: Vec<(&str, &str)> = lines
        .iter()
        .filter(|l| l["tick"] == 1)
        .map(|l| (l["type"].as_str().unwrap(), l["dir"].as_str().unwrap()))
        .collect();
    assert_eq!(
        directions,
        [
            ("Input", "fe>be"),
            ("Evaded", "be>fe"),
            ("StateHash", "be>fe")
        ]
    );
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

// --- Runs with commands between ticks (ADR-079 §5) --------------------------------------------

fn lines_of(text: &str) -> Vec<Value> {
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn a_level_up_and_its_stat_are_the_events_of_its_tick() {
    let point = first_point_tick();
    let text = record_run(&trial_setup(), &play_run(point + 1)).unwrap();
    let lines = lines_of(&text);
    let at_point: Vec<(&str, &str)> = lines
        .iter()
        .filter(|l| l["tick"] == json!(point))
        .map(|l| (l["dir"].as_str().unwrap(), l["type"].as_str().unwrap()))
        .collect();
    // The tick's input, its events (the level-up, then the stat it raised) and its hash; the
    // next tick's input follows. No command is recorded: the priority spends it.
    assert_eq!(at_point.first(), Some(&("fe>be", "Input")));
    let hash = at_point.iter().position(|l| l.1 == "StateHash").unwrap();
    let events = &at_point[1..hash];
    assert!(
        events
            .windows(2)
            .any(|w| w == [("be>fe", "LeveledUp"), ("be>fe", "StatRaised")])
    );
    assert_eq!(hash + 1, at_point.len());
    assert!(lines.iter().all(|l| l["type"] != "Spend"));
    assert_eq!(lines.last().unwrap()["tick"], json!(point + 1));
}

#[test]
fn commands_go_to_a_recording_and_back_unchanged() {
    let commands = play_run(first_point_tick() + 30);
    let text = record_run(&trial_setup(), &commands).unwrap();
    assert!(text.contains("StatRaised"));
    assert_eq!(commands_from_jsonl(&text).unwrap(), commands);

    // The other readers still read a run's recording: its inputs, and one hash a tick.
    let inputs: Vec<_> = commands
        .iter()
        .filter_map(|c| match c {
            Command::Step(input) => Some(*input),
            Command::Choose(_) => None,
        })
        .collect();
    assert_eq!(inputs_from_jsonl(&text).unwrap(), inputs);
    assert_eq!(hashes_from_jsonl(&text).unwrap().len(), inputs.len());
}

#[test]
fn a_choose_line_reads_back_as_a_choice() {
    let text = concat!(
        r#"{"tick":1,"dir":"fe>be","type":"Input","data":{"move_x":0,"move_y":0,"main":false,"smash":false,"evade":false,"skill":false,"assist":false,"auto":false}}"#,
        "\n",
        r#"{"tick":1,"dir":"fe>be","type":"Choose","data":{"index":2}}"#,
        "\n"
    );
    let commands = commands_from_jsonl(text).unwrap();
    assert_eq!(commands[1], Command::Choose(2));
}

#[test]
fn a_run_recording_fails_closed_like_the_replay() {
    let mut commands = play_run(10);
    commands.insert(2, Command::Choose(0));
    assert!(matches!(
        record_run(&trial_setup(), &commands),
        Err(RunReplayError::ChoiceRefused { tick: 2, .. })
    ));
}

#[test]
fn a_spend_line_is_refused_since_spend_was_removed() {
    let text = concat!(
        r#"{"tick":0,"dir":"fe>be","type":"Spend","data":{"index":1}}"#,
        "
"
    );
    let err = commands_from_jsonl(text).unwrap_err();
    assert_eq!((err.line, err.reason), (1, "a spend, which DS18 removed"));
}

#[test]
fn a_damaged_command_line_is_refused_with_its_line() {
    let good = record_run(&trial_setup(), &play_run(3)).unwrap();
    // The line after the first tick's hash: the second tick's input, replaced by a command.
    let at = good
        .lines()
        .position(|l| l.contains("\"StateHash\""))
        .unwrap()
        + 1;
    let line = |replacement: String| {
        let mut lines: Vec<String> = good.lines().map(str::to_owned).collect();
        lines[at] = replacement;
        lines.join(
            "
",
        )
    };
    let command = |tick: u32, data: &str| {
        format!(r#"{{"tick":{tick},"dir":"fe>be","type":"Choose","data":{data}}}"#)
    };
    let err = commands_from_jsonl(&line(command(0, r#"{"index":1}"#))).unwrap_err();
    assert_eq!(
        (err.line, err.reason),
        (at + 1, "a choice is not after its tick")
    );
    for data in [
        r#"{"index":-1}"#,
        r#"{"index":"1"}"#,
        r#"{}"#,
        r#"{"index":1,"x":0}"#,
    ] {
        let err = commands_from_jsonl(&line(command(1, data))).unwrap_err();
        assert_eq!((err.line, err.reason), (at + 1, "not a choice"), "{data}");
    }
}
