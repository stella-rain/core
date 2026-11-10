//! The range reader of the step log (ADR-REPL-5): the steps of world ticks `a..b`, cut from the
//! corpus cases of both schemas, refused like the full decoder refuses, and never sized by the
//! claimed tick count (ADR-SVC-1).

use crate::alloc::{live_bytes, peak_bytes, reset_peak};
use crate::common::schema_1_form;
use crate::data_root::data_root;
use std::fs;
use stella_rain_core::base64;
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::run_replay::{RunReplay, RunReplayError};
use stella_rain_core::{RangeStep, StepRange, read_step_range};

const MAX: u32 = u32::MAX;

fn names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(data_root().join("tests/corpus/current/runs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn replay(tree: &str, name: &str) -> RunReplay {
    let path = data_root()
        .join("tests/corpus")
        .join(tree)
        .join("runs")
        .join(name)
        .join("run.replay");
    RunReplay::from_bytes(&fs::read(path).unwrap()).unwrap()
}

fn raw_of(replay: &RunReplay) -> Vec<u8> {
    base64::decode(&replay.inputs).unwrap()
}

#[derive(Debug, PartialEq)]
enum Entry {
    Tick(Vec<Input>),
    Picks(Vec<Option<u32>>),
    Actions(Vec<Option<EquipAction>>),
}

fn expand(steps: &[RangeStep]) -> Vec<Entry> {
    let mut out = Vec::new();
    for step in steps {
        match step {
            RangeStep::Actions { actions } => out.push(Entry::Actions(actions.clone())),
            RangeStep::Ticks { run, inputs } => {
                out.extend((0..*run).map(|_| Entry::Tick(inputs.clone())));
            }
            RangeStep::Picks { picks } => out.push(Entry::Picks(picks.clone())),
            RangeStep::Join { .. } | RangeStep::Leave { .. } => {
                unreachable!("these logs hold no join or leave")
            }
        }
    }
    out
}

/// The oracle: the whole log, one entry per tick, the picks by position, cut to `a..b`: a pick
/// after `p` ticks is in when `a <= p < b`, and a range ending at `ticks` takes the trailing
/// picks; an empty range holds nothing.
fn oracle(full: &StepRange, ticks: u32, a: u32, b: u32) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut world = 0u32;
    for step in &full.steps {
        match step {
            RangeStep::Actions { actions } => {
                if (a <= world && world < b) || (a < b && b == ticks && world == ticks) {
                    out.push(Entry::Actions(actions.clone()));
                }
            }
            RangeStep::Ticks { run, inputs } => {
                for t in world..world + run {
                    if a <= t && t < b {
                        out.push(Entry::Tick(inputs.clone()));
                    }
                }
                world += run;
            }
            RangeStep::Picks { picks } => {
                if (a <= world && world < b) || (a < b && b == ticks && world == ticks) {
                    out.push(Entry::Picks(picks.clone()));
                }
            }
            RangeStep::Join { .. } | RangeStep::Leave { .. } => {
                unreachable!("these logs hold no join or leave")
            }
        }
    }
    out
}

fn ranges(ticks: u32, seed: u64) -> Vec<(u32, u32)> {
    let mut out = vec![
        (0, 0),
        (0, ticks),
        (ticks, ticks),
        (ticks - 1, ticks),
        (0, 1),
    ];
    let mut k = 0;
    while k < ticks {
        out.push((k, (k + 60).min(ticks)));
        k += 60;
    }
    let mut state = seed | 1;
    for _ in 0..200 {
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            u32::try_from(state % (u64::from(ticks) + 1)).unwrap()
        };
        let (x, y) = (next(), next());
        out.push((x.min(y), x.max(y)));
    }
    out
}

#[test]
fn every_corpus_case_reads_like_its_oracle_at_both_schemas() {
    let (mut same_runs, mut frozen_read) = (0, 0);
    for name in names() {
        let two = replay("current", &name);
        let ticks = two.ticks;
        let full = read_step_range(&raw_of(&two), ticks, MAX, 0, ticks).unwrap();
        // A case recorded after `v1/` was frozen has no frozen file: it is read at the current
        // schema only.
        let frozen = data_root()
            .join("tests/corpus/v1/runs")
            .join(&name)
            .join("run.replay")
            .exists();
        let one = frozen.then(|| replay("v1", &name));
        let mut same_run = false;
        if let Some(one) = &one {
            assert_eq!(one.setup.schema_version, 1, "{name}");
            frozen_read += 1;
            // The frozen file holds the run as version 1 recorded it. Where the current run is
            // another one (the 64 px pickup circle moved its picks and inputs), the cross-schema
            // equality has nothing to compare; the frozen file is then read against its own whole
            // log, so that every case is still read at both schemas.
            let old_full = one.step_range(0, one.ticks, MAX).unwrap();
            same_run = one.ticks == ticks && old_full == full;
            same_runs += usize::from(same_run);
            if !same_run {
                for (a, b) in ranges(one.ticks, 0x2545_f491_4f6c_dd1d) {
                    let old = one.step_range(a, b, MAX).unwrap();
                    assert_eq!(
                        expand(&old.steps),
                        oracle(&old_full, one.ticks, a, b),
                        "{name} {a}..{b} at schema 1, against its own log"
                    );
                }
            }
        }
        for (a, b) in ranges(ticks, 0x2545_f491_4f6c_dd1d) {
            let got = two.step_range(a, b, MAX).unwrap();
            assert_eq!(
                expand(&got.steps),
                oracle(&full, ticks, a, b),
                "{name} {a}..{b}"
            );
            // The rule is on ticks, not on how the file is stored.
            if same_run {
                let old = one.as_ref().unwrap().step_range(a, b, MAX).unwrap();
                assert_eq!(old, got, "{name} {a}..{b} at schema 1");
            }
        }
        assert_eq!(two.step_range(0, ticks, MAX).unwrap(), full, "{name}");
    }
    // Every current case is an equipment run, which is never its frozen offers file; each of the
    // 9 frozen files was read against its own log.
    assert_eq!(
        same_runs, 0,
        "every current case is an equipment run, which is never its frozen offers file"
    );
    assert_eq!(
        frozen_read, 9,
        "the frozen files read against their own log"
    );
}

