//! The fuzz targets on stable (ADR-SVC-1), every PR: each target reaches the code behind its first
//! gate on a real stage, replay, run replay, share code, run setup and content pack, rejects
//! garbage, and survives a deterministic sweep over those files: every prefix, and a flipped
//! byte at a regular step. Crashes that a real fuzz run found are kept in
//! `regressions/<target>/` and replayed here.

use std::fs;
use std::path::{Path, PathBuf};

use stella_rain_core::content_hash::to_hex;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::EquipAction;
use stella_rain_core::pack::Pack;
use stella_rain_core::replay::{Replay, ReplayError};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{
    Command, RunRecorder, RunReplay, RunReplayError, record_run_with,
};
use stella_rain_core::savepoint::SavepointError;
use stella_rain_core::share;
use stella_rain_core::version::{NEWEST_READ_SCHEMA_VERSION, SCHEMA_VERSION, SIM_VERSION};
use stella_rain_core_fuzz::{
    MINISIGN_SEED_MANIFEST, change_recorder, commands, minisign_verify, net_decode, pack_load,
    packs_load, party_recorder, replay_load, run_setup, savepoint_load, savepoint_prefixes,
    savepoint_recorder, savepoint_seeds, share_decode, stage_parse, stage_run, step_log_range,
    step_log_range_input,
};

type Target = fn(&[u8]) -> bool;

