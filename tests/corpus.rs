//! The replay corpus (ADR-019): `tests/corpus/current/stages/<name>/` holds a `stage.json`, a `run.replay`
//! and `hashes.bin` (the state hash after every tick, 8 bytes little-endian,
//! `replay::hashes_from_bytes`). Each case
//! is re-simulated and every tick compared, so a change that moves any hash fails on the first
//! tick it moved, on x86_64 and on the arm64 runner alike. The device run reads them with the same
//! code (ADR-054).
//!
//! On a mismatch the failure names the first diverging tick and writes the snapshots of that
//! tick and the one before to `target/corpus-diff/<name>.json`; CI uploads that folder per
//! architecture. After an intended change, run
//! `UPDATE_CORPUS=1 cargo test --test integration corpus::` and say `Corpus regenerated: <why>`
//! in the PR (ADR-035).
//!
//! Runs of the launch game (ADR-079) are kept apart, in `tests/corpus/current/runs/<name>/`: a
//! `run.replay` alone (a run replay: setup, content hash, inputs, commands and `checkpoints`, the
//! hash after every 60th tick, which are the expected hashes). A failing second is re-run and its
//! 61 snapshots written to `target/corpus-diff/<name>/`. They need no stage file, because the
//! setup names a mode bundled in `core`; the stage cases' folders each hold one.

use crate::common;
use stella_rain_core::version::Versions;

use std::fs;
use std::path::{Path, PathBuf};

use crate::corpus_pins::{
    case_pack, check_frozen_tree, ensure_writable, is_frozen_name, override_pack, prune_packs,
    run_case_pack, write_case_json, write_pack_file,
};
use crate::run_play::play_run_with;
use common::*;
use serde_json::{Value, json};
use stella_rain_core::attack::MatchUp;
use stella_rain_core::content_hash::to_hex;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::pack::Pack;
use stella_rain_core::replay::{self, Replay, hash_to_hex};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{self, Command, CommandKind, RunReplay, TickCommand};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::snapshot::{EntityKind, OfferOption, Snapshot};
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::limits::MAX_TICKS;
use stella_rain_core::validate::parse_and_validate;

pub(crate) fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/current/stages")
}

pub(crate) fn runs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/current/runs")
}

pub(crate) fn current_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/current")
}

pub(crate) fn diff_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/corpus-diff")
}

/// Re-simulates one case and compares every tick's hash. `Err` is the message the test fails
/// with; a divergence also leaves `<diff_dir>/<name>.json`.
pub(crate) fn check_case(case: &Path, diff_dir: &Path) -> Result<(), String> {
    let name = case.file_name().unwrap().to_string_lossy().into_owned();
    let read = |file: &str| {
        fs::read(case.join(file)).map_err(|e| format!("corpus case `{name}`: {file}: {e}"))
    };
    let stage_bytes = read("stage.json")?;
    let replay_bytes = read("run.replay")?;
    let hashes = read_hashes(case, &name)?;

    let stage = parse_and_validate(&stage_bytes)
        .map_err(|e| format!("corpus case `{name}`: stage.json: {e:?}"))?;
    let recorded = Replay::from_bytes(&replay_bytes)
        .map_err(|e| format!("corpus case `{name}`: run.replay: {e}"))?;
    let inputs = recorded
        .unpack_inputs(stage.length_ticks)
        .map_err(|e| format!("corpus case `{name}`: run.replay: {e}"))?;
    if hashes.len() != inputs.len() {
        return Err(format!(
            "corpus case `{name}`: hashes.bin has {} hashes, run.replay has {} ticks",
            hashes.len(),
            inputs.len()
        ));
    }

    let pack = case_pack(case)?;
    let mut engine = Engine::new_with(&stage, pack.content())
        .map_err(|e| format!("corpus case `{name}`: {e:?}"))?;
    let mut previous = engine.snapshot();
    for (input, expected) in inputs.iter().zip(&hashes) {
        engine.step(*input);
        if engine.state_hash() != *expected {
            return Err(divergence(&name, *expected, &engine, &previous, diff_dir));
        }
        previous = engine.snapshot();
    }

    if recorded.final_hash != hash_to_hex(engine.state_hash()) || recorded.ticks != engine.tick() {
        return Err(format!(
            "corpus case `{name}`: run.replay's ticks or final hash do not match hashes.bin"
        ));
    }
    // The library path plays the bundled pack only (`replay::verify_against` has no pack).
    if engine.outcome() == Outcome::Cleared && pack.sha256() == Pack::bundled().sha256() {
        replay::verify_against(&stage_bytes, &replay_bytes, &hashes)
            .map_err(|e| format!("corpus case `{name}`: {e}"))?;
    }
    Ok(())
}

/// Reads `hashes.bin` of a case; a `hashes.txt` beside it is the old format and fails.
fn read_hashes(case: &Path, name: &str) -> Result<Vec<u64>, String> {
    if case.join("hashes.txt").exists() {
        return Err(format!(
            "corpus case `{name}`: hashes.txt is the old format; run `cargo xtask bless`"
        ));
    }
    let bytes = fs::read(case.join("hashes.bin"))
        .map_err(|e| format!("corpus case `{name}`: hashes.bin: {e}"))?;
    replay::hashes_from_bytes(&bytes).map_err(|e| format!("corpus case `{name}`: hashes.bin: {e}"))
}

/// Writes a case's `hashes.bin` and removes the `hashes.txt` an older bless left.
fn write_hashes(dir: &Path, hashes: &[u64]) {
    write_atomic(&dir.join("hashes.bin"), &replay::hashes_to_bytes(hashes));
    match fs::remove_file(dir.join("hashes.txt")) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => panic!("cannot remove hashes.txt: {e}"),
    }
}

/// Applies one command between ticks, as a replay's verification does.
pub(crate) fn apply(engine: &mut Engine, c: &TickCommand) -> Result<(), String> {
    match c.kind {
        CommandKind::Choose => engine.choose(c.index).map_err(|e| format!("{e:?}")),
    }
}

/// Verifies one run case (ADR-079): its `run.replay` carries the expected hashes as checkpoints,
/// one every 60 ticks, so it is the one file of the case. A `hashes.bin` or `hashes.txt` beside
/// it would be a second source of truth and fails the case. When a checkpoint differs, the
/// snapshots of the ticks of that second are written to `<diff_dir>/<name>/`.
pub(crate) fn check_run_case(case: &Path, diff_dir: &Path) -> Result<(), String> {
    let name = case.file_name().unwrap().to_string_lossy().into_owned();
    let fail = |e: &dyn std::fmt::Display| format!("corpus case `{name}`: {e}");
    for stale in ["hashes.bin", "hashes.txt"] {
        if case.join(stale).exists() {
            return Err(fail(&format!(
                "{stale} beside run.replay; a run case keeps its hashes as checkpoints in \
                 run.replay; run `cargo xtask bless`"
            )));
        }
    }
    let replay_bytes = fs::read(case.join("run.replay")).map_err(|e| fail(&e))?;
    let recorded = RunReplay::from_bytes(&replay_bytes).map_err(|e| fail(&e))?;
    let pack = run_case_pack(case, &replay_bytes)?;
    match run_replay::verify_run_with_pack(&replay_bytes, &pack) {
        Ok(_) => Ok(()),
        Err(run_replay::RunReplayError::CheckpointDiffers { second }) => Err(dump_second(
            &name,
            &recorded,
            pack.content(),
            second,
            diff_dir,
        )),
        Err(e) => Err(fail(&e)),
    }
}

/// Re-runs a run case from tick 0 and writes `<diff_dir>/<name>/tick-<NNNNN>.json` for each tick
/// from the start of `second` to its checkpoint (61 files): the hash and the snapshot after that
/// tick, before the commands that follow it. Returns the message the case fails with.
fn dump_second(
    name: &str,
    recorded: &RunReplay,
    content: &RunContent,
    second: u32,
    diff_dir: &Path,
) -> String {
    let (from, to) = (
        (second - 1) * run_replay::CHECKPOINT_TICKS,
        second * run_replay::CHECKPOINT_TICKS,
    );
    let dir = diff_dir.join(name);
    fs::create_dir_all(&dir).unwrap();
    let dump = |engine: &Engine| {
        let tick = engine.tick();
        let file = json!({
            "case": name,
            "tick": tick,
            "hash": hash_to_hex(engine.state_hash()),
            "snapshot": engine.snapshot(),
        });
        let path = dir.join(format!("tick-{tick:05}.json"));
        fs::write(path, serde_json::to_vec_pretty(&file).unwrap()).unwrap();
    };
    let mut engine = Engine::start_with(&recorded.setup, content).unwrap();
    let commands = recorded.unpack_commands().unwrap();
    let mut next = commands.iter().peekable();
    if from == 0 {
        dump(&engine);
    }
    for input in recorded.unpack_inputs(u32::MAX).unwrap() {
        while let Some(c) = next.next_if(|c| c.tick == engine.tick()) {
            apply(&mut engine, c).unwrap();
        }
        engine.step(input);
        if engine.tick() >= from {
            dump(&engine);
        }
        if engine.tick() == to {
            break;
        }
    }
    let at = 16 * (second as usize - 1);
    format!(
        "corpus case `{name}` diverges in second {second}, between tick {from} and tick {to}: \
         expected {}, got {}; the snapshots of ticks {from} to {to} are in {}",
        &recorded.checkpoints[at..at + 16],
        hash_to_hex(engine.state_hash()),
        dir.display()
    )
}

/// The content a run case plays with: the content of the pack its `case.json` pins.
pub(crate) fn case_content(case: &Path) -> Result<RunContent, String> {
    case_pack(case).map(|pack| pack.content().clone())
}

/// Writes a run case's files from a setup and the commands to run, on the pack `pack_bytes` (a
/// derived one, stored in `packs/`) or on the bundled pack, and pins it in `case.json`.
fn write_run_case(dir: &Path, setup: &RunSetup, pack_bytes: Option<&[u8]>, commands: &[Command]) {
    ensure_writable(dir).unwrap();
    fs::create_dir_all(dir).unwrap();
    let (pack, sha) = match pack_bytes {
        Some(bytes) => {
            let sha = write_pack_file(&current_dir(), bytes).unwrap();
            (Pack::from_bytes(bytes).unwrap(), sha)
        }
        None => (Pack::bundled().clone(), to_hex(&Pack::bundled().sha256())),
    };
    write_case_json(dir, &sha).unwrap();
    let recorded = run_replay::record_run_with_pack(setup, &pack, commands).unwrap();
    write_atomic(&dir.join("run.replay"), &recorded.to_bytes());
    for stale in ["hashes.bin", "hashes.txt", "mode.json"] {
        match fs::remove_file(dir.join(stale)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("cannot remove {stale}: {e}"),
        }
    }
}

/// Writes the snapshots of the first diverging tick (`engine`'s) and of the one before to
/// `<diff_dir>/<name>.json`, and returns the message the case fails with.
fn divergence(
    name: &str,
    expected: u64,
    engine: &Engine,
    previous: &Snapshot,
    diff_dir: &Path,
) -> String {
    let tick = engine.tick();
    let actual = engine.state_hash();
    let dump = json!({
        "case": name,
        "tick": tick,
        "expected_hash": hash_to_hex(expected),
        "actual_hash": hash_to_hex(actual),
        "previous": previous,
        "at": engine.snapshot(),
    });
    let path = diff_dir.join(format!("{name}.json"));
    fs::create_dir_all(diff_dir).unwrap();
    fs::write(&path, serde_json::to_vec_pretty(&dump).unwrap()).unwrap();
    format!(
        "corpus case `{name}` diverges first on tick {tick}: expected {}, got {}; \
         snapshots of ticks {} and {tick} are in {}",
        hash_to_hex(expected),
        hash_to_hex(actual),
        tick - 1,
        path.display()
    )
}

/// A scratch directory under the target directory, emptied first.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("corpus-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes `bytes` to `path` so that no reader ever sees half of them: the other tests of this
/// file read the cases while `UPDATE_CORPUS=1` rewrites them, and a plain `fs::write` truncates
/// the file first.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).unwrap();
    fs::rename(&tmp, path).unwrap();
}

/// Writes a case's three files from a stage and the inputs to run.
fn write_case(dir: &Path, stage_bytes: &[u8], inputs: &[Input]) {
    let recorded = replay::record(stage_bytes, inputs).unwrap();
    let ran = recorded.unpack_inputs(u32::MAX).unwrap();
    let stage = parse_and_validate(stage_bytes).unwrap();
    let mut engine = Engine::new(&stage).unwrap();
    let mut hashes = Vec::new();
    for input in &ran {
        engine.step(*input);
        hashes.push(engine.state_hash());
    }
    ensure_writable(dir).unwrap();
    fs::create_dir_all(dir).unwrap();
    write_case_json(dir, &to_hex(&Pack::bundled().sha256())).unwrap();
    write_atomic(&dir.join("stage.json"), stage_bytes);
    write_atomic(&dir.join("run.replay"), &recorded.to_bytes());
    write_hashes(dir, &hashes);
}

