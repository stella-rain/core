//! The `replay_load` fuzz regression for an equipment action (ADR-SVC-1): the `runs/tutorial`
//! run replay with a Replace(0, 0) step inserted at tick 0, so it loads and ranges but does not
//! verify. It carries the pack pin of that run, so it moves with every corpus or pack bless:
//! `cargo xtask bless` writes it (`UPDATE_FUZZ_REGRESSION=1`) and `--check` compares it.
//! `split-input-run.replay` beside it is hand-made and not a generated file.

use std::path::PathBuf;
use stella_rain_core::RangeStep;
use stella_rain_core::run_replay::{RunReplay, RunReplayError, verify_run};

fn path() -> PathBuf {
    crate::data_root::data_root().join("fuzz/regressions/replay_load/equipment-action.replay")
}

/// The regression derived from a run replay: the step log with `[0x04, 0x02, 0x00, 0x00]`
/// spliced in at index 1.
fn derive(run_replay: &[u8]) -> Vec<u8> {
    let mut run = RunReplay::from_bytes(run_replay).expect("a run");
    let mut log = stella_rain_core::base64::decode(&run.inputs).expect("the step log");
    log.splice(1..1, [0x04, 0x02, 0x00, 0x00]);
    run.inputs = stella_rain_core::base64::encode(&log);
    run.to_bytes()
}

fn derived() -> Vec<u8> {
    let source =
        crate::data_root::data_root().join("tests/corpus/current/runs/tutorial/run.replay");
    derive(&std::fs::read(source).expect("tests/corpus/current/runs/tutorial/run.replay"))
}

/// `Ok` when `file` is the derived bytes; the error names the fix.
fn check(file: &[u8], derived: &[u8]) -> Result<(), String> {
    if file == derived {
        Ok(())
    } else {
        Err(format!(
            "fuzz/regressions/replay_load/equipment-action.replay is stale ({} bytes, derived {}): \
             run `cargo xtask bless`",
            file.len(),
            derived.len()
        ))
    }
}

#[expect(clippy::disallowed_methods, reason = "test knob")]
fn updating() -> bool {
    std::env::var("UPDATE_FUZZ_REGRESSION").as_deref() == Ok("1")
}

#[test]
fn a_replay_with_an_equipment_action_loads_ranges_and_is_refused() {
    let bytes = derived();
    let run = RunReplay::from_bytes(&bytes).expect("it loads");
    let range = run.step_range(0, run.ticks, 10_000_000).expect("it ranges");
    assert!(matches!(
        range.steps.first(),
        Some(RangeStep::Actions { .. })
    ));
    assert!(matches!(
        verify_run(&bytes),
        Err(RunReplayError::ActionRefused { tick: 0, .. })
    ));
}

#[test]
fn a_file_that_is_not_the_derived_bytes_is_stale() {
    let good = derived();
    assert_eq!(check(&good, &good), Ok(()));
    let mut bad = good.clone();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    let err = check(&bad, &good).unwrap_err();
    assert!(err.contains("cargo xtask bless"), "{err}");
    assert!(check(&good[..good.len() - 1], &good).is_err());
}

/// `UPDATE_FUZZ_REGRESSION=1` writes the file; otherwise the file is compared with the derivation.
#[test]
fn the_equipment_action_regression_is_as_blessed() {
    let derived = derived();
    if updating() {
        std::fs::write(path(), &derived).expect("the regression file is writable");
    }
    let file = std::fs::read(path()).unwrap_or_default();
    if let Err(e) = check(&file, &derived) {
        panic!("{e}");
    }
}
