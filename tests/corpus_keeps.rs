//! What a re-blessed `current/` case must keep (ADR-SIM-3, ADR-REPL-2): the hashes of the
//! `current/` cases are re-recorded when the hash layout moves, but the run they record is the
//! same run. The table is written by hand from the tree before the layout move; the bless never
//! writes it. `gameplay_observables::PINS` guard the same runs by digest, this table says which
//! of the four moved: the ticks played, the outcome, the rewards taken (and the stars named) and
//! the Stardust.

use std::fmt::Write as _;
use std::path::Path;

use stella_rain_core::engine::Engine;
use stella_rain_core::replay::Replay;
use stella_rain_core::run_replay::RunReplay;
use stella_rain_core::validate::parse_and_validate;

use crate::common::run_steps::{self, RunSteps};
use crate::corpus::current_dir;
use crate::corpus_pins::{case_dirs, case_pack, run_case_pack};

/// One case: name, ticks played, `Debug` of the outcome, the rewards taken as `id=count` and the
/// star names in event order, the Stardust (times 10).
type Keep = (&'static str, u32, &'static str, &'static str, u32);

const KEEPS: &[Keep] = &[
    ("runs/easy", 6000, "Running", "|", 12840),
    ("runs/elites", 48700, "Failed", "|", 880),
    ("runs/rainy", 4484, "Failed", "|", 23740),
    ("runs/stage_skill", 5400, "Failed", "|", 0),
    ("runs/time_limit", 17871, "Failed", "|", 29160),
    ("runs/time_limit_kill", 1810, "Cleared", "|", 0),
    ("runs/time_limit_overtime", 600, "Failed", "|", 0),
    ("runs/trial", 2706, "Failed", "|", 0),
    ("runs/trial_kinds", 4842, "Cleared", "|", 0),
    ("runs/tutorial", 4135, "Cleared", "|", 0),
    ("runs/tutorial_opening", 240, "Running", "|", 0),
    ("stages/agents", 80, "Running", "|", 0),
    ("stages/assist", 360, "Running", "|", 0),
    ("stages/boss", 68, "Cleared", "|", 0),
    ("stages/clear", 35, "Cleared", "|", 0),
    ("stages/dash", 300, "Running", "|", 0),
    ("stages/evade", 300, "Failed", "|", 0),
    ("stages/fail", 16, "Failed", "|", 0),
    ("stages/landscape", 300, "Failed", "|", 0),
    ("stages/leash_edge", 300, "Running", "|", 0),
    ("stages/melee", 300, "Running", "|", 0),
    ("stages/orb_cage", 300, "Running", "|", 0),
    ("stages/orbs", 300, "Running", "|", 0),
    ("stages/ranged_cap", 300, "Running", "|", 0),
    ("stages/timeout", 90, "Failed", "|", 0),
    ("stages/warn_anchor", 300, "Running", "|", 0),
    ("stages/zones", 300, "Running", "|", 0),
];

fn observed(engine: &Engine, inputs: usize) -> (u32, String, String, u32) {
    let tick = engine.tick();
    assert_eq!(
        tick as usize, inputs,
        "the engine played every recorded tick"
    );
    let tally = engine.tally();
    let mut picks = String::new();
    for (id, n) in tally.taken() {
        write!(picks, "{id}={n};").unwrap();
    }
    picks.push('|');
    for name in tally.named() {
        write!(picks, "{name};").unwrap();
    }
    (
        tick,
        format!("{:?}", engine.outcome()),
        picks,
        engine.stardust(),
    )
}

fn stage_row(case: &Path) -> (u32, String, String, u32) {
    let name = case.file_name().unwrap().to_string_lossy().into_owned();
    let stage_bytes = std::fs::read(case.join("stage.json")).expect("stage.json");
    let stage = parse_and_validate(&stage_bytes).expect("the stage parses");
    let recorded = Replay::from_bytes(&std::fs::read(case.join("run.replay")).expect("run.replay"))
        .expect("the replay parses");
    let inputs = recorded.unpack_inputs(stage.length_ticks).expect("inputs");
    let pack = case_pack(case).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut engine = Engine::new_with(&stage, pack.content()).expect("the stage starts");
    for input in &inputs {
        engine.step(&[*input, Default::default(), Default::default()]);
    }
    observed(&engine, inputs.len())
}

fn run_row(case: &Path) -> (u32, String, String, u32) {
    let name = case.file_name().unwrap().to_string_lossy().into_owned();
    let bytes = std::fs::read(case.join("run.replay")).expect("run.replay");
    let recorded = RunReplay::from_bytes(&bytes).expect("the run replay parses");
    let pack = run_case_pack(case, &bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut engine = run_steps::start(&recorded, pack.content());
    let steps = RunSteps::of(&recorded);
    for step in steps.steps() {
        run_steps::apply(&mut engine, step);
    }
    observed(&engine, steps.ticks as usize)
}

fn rows() -> Vec<(String, (u32, String, String, u32))> {
    let mut out = Vec::new();
    for (sub, row) in [
        ("stages", stage_row as fn(&Path) -> _),
        ("runs", run_row as fn(&Path) -> _),
    ] {
        let dirs = case_dirs(&current_dir().join(sub));
        assert!(!dirs.is_empty(), "no cases in {sub}");
        std::thread::scope(|scope| {
            let handles: Vec<_> = dirs
                .iter()
                .map(|dir| {
                    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
                    (format!("{sub}/{name}"), scope.spawn(move || row(dir)))
                })
                .collect();
            for (name, h) in handles {
                out.push((name, h.join().expect("the case runs")));
            }
        });
    }
    out
}

#[test]
fn every_current_case_keeps_its_ticks_outcome_picks_and_stardust() {
    let mut wrong = String::new();
    for (name, (ticks, outcome, picks, dust)) in rows() {
        let computed = format!("    ({name:?}, {ticks}, {outcome:?}, {picks:?}, {dust}),\n");
        match KEEPS.iter().find(|k| k.0 == name) {
            Some(&(_, t, o, p, d)) => {
                if (t, o, p, d) != (ticks, outcome.as_str(), picks.as_str(), dust) {
                    let _ = write!(wrong, "{name}: moved, computed\n{computed}");
                }
            }
            None => {
                let _ = write!(wrong, "{name}: no row, computed\n{computed}");
            }
        }
    }
    assert!(wrong.is_empty(), "a case moved or has no row:\n{wrong}");
}

#[test]
fn every_keep_names_a_case() {
    let mut names = Vec::new();
    for sub in ["stages", "runs"] {
        for dir in case_dirs(&current_dir().join(sub)) {
            names.push(format!(
                "{sub}/{}",
                dir.file_name().unwrap().to_string_lossy()
            ));
        }
    }
    for k in KEEPS {
        assert!(names.iter().any(|n| n == k.0), "{} is not a case", k.0);
    }
    assert_eq!(KEEPS.len(), names.len(), "one row per case");
}