fn changed_hash_case(name: &str) -> (PathBuf, PathBuf) {
    let dir = scratch(name);
    let case = dir.join("case");
    write_case(&case, &common::stage_bytes(), &common::clearing_inputs());
    (case, dir.join("diff"))
}

/// Flips bit 0 of the hash on `tick` (1-based); returns the new hash and the one it replaced.
fn change_hash(case: &Path, tick: usize) -> (u64, u64) {
    let path = case.join("hashes.bin");
    let mut hashes = replay::hashes_from_bytes(&fs::read(&path).unwrap()).unwrap();
    let was = hashes[tick - 1];
    hashes[tick - 1] ^= 1;
    fs::write(&path, replay::hashes_to_bytes(&hashes)).unwrap();
    (hashes[tick - 1], was)
}

#[test]
fn an_unchanged_case_passes() {
    let (case, diff) = changed_hash_case("unchanged");
    assert_eq!(check_case(&case, &diff), Ok(()));
    assert!(!diff.exists(), "a passing case writes no diff");
}

#[test]
fn a_changed_hash_at_tick_n_fails_naming_n() {
    let (case, diff) = changed_hash_case("tick-n");
    let (flipped, engine) = change_hash(&case, 20);
    let message = check_case(&case, &diff).unwrap_err();
    assert!(message.contains("tick 20"), "{message}");
    assert!(
        message.contains(&format!("expected {}", hash_to_hex(flipped))),
        "{message}"
    );
    assert!(
        message.contains(&format!("got {}", hash_to_hex(engine))),
        "{message}"
    );
    assert!(message.contains("`case`"), "names the case: {message}");

    // The dump holds the snapshots of tick 20 and of the tick before.
    let dump: Value = serde_json::from_slice(&fs::read(diff.join("case.json")).unwrap()).unwrap();
    assert_eq!(dump["tick"], json!(20));
    assert_eq!(dump["previous"]["tick"], json!(19));
    assert_eq!(dump["at"]["tick"], json!(20));
}

#[test]
fn only_the_first_diverging_tick_is_reported() {
    let (case, diff) = changed_hash_case("first");
    change_hash(&case, 30);
    change_hash(&case, 12);
    let message = check_case(&case, &diff).unwrap_err();
    assert!(message.contains("tick 12"), "{message}");
    assert!(!message.contains("tick 30"), "{message}");
}

#[test]
fn a_change_on_the_first_tick_dumps_the_initial_state_as_the_one_before() {
    let (case, diff) = changed_hash_case("tick-1");
    change_hash(&case, 1);
    let message = check_case(&case, &diff).unwrap_err();
    assert!(message.contains("tick 1"), "{message}");
    let dump: Value = serde_json::from_slice(&fs::read(diff.join("case.json")).unwrap()).unwrap();
    assert_eq!(dump["previous"]["tick"], json!(0));
}

#[test]
fn hashes_that_are_not_one_per_tick_fail() {
    let (case, diff) = changed_hash_case("count");
    let path = case.join("hashes.bin");
    let bytes = fs::read(&path).unwrap();
    fs::write(&path, &bytes[..80]).unwrap();
    let message = check_case(&case, &diff).unwrap_err();
    assert!(message.contains("hashes.bin has 10 hashes"), "{message}");
}

#[test]
fn a_hashes_file_cut_by_a_byte_fails() {
    let (case, diff) = changed_hash_case("cut");
    let path = case.join("hashes.bin");
    let bytes = fs::read(&path).unwrap();
    fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
    let message = check_case(&case, &diff).unwrap_err();
    assert!(message.contains("hashes.bin"), "{message}");
    assert!(
        message.contains("not a whole number of 8-byte hashes"),
        "{message}"
    );
}

#[test]
fn a_text_hashes_file_beside_the_binary_one_fails() {
    let (case, diff) = changed_hash_case("leftover");
    fs::write(case.join("hashes.txt"), "x\n").unwrap();
    let message = check_case(&case, &diff).unwrap_err();
    assert!(
        message.contains("hashes.txt is the old format"),
        "{message}"
    );
}

#[test]
fn the_writers_write_their_files_and_remove_stale_hashes() {
    let (case, diff) = changed_hash_case("writer");
    fs::write(case.join("hashes.txt"), "x\n").unwrap();
    write_case(&case, &common::stage_bytes(), &common::clearing_inputs());
    assert!(!case.join("hashes.txt").exists());
    assert_eq!(fs::read(case.join("hashes.bin")).unwrap().len(), 280);
    assert_eq!(check_case(&case, &diff), Ok(()));

    let (case, diff) = changed_run_case("writer");
    fs::write(case.join("hashes.txt"), "x\n").unwrap();
    fs::write(case.join("hashes.bin"), [0u8; 8]).unwrap();
    let setup = common::run::trial_setup();
    let point = crate::run_play::first_point_tick();
    write_run_case(&case, &setup, None, &crate::run_play::play_run(point + 200));
    assert!(!case.join("hashes.txt").exists());
    assert!(!case.join("hashes.bin").exists());
    let replay = RunReplay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
    assert_eq!(replay.checkpoints.len(), 16 * (replay.ticks / 60) as usize);
    assert_eq!(check_run_case(&case, &diff), Ok(()));
}

#[test]
fn a_replay_that_is_not_the_stages_fails() {
    let (case, diff) = changed_hash_case("replay");
    let mut stage: Value =
        serde_json::from_slice(&fs::read(case.join("stage.json")).unwrap()).unwrap();
    stage["title"] = json!("Another Title");
    fs::write(case.join("stage.json"), serde_json::to_vec(&stage).unwrap()).unwrap();
    // The replay holds the old file's SHA-256, so a cleared case no longer verifies.
    let message = check_case(&case, &diff).unwrap_err();
    assert!(message.contains("stage file has changed"), "{message}");
}

// --- Run cases (ADR-079) -----------------------------------------------------------------------

fn changed_run_case(name: &str) -> (PathBuf, PathBuf) {
    let dir = scratch(&format!("run-{name}"));
    let case = dir.join("case");
    write_run_case(
        &case,
        &common::run::trial_setup(),
        None,
        &crate::run_play::play_run(300),
    );
    (case, dir.join("diff"))
}

#[test]
fn an_unchanged_run_case_passes() {
    let (case, diff) = changed_run_case("unchanged");
    assert_eq!(check_run_case(&case, &diff), Ok(()));
    assert!(!diff.exists(), "a passing case writes no diff");
}

/// Flips the last digit of the replay's `second`th checkpoint; returns the flipped checkpoint
/// and the engine's value for it, as numbers.
fn flip_checkpoint(case: &Path, second: usize) -> (u64, u64) {
    let path = case.join("run.replay");
    let mut replay = RunReplay::from_bytes(&fs::read(&path).unwrap()).unwrap();
    let range = 16 * (second - 1)..16 * second;
    let engine = replay::hash_from_hex(&replay.checkpoints[range.clone()]).unwrap();
    let mut digits: Vec<u8> = replay.checkpoints.bytes().collect();
    let last = range.end - 1;
    digits[last] = if digits[last] == b'0' { b'1' } else { b'0' };
    replay.checkpoints = String::from_utf8(digits).unwrap();
    let flipped = replay::hash_from_hex(&replay.checkpoints[range]).unwrap();
    fs::write(&path, replay.to_bytes()).unwrap();
    (flipped, engine)
}

#[test]
fn a_changed_run_case_names_its_second_and_dumps_it() {
    let (case, diff) = changed_run_case("second");
    let (flipped, engine) = flip_checkpoint(&case, 2);
    let message = check_run_case(&case, &diff).unwrap_err();
    for part in [
        "second 2",
        "between tick 60 and tick 120",
        &format!("expected {}", hash_to_hex(flipped)),
        &format!("got {}", hash_to_hex(engine)),
    ] {
        assert!(message.contains(part), "{message}");
    }
    let dir = diff.join("case");
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names.len(), 61);
    assert_eq!(names[0], "tick-00060.json");
    assert_eq!(names[60], "tick-00120.json");
    let dump: Value =
        serde_json::from_slice(&fs::read(dir.join("tick-00120.json")).unwrap()).unwrap();
    assert_eq!(dump["case"], json!("case"));
    assert_eq!(dump["tick"], json!(120));
    assert_eq!(dump["hash"], json!(hash_to_hex(engine)));
    assert_eq!(dump["snapshot"]["tick"], json!(120));
}

#[test]
fn a_run_case_whose_stat_priority_changed_fails() {
    let (case, diff) = changed_run_case("priority");
    let path = case.join("run.replay");
    let mut replay = RunReplay::from_bytes(&fs::read(&path).unwrap()).unwrap();
    assert_ne!(replay.setup.stat_priority, ["speed".into()]);
    replay.setup.stat_priority = vec!["speed".into()];
    fs::write(&path, replay.to_bytes()).unwrap();
    // The priority is in the state from tick 0, so the run differs from the first tick on.
    let message = check_run_case(&case, &diff).unwrap_err();
    assert!(
        message.contains("diverges in second 1, between tick 0 and tick 60"),
        "{message}"
    );
}

#[test]
fn a_run_case_with_damaged_checkpoints_fails() {
    let (case, diff) = changed_run_case("damaged");
    let path = case.join("run.replay");
    let mut replay = RunReplay::from_bytes(&fs::read(&path).unwrap()).unwrap();
    replay.checkpoints.truncate(replay.checkpoints.len() - 16);
    fs::write(&path, replay.to_bytes()).unwrap();
    let message = check_run_case(&case, &diff).unwrap_err();
    assert!(message.contains("checkpoints"), "{message}");
}

#[test]
fn a_hashes_file_beside_a_run_case_fails() {
    for file in ["hashes.bin", "hashes.txt"] {
        let (case, diff) = changed_run_case("beside");
        fs::write(case.join(file), [0u8; 8]).unwrap();
        let message = check_run_case(&case, &diff).unwrap_err();
        assert!(message.contains(file), "{message}");
    }
}

// --- The cases, built from the test helpers --------------------------------------------------
//
// These are only read by `UPDATE_CORPUS=1`, which writes the files; a normal run reads the files
// and nothing else, so what the device run sees is exactly what CI checked.

pub(crate) type Definition = (&'static str, Vec<u8>, Vec<Input>);

fn bytes(stage: &impl serde::Serialize) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(stage).unwrap();
    bytes.push(b'\n');
    bytes
}

/// Standing still for `ticks` ticks, the Main weapon firing every 6.
fn firing_for(ticks: u32) -> Vec<Input> {
    (0..ticks).map(|t| firing(idle(), t)).collect()
}

/// The `giant_sword` played by hand in a 60-tick cycle from tick `t`: `main` on 0, 8 and 16 (slash,
/// slash, thrust) and `smash` on 40 and 48 (two smashes from the root, each a slash that pulls).
fn combos(t: u32, input: Input) -> Input {
    let c = t % 60;
    Input {
        main: matches!(c, 0 | 8 | 16),
        smash: c == 40 || c == 48,
        ..input
    }
}

/// Agents of every movement type: a companion that orbits the main character and buffs it, a
/// healer that follows it, and an enemy that drifts down while it shoots at the nearest ally.
fn agents() -> Definition {
    let shoot = |selector: &str, cooldown| {
        rule(
            always(),
            json!(selector),
            json!({ "type": "attack",
                    "attack": { "preset": { "id": "aimed_single", "args": [15] } } }),
            cooldown,
        )
    };
    let k = companion(
        10,
        json!({ "type": "orbit", "radius": 10_240 }),
        json!([rule(always(), json!("player"), atk_up(25, 40), 30)]),
    );
    let healer = companion(
        10,
        json!({ "type": "follow", "distance": 12_288 }),
        json!([rule(
            json!({ "type": "hp_below", "who": "lowest_hp_ally", "pct": 100 }),
            json!("lowest_hp_ally"),
            cast(json!({ "type": "heal", "amount": 5 })),
            0
        )]),
    );
    let shooter = enemy(
        50,
        json!({ "type": "straight", "vx": 0, "vy": 96 }),
        json!([shoot("nearest_ally", 5)]),
    );
    // The test stage's Main weapon: a ray along the facing, pressed every 6 ticks. The main
    // character holds the most a stage may: a hit is 100 shown.
    let mut stage = stage_full(
        600,
        Some(far_boss()),
        vec![wave(1, 1, 0, shooter, 46_080, 100_000)],
        vec![k, healer],
    );
    stage.player.hp = 999;
    // Standing still for 40 ticks, shooting up at the enemy, then walking right, which turns
    // the Main weapon's ray away from it.
    let inputs = (0..80)
        .map(|t| firing(if t < 40 { idle() } else { moving(64, 0) }, t))
        .collect();
    ("agents", bytes(&stage), inputs)
}

