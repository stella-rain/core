//! Every writer writes replay schema 2 (ADR-REPL-7 point 4, ADR-REPL-10): the run recorder
//! writes the step log with each pick inside its step and an empty `commands`, the stage replay
//! writer keeps `interact`, and a schema 1 file of each type still loads and verifies
//! (ADR-REPL-9).

use crate::corpus::case_content;
use crate::corpus_pins::case_pack;
use crate::data_root::data_root;
use std::fs;
use std::path::{Path, PathBuf};
use stella_rain_core::input::Input;
use stella_rain_core::pack::Pack;
use stella_rain_core::replay::{self, Replay, pack_inputs};
use stella_rain_core::run_replay::{
    self, Command, CommandKind, RunRecorder, RunReplay, verify_run_with,
};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::savepoint::Savepoint;
use stella_rain_core::version::{NEWEST_READ_SCHEMA_VERSION, SCHEMA_VERSION};

fn tree(name: &str) -> PathBuf {
    data_root().join("tests/corpus").join(name)
}

fn run_cases(name: &str) -> Vec<PathBuf> {
    let mut cases: Vec<_> = fs::read_dir(tree(name).join("runs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    cases.sort();
    cases
}

fn read_run(case: &Path) -> RunReplay {
    RunReplay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap()
}

/// A run replay's commands in the order they were made, from its solo views.
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

#[test]
fn the_versions_the_writers_write() {
    assert_eq!(SCHEMA_VERSION, 2);
    assert_eq!(SCHEMA_VERSION, NEWEST_READ_SCHEMA_VERSION);
    assert_eq!(RunSetup::default().schema_version, 2);
    let pack = String::from_utf8(Pack::BUNDLED.to_vec()).unwrap();
    assert!(pack.contains("\"schema_version\": 2"), "{}", &pack[..80]);
}

#[test]
fn a_recorded_run_is_a_step_log() {
    // `stage_skill` holds picks: the real offers.
    let case = tree("v1").join("runs/stage_skill");
    let old = read_run(&case);
    assert!(!old.commands.is_empty(), "the case holds picks");
    let content = case_content(&case).unwrap();
    let recorded = run_replay::record_run_with(&old.setup, &content, &commands_of(&old)).unwrap();
    assert_eq!(recorded.setup.schema_version, 2);
    assert_eq!(recorded.commands, "");
    assert_eq!(recorded.ticks, old.ticks);
    assert_eq!(recorded.final_hash, old.final_hash);
    assert_eq!(recorded.checkpoints, old.checkpoints);
    let verified = verify_run_with(&recorded.to_bytes(), &content).unwrap();
    let before =
        run_replay::verify_run_with(&fs::read(case.join("run.replay")).unwrap(), &content).unwrap();
    assert_eq!(verified.commands, before.commands);
    assert_eq!(verified.final_hash, before.final_hash);
}

#[test]
fn recorded_replays_have_no_command_list() {
    for case in run_cases("v1") {
        let old = read_run(&case);
        let content = case_content(&case).unwrap();
        let mut rec = RunRecorder::start_with(&old.setup, &content).unwrap();
        for command in commands_of(&old) {
            match command {
                Command::Step(input) => rec.step(input).unwrap(),
                Command::Choose(i) => rec.choose(i).unwrap(),
            }
        }
        let whole = rec.replay();
        assert_eq!(whole.commands, "", "{}", case.display());
        // A cut at every pick: `replay_at` keeps the picks at or before the tick, and verifies.
        for command in old.unpack_commands().unwrap() {
            let at = rec.replay_at(command.tick, 0);
            assert_eq!(at.commands, "");
        }
    }
}

#[test]
fn the_stage_replay_writer_writes_schema_2_and_keeps_interact() {
    let dir = tree("v1").join("stages/clear");
    let stage = fs::read(dir.join("stage.json")).unwrap();
    let old = Replay::from_bytes(&fs::read(dir.join("run.replay")).unwrap()).unwrap();
    assert_eq!(old.schema_version, 1);
    let mut inputs = old.unpack_inputs(u32::MAX).unwrap();
    let plain = replay::record(&stage, &inputs).unwrap();
    inputs[3].interact = true;
    let held = replay::record(&stage, &inputs).unwrap();
    assert_eq!(held.schema_version, 2);
    assert_eq!(held.final_hash, plain.final_hash);
    // Bit 6 of the flags is set in the unpacked blocks, and the replay reads back with it.
    let back = held.unpack_inputs(u32::MAX).unwrap();
    assert!(back[3].interact);
    // The stage file stays at schema 1; a replay at 2 pairs with it.
    assert!(replay::verify(&stage, &held.to_bytes()).is_ok());
    // `pack_inputs` is the schema 1 packer and still clears it.
    assert_eq!(
        pack_inputs(&inputs),
        pack_inputs(
            &back
                .iter()
                .map(|i| Input {
                    interact: false,
                    ..*i
                })
                .collect::<Vec<_>>()
        )
    );
}

#[test]
fn the_unpack_views_read_both_schemas() {
    for case in run_cases("v1") {
        let old = read_run(&case);
        let content = case_content(&case).unwrap();
        let two = run_replay::record_run_with(&old.setup, &content, &commands_of(&old)).unwrap();
        assert_eq!(two.setup.schema_version, 2);
        let name = case.display();
        let inputs = two.unpack_inputs(u32::MAX).unwrap();
        assert_eq!(inputs.len(), usize::try_from(two.ticks).unwrap(), "{name}");
        assert_eq!(inputs, old.unpack_inputs(u32::MAX).unwrap(), "{name}");
        assert_eq!(
            two.unpack_commands().unwrap(),
            old.unpack_commands().unwrap(),
            "{name}"
        );
    }
}

#[test]
fn a_schema_1_file_of_each_type_still_loads() {
    let v1 = tree("v1");
    for case in run_cases("v1") {
        let run = read_run(&case);
        assert_eq!(run.setup.schema_version, 1);
        let content = case_content(&case).unwrap();
        assert!(verify_run_with(&run.to_bytes(), &content).is_ok());
    }
    let dir = v1.join("stages/clear");
    let stage = fs::read(dir.join("stage.json")).unwrap();
    assert!(replay::verify(&stage, &fs::read(dir.join("run.replay")).unwrap()).is_ok());
    let pack = case_pack(&v1.join("runs/trial")).unwrap();
    assert_eq!(
        pack.sha256(),
        Pack::from_bytes(
            &fs::read(v1.join("packs").join(format!(
                "{}.json",
                stella_rain_core::content_hash::to_hex(&pack.sha256())
            )))
            .unwrap_or_else(|_| Pack::BUNDLED.to_vec())
        )
        .unwrap()
        .sha256()
    );
}

#[test]
fn resume_from_a_schema_2_replay_works() {
    let case = tree("v1").join("runs/stage_skill");
    let old = read_run(&case);
    let pack = case_pack(&case).unwrap();
    let mut whole = RunRecorder::start_with_pack(&old.setup, &pack).unwrap();
    let mut cut = None;
    let mut ticks = 0;
    for command in commands_of(&old) {
        match command {
            Command::Step(input) => {
                whole.step(input).unwrap();
                ticks += 1;
                if ticks == old.ticks / 2 {
                    cut = Some(whole.clone());
                }
            }
            Command::Choose(i) => whole.choose(i).unwrap(),
        }
    }
    let prefix = cut.unwrap().replay();
    assert_eq!(prefix.setup.schema_version, 2);
    let mut resumed = RunRecorder::resume_with_pack(&prefix, &pack).unwrap();
    let mut seen = 0;
    for command in commands_of(&old) {
        match command {
            Command::Step(input) => {
                seen += 1;
                if seen > prefix.ticks {
                    resumed.step(input).unwrap();
                }
            }
            Command::Choose(i) => {
                if seen >= prefix.ticks {
                    resumed.choose(i).unwrap();
                }
            }
        }
    }
    assert_eq!(resumed.replay(), whole.replay());
}

#[test]
fn a_saved_header_carries_the_written_schema() {
    let rec = RunRecorder::start(&RunSetup::default()).unwrap();
    let bytes = rec.save();
    assert_eq!(bytes[12..16], 2u32.to_le_bytes());
    assert_eq!(
        Savepoint::from_bytes(&bytes)
            .unwrap()
            .header()
            .schema_version,
        2
    );
}
