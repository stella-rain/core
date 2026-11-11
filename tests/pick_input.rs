//! The offer pick inside `Input` (ADR-REPL-5, ADR-SIM-7): the pick is not part of the schema 1
//! wire, replays are recorded and verified byte for byte as before, and a version 1 run takes a
//! one-entry slice.

use crate::common::run_steps;
use crate::corpus::{case_content, check_case};
use crate::data_root::data_root;
use std::fs;
use std::path::{Path, PathBuf};
use stella_rain_core::engine::Engine;
use stella_rain_core::replay::{self, Replay};
use stella_rain_core::run_replay::{self, RunReplay};
use stella_rain_core::validate::parse_and_validate;

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
    // The current runs only: a frozen v1 run holds PICKS steps no recorder writes any more; it
    // verifies through `verify_run*` (`corpus::`).
    for case in run_cases("current") {
        let bytes = fs::read(case.join("run.replay")).unwrap();
        let run = RunReplay::from_bytes(&bytes).unwrap();
        let content = case_content(&case).unwrap();
        let recorded =
            run_replay::record_run_with(&run.setup, &content, &run_steps::commands(&run)).unwrap();
        let name = case.display();
        assert_eq!(recorded.inputs, run.inputs, "{name}: inputs");
        assert_eq!(recorded.commands, run.commands, "{name}: commands");
        assert_eq!(recorded.checkpoints, run.checkpoints, "{name}: checkpoints");
        assert_eq!(recorded.final_hash, run.final_hash, "{name}: final hash");
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
        engine.step(&[*input, Default::default(), Default::default()]);
        assert_eq!(engine.state_hash(), *expected);
    }
    let diff = std::env::temp_dir();
    assert_eq!(check_case(&case, Path::new(&diff)), Ok(()));
}

#[test]
fn a_choice_cannot_be_recorded() {
    use crate::common::run::trial_setup;
    use stella_rain_core::run_replay::{Command, RunReplayError};
    assert_eq!(
        run_replay::record_run(&trial_setup(), &[Command::Choose(0)]).map(|_| ()),
        Err(RunReplayError::BadCommands("a choice cannot be recorded"))
    );
}