/// The boss of `tests/golden.rs`: a horn that breaks, two phases and a timeline, shot until it
/// falls.
fn boss() -> Definition {
    let horn = json!({ "id": "horn", "asset": "horns_2", "hp": 30,
                       "offset": { "x": 0, "y": 16_000 }, "radius": 6_000 });
    let shot = json!({ "type": "attack",
                       "attack": { "preset": { "id": "aimed_single", "args": [15] } } });
    let boss = json!({
        "base": "golem", "hp": 100, "radius": 8192, "spawn": { "x": 46_080, "y": 60_000 },
        "parts": [horn],
        "phases": [
            {
                "until": { "type": "part_broken", "part": "horn" },
                "timeline": [
                    { "type": "telegraph", "ticks": 5 }, shot.clone(),
                    { "type": "move_to", "to": { "x": 49_000, "y": 60_000 }, "ticks": 10 },
                    { "type": "wait", "ticks": 5 }, { "type": "loop" }
                ],
                "rules": [rule(
                    json!({ "type": "hp_below", "who": { "part": "horn" }, "pct": 50 }),
                    json!("self"), telegraph(), 0
                )]
            },
            {
                "transition": [{ "type": "clear_orbs" },
                               { "type": "invulnerable_ticks", "ticks": 10 }],
                "timeline": [shot, { "type": "wait", "ticks": 8 }, { "type": "loop" }]
            }
        ]
    });
    // The Main weapon's ray hits the horn and the boss every 6 ticks. The main character stands
    // in the boss's zones, and holds the most a stage may: a hit is 100 shown.
    let mut stage = stage_full(600, Some(boss), vec![], vec![]);
    stage.player.hp = 999;
    ("boss", bytes(&stage), firing_for(600))
}

/// A shooter in front of a main character with 3 hp who does not move.
fn fail() -> Definition {
    let shooter = enemy(
        1000,
        hold(46_080, 120_000),
        json!([rule(
            always(),
            json!("player"),
            json!({ "type": "attack",
                            "attack": { "preset": { "id": "aimed_single", "args": [15] } } }),
            10
        )]),
    );
    let stage = stage_with(
        600,
        Some(far_boss()),
        vec![wave(1, 1, 0, shooter, 46_080, 120_000)],
    );
    (
        "fail",
        bytes(&stage),
        (0..300).map(|_| common::idle()).collect(),
    )
}

/// Melee (ADR-055): three enemies trail the main character and swing at it, a wind-up, an arc
/// and a recovery, rooted while they wind up and swing; a companion orbiting it cleaves them
/// with a wide arc that hits again every 2 ticks, through their defence. The main character
/// stands still, then walks from side to side, playing the `giant_sword`'s combos, whose smashes
/// pull what they hit. Nobody fires an orb.
fn melee() -> Definition {
    let phase = |kind: &str, ticks: u32, hit_box: Value, damage: u32, move_pct: u8| {
        json!({ "kind": kind, "ticks": ticks, "hit_box": hit_box, "damage": damage,
                "move_pct": move_pct })
    };
    let arc = |radius: i32, half: i32| {
        json!({ "type": "arc", "offset": { "forward": 1_024 }, "radius": radius,
                "half_angle": half * 256 })
    };
    let swing = json!([
        phase("wind_up", 12, arc(6_144, 60), 2, 0),
        phase("active", 3, arc(6_144, 60), 2, 0),
        json!({ "kind": "recovery", "ticks": 10 })
    ]);
    let mut swordsman = enemy(
        12,
        json!({ "type": "follow", "distance": 4_096 }),
        json!([rule(
            always(),
            json!("player"),
            json!({ "type": "attack", "attack": { "inline": swing } }),
            0
        )]),
    );
    // Defence 50 (ADR-056): each cleave of 3 lands as 3 × 100 / 150 = 2.
    swordsman["defence"] = json!(50);
    let mut cleave = phase("active", 4, arc(10_240, 90), 3, 100);
    cleave["rehit_ticks"] = json!(2);
    let guard = companion(
        30,
        json!({ "type": "orbit", "radius": 6_144 }),
        json!([rule(
            always(),
            json!("nearest_enemy"),
            json!({ "type": "attack", "attack": { "inline": [
                { "kind": "wind_up", "ticks": 6, "hit_box": arc(10_240, 90) },
                cleave,
                { "kind": "recovery", "ticks": 8 }
            ] } }),
            0
        )]),
    );
    let waves = [(1, 30_000), (40, 46_080), (80, 62_000)]
        .into_iter()
        .map(|(at, x)| wave(at, 1, 0, swordsman.clone(), x, 128_000))
        .collect();
    let stage = armed(
        stage_full(600, Some(far_boss()), waves, vec![guard]),
        json!({ "preset": { "id": "giant_sword" } }),
    );
    let inputs = (0..300)
        .map(|t| {
            let input = match t {
                0..150 => idle(),
                _ if (t / 40) % 2 == 0 => moving(33, 0),
                _ => moving(-33, 0),
            };
            combos(t, input)
        })
        .collect();
    ("melee", bytes(&stage), inputs)
}

/// Zones (core Z1): three enemies and a boss lay out telegraphed areas that do not move, and a
/// companion lays out one of its own. One enemy puts a circle on the spot where the main character
/// stands, with a ring of eight round it, and swings a hit box anchored at that spot after a
/// wind-up; one fans five lines at the main character; one sweeps a line. The boss rings
/// circles round itself, and its second phase begins with `clear_zones`. The main character
/// stands still, then walks from side to side, so some zones catch it and some find it gone.
fn zones() -> Definition {
    let zone = |shape: Value, anchor: &str, wind_up: u32, active: u32| {
        json!({ "type": "zone", "shape": shape, "anchor": anchor,
                "wind_up_ticks": wind_up, "active_ticks": active })
    };
    let circle = |radius: i32| json!({ "type": "circle", "radius": radius });
    let line =
        |length: i32, width: i32| json!({ "type": "line", "length": length, "width": width });
    let attack = |phases: Value| {
        json!([rule(
            always(),
            json!("player"),
            json!({ "type": "attack", "attack": { "inline": phases } }),
            90
        )])
    };
    let mut round = zone(circle(3_072), "target", 30, 4);
    round["offset"] = json!({ "forward": 12_288 });
    let spotter = enemy(
        20,
        hold(30_000, 120_000),
        attack(json!([
            { "kind": "wind_up", "ticks": 2, "damage": 2, "spawn": [
                zone(circle(6_144), "target", 30, 6),
                { "type": "ring", "count": 8, "body": [round] },
            ] },
            { "kind": "recovery", "ticks": 20 },
            { "kind": "wind_up", "ticks": 10 },
            { "kind": "active", "ticks": 3, "damage": 2,
              "hit_box": { "type": "circle", "anchor": "target", "radius": 5_120 } },
        ])),
    );
    let fanner = enemy(
        20,
        hold(60_000, 110_000),
        attack(json!([
            { "kind": "wind_up", "ticks": 2, "damage": 2, "spawn": [
                { "type": "spread", "count": 5, "angle": 40 * 256,
                  "body": [zone(line(48_000, 3_072), "attacker", 24, 6)] },
            ] },
            { "kind": "recovery", "ticks": 40 },
        ])),
    );
    let sweeper = enemy(
        20,
        hold(46_080, 100_000),
        attack(json!([
            { "kind": "wind_up", "ticks": 12 },
            { "kind": "active", "ticks": 3, "damage": 2,
              "hit_box": { "type": "line", "angle": 10 * 256, "length": 40_000,
                           "width": 2_048 } },
            { "kind": "recovery", "ticks": 15 },
        ])),
    );
    let mut arc_zone = zone(
        json!({ "type": "arc", "radius": 12_288, "half_angle": 60 * 256 }),
        "attacker",
        6,
        10,
    );
    arc_zone["direction"] = json!({ "type": "aimed", "offset": 0 });
    let guard = companion(
        30,
        json!({ "type": "orbit", "radius": 6_144 }),
        json!([rule(
            always(),
            json!("nearest_enemy"),
            json!({ "type": "attack", "attack": { "inline": [
                { "kind": "wind_up", "ticks": 2, "damage": 3, "spawn": [arc_zone] },
                { "kind": "recovery", "ticks": 10 },
            ] } }),
            40
        )]),
    );
    // The first phase's rings warn for a long time, so that they are still alive when the
    // phase ends; the second phase's are short.
    let boss_attack = |wind_up: u32| {
        let mut ring = zone(circle(4_096), "attacker", wind_up, 10);
        ring["offset"] = json!({ "forward": 20_480 });
        json!({ "type": "attack", "attack": { "inline": [
            { "kind": "active", "ticks": 1, "damage": 2, "spawn": [
                { "type": "ring", "count": 6, "body": [ring] },
            ] },
        ] } })
    };
    let mut boss = far_boss();
    boss["phases"] = json!([
        { "until": { "type": "time_above", "ticks": 149 },
          "timeline": [boss_attack(300), { "type": "wait", "ticks": 60 }, { "type": "loop" }] },
        { "transition": [{ "type": "clear_zones" }],
          "timeline": [boss_attack(40), { "type": "wait", "ticks": 90 }, { "type": "loop" }] },
    ]);
    let waves = vec![
        wave(1, 1, 0, spotter, 30_000, 120_000),
        wave(1, 1, 0, fanner, 60_000, 110_000),
        wave(1, 1, 0, sweeper, 46_080, 100_000),
    ];
    let stage = quiet(stage_full(600, Some(boss), waves, vec![guard]));
    let inputs = (0..300)
        .map(|t| match t {
            0..60 => idle(),
            _ if (t / 40) % 2 == 0 => moving(60, 0),
            _ => moving(-60, 0),
        })
        .collect();
    ("zones", bytes(&stage), inputs)
}

/// The ranged cap (core Z2): eight shooters line the top of the field and fire an aimed shot
/// after a wind-up, each marked `ranged`, so that at most three (the zone pool, RA1) are winding
/// up or active at any tick and the others wait with their cooldown ready; two more shoot the same way without the
/// mark and are never held back. The main character walks from side to side.
fn ranged_cap() -> Definition {
    let shot = |ranged: bool| {
        let mut action = json!({ "type": "attack", "attack": { "inline": [
            { "kind": "wind_up", "ticks": 12, "damage": 2, "spawn": [
                { "type": "orb", "speed": 768, "ticks": 200,
                  "direction": { "type": "aimed", "offset": 0 } } ] },
            { "kind": "active", "ticks": 8 },
            { "kind": "recovery", "ticks": 10 },
        ] } });
        if ranged {
            action["ranged"] = json!(true);
        }
        json!([rule(always(), json!("player"), action, 30)])
    };
    let waves = (0..10)
        .map(|i| {
            let x = 10_000 + i * 8_000;
            let shooter = enemy(20, hold(x, 110_000), shot(i < 8));
            wave(1, 1, 0, shooter, x, 110_000)
        })
        .collect();
    let stage = quiet(stage_full(600, Some(far_boss()), waves, vec![]));
    let inputs = (0..300)
        .map(|t| match t {
            0..40 => idle(),
            _ if (t / 40) % 2 == 0 => moving(40, 0),
            _ => moving(-40, 0),
        })
        .collect();
    ("ranged_cap", bytes(&stage), inputs)
}

fn baseline_sword() -> serde_json::Value {
    let baseline: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/weapons_before.json")).unwrap();
    common::unscaled(
        baseline["giant_sword"]["giant_sword_base"].clone(),
        common::SCALE,
    )
}