#[test]
fn an_equipment_run_reads_like_its_oracle() {
    let two = crate::run_play::equipment_rig(1).play().replay();
    let ticks = two.ticks;
    let full = read_step_range(&raw_of(&two), ticks, MAX, 0, ticks).unwrap();
    let acts = |e: &[Entry]| e.iter().filter(|e| matches!(e, Entry::Actions(_))).count();
    assert!(acts(&expand(&full.steps)) > 0, "the rig run holds actions");
    for (a, b) in ranges(ticks, 0x2545_f491_4f6c_dd1d) {
        let got = two.step_range(a, b, MAX).unwrap();
        assert_eq!(expand(&got.steps), oracle(&full, ticks, a, b), "{a}..{b}");
    }
}

#[test]
fn a_schema_1_form_of_a_case_reads_the_same_ranges() {
    // An equipment case has no schema 1 form: the offers-era trial run, recorded here, has one.
    let two = crate::corpus::offers_trial_run();
    let picks = crate::common::run_steps::RunSteps::of(&two)
        .steps()
        .iter()
        .filter(|s| matches!(s, crate::common::run_steps::Step::Picks(_)))
        .count();
    assert!(picks > 0, "the run holds PICKS steps");
    let one = schema_1_form(&two);
    for (a, b) in ranges(two.ticks, 7) {
        assert_eq!(
            one.step_range(a, b, MAX).unwrap(),
            two.step_range(a, b, MAX).unwrap(),
            "{a}..{b}"
        );
    }
}

#[test]
fn the_replay_entry_refuses_what_the_log_refuses() {
    let two = replay("current", "easy");
    let mut commands = two.clone();
    commands.commands = "AA".to_owned();
    assert_eq!(
        commands.step_range(0, 1, MAX),
        Err(RunReplayError::BadCommands("schema 2 has no command list"))
    );
    let mut text = two.clone();
    text.inputs = "!!!".to_owned();
    assert_eq!(
        text.step_range(0, 1, MAX),
        Err(RunReplayError::BadInputs("not valid base64"))
    );
    assert_eq!(
        two.step_range(0, two.ticks + 1, MAX),
        Err(RunReplayError::BadInputs("a tick range past the last tick"))
    );
    assert_eq!(
        two.step_range(2, 1, MAX),
        Err(RunReplayError::BadInputs("a tick range that is backwards"))
    );
    assert!(matches!(
        two.step_range(0, 1, two.ticks - 1),
        Err(RunReplayError::TooManyTicks { .. })
    ));
}

/// The most the thread's heap rose above its start while `read` ran, and what it returned.
fn rise<T>(read: impl FnOnce() -> T) -> (i64, T) {
    let start = live_bytes();
    reset_peak();
    let out = read();
    (peak_bytes() - start, out)
}

#[test]
fn a_huge_claim_allocates_nothing_from_it() {
    let log = [0x01, 0x00, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x02, 0x00, 0x00];
    let held = Input {
        main: true,
        ..Input::default()
    };
    for (a, b, run) in [
        (0, 10, 10),
        (4_000_000_000, 4_000_000_010, 10),
        (0, MAX, MAX),
    ] {
        let (cost, got) = rise(|| read_step_range(&log, MAX, MAX, a, b));
        assert_eq!(
            got.unwrap().steps,
            [RangeStep::Ticks {
                run,
                inputs: vec![held]
            }]
        );
        assert!(cost < 4096, "{a}..{b} rose by {cost}");
    }
    // A run of `u32::MAX - 1`, then one idle tick.
    let log = [
        0x01, 0x00, 0xfe, 0xff, 0xff, 0xff, 0x0f, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    ];
    for (a, b, steps) in [(0, 10, 1), (MAX - 5, MAX, 2), (0, MAX, 2)] {
        let (cost, got) = rise(|| read_step_range(&log, MAX, MAX, a, b));
        assert_eq!(got.unwrap().steps.len(), steps, "{a}..{b}");
        assert!(cost < 4096, "{a}..{b} rose by {cost}");
    }
}

#[test]
fn the_memory_follows_the_range_not_the_log() {
    let (name, raw, ticks) = names()
        .into_iter()
        .map(|n| {
            let r = replay("current", &n);
            (n, raw_of(&r), r.ticks)
        })
        .max_by_key(|(_, raw, _)| raw.len())
        .unwrap();
    let (small, got) = rise(|| read_step_range(&raw, ticks, MAX, 0, 1));
    assert_eq!(got.unwrap().steps.len(), 1, "{name}");
    let (whole, _) = rise(|| read_step_range(&raw, ticks, MAX, 0, ticks));
    assert!(small < 4096, "{name}: 0..1 rose by {small}");
    assert!(small <= whole, "{name}: {small} over {whole}");
}