const TARGETS: [(&str, Target); 11] = [
    ("stage_parse", stage_parse),
    ("stage_run", stage_run),
    ("replay_load", replay_load),
    ("share_decode", share_decode),
    ("run_setup", run_setup),
    ("pack_load", pack_load),
    ("packs_load", packs_load),
    ("minisign_verify", minisign_verify),
    ("savepoint_load", savepoint_load),
    ("step_log_range", step_log_range),
    ("net_decode", net_decode),
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// `(case, stage.json, run.replay)` of every corpus case.
fn corpus() -> Vec<(String, Vec<u8>, Vec<u8>)> {
    let dir = root().join("tests/corpus/current/stages");
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

/// Whether the run case pins the bundled pack in its `case.json`.
fn pins_bundled(case: &Path) -> bool {
    let json = fs::read_to_string(case.join("case.json")).expect("case.json");
    json.contains(&to_hex(&Pack::bundled().sha256()))
}

/// The `run.replay` of every run case of the corpus (ADR-REPL-5) that plays on the bundled pack,
/// which `replay_load` also loads. A case that pins a derived pack verifies only with that pack,
/// so it is left out here (CI still seeds the fuzz targets with it).
fn run_replays() -> Vec<Vec<u8>> {
    let dir = root().join("tests/corpus/current/runs");
    let mut cases: Vec<_> = fs::read_dir(&dir)
        .expect("the run corpus directory")
        .map(|e| e.expect("a run case").path())
        .filter(|case| pins_bundled(case))
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "the corpus has no run cases");
    cases
        .iter()
        .map(|case| fs::read(case.join("run.replay")).expect("run.replay"))
        .collect()
}

/// Run replays cut half-way: the first three run replays of the corpus, played again by their own
/// commands up to the middle tick, so each ends where its run is still going.
fn run_prefixes() -> Vec<Vec<u8>> {
    run_replays()
        .iter()
        .take(3)
        .map(|bytes| {
            let run = RunReplay::from_bytes(bytes).expect("a corpus run replay");
            let mut recorder = RunRecorder::start(&run.setup).expect("the corpus setup starts");
            let mut steps = 0;
            for command in commands(&run) {
                match command {
                    Command::Step(input) => {
                        if steps == run.ticks / 2 {
                            break;
                        }
                        steps += 1;
                        recorder
                            .step(&[input, Default::default(), Default::default()])
                            .expect("a corpus run steps");
                    }
                    Command::Choose(index) => recorder
                        .choose(PlayerIdx::SOLO, index)
                        .expect("a corpus choice"),
                }
            }
            recorder.replay().to_bytes()
        })
        .collect()
}

/// `commands` is the inverse of `record_run_with` on every run case that plays on the bundled
/// pack: recorded again, the commands give the file back byte for byte.
#[test]
fn the_commands_of_a_run_record_the_run_again() {
    for bytes in run_replays() {
        let run = RunReplay::from_bytes(&bytes).expect("a corpus run replay");
        let again = record_run_with(&run.setup, RunContent::bundled(), &commands(&run))
            .expect("the commands record");
        assert_eq!(again.to_bytes(), bytes);
    }
}

/// The seeds are the whole corpus: a case moved out of `current/` or left unread would shrink
/// what the fuzz targets start from.
#[test]
fn the_smoke_corpus_has_every_case() {
    let count = |sub: &str| {
        fs::read_dir(root().join("tests/corpus/current").join(sub))
            .expect("a corpus folder")
            .filter(|e| e.as_ref().unwrap().path().is_dir())
            .count()
    };
    assert_eq!(count("stages"), 16);
    assert_eq!(count("runs"), 9);
    assert_eq!(corpus().len(), 16);
    assert_eq!(run_replays().len(), 5);
}

fn clear() -> (Vec<u8>, Vec<u8>) {
    let (_, stage, replay) = corpus().into_iter().find(|c| c.0 == "clear").unwrap();
    (stage, replay)
}

#[test]
fn a_run_prefix_resumes() {
    let prefixes = run_prefixes();
    assert_eq!(prefixes.len(), 3);
    for prefix in prefixes {
        assert!(replay_load(&prefix), "replay_load does not verify a prefix");
        let run = RunReplay::from_bytes(&prefix).unwrap();
        let resumed = RunRecorder::resume_with_pack(&run, Pack::bundled());
        assert_eq!(resumed.unwrap().replay(), run);
    }
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
fn a_replay_with_a_raised_version_is_newer_and_not_a_load() {
    let (_, replay) = clear();
    let mut v: serde_json::Value = serde_json::from_slice(&replay).unwrap();
    v["sim_version"] = serde_json::json!(SIM_VERSION + 1);
    let raised = v.to_string().into_bytes();
    assert_eq!(
        Replay::from_bytes(&raised),
        Err(ReplayError::Newer {
            schema_version: SCHEMA_VERSION,
            sim_version: SIM_VERSION + 1
        })
    );
    assert!(!replay_load(&raised), "a newer replay verifies");
    let run = run_replays().remove(0);
    let mut v: serde_json::Value = serde_json::from_slice(&run).unwrap();
    v["setup"]["schema_version"] = serde_json::json!(NEWEST_READ_SCHEMA_VERSION + 1);
    let raised = v.to_string().into_bytes();
    assert!(matches!(
        RunReplay::from_bytes(&raised),
        Err(RunReplayError::Newer { .. })
    ));
    assert!(!replay_load(&raised), "a newer run replay verifies");
}

#[test]
fn a_real_run_replay_verifies() {
    for replay in run_replays() {
        assert!(
            replay_load(&replay),
            "replay_load does not verify a run replay"
        );
        let run = RunReplay::from_bytes(&replay).expect("a corpus run replay");
        assert_eq!(
            run.checkpoints.len(),
            16 * usize::try_from(run.ticks / 60).unwrap(),
            "a corpus run replay holds one checkpoint per 60 ticks"
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
        pack_load(Pack::BUNDLED),
        "pack_load rejects the bundled pack"
    );
    assert!(
        pack_load(&pack_with_constellations()),
        "pack_load rejects the pack with a set"
    );
}

/// One packet of each kind, and a frame with an action, built through net's own API: the seeds
/// of `net_decode`.
fn net_seeds() -> Vec<Vec<u8>> {
    use stella_rain_net::wire::{
        Ack, Bye, ByeReason, Checkpoint, Hello, InputFrame, Packet, WireInput,
    };
    let slot = |i| PlayerIdx::new(i).unwrap();
    let mut pack_hash = [0u8; 32];
    for (i, b) in pack_hash.iter_mut().enumerate() {
        *b = u8::try_from(i).unwrap();
    }
    let walk = WireInput {
        move_x: 127,
        move_y: -127,
        evade: true,
        main: true,
        ..WireInput::default()
    };
    let equip = WireInput {
        interact: true,
        action: Some(EquipAction::Equip { item: 7, slot: 2 }),
        ..WireInput::default()
    };
    let tree = WireInput {
        action: Some(EquipAction::TreePick { option: 1 }),
        ..WireInput::default()
    };
    [
        Packet::Hello(Hello {
            protocol: 1,
            sim_version: 2,
            pack_hash,
        }),
        Packet::Input(InputFrame {
            slot: slot(0),
            start_tick: 0,
            inputs: vec![WireInput::default()],
        }),
        Packet::Input(InputFrame {
            slot: slot(1),
            start_tick: 258,
            inputs: vec![walk, equip, tree],
        }),
        Packet::Ack(Ack {
            slot: slot(2),
            next_tick: 259,
        }),
        Packet::Checkpoint(Checkpoint {
            slot: slot(0),
            tick: 3600,
            hash: 0x0123_4567_89ab_cdef,
        }),
        Packet::Bye(Bye {
            slot: slot(1),
            tick: 7200,
            reason: ByeReason::Left,
        }),
    ]
    .iter()
    .map(|p| p.encode().expect("a seed packet encodes"))
    .collect()
}

#[test]
fn every_net_seed_decodes_and_the_empty_and_noise_do_not() {
    for (i, seed) in net_seeds().iter().enumerate() {
        assert!(net_decode(seed), "seed {i}");
    }
    assert!(!net_decode(&[]));
    assert!(!net_decode(&[0xff; 64]));
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
        b"{\"schema_version\":1,\"sim_version\":1}",
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
        serde_json::from_slice(Pack::BUNDLED).expect("the exported pack is JSON");
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
                "effects": [{ "effect": "attack_pct", "amount": 1 }]
            }));
        pack["introduced"]["rewards"]
            .as_array_mut()
            .expect("introduced rewards")
            .push(serde_json::json!({ "id": id, "sim_version": SIM_VERSION }));
    }
    pack["constellations"] = serde_json::json!({
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

/// The seeds of `step_log_range`, named for what they are: for each run case of the corpus (all
/// of them, the target takes the step log alone) the ranges `0..0`, `0..ticks`, the middle
/// third, the last tick and `ticks..ticks`, then three forged logs: a run of `u32::MAX`
/// ticks, a pick before the first tick and an unknown record. Generated, never committed.
fn step_log_range_seeds() -> Vec<(String, Vec<u8>)> {
    let dir = root().join("tests/corpus/current/runs");
    let mut cases: Vec<_> = fs::read_dir(&dir)
        .expect("the run corpus directory")
        .map(|e| e.expect("a run case").path())
        .collect();
    cases.sort();
    assert!(cases.len() >= 9, "the corpus has fewer than 9 run cases");
    let mut seeds = Vec::new();
    for case in cases {
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        let run = RunReplay::from_bytes(&fs::read(case.join("run.replay")).expect("run.replay"))
            .expect("a corpus run replay");
        assert_eq!(run.setup.schema_version, 2, "{name}");
        let log = stella_rain_core::base64::decode(&run.inputs).expect("the step log");
        let t = run.ticks;
        for (label, a, b) in [
            ("empty", 0, 0),
            ("whole", 0, t),
            ("middle", t / 3, 2 * t / 3),
            ("last", t - 1, t),
            ("end", t, t),
        ] {
            seeds.push((
                format!("{name}-{label}"),
                step_log_range_input(t, a, b, &log),
            ));
        }
    }
    // The party cases hold joins and leaves: the whole log and the middle third of each.
    let parties = root().join("tests/corpus/current/parties");
    for case in ["join_two", "leave_hole"] {
        let run =
            RunReplay::from_bytes(&fs::read(parties.join(case).join("run.replay")).expect("run"))
                .expect("a corpus party replay");
        let log = stella_rain_core::base64::decode(&run.inputs).expect("the step log");
        let t = run.ticks;
        for (label, a, b) in [("whole", 0, t), ("middle", t / 3, 2 * t / 3)] {
            seeds.push((
                format!("{case}-{label}"),
                step_log_range_input(t, a, b, &log),
            ));
        }
    }
    let forged: [(&str, u32, &[u8]); 11] = [
        // A join, a leave, then a double join and a leave of slot 0, which are refused.
        (
            "forged-join",
            2,
            &[
                0x01, 0x00, 0x01, 0x02, 0x00, 0x00, 0x02, 0x01, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00,
                0x00, 0x00,
            ],
        ),
        (
            "forged-leave",
            1,
            &[
                0x03, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x01,
            ],
        ),
        (
            "forged-refused-double-join",
            0,
            &[0x01, 0x02, 0x01, 0x02, 0x01],
        ),
        ("forged-refused-leave-first", 0, &[0x01, 0x03, 0x00]),
        (
            "forged-huge-run",
            u32::MAX,
            &[0x01, 0x00, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x02, 0x00, 0x00],
        ),
        (
            "forged-pick-first",
            5,
            &[0x01, 0x01, 0x01, 0x00, 0x00, 0x05, 0x02, 0x00, 0x00],
        ),
        (
            "forged-unknown-record",
            1,
            &[0x01, 0x07, 0x01, 0x00, 0x00, 0x00],
        ),
        // A Replace(3, 7) between two runs; a party of 2 with a TreePick(4) by slot 1; an
        // ACTIONS record with no action; one with an unknown kind.
        (
            "forged-actions",
            2,
            &[
                0x01, 0x00, 0x01, 0x02, 0x00, 0x00, 0x04, 0x02, 0x03, 0x07, 0x00, 0x01, 0x00, 0x00,
                0x00,
            ],
        ),
        (
            "forged-actions-party",
            2,
            &[
                0x03, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x05, 0x04, 0x00,
                0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ],
        ),
        ("forged-actions-none", 0, &[0x01, 0x04, 0x00]),
        ("forged-actions-kind", 0, &[0x01, 0x04, 0x06, 0x00]),
    ];
    for (name, ticks, log) in forged {
        seeds.push((name.to_owned(), step_log_range_input(ticks, 0, ticks, log)));
    }
    seeds
}

/// The files each target is given: stages for the stage targets, and for `stage_parse` also the
/// bundled rewards (with their stage skills), run stats and expertise, which it reads too; stage
/// and run replays for the replay target, share codes (of the corpus pairs) for the share target
/// and run setups for the setup target.
fn seeds(target: &str) -> Vec<Vec<u8>> {
    if target == "minisign_verify" {
        return vec![minisign_seed()];
    }
    if target == "run_setup" {
        return setups();
    }
    if target == "packs_load" {
        return vec![packs_manifest()];
    }
    if target == "savepoint_load" {
        return savepoint_seeds()
            .into_iter()
            .map(|(_, bytes)| bytes)
            .collect();
    }
    if target == "step_log_range" {
        return step_log_range_seeds()
            .into_iter()
            .map(|(_, bytes)| bytes)
            .collect();
    }
    if target == "net_decode" {
        return net_seeds();
    }
    if target == "pack_load" {
        return vec![Pack::BUNDLED.to_vec(), pack_with_constellations()];
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
        seeds.extend(run_prefixes());
    }
    if target == "stage_parse" {
        for file in [
            "content/rewards.json",
            "content/run_stats.json",
            "content/stage_skills.json",
            "content/expertise.json",
            "content/match_ups.json",
            "content/runes.json",
            "content/attack_presets.json",
            "content/text.en.json",
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

fn minisign_seed() -> Vec<u8> {
    b"untrusted comment: signature from minisign secret key\n\
RWRTVEVMTEEwMa8N4UsS3HvJAN2N9Zl4kDmI2Qh5RNGkT4DujF+P4m2ftL0XuquAqYOna2yJuTHsIqrz6tDQg9o0EW+ttwPnMQ8=\n\
trusted comment: timestamp:1700000000\tfile:packs.json\n\
VdvQLUtkDQx1XIsVO20qLa/lxv0+Uq4ocstxuLuShOgVMQ1UM8AwDzn9kEjXOkbxHx7NS6W5plxlT6tsLHDrCQ=="
        .to_vec()
}

#[test]
fn a_real_minisign_signature_gets_past_the_verifier() {
    assert!(
        minisign_verify(&minisign_seed()),
        "minisign_verify rejects the seed"
    );
}

#[test]
fn the_seed_is_refused_by_the_shipped_key() {
    // The seed carries a placeholder key id, so the fuzz target goes through
    // `verify_packs_with_keys`; the shipped `verify_packs` stops it at the key-id gate.
    assert_eq!(
        stella_rain_core::pack::trust::verify_packs(MINISIGN_SEED_MANIFEST, &minisign_seed()),
        Err(stella_rain_core::pack::trust::TrustError::UnknownKey)
    );
}

#[test]
fn the_bundled_attack_presets_parse_as_such_and_a_pack_seed_still_loads() {
    let bytes = fs::read(root().join("content/attack_presets.json")).expect("bundled content");
    assert!(
        stella_rain_core::attack_presets::AttackPresets::parse(
            &bytes,
            stella_rain_core::version::Versions::CURRENT
        )
        .is_ok()
    );
    // Whatever stage_parse makes of the bytes, it must not panic.
    stage_parse(&bytes);
    assert!(
        pack_load(Pack::BUNDLED),
        "pack_load rejects the bundled pack"
    );
}

#[test]
fn every_target_has_a_file_and_the_other_way_round() {
    let mut files: Vec<String> = fs::read_dir(root().join("fuzz/fuzz_targets"))
        .expect("the fuzz target folder")
        .map(|e| {
            let name = e.expect("a target file").file_name();
            name.to_string_lossy().trim_end_matches(".rs").to_owned()
        })
        .collect();
    files.sort();
    let mut listed: Vec<&str> = TARGETS.iter().map(|t| t.0).collect();
    listed.sort_unstable();
    assert_eq!(TARGETS.len(), 11);
    assert_eq!(files, listed);
}

#[test]
fn a_real_savepoint_gets_past_the_loader() {
    assert!(
        savepoint_load(&savepoint_recorder(Some(150)).save()),
        "tick 150"
    );
    // The corpus run is cut before it ends: a re-bless that moves this fails here, not silently in
    // the target. The run is an equipment run, which opens no offer, so the savepoint at tick 230
    // is a running run's, as the one at tick 150 is.
    let late = savepoint_recorder(Some(230));
    assert_eq!(late.engine().tick(), 230);
    assert!(
        !late.engine().offer_open(),
        "an equipment run opens no offer"
    );
    assert!(savepoint_load(&late.save()), "tick 230");
    // A savepoint of a tick no prefix of the target holds, and a finished run's.
    assert!(!savepoint_load(&savepoint_recorder(Some(100)).save()));
    assert!(!savepoint_load(&savepoint_recorder(None).save()));
}

#[test]
fn a_loaded_savepoint_is_its_own_bytes() {
    let bytes = savepoint_recorder(Some(150)).save();
    assert!(savepoint_load(&bytes));
    for at in 0..160 {
        let mut changed = bytes.clone();
        changed[at] ^= 1;
        assert!(!savepoint_load(&changed), "a changed byte at {at} loads");
    }
}

#[test]
fn the_savepoint_seeds_are_deterministic() {
    assert_eq!(savepoint_seeds(), savepoint_seeds());
}

#[test]
fn the_seeds_reach_every_error() {
    let mut refusals = Vec::new();
    let mut loaded = false;
    for (_, bytes) in savepoint_seeds() {
        for prefix in savepoint_prefixes() {
            match RunRecorder::resume_from_savepoint(&bytes, prefix, Pack::bundled()) {
                Ok(_) => loaded = true,
                Err(e) => refusals.push(e),
            }
        }
    }
    assert!(loaded, "no seed loads");
    for want in [
        SavepointError::Format,
        SavepointError::SimVersion,
        SavepointError::Pack,
        SavepointError::Damaged,
        SavepointError::HashMismatch,
        SavepointError::PrefixMismatch,
        SavepointError::Ended,
    ] {
        assert!(refusals.contains(&want), "no seed is refused with {want:?}");
    }
}

#[test]
fn a_forged_schema_version_is_refused() {
    let (_, seed) = savepoint_seeds()
        .into_iter()
        .find(|(name, _)| name == "schema-version")
        .expect("the schema-version seed");
    for prefix in savepoint_prefixes() {
        assert_eq!(
            RunRecorder::resume_from_savepoint(&seed, prefix, Pack::bundled()).err(),
            Some(SavepointError::SimVersion)
        );
    }
    assert!(!savepoint_load(&seed));
}

#[test]
fn a_forged_equipment_block_is_refused() {
    let mut seen = 0;
    for (name, seed) in savepoint_seeds() {
        if !name.starts_with("equipment-") {
            continue;
        }
        seen += 1;
        assert!(!savepoint_load(&seed), "{name} loads");
        for prefix in savepoint_prefixes() {
            assert!(
                RunRecorder::resume_from_savepoint(&seed, prefix, Pack::bundled()).is_err(),
                "{name}"
            );
        }
    }
    assert_eq!(seen, 4);
}

/// Writes the seeds of `savepoint_load` to the folder `SAVEPOINT_SEEDS_DIR` names, for a
/// `cargo fuzz run` (they are generated, never committed).
#[test]
#[ignore = "writes files: set SAVEPOINT_SEEDS_DIR"]
#[allow(
    clippy::disallowed_methods,
    reason = "a tool, not simulation code: it reads the folder"
)]
fn write_savepoint_seeds() {
    let dir = std::env::var("SAVEPOINT_SEEDS_DIR").expect("SAVEPOINT_SEEDS_DIR names the folder");
    fs::create_dir_all(&dir).expect("the seed folder");
    for (name, bytes) in savepoint_seeds() {
        fs::write(Path::new(&dir).join(name), bytes).expect("a seed file");
    }
}

#[test]
fn a_real_log_range_gets_past_the_scanner() {
    let seeds = step_log_range_seeds();
    assert!(seeds.len() >= 9 * 5 + 3);
    for (name, bytes) in &seeds {
        let real = ![
            "forged-unknown",
            "forged-refused",
            "forged-actions-none",
            "forged-actions-kind",
        ]
        .iter()
        .any(|refused| name.starts_with(refused));
        assert_eq!(step_log_range(bytes), real, "{name}");
    }
}

#[test]
fn the_forged_huge_log_is_survived() {
    let huge = step_log_range_input(
        u32::MAX,
        4_000_000_000,
        u32::MAX,
        &[0x01, 0x00, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x02, 0x00, 0x00],
    );
    assert!(step_log_range(&huge));
}

#[test]
fn a_step_log_range_seed_framing_round_trips() {
    let bytes = step_log_range_input(7, 2, 5, &[0x01, 0x00]);
    assert_eq!(&bytes[0..4], &7u32.to_le_bytes());
    assert_eq!(&bytes[4..8], &2u32.to_le_bytes());
    assert_eq!(&bytes[8..12], &5u32.to_le_bytes());
    assert_eq!(&bytes[12..], &[0x01, 0x00]);
}

#[test]
fn the_equipment_action_seeds_are_there_and_read_as_they_should() {
    let seeds = step_log_range_seeds();
    for (name, past_the_scanner) in [
        ("forged-actions", true),
        ("forged-actions-party", true),
        ("forged-actions-none", false),
        ("forged-actions-kind", false),
    ] {
        let (_, bytes) = seeds
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("no seed {name}"));
        assert_eq!(step_log_range(bytes), past_the_scanner, "{name}");
    }
}

/// The committed regression of `replay_load` for an equipment action: the `runs/tutorial` run
/// replay with a Replace(0, 0) step inserted at tick 0, so it loads and ranges but does not
/// verify.
fn equipment_action_replay() -> Vec<u8> {
    let path = root().join("tests/corpus/current/runs/tutorial/run.replay");
    let mut run = RunReplay::from_bytes(&fs::read(path).expect("run.replay")).expect("a run");
    let mut log = stella_rain_core::base64::decode(&run.inputs).expect("the step log");
    log.splice(1..1, [0x04, 0x02, 0x00, 0x00]);
    run.inputs = stella_rain_core::base64::encode(&log);
    run.to_bytes()
}

fn equipment_action_regression() -> PathBuf {
    root()
        .join("fuzz")
        .join("regressions")
        .join("replay_load")
        .join("equipment-action.replay")
}

/// Writes the regression from `equipment_action_replay`, after a re-bless of `runs/tutorial`:
/// `cargo test --test smoke rewrite_the_equipment_action_regression -- --ignored`.
#[test]
#[ignore = "rewrites a committed file, run by hand after a re-bless"]
fn rewrite_the_equipment_action_regression() {
    fs::write(equipment_action_regression(), equipment_action_replay()).expect("the regression");
}

#[test]
fn the_equipment_action_regression_is_the_derived_file() {
    let committed = fs::read(equipment_action_regression()).expect("the regression");
    assert_eq!(committed, equipment_action_replay());
}

#[test]
fn a_replay_with_an_equipment_action_loads_ranges_and_is_refused() {
    let bytes = equipment_action_replay();
    replay_load(&bytes);
    let run = RunReplay::from_bytes(&bytes).expect("it loads");
    let range = run.step_range(0, run.ticks, 10_000_000).expect("it ranges");
    assert!(matches!(
        range.steps.first(),
        Some(stella_rain_core::RangeStep::Actions { .. })
    ));
    assert!(matches!(
        stella_rain_core::run_replay::verify_run(&bytes),
        Err(RunReplayError::ActionRefused { tick: 0, .. })
    ));
}

/// Writes the regression file of `replay_load` for an equipment action; run once, the output is
/// committed.
#[test]
#[ignore = "writes a file in fuzz/regressions"]
fn write_equipment_action_regression() {
    fs::write(equipment_action_regression(), equipment_action_replay())
        .expect("the regression file");
}

/// Writes the seeds of `step_log_range` to the folder `STEP_LOG_RANGE_SEEDS_DIR` names, for a
/// `cargo fuzz run` (they are generated, never committed).
#[test]
#[ignore = "writes files: set STEP_LOG_RANGE_SEEDS_DIR"]
#[allow(
    clippy::disallowed_methods,
    reason = "a tool, not simulation code: it reads the folder"
)]
fn write_step_log_range_seeds() {
    let dir = std::env::var("STEP_LOG_RANGE_SEEDS_DIR")
        .expect("STEP_LOG_RANGE_SEEDS_DIR names the folder");
    fs::create_dir_all(&dir).expect("the seed folder");
    for (name, bytes) in step_log_range_seeds() {
        fs::write(Path::new(&dir).join(name), bytes).expect("a seed file");
    }
}