/// Orbs (core Z2): a breakable orb ring and a plain one, each bursting into zones where its orb
/// ended, and a boss that fills the field past the caps. The main character swings the
/// `giant_sword` while it walks, so some breakable orbs are cut before they burst.
fn orbs() -> Definition {
    let zone = json!({ "type": "zone", "shape": { "type": "circle", "radius": 4_096 },
                       "anchor": "attacker", "wind_up_ticks": 10, "active_ticks": 8 });
    // A pattern is at most four levels deep: a ring of four zones under an orb under a ring of
    // orbs fits, a ring of orbs under a repeat does not, so the boss's orbs burst into one zone.
    let orb = |breakable: bool, ticks: u32, burst: Value| {
        json!({ "type": "orb", "speed": 160, "ticks": ticks, "breakable": breakable,
                "direction": { "type": "aimed", "offset": 0 }, "burst": [burst] })
    };
    let four = json!({ "type": "ring", "count": 4, "body": [zone.clone()] });
    let attack = |nodes: Value, cooldown: u32| {
        json!([rule(
            always(),
            json!("player"),
            json!({ "type": "attack", "attack": { "inline": [
                { "kind": "wind_up", "ticks": 4, "damage": 2, "spawn": [nodes] },
                { "kind": "recovery", "ticks": 10 },
            ] } }),
            cooldown
        )])
    };
    let ring = |orb: Value| json!({ "type": "ring", "count": 8, "body": [orb] });
    let shooter = |x: i32, y: i32, hp: u32, breakable: bool, ticks: u32| {
        let e = enemy(
            hp,
            hold(x, y),
            attack(ring(orb(breakable, ticks, four.clone())), 100),
        );
        wave(1, 1, 0, e, x, y)
    };
    let mut boss = far_boss();
    boss["phases"] = json!([{ "timeline": [
        { "type": "attack", "attack": { "inline": [
            { "kind": "active", "ticks": 1, "damage": 2, "spawn": [
                { "type": "repeat", "count": 12, "interval_ticks": 3, "body": [
                    { "type": "ring", "count": 3, "body": [orb(true, 150, zone.clone())] } ] },
            ] },
        ] } },
        { "type": "wait", "ticks": 200 },
        { "type": "loop" },
    ] }]);
    let waves = vec![
        // First, close to the main character, so that its sword reaches the orbs as they leave.
        shooter(46_080, 142_000, 100_000, true, 90),
        shooter(20_000, 120_000, 50, true, 90),
        shooter(40_000, 120_000, 50, true, 90),
        shooter(60_000, 120_000, 50, false, 70),
        shooter(80_000, 120_000, 50, false, 70),
    ];
    let stage = armed(
        stage_full(600, Some(boss), waves, vec![]),
        json!({ "inline": baseline_sword() }),
    );
    let inputs = (0..300)
        .map(|t| {
            let input = match t {
                0..30 => idle(),
                _ if (t / 40) % 2 == 0 => moving(30, 0),
                _ => moving(-30, 0),
            };
            combos(t, input)
        })
        .collect();
    ("orbs", bytes(&stage), inputs)
}

/// The orb cage (Z3a-3): two foes fire rings of orbs that halt and are joined by beams, again every
/// 120 ticks. The near one fires a ring of five (beams of the orb's own damage) close above the
/// main character, whose `giant_sword` cuts some orbs, so their beams are cut with them; the far
/// one fires a ring of four whose beams deal their own damage. The main character walks to and fro
/// through both.
fn orb_cage() -> Definition {
    let cage = |count: u32, link: Value| {
        json!({ "type": "ring", "count": count, "body": [{
            "type": "orb", "speed": 512, "ticks": 200, "breakable": true,
            "direction": { "type": "absolute", "angle": 5_120 },
            "stop_after": 40, "link": link }] })
    };
    let shooter = |x: i32, y: i32, nodes: Value| {
        let e = enemy(
            100_000,
            hold(x, y),
            json!([rule(
                always(),
                json!("player"),
                json!({ "type": "attack", "attack": { "inline": [
                    { "kind": "wind_up", "ticks": 4, "damage": 2, "spawn": [nodes] },
                    { "kind": "recovery", "ticks": 10 },
                ] } }),
                120
            )]),
        );
        wave(1, 1, 0, e, x, y)
    };
    let waves = vec![
        shooter(
            46_080,
            132_000,
            cage(5, json!({ "width": 1_536, "rehit_ticks": 40 })),
        ),
        shooter(
            61_440,
            110_000,
            cage(4, json!({ "width": 2_048, "damage": 3, "rehit_ticks": 60 })),
        ),
    ];
    let stage = armed(
        stage_full(600, Some(far_boss()), waves, vec![]),
        json!({ "inline": baseline_sword() }),
    );
    let inputs = (0..300)
        .map(|t| {
            let input = match t {
                0..20 => idle(),
                _ if (t / 60) % 2 == 0 => moving(50, 0),
                _ => moving(-50, 0),
            };
            combos(t, input)
        })
        .collect();
    ("orb_cage", bytes(&stage), inputs)
}

/// Warn-only zones and the target-anchored cage (Z3a-4): a charger lays a warning lane on the main
/// character in its wind-up and body-hits when it ends (the hit lands, as no invulnerability came
/// first), and a caster fires a ring of four orbs `anchor: target` with an offset, which halt and
/// are linked, again every 150 ticks. The main character walks to and fro.
fn warn_anchor() -> Definition {
    let phases = |nodes: Value, body: Value| {
        json!([
            { "kind": "wind_up", "ticks": 40, "damage": 2, "spawn": nodes },
            body,
            { "kind": "recovery", "ticks": 20 },
        ])
    };
    let rule = |phases: Value, cooldown: u32| {
        json!([rule(
            always(),
            json!("player"),
            json!({ "type": "attack", "attack": { "inline": phases } }),
            cooldown
        )])
    };
    let lane = json!({ "type": "zone", "anchor": "target", "wind_up_ticks": 40, "warn_only": true,
        "shape": { "type": "line", "length": 8_192, "width": 3_072 } });
    let strike = json!({ "kind": "active", "ticks": 6, "damage": 3,
        "hit_box": { "type": "circle", "anchor": "target", "radius": 4_096 } });
    let cage = json!({ "type": "ring", "count": 4, "body": [{
        "type": "orb", "speed": 512, "ticks": 200, "anchor": "target",
        "offset": { "forward": 4_096 }, "direction": { "type": "aimed", "offset": 5_120 },
        "stop_after": 40, "link": { "width": 2_048, "rehit_ticks": 60 } }] });
    let charger = enemy(
        100_000,
        hold(46_080, 120_000),
        rule(phases(json!([lane]), strike), 150),
    );
    let caster = enemy(
        100_000,
        hold(61_440, 100_000),
        rule(
            phases(json!([cage]), json!({ "kind": "recovery", "ticks": 4 })),
            150,
        ),
    );
    let waves = vec![
        wave(1, 1, 0, charger, 46_080, 120_000),
        wave(1, 1, 0, caster, 61_440, 100_000),
    ];
    let stage = quiet(stage_full(600, Some(far_boss()), waves, vec![]));
    let inputs = (0..300)
        .map(|t| match t {
            0..50 => idle(),
            _ if (t / 50) % 2 == 0 => moving(50, 0),
            _ => moving(-50, 0),
        })
        .collect();
    ("warn_anchor", bytes(&stage), inputs)
}

/// Aim assist and auto mode (ADR-072, ADR-073) with the giant_sword, whose assist range is 160
/// pixels: three bats to the left and an elite to the right, all within it, and a boss far up
/// the field. Auto mode alone for 100 ticks (the elite first, then the bats as they fall);
/// combos by hand with assist for 80 ticks; then assist on while the main character walks up to the
/// boss, pressing `main` twice and holding it from the third step's start for a charge (which
/// halves its speed), with auto mode on again from tick 302, until the boss is the target. The
/// presses wait until the combos by hand have run out of their window (the Sword's chain is five
/// A's deep), so they start from the root.
fn assist() -> Definition {
    let bat = enemy(30, hold(0, 0), json!([]));
    let mut elite = enemy(40, hold(0, 0), json!([]));
    elite["elite"] = json!({ "range": 1 });
    let at = |e: &Value, x: i32, y: i32| {
        let mut e = e.clone();
        e["movement"] = hold(x, y);
        wave(1, 1, 0, e, x, y)
    };
    let waves = vec![
        at(&bat, 20_000, 140_000),
        at(&bat, 22_000, 152_000),
        at(&bat, 26_000, 147_456),
        at(&elite, 76_000, 147_456),
    ];
    let stage = armed(
        stage_full(600, Some(boss_at(1_000_000, 40_000)), waves, vec![]),
        json!({ "preset": { "id": "giant_sword" } }),
    );
    let inputs = (0..360)
        .map(|t| match t {
            0..100 => Input {
                auto: true,
                ..idle()
            },
            100..180 => Input {
                assist: true,
                ..combos(t, idle())
            },
            _ => Input {
                main: t == 245 || t == 265 || (280..325).contains(&t),
                assist: true,
                auto: t >= 302,
                ..moving(0, -127)
            },
        })
        .collect();
    ("assist", bytes(&stage), inputs)
}

/// The evade on three charges (ADR-074): a shooter fires at the main character, who walks from
/// side to side and presses the evade four times in a row, each during the dash before it. The
/// first three dash, the fourth finds no charge ready; the charges come back one by one before
/// the next cycle, which does it again.
fn evade_charges() -> Definition {
    let shooter = enemy(
        1000,
        hold(46_080, 100_000),
        json!([rule(
            always(),
            json!("player"),
            json!({ "type": "attack",
                    "attack": { "preset": { "id": "aimed_single", "args": [15] } } }),
            10
        )]),
    );
    let stage = stage_full(
        300,
        Some(far_boss()),
        vec![wave(1, 1, 0, shooter, 46_080, 100_000)],
        vec![],
    );
    let mut v = serde_json::to_value(stage).unwrap();
    v["player"]["evade"]["charges"] = json!(3);
    // A hit is 100 shown (the bundled run stats): the most hit points a stage may hold last the cycle.
    v["player"]["hp"] = json!(999);
    let inputs = (0..300)
        .map(|t| {
            let c = t % 100;
            Input {
                evade: matches!(c, 10 | 13 | 16 | 19),
                ..moving(if c < 50 { 127 } else { -127 }, 0)
            }
        })
        .collect();
    ("evade", bytes(&v), inputs)
}

/// A landscape map of 2,048 by 1,024 pixels (core#87). The main character starts near the
/// right edge, walks into the top-right corner (clamped there), then left along the top, shooting
/// along its facing; enemies cross the middle of the map to the right, shooting at it.
fn landscape() -> Definition {
    let (width, height) = (524_288, 262_144);
    let crosser = enemy(
        30,
        json!({ "type": "straight", "vx": 512, "vy": 0 }),
        json!([attack_rule(always(), 20)]),
    );
    let mut stage = serde_json::to_value(stage_full(
        300,
        Some(far_boss()),
        vec![
            wave(1, 4, 25, crosser.clone(), width - 153_600, 30_000),
            wave(60, 4, 25, crosser, width - 153_600, 8_000),
        ],
        vec![],
    ))
    .unwrap();
    stage["map"] = json!({ "width": width, "height": height,
                           "start": { "x": width - 30_720, "y": 20_000 } });
    // The main character holds the most a stage may, so that the zones do not end the walk: a
    // hit is 100 shown.
    stage["player"]["hp"] = json!(999);
    let inputs = (0..300)
        .map(|t| {
            let input = match t {
                0..60 => moving(127, 0),
                60..100 => moving(0, -127),
                100..220 => moving(-127, 0),
                _ => moving(-64, 64),
            };
            firing(input, t)
        })
        .collect();
    ("landscape", bytes(&stage), inputs)
}

/// A stage that runs out of ticks with the boss standing.
fn timeout() -> Definition {
    let stage = stage_with(90, Some(far_boss()), vec![]);
    (
        "timeout",
        bytes(&stage),
        (0..90).map(|_| common::idle()).collect(),
    )
}

/// Attack phases that dash (core Z3a-2): enemies charge the main character from three sides
/// again and again, a crowd stands on their lines so that the push-apart meets a dasher, and the
/// boss charges with its horn.
fn dash() -> Definition {
    let phases = json!([
        { "kind": "wind_up", "ticks": 20, "move_pct": 0 },
        { "kind": "active", "ticks": 12, "damage": 3, "move_pct": 0,
          "dash": { "distance": 150 * 256 },
          "hit_box": { "type": "circle", "radius": 12 * 256 } },
        { "kind": "recovery", "ticks": 10, "move_pct": 0 },
    ]);
    let rules = json!([{ "when": always(), "target": "player", "cooldown_ticks": 90,
        "do": { "type": "attack", "attack": { "inline": phases.clone() } } }]);
    let still = json!({ "type": "straight", "vx": 0, "vy": 0 });
    let charger = |x: i32, y: i32| {
        wave(
            1,
            1,
            0,
            enemy(100, still.clone(), rules.clone()),
            x * 256,
            y * 256,
        )
    };
    let bystander = |x: i32, y: i32| {
        wave(
            1,
            1,
            0,
            enemy(100, still.clone(), json!([])),
            x * 256,
            y * 256,
        )
    };
    let mut waves = vec![charger(40, 300), charger(180, 230), charger(320, 300)];
    // The crowd on the middle charger's line.
    waves.extend([
        bystander(178, 300),
        bystander(184, 306),
        bystander(180, 312),
    ]);
    let boss = json!({
        "base": "golem", "hp": 100_000, "radius": 8192,
        "spawn": { "x": 46_080, "y": 30_000 },
        "parts": [{ "id": "horn", "asset": "horns_2", "hp": 100,
                    "offset": { "x": 0, "y": 4_096 }, "radius": 2_048 }],
        "phases": [{ "timeline": [
            { "type": "attack", "attack": { "inline": phases } },
            { "type": "wait", "ticks": 80 },
            { "type": "loop" } ] }],
    });
    let stage = quiet(stage_full(600, Some(boss), waves, vec![]));
    let inputs = (0..300)
        .map(|t| match t {
            0..60 => idle(),
            _ if (t / 50) % 2 == 0 => moving(40, 0),
            _ => moving(-40, 0),
        })
        .collect();
    ("dash", bytes(&stage), inputs)
}

