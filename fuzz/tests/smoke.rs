//! The fuzz targets on stable (ADR-020), every PR: each target reaches the code behind its first
//! gate on a real stage, replay, run replay, share code, run setup and content pack, rejects
//! garbage, and survives a deterministic sweep over those files: every prefix, and a flipped
//! byte at a regular step. Crashes that a real fuzz run found are kept in
//! `regressions/<target>/` and replayed here.

use std::fs;
use std::path::{Path, PathBuf};

use stella_rain_core::pack;
use stella_rain_core::share;
use stella_rain_core_fuzz::{
    pack_load, packs_load, replay_load, run_setup, share_decode, stage_parse, stage_run,
};

type Target = fn(&[u8]) -> bool;

const TARGETS: [(&str, Target); 7] = [
    ("stage_parse", stage_parse),
    ("stage_run", stage_run),
    ("replay_load", replay_load),
    ("share_decode", share_decode),
    ("run_setup", run_setup),
    ("pack_load", pack_load),
    ("packs_load", packs_load),
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

/// The `run.replay` of every run case of the corpus (ADR-051) that plays with the bundled
/// files, which `replay_load` also loads. A case with its own `mode.json` verifies only with
/// that file, so it is left out here (CI still seeds the fuzz targets with it).
fn run_replays() -> Vec<Vec<u8>> {
    let dir = root().join("tests/corpus/runs/v0");
    let mut cases: Vec<_> = fs::read_dir(&dir)
        .expect("the run corpus directory")
        .map(|e| e.expect("a run case").path())
        // The `stage_skill` case plays with an enemy attack of its own (see `tests/corpus.rs`),
        // so it does not verify with the bundled content.
        .filter(|case| !case.join("mode.json").exists() && !case.ends_with("stage_skill"))
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
fn a_real_pack_gets_past_the_loader_and_runs() {
    assert!(
        pack_load(&pack::export_bundled()),
        "pack_load rejects the bundled pack"
    );
    assert!(
        pack_load(&pack_with_constellations()),
        "pack_load rejects the pack with a set"
    );
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

/// The bundled pack with two named stars and a set of them in place of its own sets, and no
/// list of the sets' ids, as a pack released with the file but before the list (core#NS3,
/// core#NS4): a seed for the form the bundled pack does not have.
fn pack_with_constellations() -> Vec<u8> {
    let mut pack: serde_json::Value =
        serde_json::from_slice(&pack::export_bundled()).expect("the exported pack is JSON");
    pack["introduced"]
        .as_object_mut()
        .expect("introduced")
        .remove("constellations");
    for id in ["star_a", "star_b"] {
        pack["rewards"]["rewards"]
            .as_array_mut()
            .expect("rewards")
            .push(serde_json::json!({
                "id": id, "rarity": "epic", "named": true, "star": id, "limit": 1,
                "effects": [{ "effect": "attack_pct", "amount": 1 }],
                "title": id, "description": ""
            }));
        pack["introduced"]["rewards"]
            .as_array_mut()
            .expect("introduced rewards")
            .push(serde_json::json!({ "id": id, "sim_version": 0 }));
    }
    pack["constellations"] = serde_json::json!({
        "schema_version": 0, "sim_version": 0,
        "constellations": [{
            "id": "pair", "name": "The Pair", "members": ["star_a", "star_b"],
            "tiers": [
                { "threshold": 1, "effects": [{ "effect": "rerolls", "amount": 1 }] },
                { "threshold": 2, "effects": [{ "effect": "rerolls", "amount": 3 }] }
            ]
        }]
    });
    serde_json::to_vec(&pack).expect("a pack serialises")
}

/// The files each target is given: stages for the stage targets, and for `stage_parse` also the
/// bundled rewards (with their stage skills), run stats and expertise, which it reads too; stage
/// and run replays for the replay target, share codes (of the corpus pairs) for the share target
/// and run setups for the setup target.
fn seeds(target: &str) -> Vec<Vec<u8>> {
    if target == "run_setup" {
        return setups();
    }
    if target == "packs_load" {
        return vec![packs_manifest()];
    }
    if target == "pack_load" {
        return vec![pack::export_bundled(), pack_with_constellations()];
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
            "content/expertise.json",
            "content/match_ups.json",
            "content/runes.json",
            "modes/time_limit.json",
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

/// The `orbs` and `ranged_cap` corpus stages (core Z2) seed the stage targets with an `orb` node
/// and a `ranged` rule, which the parser, the validator and a run reach.
#[test]
fn the_orb_and_ranged_stages_get_past_the_parser_and_run() {
    for name in ["orbs", "ranged_cap"] {
        let (_, stage, _) = corpus().into_iter().find(|c| c.0 == name).unwrap();
        assert!(stage_parse(&stage), "stage_parse rejects the {name} stage");
        assert!(stage_run(&stage), "stage_run rejects the {name} stage");
    }
}

/// A two-channel manifest in the shape content's `release.cjs` writes: the seed of `packs_load`.
fn packs_manifest() -> Vec<u8> {
    let hash = "0123456789abcdef".repeat(4);
    let channel = |minor: &str, text: &str| {
        format!(
            r#""{minor}": {{"pack_sha256": "{hash}", "tag": "v{minor}.0", "text_sha256": {text}, "url": "https://example.com/{minor}/pack.json"}}"#
        )
    };
    format!(
        "{{\"channels\": {{{}, {}}}, \"schema\": 1, \"seq\": 2}}\n",
        channel("0.2", &format!("\"{hash}\"")),
        channel("0.3", "null")
    )
    .into_bytes()
}

#[test]
fn a_real_manifest_gets_past_the_loader() {
    assert!(packs_load(&packs_manifest()), "packs_load rejects the seed");
}
