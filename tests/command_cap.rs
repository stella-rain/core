//! The most commands the longest run takes, which `MAX_COMMANDS` is at least twice (ADR-REPL-5):
//! measured from the command lists of the frozen v1 run replays (`tests/corpus/v1/runs`), and the
//! cap itself, tried on a file with one command too many.

use std::fs;
use std::path::PathBuf;
use stella_rain_core::run_replay::{
    CommandKind, MAX_COMMANDS, RunReplay, RunReplayError, TickCommand, pack_commands,
    verify_run_with,
};

/// The most commands of a frozen v1 run (`tests/corpus/v1/runs`, `measure_the_longest_run`),
/// pinned: `MAX_COMMANDS` is at least twice it. 317 (`time_limit`); 400 before, when a bot
/// played the runs. The v1 folder is frozen, so this can only fall.
const MEASURED_MOST: usize = 317;

#[test]
fn the_cap_is_twice_the_longest_run_or_more() {
    assert!(
        2 * MEASURED_MOST <= MAX_COMMANDS as usize,
        "MAX_COMMANDS {MAX_COMMANDS} is under twice {MEASURED_MOST}"
    );
}

#[test]
fn a_crafted_replay_with_one_command_over_the_cap_is_refused() {
    // The frozen v1 `time_limit` run is schema 1, whose file holds a command list.
    let case = crate::data_root::data_root().join("tests/corpus/v1/runs/time_limit");
    let mut replay = RunReplay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
    let pack = crate::corpus_pins::case_pack(&case).unwrap();
    let content = pack.content();
    let choices = |n: u32| -> String {
        let commands: Vec<TickCommand> = (0..n)
            .map(|_| TickCommand {
                tick: 0,
                kind: CommandKind::Choose,
                index: 0,
            })
            .collect();
        pack_commands(&commands)
    };
    // At the cap the file loads (the choices are then refused by the engine for lack of an
    // offer, not for their number); one over it never gets that far.
    replay.commands = choices(MAX_COMMANDS);
    assert_eq!(
        RunReplay::from_bytes(&replay.to_bytes())
            .unwrap()
            .unpack_commands()
            .unwrap()
            .len(),
        MAX_COMMANDS as usize
    );
    replay.commands = choices(MAX_COMMANDS + 1);
    assert_eq!(
        verify_run_with(&replay.to_bytes(), content),
        Err(RunReplayError::TooManyCommands {
            limit: MAX_COMMANDS
        })
    );
}

/// The case folders of the frozen v1 runs, in name order.
fn v1_run_cases() -> Vec<PathBuf> {
    let mut cases: Vec<PathBuf> =
        fs::read_dir(crate::data_root::data_root().join("tests/corpus/v1/runs"))
            .expect("tests/corpus/v1/runs exists")
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_dir())
            .collect();
    cases.sort();
    assert!(
        cases.len() >= 5,
        "the v1 corpus has {} run cases, want at least 5",
        cases.len()
    );
    cases
}

/// The most commands of a frozen v1 run (`tests/corpus/v1/runs`), which a run's file holds as its
/// command list. Read from the files, so fast; `#[ignore]`d only because the deep tier runs it
/// by name: `cargo test --release --test integration command_cap:: -- --ignored --nocapture`.
#[test]
#[ignore = "the measurement of MAX_COMMANDS: the deep tier runs it by name"]
fn measure_the_longest_run() {
    let mut most = 0;
    let mut longest = None;
    for case in v1_run_cases() {
        let bytes = fs::read(case.join("run.replay")).unwrap();
        let n = RunReplay::from_bytes(&bytes)
            .unwrap()
            .unpack_commands()
            .unwrap()
            .len();
        println!(
            "case {}: {n} commands",
            case.file_name().unwrap().to_string_lossy()
        );
        if n > most {
            most = n;
            longest = Some((case, bytes));
        }
    }
    println!("most commands: {most}");
    assert!(
        most == MEASURED_MOST,
        "a run takes {most} commands: re-pin MEASURED_MOST"
    );
    assert!(
        2 * most <= MAX_COMMANDS as usize,
        "MAX_COMMANDS is under twice {most}"
    );
    // The longest run verifies from its file, on the pack its case pins.
    let (case, bytes) = longest.unwrap();
    let content = crate::corpus_pins::case_pack(&case).unwrap();
    let verified = verify_run_with(&bytes, content.content()).unwrap();
    assert_eq!(verified.commands as usize, most);
}