pub(crate) fn definitions() -> Vec<Definition> {
    vec![
        (
            "clear",
            bytes(&common::clearing_stage_json()),
            common::clearing_inputs(),
        ),
        agents(),
        assist(),
        boss(),
        evade_charges(),
        fail(),
        landscape(),
        melee(),
        orbs(),
        orb_cage(),
        warn_anchor(),
        dash(),
        ranged_cap(),
        timeout(),
        zones(),
    ]
}

/// The run cases: the trial mode played until the main character falls, raising its stats by priority
/// and answering its offers on the way, with every kind of option; and the same from another
/// seed, whose run takes a stage skill from an elite's star and uses it.
/// A run case: its name, setup, the derived pack it plays on (the bundled pack for `None`) and its commands.
type RunDefinition = (&'static str, RunSetup, Option<Vec<u8>>, Vec<Command>);

fn run_definitions() -> Vec<RunDefinition> {
    use crate::run_play::{play_run, play_run_from, play_run_in};
    use common::run::{
        ELITES_SEED, KILL_SEED, STAGE_SKILL_SEED, TIME_LIMIT_SEED, TRIAL_LENGTH, time_limit_setup,
        trial_setup,
    };
    // `STAGE_SKILL_SEED` was found with every point on attack: the priority keeps that run.
    let stage_skill = RunSetup {
        seed: STAGE_SKILL_SEED,
        stat_priority: vec!["attack".into()],
        star_pool: Some(vec!["black_hole_rare".into()]),
        ..trial_setup()
    };
    let stage_skill_content = RunContent::bundled()
        .clone()
        .with_run_stats(common::design_run_stats());
    let stage_skill_commands = play_run_in(&stage_skill, &stage_skill_content, TRIAL_LENGTH);
    // The time_limit mode's waves around the main character, played until it falls (it does
    // before the first boss time; the bosses are in `tests/time_limit_run.rs`).
    let time_limit = RunSetup {
        seed: TIME_LIMIT_SEED,
        ..time_limit_setup()
    };
    let time_limit_commands = play_run_from(&time_limit, MAX_TICKS);
    // The time_limit mode on easy and on rainy (ADR-110): the enemies' hit points and damage
    // scaled, the stars inside the rarity range; easy with its own boss times and level table.
    let easy = RunSetup {
        difficulty: "easy".into(),
        ..time_limit_setup()
    };
    let easy_commands = play_run_from(&easy, 6_000);
    let rainy = RunSetup {
        difficulty: "rainy".into(),
        ..time_limit_setup()
    };
    let rainy_commands = play_run_from(&rainy, 6_000);
    // Two short modes with no waves and the final boss at tick 60, the only boss: in the first
    // the overtime is long enough for the main character to kill it (it does, at tick 8098);
    // in the second it ends at tick 600 with the boss alive.
    let (_, kill_content) = short_time_limit(12_000);
    let kill_setup = RunSetup {
        seed: KILL_SEED,
        ..time_limit_setup()
    };
    let kill_commands = play_run_with(&kill_setup, &kill_content, 20_060, boss_input);
    let (_, overtime_content) = short_time_limit(540);
    let kill_setup_2 = time_limit_setup();
    let overtime_commands = play_run_with(&kill_setup_2, &overtime_content, 600, boss_input);
    // The elites run: every elite type, the cap of 5 reached, and each elite's event; its own
    // mode file, below.
    let (_, elites_content) = elites_time_limit();
    let elites_setup = RunSetup {
        seed: ELITES_SEED,
        ..time_limit_setup()
    };
    let elites_script = EliteScript::default();
    let elites_commands = play_run_with(&elites_setup, &elites_content, ELITES_TICKS, |e, t| {
        elites_script.input(e, t)
    });
    // The tutorial mode (ADR-086): three minutes of bats and one weak golem, which the bot kills.
    let tutorial = RunSetup {
        mode: "tutorial".into(),
        ..trial_setup()
    };
    let tutorial_commands = play_run_with(&tutorial, RunContent::bundled(), 10_800, boss_input);
    vec![
        ("tutorial", tutorial, None, tutorial_commands),
        ("trial", trial_setup(), None, play_run(TRIAL_LENGTH)),
        (
            "stage_skill",
            stage_skill,
            Some(override_pack("stage_skill")),
            stage_skill_commands,
        ),
        ("time_limit", time_limit, None, time_limit_commands),
        ("easy", easy, None, easy_commands),
        ("rainy", rainy, None, rainy_commands),
        (
            "time_limit_kill",
            kill_setup,
            Some(override_pack("time_limit_kill")),
            kill_commands,
        ),
        (
            "time_limit_overtime",
            kill_setup_2,
            Some(override_pack("time_limit_overtime")),
            overtime_commands,
        ),
        (
            "elites",
            elites_setup,
            Some(override_pack("elites")),
            elites_commands,
        ),
    ]
}

/// The ticks of the `elites` run: its length and its overtime.
const ELITES_TICKS: u32 = 48_700;

/// The elite each whole minute brings in the `elites` run, from minute 1: a leader and its
/// escort, a goblin, a shrine and a mimic, then a low elite that fills the cap of 5, one that the
/// cap holds back, the high and the middle grades, and again a goblin, a shrine and a mimic, which
/// stay untouched until the boss comes at minute 13.
const ELITES_BY_MINUTE: [&str; 12] = [
    "elite_leader",
    "elite_goblin",
    "elite_shrine",
    "elite_mimic",
    "elite_low",
    "elite_low",
    "elite_high",
    "elite_mid",
    "elite_goblin",
    "elite_shrine",
    "elite_mimic",
    "elite_low",
];

/// The `time_limit` mode of the `elites` run, as its file and as the content a run reads it
/// with: the bundled mode with no waves, one tier of the elite table for each minute naming the
/// one elite that minute brings (so that the draws, which the seed still makes, never change
/// what comes), the final boss as the only boss at minute 13, 1900 ticks of overtime (the boss's call is due 1800 ticks after it comes),
/// and the leader's aura at 100%, so that its raise is plain: the escort shoots for the default
/// 100 (its swing, since BR1a), which 100% makes a hit of 200 (the bundled 25% makes 125, `tests/elite_pack_leader.rs`).
pub(crate) fn elites_time_limit() -> (String, RunContent) {
    let mut v: Value = serde_json::from_slice(common::bundled_file("time_limit")).unwrap();
    v["length_ticks"] = json!(46_800);
    v["boss_ticks"] = json!([46_800]);
    v["overtime_ticks"] = json!(1_900);
    v["waves"] = json!([]);
    let tiers: Vec<Value> = std::iter::once("elite_low")
        .chain(ELITES_BY_MINUTE)
        .enumerate()
        .map(|(minute, elite)| {
            json!({ "start_tick": minute * 3600,
                    "weights": [{ "elite": elite, "weight": 1 }] })
        })
        .collect();
    v["elite_table"] = json!(tiers);
    let leader = v["elite_types"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|t| t["id"] == "elite_leader")
        .unwrap();
    leader["enemy"]["elite"]["aura"]["damage_percent"] = json!(100);
    // Compact: pretty-printed, the mode file passes the 128 KiB limit (Z3b content).
    let json = serde_json::to_string(&v).unwrap() + "\n";
    let content = RunContent::bundled()
        .clone()
        .with_time_limit(TimeLimit::parse(json.as_bytes(), Versions::CURRENT).unwrap())
        .with_run_stats(common::design_run_stats());
    (json, content)
}

/// How near the scripted main character of the `elites` run stands to an enemy it fights: 32
/// pixels, inside the `giant_sword`'s reach.
const FIGHT_NEAR: i64 = 8_192;

/// The scripted main character of the `elites` run. It meets the leader and kills its escort
/// (the escort's shots land inside the aura), then waits, away from the elites, until the cap of
/// 5 has held an elite minute back; then it disturbs the goblin, touches the shrine and kills the
/// guardians, reveals the mimic and kills the elites that are left; it leaves the second goblin,
/// shrine and mimic alone, and fights the boss.
#[derive(Default)]
struct EliteScript {
    leader_met: std::cell::Cell<bool>,
    goblin_met: std::cell::Cell<bool>,
    shrine_touched: std::cell::Cell<bool>,
    /// The mimic once it is revealed, from then on an elite like the others to kill.
    mimic_revealed: std::cell::Cell<Option<EntityId>>,
}

impl EliteScript {
    /// The buttons of tick `tick`: the script's, with the dodging of the run script's.
    fn input(&self, engine: &Engine, tick: u32) -> Input {
        let wanted = self.wanted(engine, tick);
        if engine
            .snapshot()
            .entities
            .iter()
            .any(|e| e.kind == EntityKind::Boss)
        {
            return wanted;
        }
        // It does not evade the swings: its fight with the leader's escort is the point of the case.
        crate::zone_bot::avoid_zones(engine, common::run::avoid_orbs(engine, wanted, false))
    }

    fn wanted(&self, engine: &Engine, tick: u32) -> Input {
        for e in engine.events() {
            match e {
                DomainEvent::ShrineActivated { .. } => self.shrine_touched.set(true),
                DomainEvent::MimicRevealed { elite } => self.mimic_revealed.set(Some(*elite)),
                _ => {}
            }
        }
        let s = engine.snapshot();
        if s.entities.iter().any(|e| e.kind == EntityKind::Boss) {
            return crate::zone_bot::run_input_for(engine, tick, &[EntityKind::Boss], u32::MAX);
        }
        let me = s.player.at;
        let enemies = || {
            s.entities
                .iter()
                .filter(|e| matches!(e.kind, EntityKind::Enemy | EntityKind::Shrine))
        };
        let d2 = |at: stella_rain_core::fixed::Point| {
            let (x, y) = (i64::from(at.x.0 - me.x.0), i64::from(at.y.0 - me.y.0));
            x * x + y * y
        };
        let nearest = |pick: &dyn Fn(&&stella_rain_core::snapshot::EntityView) -> bool| {
            enemies().filter(|e| pick(e)).min_by_key(|e| d2(e.at))
        };
        let toward = |d: i32| match d {
            d if d > 256 => 127,
            d if d < -256 => -127,
            _ => 0,
        };
        // Walks to within `stop` of `at`, pressing the buttons of the run script if `fight`.
        let go = |at: stella_rain_core::fixed::Point, stop: i64, fight: bool| {
            let walk = i8::from(d2(at) > stop * stop);
            let input = if fight {
                common::run::run_script(tick)
            } else {
                Input::default()
            };
            Input {
                move_x: walk * toward(at.x.0 - me.x.0),
                move_y: walk * toward(at.y.0 - me.y.0),
                ..input
            }
        };
        let plain_elite = |e: &&stella_rain_core::snapshot::EntityView| {
            e.elite
                && e.asset.as_str() != "golem"
                && (Some(e.id) == self.mimic_revealed.get()
                    || !(e.asset.as_str() == "fairy_a" && e.hp <= 12_000))
        };
        let away = |from: stella_rain_core::fixed::Point| Input {
            move_x: -toward(from.x.0 - me.x.0),
            move_y: -toward(from.y.0 - me.y.0),
            ..Input::default()
        };

        // The leader: walk up to it, kill the escort that comes, then leave it be.
        if !self.leader_met.get() && tick >= 3_600 {
            match nearest(&|e| e.aura_radius.is_some()) {
                Some(leader) if d2(leader.at) > 100 * 100 * 65_536 => {
                    return go(leader.at, 100 * 256, false);
                }
                Some(_) => {
                    match nearest(&|e| e.kind == EntityKind::Enemy && !e.elite && !e.disguised) {
                        Some(escort) => return go(escort.at, FIGHT_NEAR, true),
                        None => self.leader_met.set(true),
                    }
                }
                None => self.leader_met.set(true),
            }
        }

        // Once the cap has held a minute back: the goblin, the shrine, the mimic, the rest.
        if (21_660..25_100).contains(&tick) {
            if !self.goblin_met.get() {
                match nearest(&|e| e.asset.as_str() == "fairy_a" && e.hp <= 12_000 && e.elite) {
                    Some(g) if d2(g.at) > 40 * 40 * 65_536 => return go(g.at, 40 * 256, false),
                    _ => self.goblin_met.set(true),
                }
            }
            if !self.shrine_touched.get() {
                if let Some(shrine) = nearest(&|e| e.asset.as_str() == "golem") {
                    return go(shrine.at, 0, false);
                }
            }
            if let Some(guardian) =
                nearest(&|e| e.kind == EntityKind::Enemy && !e.elite && !e.disguised)
            {
                return go(guardian.at, FIGHT_NEAR, true);
            }
            if self.mimic_revealed.get().is_none() {
                if let Some(mimic) = nearest(&|e| e.disguised) {
                    return go(mimic.at, 100 * 256, false);
                }
            }
            if let Some(rest) = nearest(&plain_elite) {
                return go(rest.at, FIGHT_NEAR, true);
            }
        }

        // Otherwise wait, out of the range of the elites that act (200 pixels).
        match nearest(&|e| e.elite && e.asset.as_str() != "golem") {
            Some(e) if d2(e.at) < 240 * 240 * 65_536 => away(e.at),
            _ => Input::default(),
        }
    }
}

/// A `time_limit` mode with no waves, the final boss at tick 60 and `overtime` ticks of overtime,
/// as its file and as the content a run reads it with.
pub(crate) fn short_time_limit(overtime: u32) -> (String, RunContent) {
    let mut v: Value = serde_json::from_slice(common::bundled_file("time_limit")).unwrap();
    v["length_ticks"] = json!(60);
    v["boss_ticks"] = json!([60]);
    v["overtime_ticks"] = json!(overtime);
    v["waves"] = json!([]);
    v["elite_table"] =
        json!([{ "start_tick": 0, "weights": [{ "elite": "elite_low", "weight": 1 }] }]);
    let json = serde_json::to_string_pretty(&v).unwrap() + "\n";
    let content = RunContent::bundled()
        .clone()
        .with_time_limit(TimeLimit::parse(json.as_bytes(), Versions::CURRENT).unwrap());
    (json, content)
}

/// The run's buttons going for the boss as well as the enemies, getting out of the way of
/// orbs throughout.
fn boss_input(engine: &Engine, tick: u32) -> Input {
    crate::zone_bot::run_input_for(
        engine,
        tick,
        &[EntityKind::Enemy, EntityKind::Shrine, EntityKind::Boss],
        u32::MAX,
    )
}

/// What a run case does, re-simulated from its file.
struct RunCase {
    /// The kind of option each choice takes.
    kinds: Vec<std::mem::Discriminant<OfferOption>>,
    /// Each stage skill used, with the level it is used at.
    used: Vec<(String, u8)>,
    /// How many hits got each match-up: weak, strong, resisted, none (ADR-100).
    match_ups: [u32; 4],
}

/// A run case re-simulated from its file, which must verify and end by the main character falling.
fn replay_run_case(name: &str) -> RunCase {
    replay_run_case_ending(name, Outcome::Failed)
}

/// A run case re-simulated from its file, which must verify and end in `outcome`.
fn replay_run_case_ending(name: &str, outcome: Outcome) -> RunCase {
    let bytes = fs::read(runs_dir().join(name).join("run.replay")).unwrap();
    let content = case_content(&runs_dir().join(name)).unwrap();
    let verified = run_replay::verify_run_with(&bytes, &content).unwrap();
    assert_eq!(verified.outcome, outcome, "the outcome of `{name}`");
    let replay = RunReplay::from_bytes(&bytes).unwrap();
    let commands = replay.unpack_commands().unwrap();
    assert_eq!(verified.commands, u32::try_from(commands.len()).unwrap());
    let mut engine = Engine::start_with(&replay.setup, &content).unwrap();
    let mut next = commands.iter().peekable();
    let (mut kinds, mut used, mut match_ups) = (Vec::new(), Vec::new(), [0; 4]);
    let mut raised = false;
    for input in replay.unpack_inputs(u32::MAX).unwrap() {
        while let Some(c) = next.next_if(|c| c.tick == engine.tick()) {
            if c.kind == CommandKind::Choose {
                let s = engine.snapshot();
                let options = match s.expertise.and_then(|g| g.offer) {
                    Some(offer) => offer.options,
                    None => s.progress.unwrap().offer.unwrap().options,
                };
                kinds.push(std::mem::discriminant(&options[c.index as usize]));
                engine.choose(c.index).unwrap();
            }
        }
        engine.step(input);
        for e in engine.events() {
            match e {
                DomainEvent::StageSkillUsed { skill, level } => {
                    used.push((skill.to_string(), *level));
                }
                DomainEvent::Hit { match_up, .. } => {
                    let i = match match_up {
                        MatchUp::Weak => 0,
                        MatchUp::Strong => 1,
                        MatchUp::Resisted => 2,
                        MatchUp::None => 3,
                    };
                    match_ups[i] += 1;
                }
                DomainEvent::StatRaised { .. } => raised = true,
                _ => {}
            }
        }
    }
    assert!(raised, "level-ups raise stats in `{name}`");
    RunCase {
        kinds,
        used,
        match_ups,
    }
}

#[test]
fn the_run_cases_are_what_their_names_say() {
    // Re-simulated, the trial's choices take each kind of option at least once.
    let kinds = replay_run_case("trial").kinds;
    use OfferOption::{Banish, Reroll, Skip, Take};
    for kind in [
        Take { candidate: 0 },
        Reroll { candidate: 0 },
        Skip,
        Banish { candidate: 0 },
    ] {
        assert!(
            kinds.contains(&std::mem::discriminant(&kind)),
            "no {kind:?} in the run case"
        );
    }
    // The time_limit run ends with the main character falling, after level-ups.
    // The healing keeps the scripted main character alive to the end of its script (MAX_TICKS,
    // the most ticks a replay the fuzz targets read may hold), where it used to fall.
    replay_run_case_ending("time_limit", Outcome::Running);
    // The short time_limit runs end by the final boss's kill and by the overtime limit.
    for (name, outcome, ticks, boss_alive) in [
        ("time_limit_kill", Outcome::Cleared, None, false),
        ("time_limit_overtime", Outcome::Failed, Some(600), true),
    ] {
        let case = runs_dir().join(name);
        let content = case_content(&case).unwrap();
        let bytes = fs::read(case.join("run.replay")).unwrap();
        let verified = run_replay::verify_run_with(&bytes, &content).unwrap();
        assert_eq!(verified.outcome, outcome, "`{name}` ends");
        if let Some(ticks) = ticks {
            assert_eq!(
                verified.ticks, ticks,
                "`{name}` ends at the length and the overtime"
            );
        }
        let replay = RunReplay::from_bytes(&bytes).unwrap();
        let mut engine = Engine::start_with(&replay.setup, &content).unwrap();
        let commands = replay.unpack_commands().unwrap();
        let mut next = commands.iter().peekable();
        for input in replay.unpack_inputs(u32::MAX).unwrap() {
            while let Some(c) = next.next_if(|c| c.tick == engine.tick()) {
                apply(&mut engine, c).unwrap();
            }
            engine.step(input);
        }
        let boss = engine
            .snapshot()
            .entities
            .into_iter()
            .find(|v| v.kind == EntityKind::Boss);
        assert_eq!(
            boss.is_some_and(|b| b.hp > 0),
            boss_alive,
            "`{name}`: the final boss"
        );
        let last = engine.events();
        let ended = if outcome == Outcome::Cleared {
            DomainEvent::StageCleared
        } else {
            DomainEvent::StageFailed
        };
        assert!(last.contains(&ended), "`{name}` reports {ended:?}");
    }
    // An elite's rare star offers the black hole, taken, then used each time it is ready.
    let used = replay_run_case("stage_skill").used;
    assert!(
        !used.is_empty() && used.iter().all(|u| *u == ("black_hole".into(), 1)),
        "the black hole is used: {used:?}"
    );
}

/// The kind of elite a spawned one is, told from what the snapshot shows of it on the tick it
/// comes (the mode's elite types, whose sprites are shared): a mimic by its disguise, a shrine
/// by the golem it stands as, a leader by its aura; the rest by sprite and hit points.
fn elite_kind(asset: &str, hp: u32, aura: bool) -> &'static str {
    match (asset, hp) {
        ("icon_hex", _) => "mimic",
        ("golem", _) => "shrine",
        ("horns_2", _) if aura => "leader",
        ("horns_2", _) => "high",
        ("bat", _) => "low",
        ("fairy_a", 12_000) => "goblin",
        ("fairy_a", _) => "mid",
        other => panic!("no elite type looks like {other:?}"),
    }
}

