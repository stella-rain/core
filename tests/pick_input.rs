//! The offer pick inside `Input` (ADR-REPL-3, ADR-SIM-12): the pick is not part of the schema 1
//! wire, replays are recorded and verified byte for byte as before, and a version 1 run takes a
//! one-entry slice.

use crate::corpus::{case_content, check_case};
use crate::data_root::data_root;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use stella_rain_core::engine::{ChoiceError, Engine};
use stella_rain_core::input::Input;
use stella_rain_core::replay::{self, Replay, pack_inputs};
use stella_rain_core::run_replay::{
    self, Command, CommandKind, RunRecorder, RunReplay, RunReplayError,
};
use stella_rain_core::validate::parse_and_validate;

#[test]
fn pick_is_not_in_the_wire() {
    let picked = serde_json::to_value(Input::picking(2)).unwrap();
    assert_eq!(picked["pick"], json!(2));
    let plain = serde_json::to_value(Input::default()).unwrap();
    assert_eq!(plain.as_object().unwrap().len(), 8);
    assert!(plain.get("pick").is_none());

    let without: Input = serde_json::from_value(plain).unwrap();
    assert_eq!(without.pick, None);
    let text = r#"{"move_x":0,"move_y":0,"main":false,"smash":false,"evade":false,"skill":false,"assist":false,"auto":false,"pick":"1"}"#;
    assert!(serde_json::from_str::<Input>(text).is_err());
    assert_eq!(
        pack_inputs(&[Input::picking(2)]),
        pack_inputs(&[Input::default()])
    );
}

/// A run replay's commands in the order they were made.
fn commands_of(run: &RunReplay) -> Vec<Command> {
    let inputs = run.unpack_inputs(u32::MAX).unwrap();
    let between = run.unpack_commands().unwrap();
    let mut next = between.iter().peekable();
    let command = |c: &run_replay::TickCommand| match c.kind {
        CommandKind::Choose => Command::Choose(c.index),
    };
    let mut out = Vec::new();
    for (tick, input) in (0u32..).zip(inputs) {
        while let Some(c) = next.next_if(|c| c.tick == tick) {
            out.push(command(c));
        }
        out.push(Command::Step(input));
    }
    out.extend(next.map(command));
    out
}

fn run_cases(tree: &str) -> Vec<PathBuf> {
    let dir = data_root().join("tests/corpus").join(tree).join("runs");
    let mut cases: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    cases.sort();
    cases
}

#[test]
fn picks_leave_the_replay_bytes_alone() {
    for tree in ["current", "v1"] {
        for case in run_cases(tree) {
            let bytes = fs::read(case.join("run.replay")).unwrap();
            let run = RunReplay::from_bytes(&bytes).unwrap();
            let content = case_content(&case).unwrap();
            let recorded =
                run_replay::record_run_with(&run.setup, &content, &commands_of(&run)).unwrap();
            let name = case.display();
            // The recorder writes schema 2; a frozen schema 1 file is compared in its form.
            let recorded = if run.setup.schema_version == 1 {
                crate::common::schema_1_form(&recorded)
            } else {
                recorded
            };
            assert_eq!(recorded.inputs, run.inputs, "{name}: inputs");
            assert_eq!(recorded.commands, run.commands, "{name}: commands");
            assert_eq!(recorded.checkpoints, run.checkpoints, "{name}: checkpoints");
            assert_eq!(recorded.final_hash, run.final_hash, "{name}: final hash");
        }
    }
}

#[test]
fn a_version_1_run_takes_a_one_entry_slice() {
    let case = data_root().join("tests/corpus/v1/stages/clear");
    let stage = parse_and_validate(&fs::read(case.join("stage.json")).unwrap()).unwrap();
    let recorded = Replay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
    let hashes = replay::hashes_from_bytes(&fs::read(case.join("hashes.bin")).unwrap()).unwrap();
    let mut engine = Engine::new(&stage).unwrap();
    assert_eq!(engine.players(), 1);
    let inputs = recorded.unpack_inputs(stage.length_ticks).unwrap();
    assert_eq!(inputs.len(), hashes.len());
    for (input, expected) in inputs.iter().zip(&hashes) {
        engine.step(&[*input]);
        assert_eq!(engine.state_hash(), *expected);
    }
    let diff = std::env::temp_dir();
    assert_eq!(check_case(&case, Path::new(&diff)), Ok(()));
}

#[test]
fn the_recorder_refuses_as_before() {
    let setup = crate::common::run::trial_setup();
    let mut recorder = RunRecorder::start(&setup).unwrap();
    assert!(matches!(
        recorder.choose(0),
        Err(RunReplayError::ChoiceRefused {
            error: ChoiceError::NoOffer,
            ..
        })
    ));
    assert_eq!(recorder.replay().commands, "");
}