/// The most ticks the `step_log_range` seeds may claim in all, counting the seeds the target's
/// oracle runs on (those at or under `EXPAND_LIMIT`, 65,536 ticks). Each seed is swept by
/// `every_prefix_and_flipped_byte_of_the_seeds_is_survived`, so this bounds the cost of that
/// sweep for the target. Raising it is a decision to take the CI minutes.
const STEP_LOG_RANGE_TICK_BUDGET: u64 = 490_000;

/// The most ticks the run replays seeded to `replay_load` may hold in all (what it re-simulates
/// per call, before the sweep multiplies it). Raising it is a decision to take the CI minutes.
const RUN_SEED_TICK_BUDGET: u64 = 53_000;

/// The bound of the target's oracle on the ticks a log claims (`EXPAND_LIMIT`).
const ORACLE_TICK_LIMIT: u32 = 65_536;

#[test]
fn the_fuzz_crate_tests_are_built_optimised() {
    // The sweep simulates whole runs; at opt-level 0 it takes about 13 minutes, at 1 about 1.5
    // (ADR-SIM-3). Overflow checks must stay on beside it.
    let text = fs::read_to_string(root().join("fuzz/Cargo.toml")).expect("fuzz/Cargo.toml");
    let section: Vec<String> = text
        .lines()
        .skip_while(|line| line.trim() != "[profile.dev]")
        .skip(1)
        .take_while(|line| !line.starts_with('['))
        .map(|line| {
            // Without an inline comment or spaces, so `opt-level=1 # note` reads as it is.
            let code = line.split('#').next().unwrap_or_default();
            code.split_whitespace().collect::<String>()
        })
        .collect();
    assert!(!section.is_empty(), "fuzz/Cargo.toml has no [profile.dev]");
    let level = section
        .iter()
        .find_map(|line| line.strip_prefix("opt-level="))
        .and_then(|value| value.parse::<u8>().ok());
    assert!(
        level.is_some_and(|n| n >= 1),
        "[profile.dev] of fuzz/Cargo.toml needs opt-level = 1 or higher, has {level:?}"
    );
    assert!(
        section.iter().any(|line| line == "overflow-checks=true"),
        "[profile.dev] of fuzz/Cargo.toml must keep overflow-checks = true"
    );
}