/// What the `elites` run case does, found by re-simulating it from its file.
#[derive(Default)]
struct EliteReach {
    /// Each elite spawned: its kind and the tick it came.
    spawned: Vec<(EntityId, &'static str, u32)>,
    /// The most elites alive at once (a disguised mimic counts).
    peak: usize,
    /// The elite minutes, whole minutes before the length that start no boss, on whose tick the
    /// cap on elites alive was full and nobody came.
    held_back: Vec<u32>,
    /// The elites that left, with the tick they did.
    left: Vec<(EntityId, u32)>,
    /// The shrines the main character touched, and the elites that died.
    activated: Vec<EntityId>,
    died: Vec<EntityId>,
    revealed: Vec<EntityId>,
    /// The damage of every hit on the main character.
    hurts: Vec<(u32, u32)>,
    /// The tick the first boss came.
    boss_at: Option<u32>,
    /// Each call of a boss: its tick, the elites it called, the kinds alive then that it called
    /// by the rules (all but a shrine, a goblin and a mimic still disguised) and those it skipped.
    calls: Vec<Call>,
}

/// A boss's call of its elites: its tick, how many it called, and the kinds of the elites alive
/// then that join and of those that do not.
type Call = (u32, u32, Vec<&'static str>, Vec<&'static str>);

impl EliteReach {
    fn of(&self, kind: &str) -> Vec<(EntityId, u32)> {
        let found = self.spawned.iter().filter(|s| s.1 == kind);
        found.map(|s| (s.0, s.2)).collect()
    }
}

fn elite_reach(name: &str) -> EliteReach {
    let case = runs_dir().join(name);
    let content = case_content(&case).unwrap();
    let bytes = fs::read(case.join("run.replay")).unwrap();
    let verified = run_replay::verify_run_with(&bytes, &content).unwrap();
    assert_eq!(
        verified.outcome,
        Outcome::Failed,
        "`{name}` ends by the overtime"
    );
    let replay = RunReplay::from_bytes(&bytes).unwrap();
    let mode = content.time_limit();
    let mut engine = Engine::start_with(&replay.setup, &content).unwrap();
    let commands = replay.unpack_commands().unwrap();
    let mut next = commands.iter().peekable();
    let mut r = EliteReach::default();
    let mut hp = engine.snapshot().player.hp;
    for input in replay.unpack_inputs(u32::MAX).unwrap() {
        while let Some(c) = next.next_if(|c| c.tick == engine.tick()) {
            apply(&mut engine, c).unwrap();
        }
        engine.step(input);
        let (tick, s) = (engine.tick(), engine.snapshot());
        let alive: Vec<_> = s
            .entities
            .iter()
            .filter(|e| {
                e.kind == EntityKind::Shrine
                    || (e.kind == EntityKind::Enemy && (e.elite || e.disguised))
            })
            .collect();
        r.peak = r.peak.max(alive.len());
        if r.boss_at.is_none() && s.entities.iter().any(|e| e.kind == EntityKind::Boss) {
            r.boss_at = Some(tick);
        }
        let events = engine.events();
        let came = events
            .iter()
            .any(|e| matches!(e, DomainEvent::EliteSpawned { .. }));
        let is_minute = tick % 3600 == 0
            && tick < mode.length_ticks()
            && !mode.boss_ticks().contains(&tick)
            && tick > 0;
        if is_minute && !came && alive.len() == usize::from(mode.elite_cap()) {
            r.held_back.push(tick);
        }
        for e in events {
            match e {
                DomainEvent::EliteSpawned { elite, .. } => {
                    let v = s.entities.iter().find(|v| v.id == *elite).unwrap();
                    r.spawned.push((
                        *elite,
                        elite_kind(v.asset.as_str(), v.hp, v.aura_radius.is_some()),
                        tick,
                    ));
                }
                DomainEvent::EliteLeft { elite } => r.left.push((*elite, tick)),
                DomainEvent::ShrineActivated { elite } => r.activated.push(*elite),
                DomainEvent::Died { entity, .. } => r.died.push(*entity),
                DomainEvent::MimicRevealed { elite } => r.revealed.push(*elite),
                DomainEvent::PlayerHit { hp_left, .. } => {
                    r.hurts.push((tick, hp.saturating_sub(*hp_left)));
                }
                DomainEvent::ElitesCalled { elites } => {
                    // The kinds of the elites alive that this call did not call: the ones that
                    // stay, the disguised mimics and the goblins (which left with the boss).
                    let kind = |id: EntityId| r.spawned.iter().find(|s| s.0 == id).unwrap().1;
                    let (mut called, mut skipped) = (Vec::new(), Vec::new());
                    for v in &alive {
                        let k = kind(v.id);
                        let skip = k == "shrine" || k == "goblin" || v.disguised;
                        if skip { &mut skipped } else { &mut called }.push(k);
                    }
                    r.calls.push((tick, *elites, called, skipped));
                }
                _ => {}
            }
        }
        hp = s.player.hp;
    }
    r
}

#[test]
fn the_elites_run_case_meets_every_elite_type_and_a_full_cap() {
    let r = elite_reach("elites");
    // Every elite type spawns.
    for kind in ["low", "mid", "high", "goblin", "leader", "shrine", "mimic"] {
        assert!(!r.of(kind).is_empty(), "no {kind} spawns: {:?}", r.spawned);
    }
    // The cap of 5 is reached, never passed, and an elite minute is held back by it.
    assert_eq!(r.peak, 5, "the most elites alive");
    assert!(
        !r.held_back.is_empty(),
        "no elite minute is held back by the cap"
    );
    let boss = r.boss_at.expect("a boss comes");
    // A goblin is disturbed and leaves, before any boss; another leaves when the boss comes.
    let goblins = r.of("goblin");
    let left_at = |id| r.left.iter().find(|l| l.0 == id).map(|l| l.1);
    assert!(
        goblins
            .iter()
            .any(|g| left_at(g.0).is_some_and(|t| t + 1 < boss)),
        "no goblin leaves on its own: {:?}",
        r.left
    );
    assert!(
        goblins.iter().any(|g| left_at(g.0) == Some(boss)),
        "no goblin leaves with the boss at {boss}: {:?}",
        r.left
    );
    // A shrine is touched and its fight ends, cleared (it dies) or failed (it leaves).
    let shrines = r.of("shrine");
    assert!(
        shrines.iter().any(
            |s| r.activated.contains(&s.0) && (r.died.contains(&s.0) || left_at(s.0).is_some())
        ),
        "no shrine is touched and done"
    );
    // A mimic is revealed.
    assert!(r.of("mimic").iter().any(|m| r.revealed.contains(&m.0)));
    // A leader's aura raises a hit: an escort swing of 100 lands as 200 (100 * 200 / 100). Before
    // the middle grade's elite comes (minute 8) nothing else deals 200.
    assert!(
        r.hurts
            .iter()
            .any(|&(t, damage)| t < 28_800 && damage == 200),
        "no hit of 200: {:?}",
        r.hurts
    );
    // The boss calls the elites that join, and skips a shrine, a disguised mimic and a goblin
    // that is still on the map.
    // (The call counts the boss's 1800 ticks from its arrival, ADR-103, or comes at the share
    // of its hit points that the main character takes, whichever is first.)
    let (tick, elites, called, skipped) = r.calls.first().cloned().expect("a boss calls");
    assert!(
        tick > boss && tick <= boss + 1_800,
        "the call comes within 1800 ticks of the boss: {tick}, boss at {boss}"
    );
    assert_eq!(
        usize::try_from(elites).unwrap(),
        called.len(),
        "{called:?} {skipped:?}"
    );
    assert!(elites >= 1, "an elite is called");
    assert!(skipped.contains(&"shrine"), "{skipped:?}");
    // The healing changes which elites the call finds on the map: the mimic is called here.
}

#[test]
fn the_trial_run_case_hits_every_match_up() {
    // The trial's weapons have damage types and its enemies placeholder profiles (ADR-100), so
    // the recorded run lands weak, strong, resisted and neutral hits.
    let [weak, strong, resisted, none] = replay_run_case("trial").match_ups;
    assert!(
        weak > 0 && strong > 0 && resisted > 0 && none > 0,
        "hits by match-up: weak {weak}, strong {strong}, resisted {resisted}, none {none}"
    );
}

/// How a recorded case ends and the most orbs it ever has on screen.
fn what_happens(name: &str) -> (Outcome, u32, usize) {
    let case = corpus_dir().join(name);
    let stage = parse_and_validate(&fs::read(case.join("stage.json")).unwrap()).unwrap();
    let recorded = Replay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
    let mut engine = Engine::new(&stage).unwrap();
    let mut peak = 0;
    for input in recorded.unpack_inputs(stage.length_ticks).unwrap() {
        engine.step(input);
        peak = peak.max(engine.snapshot().orbs.len());
    }
    (engine.outcome(), engine.snapshot().player.hp, peak)
}

/// The most zones a recorded case has alive at once, and whether it empties them at the end of
/// the boss's first phase.
fn zone_peaks(name: &str) -> (usize, bool) {
    let case = corpus_dir().join(name);
    let stage = parse_and_validate(&fs::read(case.join("stage.json")).unwrap()).unwrap();
    let recorded = Replay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
    let mut engine = Engine::new(&stage).unwrap();
    let (mut peak, mut cleared, mut before) = (0, false, 0);
    for input in recorded.unpack_inputs(stage.length_ticks).unwrap() {
        engine.step(input);
        let alive = engine
            .snapshot()
            .zones
            .iter()
            .filter(|z| !z.friendly)
            .count();
        peak = peak.max(alive);
        // The boss's phase 1 begins on tick 150 and its first zones of phase 1 warn for 40
        // ticks: right then only those it has just placed are alive.
        if engine.tick() == 149 {
            before = alive;
        }
        cleared |= engine.tick() == 150 && alive <= 6 && before > 6;
    }
    (peak, cleared)
}

/// What a recorded case does with ranged tokens and orbs: the most ranged shooters (entities 2
/// to 9) winding up or active at once, the most orbs alive, the orbs broken, and whether a burst
/// placed a zone.
fn orb_and_token_reach(name: &str) -> (usize, usize, usize, bool) {
    let case = corpus_dir().join(name);
    let stage = parse_and_validate(&fs::read(case.join("stage.json")).unwrap()).unwrap();
    let recorded = Replay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
    let mut engine = Engine::new(&stage).unwrap();
    let (mut held, mut orbs, mut broken, mut zoned) = (0, 0, 0, false);
    for input in recorded.unpack_inputs(stage.length_ticks).unwrap() {
        engine.step(input);
        let snapshot = engine.snapshot();
        let tokens = snapshot
            .entities
            .iter()
            .filter(|e| (2..10).contains(&e.id.0))
            .filter(|e| {
                e.attack
                    .as_ref()
                    .is_some_and(|a| a.kind != stella_rain_core::attack::PhaseKind::Recovery)
            })
            .count();
        held = held.max(tokens);
        orbs = orbs.max(snapshot.orbs.len());
        zoned |= !snapshot.zones.is_empty();
        broken += engine
            .events()
            .iter()
            .filter(|ev| matches!(ev, DomainEvent::OrbBroken { .. }))
            .count();
    }
    (held, orbs, broken, zoned)
}

#[test]
fn the_ranged_cap_and_orbs_cases_are_what_their_names_say() {
    // Ten shooters want to fire, eight of them marked: three hold a token at the most, their
    // shots laying no line zone (RA1).
    let (held, orbs, _, _) = orb_and_token_reach("ranged_cap");
    assert_eq!(held, 3);
    assert!(orbs > 0, "their shots are orbs");
    // The orbs fill the field up to the cap, the sword breaks some, and bursts place zones.
    let (_, orbs, broken, zoned) = orb_and_token_reach("orbs");
    assert_eq!(orbs, 32, "the orb cap is reached");
    assert!(broken >= 1, "the sword breaks an orb");
    assert!(zoned, "a burst places a zone");
}

#[test]
fn the_dash_case_is_what_its_name_says() {
    let (_, bytes, inputs) = definitions().into_iter().find(|d| d.0 == "dash").unwrap();
    let stage = parse_and_validate(&bytes).unwrap();
    let mut e = engine(&stage);
    let (mut farthest, mut boss_far, mut hit) = (0, 0, false);
    let from = |e: &Engine, kind| {
        e.snapshot()
            .entities
            .iter()
            .filter(|v| v.kind == kind)
            .map(|v| (v.at.x.0, v.at.y.0))
            .collect::<Vec<_>>()
    };
    let boss_start = from(&e, EntityKind::Boss);
    for input in inputs {
        e.step(input);
        hit |= e
            .events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::PlayerHit { .. }));
        // The chargers stand at y 230 and 300 px; one that has moved 100 pixels has dashed.
        for (_, y) in from(&e, EntityKind::Enemy) {
            farthest = farthest.max((y - 230 * 256).abs().min((y - 300 * 256).abs()));
        }
        let (_, by) = from(&e, EntityKind::Boss)[0];
        boss_far = boss_far.max((by - boss_start[0].1).abs());
    }
    assert!(farthest >= 100 * 256 - 1, "an enemy charges: {farthest}");
    assert!(boss_far >= 100 * 256, "the boss charges: {boss_far}");
    assert!(hit, "a dash's hit box reaches the main character");
}

