//! The corpus files another repository reads. The `app` repository reads
//! `tests/corpus/current/runs/stage_skill/run.replay` from Cargo's checkout of core, so the path is a
//! contract: these tests fail when the file is renamed, moved or stops parsing, and when
//! `.claude/rules/replay.md` stops naming it. A new path the app reads is added to
//! `APP_READ_PATHS` and to that rule in the same change.

use std::fs;
use std::path::Path;

use stella_rain_core::run_replay::RunReplay;

/// Paths relative to the repository root.
const APP_READ_PATHS: &[&str] = &["tests/corpus/current/runs/stage_skill/run.replay"];

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn the_files_another_repository_reads_exist() {
    for path in APP_READ_PATHS {
        let bytes = fs::read(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert!(!bytes.is_empty(), "{path} is empty");
    }
}

#[test]
fn they_parse_as_run_replays() {
    for path in APP_READ_PATHS {
        let bytes = fs::read(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        if let Err(e) = RunReplay::from_bytes(&bytes) {
            panic!("{path} does not parse as a run replay: {e:?}");
        }
    }
}

#[test]
fn the_rules_name_every_one_of_them() {
    let rules = fs::read_to_string(root().join(".claude/rules/replay.md")).unwrap();
    for path in APP_READ_PATHS {
        assert!(
            rules.contains(path),
            "`.claude/rules/replay.md` does not name {path}"
        );
    }
}
