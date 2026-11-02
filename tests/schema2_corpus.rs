//! The schema 2 cases of `tests/corpus/current/` are their schema 1 forms (ADR-REPL-7,
//! ADR-REPL-9, ADR-REPL-10): rebuilt from the solo views with the schema 1 packers, each case
//! verifies and reaches the same ticks, final hash, outcome and command count. The simulation
//! did not move; only the file format did.

use crate::common::schema_1_form;
use crate::corpus::case_content;
use crate::data_root::data_root;
use std::fs;
use std::path::PathBuf;
use stella_rain_core::replay::{self, Replay, pack_inputs};
use stella_rain_core::run_replay::{RunReplay, verify_run_with};

fn cases(kind: &str) -> Vec<PathBuf> {
    let dir = data_root().join("tests/corpus/current").join(kind);
    let mut cases: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    cases.sort();
    cases
}

#[test]
fn schema_2_cases_are_their_schema_1_forms() {
    let runs = cases("runs");
    assert!(runs.len() >= 9);
    for case in runs {
        let name = case.display();
        let two = RunReplay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
        assert_eq!(two.setup.schema_version, 2, "{name}");
        assert_eq!(two.commands, "", "{name}");
        let one = schema_1_form(&two);
        assert_eq!(one.setup.schema_version, 1, "{name}");
        let content = case_content(&case).unwrap();
        let (a, b) = (
            verify_run_with(&two.to_bytes(), &content).unwrap(),
            verify_run_with(&one.to_bytes(), &content).unwrap(),
        );
        assert_eq!(
            (a.ticks, a.final_hash, a.outcome, a.commands, a.stardust),
            (b.ticks, b.final_hash, b.outcome, b.commands, b.stardust),
            "{name}"
        );
        assert_eq!(
            (&one.ticks, &one.final_hash, &one.checkpoints),
            (&two.ticks, &two.final_hash, &two.checkpoints),
            "{name}"
        );
    }
    let stages = cases("stages");
    assert!(stages.len() >= 16);
    for case in stages {
        let name = case.display();
        let stage = fs::read(case.join("stage.json")).unwrap();
        let two = Replay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
        assert_eq!(two.schema_version, 2, "{name}");
        let mut one = two.clone();
        one.schema_version = 1;
        one.inputs = pack_inputs(&two.unpack_inputs(u32::MAX).unwrap());
        // A stage that is not cleared (`fail`, `timeout`) is refused the same way in both forms.
        let (a, b) = (
            replay::verify(&stage, &two.to_bytes()),
            replay::verify(&stage, &one.to_bytes()),
        );
        // The content hash covers the replay file's bytes, so it differs; the rest is equal.
        let same = |r: &Result<replay::Verified, _>| match r {
            Ok(v) => format!("{} {} {:?}", v.ticks, v.final_hash, v.parsed_stage_hash),
            Err(e) => format!("{e:?}"),
        };
        assert_eq!(same(&a), same(&b), "{name}");
    }
}