#[test]
fn the_cases_are_what_their_names_say() {
    assert_eq!(what_happens("clear").0, Outcome::Cleared);
    assert_eq!(what_happens("boss").0, Outcome::Cleared, "the boss falls");
    let (outcome, hp, _) = what_happens("fail");
    assert_eq!(
        (outcome, hp),
        (Outcome::Failed, 0),
        "the main character dies"
    );
    let (outcome, hp, _) = what_happens("timeout");
    assert_eq!(outcome, Outcome::Failed);
    assert!(hp > 0, "the main character is alive when the time runs out");
    assert_eq!(what_happens("agents").0, Outcome::Running);
    // Hit boxes alone: no orb ever flies, yet the main character is hurt and enemies die.
    let (outcome, hp, peak) = what_happens("melee");
    assert_eq!((outcome, peak), (Outcome::Running, 0));
    assert!(hp < 9_900, "the swings land: {hp} hp left");
    assert!(deaths("melee") >= 2, "the companion cleaves");
    // Zones alone: no orb flies, a good many zones are alive at once, the clear takes the
    // boss's with the phase, and the main character is hurt by them.
    let (outcome, hp, peak) = what_happens("zones");
    assert_eq!((outcome, peak), (Outcome::Running, 0));
    assert!(hp < 9_900, "the zones land: {hp} hp left");
    let (zones, cleared) = zone_peaks("zones");
    assert!(zones >= 16, "{zones} zones at most alive at once");
    assert!(cleared, "clear_zones leaves only the new phase's zones");
    // The map, not the portrait field: the corner holds the main character, and enemies and
    // zones get past where the portrait field's margin would have removed them.
    let (corner, farthest) = landscape_reach();
    assert!(corner, "the main character reaches the top-right corner");
    assert!(farthest > 92_160 + 16_384, "x {farthest}");
    // Auto mode plays on its own at the elite first, then at the bats once it falls; the boss
    // is the target once the main character has walked up to it, and a charge is released.
    let a = assist_reach();
    assert_eq!(a.first, Some(EntityKind::Enemy), "the elite comes first");
    assert!(a.first_elite, "the elite comes first");
    assert!(a.auto_steps >= 5, "{} steps by auto mode", a.auto_steps);
    assert!(a.bat_after_elite, "a bat is the target after the elite");
    assert!(a.boss, "the boss is the target in the end");
    assert!(a.released, "a charge is released");
    // Three dashes a cycle, two of them started during the dash before; a fourth press finds no
    // charge ready, and all three are back by the next cycle.
    let e = evade_reach();
    assert_eq!((e.evades, e.chained), (9, 6), "evades, of them chained");
    assert_eq!(e.refused, 3, "a press with no charge ready, each cycle");
    assert!(e.all_back_after_spending, "the charges come back");
}

/// What the evade case does: its evades, those started during a dash, the presses with no
/// charge ready, and whether all three charges were ready again after being spent.
struct EvadeReach {
    evades: usize,
    chained: usize,
    refused: usize,
    all_back_after_spending: bool,
}

fn evade_reach() -> EvadeReach {
    let case = corpus_dir().join("evade");
    let stage = parse_and_validate(&fs::read(case.join("stage.json")).unwrap()).unwrap();
    let recorded = Replay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
    let mut engine = Engine::new(&stage).unwrap();
    let mut e = EvadeReach {
        evades: 0,
        chained: 0,
        refused: 0,
        all_back_after_spending: false,
    };
    let (mut was_held, mut spent_all) = (false, false);
    for input in recorded.unpack_inputs(stage.length_ticks).unwrap() {
        let before = engine.snapshot().player.evade;
        engine.step(input);
        let evaded = engine.events().contains(&DomainEvent::Evaded);
        let pressed = input.evade && !was_held;
        was_held = input.evade;
        e.evades += usize::from(evaded);
        e.chained += usize::from(evaded && before.dash_left > 0);
        e.refused += usize::from(pressed && !evaded && before.ready == 0);
        let after = engine.snapshot().player.evade;
        spent_all |= after.ready == 0;
        e.all_back_after_spending |= spent_all && after.ready == after.charges;
    }
    e
}

/// What the assist case does: the first target's kind and whether it is the elite, the steps
/// in the first 100 ticks (where nothing is pressed by hand), whether a bat is the target after
/// the elite and the boss at some point, and whether a charge is released.
struct AssistReach {
    first: Option<EntityKind>,
    first_elite: bool,
    auto_steps: usize,
    bat_after_elite: bool,
    boss: bool,
    released: bool,
}