#[test]
fn the_step_log_range_seeds_expand_a_bounded_number_of_ticks() {
    let mut sum = 0u64;
    for (name, bytes) in step_log_range_seeds() {
        let ticks = u32::from_le_bytes(bytes[..4].try_into().expect("the ticks word"));
        if ticks <= ORACLE_TICK_LIMIT {
            sum += u64::from(ticks);
        } else {
            assert!(
                name.starts_with("forged-"),
                "{name} claims {ticks} ticks, over the oracle's {ORACLE_TICK_LIMIT}"
            );
        }
    }
    assert!(
        sum <= STEP_LOG_RANGE_TICK_BUDGET,
        "the step_log_range seeds claim {sum} ticks, over the budget {STEP_LOG_RANGE_TICK_BUDGET}"
    );
}

#[test]
fn the_run_seeds_simulate_a_bounded_number_of_ticks() {
    let sum: u64 = run_replays()
        .iter()
        .chain(run_prefixes().iter())
        .map(|bytes| u64::from(RunReplay::from_bytes(bytes).expect("a run seed").ticks))
        .sum();
    assert!(
        sum <= RUN_SEED_TICK_BUDGET,
        "the run seeds hold {sum} ticks, over the budget {RUN_SEED_TICK_BUDGET}"
    );
}

