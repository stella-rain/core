//! Version 2 is the open live version (ADR-REPL-9): a run of `sim_version` 2 runs the live
//! rules, saves and restores as any run does, never crosses versions through a savepoint, and
//! the frozen version 1 cases replay with the same hashes when their files say 2 (ADR-REPL-7).

use crate::common::run::{time_limit_setup, trial_setup};
use crate::corpus::{check_case, check_run_case, current_dir};
use crate::corpus_pins::check_frozen_tree;
use crate::run_play::play_run_from;
use std::fs;
use std::path::{Path, PathBuf};
use stella_rain_core::content_hash::sha256;
use stella_rain_core::pack::Pack;
use stella_rain_core::run_replay::{Command, RunRecorder};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::savepoint::{HEADER_LEN, Savepoint, SavepointError};
use stella_rain_core::version::{FIRST_SIM_VERSION, SIM_VERSION};

const SIM_VERSION_AT: usize = 8;
const CHECKSUM_AT: usize = 96;

#[test]
fn the_open_version_is_2() {
    assert_eq!(SIM_VERSION, 2);
    assert_eq!(FIRST_SIM_VERSION, 1);
    assert_eq!(RunSetup::default().sim_version, SIM_VERSION);
    assert_eq!(Pack::bundled().sim_version(), SIM_VERSION);
}

fn apply(rec: &mut RunRecorder, commands: &[Command]) {
    for c in commands {
        match *c {
            Command::Step(input) => rec.step(input).unwrap(),
            Command::Choose(index) => rec.choose(index).unwrap(),
        }
    }
}

fn recorded(setup: &RunSetup, ticks: u32) -> RunRecorder {
    let mut rec = RunRecorder::start_with_pack(setup, Pack::bundled()).unwrap();
    apply(&mut rec, &play_run_from(setup, ticks));
    rec
}

/// Saves a run of `setup` at tick 120, restores it, and plays both on.
fn round_trip(setup: &RunSetup, version: u32) {
    let mut rec = recorded(setup, 120);
    let played = play_run_from(setup, 120).len();
    let bytes = rec.save();
    assert_eq!(
        Savepoint::from_bytes(&bytes).unwrap().header().sim_version,
        version
    );
    let mut back =
        RunRecorder::resume_from_savepoint(&bytes, &rec.replay(), Pack::bundled()).unwrap();
    assert_eq!(back.replay().final_hash, rec.replay().final_hash);
    let more = play_run_from(setup, 180);
    for rec in [&mut rec, &mut back] {
        apply(rec, &more[played..]);
    }
    assert_eq!(back.replay().final_hash, rec.replay().final_hash);
}

#[test]
fn a_version_2_run_saves_and_restores() {
    round_trip(&trial_setup(), 2);
    round_trip(&time_limit_setup(), 2);
}

#[test]
fn a_version_1_run_still_saves_and_restores() {
    for setup in [trial_setup(), time_limit_setup()] {
        round_trip(
            &RunSetup {
                sim_version: 1,
                ..setup
            },
            1,
        );
    }
}

fn reseal(bytes: &mut [u8]) {
    let mut sealed = bytes[..CHECKSUM_AT].to_vec();
    sealed.extend_from_slice(&bytes[HEADER_LEN..]);
    bytes[CHECKSUM_AT..CHECKSUM_AT + 32].copy_from_slice(&sha256(&sealed));
}

#[test]
fn a_savepoint_never_crosses_versions() {
    let at = |v| {
        recorded(
            &RunSetup {
                sim_version: v,
                ..trial_setup()
            },
            120,
        )
    };
    let (one, two) = (at(1), at(2));
    for (rec, other) in [(&one, &two), (&two, &one)] {
        let got = RunRecorder::resume_from_savepoint(&rec.save(), &other.replay(), Pack::bundled());
        assert_eq!(got.err(), Some(SavepointError::SimVersion));
    }
    for version in [0, SIM_VERSION + 1] {
        let mut bytes = two.save();
        bytes[SIM_VERSION_AT..SIM_VERSION_AT + 4].copy_from_slice(&version.to_le_bytes());
        reseal(&mut bytes);
        let got = RunRecorder::resume_from_savepoint(&bytes, &two.replay(), Pack::bundled());
        assert_eq!(got.err(), Some(SavepointError::SimVersion), "{version}");
    }
}