fn assist_reach() -> AssistReach {
    let case = corpus_dir().join("assist");
    let stage = parse_and_validate(&fs::read(case.join("stage.json")).unwrap()).unwrap();
    let recorded = Replay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
    let mut engine = Engine::new(&stage).unwrap();
    let mut a = AssistReach {
        first: None,
        first_elite: false,
        auto_steps: 0,
        bat_after_elite: false,
        boss: false,
        released: false,
    };
    let mut seen_elite = false;
    for input in recorded.unpack_inputs(stage.length_ticks).unwrap() {
        engine.step(input);
        let s = engine.snapshot();
        let target = s
            .player
            .target
            .and_then(|id| s.entities.iter().find(|e| e.id == id));
        if let Some(t) = target {
            if a.first.is_none() {
                a.first = Some(t.kind);
                a.first_elite = t.elite;
            }
            seen_elite |= t.elite;
            a.bat_after_elite |= seen_elite && t.kind == EntityKind::Enemy && !t.elite;
            a.boss |= t.kind == EntityKind::Boss;
        }
        for ev in engine.events() {
            match ev {
                DomainEvent::ComboStep { .. } if s.tick <= 100 => a.auto_steps += 1,
                DomainEvent::ChargeReleased { .. } => a.released = true,
                _ => {}
            }
        }
    }
    a
}

/// Whether the landscape case's main character stands in the map's top-right corner on some
/// tick, and the largest x any zone or enemy reaches.
fn landscape_reach() -> (bool, i32) {
    let case = corpus_dir().join("landscape");
    let stage = parse_and_validate(&fs::read(case.join("stage.json")).unwrap()).unwrap();
    let recorded = Replay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
    let mut engine = Engine::new(&stage).unwrap();
    let (mut corner, mut farthest) = (false, 0);
    for input in recorded.unpack_inputs(stage.length_ticks).unwrap() {
        engine.step(input);
        let s = engine.snapshot();
        corner |= (s.player.at.x.0, s.player.at.y.0) == (s.map.width.0, 0);
        let xs = s.zones.iter().map(|z| z.at.x.0);
        farthest = xs
            .chain(s.entities.iter().map(|e| e.at.x.0))
            .fold(farthest, i32::max);
    }
    (corner, farthest)
}

/// How many entities die in a recorded case.
fn deaths(name: &str) -> usize {
    let case = corpus_dir().join(name);
    let stage = parse_and_validate(&fs::read(case.join("stage.json")).unwrap()).unwrap();
    let recorded = Replay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
    let mut engine = Engine::new(&stage).unwrap();
    let mut died = 0;
    for input in recorded.unpack_inputs(stage.length_ticks).unwrap() {
        engine.step(input);
        died += engine
            .events()
            .iter()
            .filter(|e| matches!(e, stella_rain_core::event::DomainEvent::Died { .. }))
            .count();
    }
    died
}

#[test]
fn the_corpus_matches() {
    #[expect(clippy::disallowed_methods, reason = "test knob")]
    if std::env::var_os("UPDATE_CORPUS").is_some() {
        // The stages are written at design values and scaled here (ADR-103).
        for (name, stage_bytes, inputs) in definitions() {
            let scaled = common::scaled_bytes(&stage_bytes, common::SCALE);
            write_case(&corpus_dir().join(name), &scaled, &inputs);
        }
        for (name, setup, pack, commands) in run_definitions() {
            write_run_case(&runs_dir().join(name), &setup, pack.as_deref(), &commands);
        }
        prune_packs(&current_dir()).unwrap();
    }
    let mut cases: Vec<PathBuf> = fs::read_dir(corpus_dir())
        .expect("tests/corpus/current/stages exists")
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    assert!(
        cases.len() >= 5,
        "the corpus has {} cases, want at least 5",
        cases.len()
    );
    let mut runs: Vec<PathBuf> = fs::read_dir(runs_dir())
        .expect("tests/corpus/current/runs exists")
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    runs.sort();
    assert!(!runs.is_empty(), "the corpus has no run case");
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|c| check_case(c, &diff_dir()).err())
        .chain(
            runs.iter()
                .filter_map(|c| check_run_case(c, &diff_dir()).err()),
        )
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every frozen folder (`tests/corpus/v<N>/`, N >= 1) is what its manifest says and still
/// verifies against the packs it holds. With none present it passes; the check itself is proved on
/// a temp tree in `corpus_pins`.
#[test]
fn the_frozen_corpus_is_unchanged() {
    let root = current_dir().parent().unwrap().to_path_buf();
    let mut failures = Vec::new();
    for entry in fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() && is_frozen_name(&name) {
            failures.extend(check_frozen_tree(&path).err());
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_orb_cage_case_is_what_its_name_says() {
    let (_, bytes, inputs) = definitions()
        .into_iter()
        .find(|d| d.0 == "orb_cage")
        .unwrap();
    let stage = parse_and_validate(&bytes).unwrap();
    let mut e = engine(&stage);
    let (mut beams, mut hits, mut broken) = (0, Vec::new(), 0);
    for input in inputs {
        e.step(input);
        let snapshot = serde_json::to_value(e.snapshot()).unwrap();
        let orbs = snapshot["orbs"].as_array().cloned().unwrap_or_default();
        let live = orbs.iter().filter(|o| o.get("link_to").is_some()).count();
        beams = beams.max(live);
        for ev in e.events() {
            match ev {
                DomainEvent::PlayerHit { .. } => hits.push((e.tick(), live)),
                DomainEvent::OrbBroken { .. } => broken += 1,
                _ => {}
            }
        }
    }
    // Two cages stand at once (4 + 5 beams) and are renewed; the sword cuts some orbs; the main
    // character is hurt, once every 60 ticks at the most, each time with a beam live.
    assert!(beams >= 9, "{beams} beams at most");
    assert!(broken >= 1, "the sword breaks an orb");
    assert!(hits.len() >= 2, "the beams land: {hits:?}");
    assert!(hits.iter().all(|(_, live)| *live > 0), "{hits:?}");
}

#[test]
fn the_warn_anchor_case_is_what_its_name_says() {
    let (_, bytes, inputs) = definitions()
        .into_iter()
        .find(|d| d.0 == "warn_anchor")
        .unwrap();
    let stage = parse_and_validate(&bytes).unwrap();
    let mut e = engine(&stage);
    let (mut warnings, mut beams, mut hits) = (0, 0, 0);
    for input in inputs {
        e.step(input);
        let snapshot = serde_json::to_value(e.snapshot()).unwrap();
        let zones = snapshot["zones"].as_array().cloned().unwrap_or_default();
        for z in zones.iter().filter(|z| z["warn_only"] == json!(true)) {
            assert_eq!(z["phase"], json!("wind_up"), "a warning never turns active");
        }
        warnings = warnings.max(
            zones
                .iter()
                .filter(|z| z["warn_only"] == json!(true))
                .count(),
        );
        let orbs = snapshot["orbs"].as_array().cloned().unwrap_or_default();
        beams = beams.max(orbs.iter().filter(|o| o.get("link_to").is_some()).count());
        hits += e
            .events()
            .iter()
            .filter(|ev| matches!(ev, DomainEvent::PlayerHit { .. }))
            .count();
    }
    assert!(warnings >= 1, "a warning lane is laid");
    assert!(beams >= 4, "the cage is linked: {beams} beams");
    assert!(hits >= 1, "the body hit lands after the warning");
}

/// The leash_edge stage case: three enemies relocated near the right edge and bottom-right corner,
/// proving that the leash lands on the opposite side (outside the view) when the mirrored side
/// is off the map (tasks#216, core#477).
#[test]
fn the_leash_edge_case_is_what_its_name_says() {
    let case = corpus_dir().join("leash_edge");
    let stage_bytes = fs::read(case.join("stage.json")).unwrap();
    let recorded_bytes = fs::read(case.join("run.replay")).unwrap();
    let stage = parse_and_validate(&stage_bytes).unwrap();
    let recorded = Replay::from_bytes(&recorded_bytes).unwrap();
    let mut engine = Engine::new(&stage).unwrap();

    const PX: i64 = 256;
    const VIEW_HALF_X: i64 = 420 * PX; // rules.json / view half width
    const VIEW_HALF_Y: i64 = 180 * PX; // rules.json / view half height
    const LAND_HALF_X: i64 = 484 * PX; // rules.json / land rectangle half width
    const LAND_HALF_Y: i64 = 244 * PX; // rules.json / land rectangle half height

    let map_w = i64::from(stage.map.as_ref().unwrap().width.0);
    let map_h = i64::from(stage.map.as_ref().unwrap().height.0);

    let mut total_relocations = 0;
    let mut relocated_entities = Vec::new();

    let inputs = recorded.unpack_inputs(stage.length_ticks).unwrap();
    assert_eq!(inputs.len(), 300);

    for (tick_idx, input) in inputs.into_iter().enumerate() {
        let tick = tick_idx as u32;
        engine.step(input);

        let events = engine.events();
        for ev in events {
            if let DomainEvent::EnemyRelocated { entity } = ev {
                total_relocations += 1;
                relocated_entities.push((*entity, tick));

                let snapshot = engine.snapshot();
                let px = i64::from(snapshot.player.at.x.0);
                let py = i64::from(snapshot.player.at.y.0);

                // View centre held 420 / 180 inside map
                let cx = px.clamp(VIEW_HALF_X, map_w - VIEW_HALF_X);
                let cy = py.clamp(VIEW_HALF_Y, map_h - VIEW_HALF_Y);

                // Wave spawn position for this entity
                let wave = stage
                    .waves
                    .iter()
                    .find(|w| w.at_tick == tick)
                    .unwrap_or_else(|| panic!("no wave at tick {tick}"));
                let spawn_pt = wave.spawn.unwrap();
                let ex = i64::from(spawn_pt.x.0);
                let ey = i64::from(spawn_pt.y.0);

                let dx = ex - cx;
                let dy = ey - cy;
                let (ax, ay) = (dx.abs(), dy.abs());

                // (b) Mirrored side's fixed coordinate is off the map
                let is_vertical = LAND_HALF_X * ay <= LAND_HALF_Y * ax;
                if is_vertical {
                    let mirrored_x = if dx < 0 {
                        cx + LAND_HALF_X
                    } else {
                        cx - LAND_HALF_X
                    };
                    assert!(
                        mirrored_x < 0 || mirrored_x > map_w,
                        "mirrored x {mirrored_x} must be off map (0..={map_w})"
                    );
                } else {
                    let mirrored_y = if dy < 0 {
                        cy + LAND_HALF_Y
                    } else {
                        cy - LAND_HALF_Y
                    };
                    assert!(
                        mirrored_y < 0 || mirrored_y > map_h,
                        "mirrored y {mirrored_y} must be off map (0..={map_h})"
                    );
                }

                // (c) Clamped mirrored point (what the old rule gave) is inside the view
                let (ox, oy) = if is_vertical {
                    (-dx.signum() * LAND_HALF_X, -dy * LAND_HALF_X / ax)
                } else {
                    (-dx * LAND_HALF_Y / ay, -dy.signum() * LAND_HALF_Y)
                };
                let old_x = (cx + ox).clamp(0, map_w);
                let old_y = (cy + oy).clamp(0, map_h);
                assert!(
                    (old_x - cx).abs() <= VIEW_HALF_X && (old_y - cy).abs() <= VIEW_HALF_Y,
                    "old clamped point ({old_x}, {old_y}) must be inside view of centre ({cx}, {cy})"
                );

                // (d) Landed position in snapshot is on the map and outside the view
                let entity_snapshot = snapshot
                    .entities
                    .iter()
                    .find(|e| e.id == *entity)
                    .unwrap_or_else(|| panic!("no entity {entity:?} in snapshot"));
                let lx = i64::from(entity_snapshot.at.x.0);
                let ly = i64::from(entity_snapshot.at.y.0);
                assert!(
                    lx >= 0 && lx <= map_w && ly >= 0 && ly <= map_h,
                    "landed position ({lx}, {ly}) must be on the map"
                );
                let outside_view =
                    (lx - cx).abs() > VIEW_HALF_X || (ly - cy).abs() > VIEW_HALF_Y;
                assert!(
                    outside_view,
                    "landed position ({lx}, {ly}) must be outside the view of centre ({cx}, {cy})"
                );
            }
        }
    }

    // (a) exactly 3 events in all 300 ticks
    assert_eq!(total_relocations, 3, "exactly 3 relocations in 300 ticks");

    // (e) the three entities are relocated once each
    assert_eq!(
        relocated_entities,
        vec![
            (EntityId(2), 5),
            (EntityId(3), 215),
            (EntityId(4), 225),
        ],
        "entities 2, 3, 4 are relocated once each on their ticks"
    );
}
