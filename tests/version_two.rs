//! Version 2 is the open live version (ADR-REPL-9): a run of `sim_version` 2 runs the live
//! rules, saves and restores as any run does, never crosses versions through a savepoint, and
//! the frozen version 1 cases replay with the same events, outcomes and kills when their files say 2
//! (ADR-REPL-7); the state hashes differ, the layouts do.

use crate::common::run::{time_limit_setup, trial_setup};
use crate::corpus::{apply as apply_command, current_dir};
use crate::corpus_pins::{case_pack, check_frozen_tree, run_case_pack};
use crate::gameplay_observables::rewrite_ids;
use crate::run_play::play_run_from;
use std::fs;
use std::path::{Path, PathBuf};
use stella_rain_core::content_hash::sha256;
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::pack::Pack;
use stella_rain_core::replay::Replay;
use stella_rain_core::run_replay::{Command, RunRecorder, RunReplay};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::savepoint::{HEADER_LEN, Savepoint, SavepointError};
use stella_rain_core::validate::parse_and_validate;
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

/// Steps `a` (version 1) and `b` (version 2) alike and compares what they show after every
/// tick: the events (the version 2 numbers mapped back to the pre-reservation ones), the outcome
/// and the kills. The state hashes differ by design: the layouts do.
/// `events` without the players they name (`player`, `buffed_by`, `source`, `killer`), which only
/// the newest rules fill.
fn plain(events: &[DomainEvent]) -> Vec<DomainEvent> {
    let mut out = events.to_vec();
    for event in &mut out {
        match event {
            DomainEvent::Hit {
                player, buffed_by, ..
            } => (*player, *buffed_by) = (None, None),
            DomainEvent::Healed { source, .. } | DomainEvent::StatusApplied { source, .. } => {
                *source = None;
            }
            DomainEvent::Died { killer, .. } => *killer = None,
            _ => {}
        }
    }
    out
}

fn same_tick(a: &Engine, b: &Engine, what: &str) {
    let tick = a.tick();
    assert_eq!(b.tick(), tick, "{what}");
    assert_eq!(
        format!("{:?}", plain(a.events())),
        rewrite_ids(&format!("{:?}", plain(b.events()))),
        "{what}, tick {tick}: events"
    );
    assert_eq!(a.outcome(), b.outcome(), "{what}, tick {tick}: outcome");
    assert_eq!(
        a.tally().kills(),
        b.tally().kills(),
        "{what}, tick {tick}: kills"
    );
}

#[test]
fn the_frozen_v1_corpus_replays_the_same_at_version_2() {
    let frozen = current_dir().parent().unwrap().join("v1");
    check_frozen_tree(&frozen).unwrap();
    let (mut stages, mut runs) = (0, 0);
    for case in dirs(&frozen.join("stages")) {
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        let stage = parse_and_validate(&fs::read(case.join("stage.json")).unwrap()).unwrap();
        let mut newer = stage.clone();
        newer.sim_version = 2;
        let recorded = Replay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
        let inputs = recorded.unpack_inputs(stage.length_ticks).unwrap();
        let pack = case_pack(&case).unwrap();
        let mut a = Engine::new_with(&stage, pack.content()).unwrap();
        let mut b = Engine::new_with(&newer, pack.content()).unwrap();
        for input in &inputs {
            a.step(&[*input]);
            b.step(&[*input]);
            same_tick(&a, &b, &name);
        }
        assert_eq!(a.stardust(), b.stardust(), "{name}");
        stages += 1;
    }
    for case in dirs(&frozen.join("runs")) {
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = fs::read(case.join("run.replay")).unwrap();
        let recorded = RunReplay::from_bytes(&bytes).unwrap();
        let pack = run_case_pack(&case, &bytes).unwrap();
        let mut newer = recorded.setup.clone();
        newer.sim_version = 2;
        let mut a = Engine::start_with(&recorded.setup, pack.content()).unwrap();
        let mut b = Engine::start_with(&newer, pack.content()).unwrap();
        let commands = recorded.unpack_commands().unwrap();
        let mut next = commands.iter().peekable();
        for input in &recorded.unpack_inputs(u32::MAX).unwrap() {
            while let Some(c) = next.next_if(|c| c.tick == a.tick()) {
                apply_command(&mut a, c).unwrap();
                apply_command(&mut b, c).unwrap();
            }
            a.step(&[*input]);
            b.step(&[*input]);
            same_tick(&a, &b, &name);
        }
        assert_eq!(a.stardust(), b.stardust(), "{name}");
        runs += 1;
    }
    assert!(stages > 0 && runs > 0);
    // Nothing was written under the frozen tree.
    check_frozen_tree(&frozen).unwrap();
}