fn dirs(parent: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(parent)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    v.sort();
    v
}

/// The `sim_version` a JSON file says, at the top (a stage, a stage replay) or in `setup`.
fn version_of(file: &Path) -> u64 {
    let v: serde_json::Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
    v.get("setup").unwrap_or(&v)["sim_version"]
        .as_u64()
        .unwrap()
}

#[test]
fn the_current_corpus_is_at_the_open_version() {
    let current = current_dir();
    let mut files = 0;
    for case in dirs(&current.join("stages")) {
        for file in ["stage.json", "run.replay"] {
            assert_eq!(version_of(&case.join(file)), 2, "{case:?} {file}");
            files += 1;
        }
    }
    for case in dirs(&current.join("runs")) {
        assert_eq!(version_of(&case.join("run.replay")), 2, "{case:?}");
        files += 1;
    }
    assert!(files > 0);
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.path().is_dir() {
            copy_tree(&e.path(), &target);
        } else {
            fs::copy(e.path(), target).unwrap();
        }
    }
}

/// `file` with its `"sim_version":1` written as 2 (compact JSON; one occurrence is the file's).
fn bump(file: &Path) {
    let text = fs::read_to_string(file).unwrap();
    assert!(text.contains("\"sim_version\":1"), "{file:?}");
    fs::write(
        file,
        text.replacen("\"sim_version\":1", "\"sim_version\":2", 1),
    )
    .unwrap();
}

#[test]
fn the_frozen_v1_corpus_replays_with_the_same_hashes_at_version_2() {
    let frozen = current_dir().parent().unwrap().join("v1");
    check_frozen_tree(&frozen).unwrap();
    let scratch = std::env::temp_dir().join(format!("sr-version-two-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let copy = scratch.join("v1");
    copy_tree(&frozen, &copy);
    let diff = scratch.join("diff");
    let mut failures = Vec::new();
    for case in dirs(&copy.join("stages")) {
        // The stage file keeps its bytes' layout: the version is the only edit.
        let stage = case.join("stage.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&stage).unwrap()).unwrap();
        value["sim_version"] = 2.into();
        fs::write(&stage, serde_json::to_vec(&value).unwrap()).unwrap();
        bump(&case.join("run.replay"));
        failures.extend(check_case(&case, &diff).err());
    }
    for case in dirs(&copy.join("runs")) {
        bump(&case.join("run.replay"));
        failures.extend(check_run_case(&case, &diff).err());
    }
    let _ = fs::remove_dir_all(&scratch);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // Nothing was written under the frozen tree.
    check_frozen_tree(&frozen).unwrap();
}

/// `before` and `after` of `tests/bench_pins.toml` at the bump's base: a version 2 run holds the
/// same states as version 1, so no bless moves them.
const STATES: [(&str, &str, &str); 6] = [
    ("worst_tick", "21735fa3b620d3f0", "7e6786c603ce48dc"),
    ("hit_tick", "c55604b22ad2cf8d", "fc6d79094e9ba2ec"),
    ("hit_tick_spread", "19b64d73364321ee", "234c6506597903e7"),
    ("time_limit_tick", "fc386858e6d3b4ac", "28fe9d13bb08ef77"),
    ("worst_minute", "6201306870d81bd7", "2f8a159160af42dd"),
    ("worst_minute_peak", "0242e84cedb6544c", "705d700709c5daed"),
];

#[test]
fn the_bench_pins_hold_the_same_states_at_version_2() {
    let pins: toml::Table = include_str!("bench_pins.toml").parse().unwrap();
    for (id, before, after) in STATES {
        let case = pins[id].as_table().unwrap();
        assert_eq!(case["before"].as_str(), Some(before), "{id}");
        assert_eq!(case["after"].as_str(), Some(after), "{id}");
    }
}