#[test]
fn the_party_cases_pass_the_replay_oracle() {
    let dir = root().join("tests/corpus/current/parties");
    let mut sizes = Vec::new();
    for case in fs::read_dir(&dir).expect("the party corpus directory") {
        let case = case.unwrap().path();
        if !case.is_dir() {
            continue;
        }
        let bytes = fs::read(case.join("run.replay")).expect("run.replay");
        // Verified, and re-recorded through the step-range walk to the same replay.
        assert!(
            replay_load(&bytes),
            "a party case does not pass replay_load"
        );
        let run = RunReplay::from_bytes(&bytes).unwrap();
        let range = run.step_range(0, 0, u32::MAX).unwrap();
        sizes.push(range.present.count_ones());
    }
    assert!(
        sizes.contains(&2) && sizes.contains(&3),
        "the corpus holds an N=2 and an N=3 case: {sizes:?}"
    );
}

#[test]
fn a_savepoint_inside_a_hole_gets_past_the_loader() {
    for cut in [1000, 1600, 2200] {
        let rec = change_recorder(Some(cut));
        assert!(savepoint_load(&rec.save()), "change run, tick {cut}");
    }
    assert_eq!(change_recorder(Some(1000)).engine().present(), 0b101);
    assert_eq!(change_recorder(Some(1600)).engine().present(), 0b001);
    assert_eq!(change_recorder(Some(2200)).engine().present(), 0b101);
}

