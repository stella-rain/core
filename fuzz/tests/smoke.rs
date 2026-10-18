//! The fuzz targets on stable (ADR-020), every PR: each target reaches the code behind its first
//! gate on a real stage, replay, run replay, share code and run setup, rejects garbage, and survives a deterministic
//! sweep over those files: every prefix, and a flipped byte at a regular step. Crashes that a
//! real fuzz run found are kept in `regressions/<target>/` and replayed here.

use std::fs;
use std::path::{Path, PathBuf};

use stella_rain_core::share;
use stella_rain_core_fuzz::{replay_load, run_setup, share_decode, stage_parse, stage_run};

type Target = fn(&[u8]) -> bool;

const TARGETS: [(&str, Target); 5] = [
    ("stage_parse", stage_parse),
    ("stage_run", stage_run),
    ("replay_load", replay_load),
    ("share_decode", share_decode),
    ("run_setup", run_setup),
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// `(case, stage.json, run.replay)` of every corpus case.
fn corpus() -> Vec<(String, Vec<u8>, Vec<u8>)> {
    let dir = root().join("tests/corpus/v0");
    let mut cases: Vec<_> = fs::read_dir(&dir)
        .expect("the corpus directory")
        .map(|e| e.expect("a corpus entry").path())
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "the corpus has no cases");
    cases
        .into_iter()
        .map(|case| {
            let name = case.file_name().unwrap().to_string_lossy().into_owned();
            let stage = fs::read(case.join("stage.json")).expect("stage.json");
            let replay = fs::read(case.join("run.replay")).expect("run.replay");
            (name, stage, replay)
        })
        .collect()
}

/// The `run.replay` of every run case of the corpus (ADR-051), which `replay_load` also loads.
fn run_replays() -> Vec<Vec<u8>> {
    let dir = root().join("tests/corpus/runs/v0");
    let mut cases: Vec<_> = fs::read_dir(&dir)
        .expect("the run corpus directory")
        .map(|e| e.expect("a run case").path())
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "the corpus has no run cases");
    cases
        .iter()
        .map(|case| fs::read(case.join("run.replay")).expect("run.replay"))
        .collect()
}

fn clear() -> (Vec<u8>, Vec<u8>) {
    let (_, stage, replay) = corpus().into_iter().find(|c| c.0 == "clear").unwrap();
    (stage, replay)
}

#[test]
fn a_real_stage_gets_past_the_parser_and_runs() {
    let (stage, _) = clear();
    assert!(stage_parse(&stage), "stage_parse rejects the clear stage");
    assert!(stage_run(&stage), "stage_run rejects the clear stage");
}

#[test]
fn a_real_replay_verifies() {
    let (_, replay) = clear();
    assert!(
        replay_load(&replay),
        "replay_load does not verify the clear replay"
    );
}

#[test]
fn a_real_run_replay_verifies() {
    for replay in run_replays() {
        assert!(
            replay_load(&replay),
            "replay_load does not verify a run replay"
        );
    }
}

#[test]
fn a_real_share_code_decodes() {
    let (stage, replay) = clear();
    let code = share::encode(&stage, &replay).expect("the clear case encodes");
    assert!(
        share_decode(code.as_bytes()),
        "share_decode rejects a share code"
    );
}

/// The run setups the `run_setup` target is seeded with (CI copies the same directory).
fn setups() -> Vec<Vec<u8>> {
    let dir = root().join("tests/fixtures/run_setup");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .expect("the run setup seeds")
        .map(|e| e.expect("a seed").path())
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no run setup seeds");
    files.iter().map(|f| fs::read(f).expect("a seed")).collect()
}

#[test]
fn a_real_setup_gets_past_the_parser_and_runs() {
    for setup in setups() {
        assert!(run_setup(&setup), "run_setup rejects a seed");
    }
}

#[test]
fn garbage_is_rejected_without_a_panic() {
    let long_run = [b'{'; 5000];
    let garbage: [&[u8]; 8] = [
        b"",
        b"{",
        b"null",
        b"[]",
        b"\xff\xfe\x00",
        b"{\"schema_version\":0,\"sim_version\":0}",
        b"AAAA",
        &long_run,
    ];
    for (name, target) in TARGETS {
        for bytes in garbage {
            assert!(
                !target(bytes),
                "{name} accepted {:?}",
                String::from_utf8_lossy(bytes)
            );
        }
    }
}

/// The files each target is given: stages for the stage targets, and for `stage_parse` also the
/// bundled rewards (with their stage skills) and run stats, which it reads too; stage and run
/// replays for the replay target, share codes (of the corpus pairs) for the share target and
/// run setups for the setup target.
fn seeds(target: &str) -> Vec<Vec<u8>> {
    if target == "run_setup" {
        return setups();
    }
    let mut seeds: Vec<Vec<u8>> = corpus()
        .into_iter()
        .flat_map(|(_, stage, replay)| match target {
            "stage_parse" | "stage_run" => vec![stage],
            "replay_load" => vec![replay],
            _ => share::encode(&stage, &replay)
                .map(String::into_bytes)
                .into_iter()
                .collect(),
        })
        .collect();
    if target == "replay_load" {
        seeds.extend(run_replays());
    }
    if target == "stage_parse" {
        for file in [
            "content/rewards.json",
            "content/run_stats.json",
            "content/stage_skills.json",
        ] {
            seeds.push(fs::read(root().join(file)).expect("bundled content"));
        }
    }
    seeds
}

#[test]
fn every_prefix_and_flipped_byte_of_the_seeds_is_survived() {
    for (name, target) in TARGETS {
        for seed in seeds(name) {
            // About 150 steps through each seed keep the test to a few seconds; the shortest
            // prefixes are all tried, because the parsers' error paths are there.
            let step = (seed.len() / 150).max(1);
            for len in (0..seed.len()).filter(|n| *n < 64 || n % step == 0) {
                target(&seed[..len]);
            }
            for at in (0..seed.len()).step_by(step) {
                for flip in [0x01, 0x20, 0x80, 0xff] {
                    let mut bytes = seed.clone();
                    bytes[at] ^= flip;
                    target(&bytes);
                }
            }
        }
    }
}

#[test]
fn the_regressions_of_real_fuzz_runs_are_survived() {
    for (name, target) in TARGETS {
        let Ok(dir) = fs::read_dir(root().join("fuzz/regressions").join(name)) else {
            continue;
        };
        for file in dir {
            let path = file.expect("a regression entry").path();
            target(&fs::read(&path).expect("a regression file"));
            println!("{name}: replayed {}", path.display());
        }
    }
}