#[test]
fn a_party_savepoint_gets_past_the_loader() {
    let running = party_recorder(Some(150));
    assert_eq!(running.engine().players(), 3);
    assert!(savepoint_load(&running.save()), "party, tick 150");
    // The cut at the first action of slot 1 or 2.
    let (_, open_seed) = savepoint_seeds()
        .into_iter()
        .find(|(name, _)| name == "valid-party-action")
        .expect("the party action seed");
    assert!(savepoint_load(&open_seed), "party, action pending");
    // The party run whole is cut by its tick limit while still running, so it loads too; a
    // savepoint of a tick no prefix holds does not.
    assert!(savepoint_load(&party_recorder(None).save()), "party, whole");
    assert!(!savepoint_load(&party_recorder(Some(100)).save()));
}

#[test]
fn a_replay_whose_setup_names_offer_mode_is_refused() {
    // The offer mode left the setup: its key is an unknown field, refused without a panic.
    let bytes =
        fs::read(root().join("tests/corpus/current/parties/two_time_limit/run.replay")).unwrap();
    assert!(replay_load(&bytes));
    let text = String::from_utf8(bytes).unwrap();
    for mode in ["stop", "timed"] {
        let forged = text.replace(
            "\"sim_version\":2",
            &format!("\"offer_mode\":\"{mode}\",\"sim_version\":2"),
        );
        assert_ne!(forged, text);
        assert!(!replay_load(forged.as_bytes()), "{mode}");
    }
}
