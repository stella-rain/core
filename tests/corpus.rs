//! The replay corpus (ADR-019): `tests/corpus/v0/<name>/` holds a `stage.json`, a `run.replay`
//! and `hashes.txt` (the state hash after every tick, one lowercase hex hash a line). Each case
//! is re-simulated and every tick compared, so a change that moves any hash fails on the first
//! tick it moved, on x86_64 and on the arm64 runner alike. The files are plain, so the device
//! run can read them too (ADR-040).
//!
//! On a mismatch the failure names the first diverging tick and writes the snapshots of that
//! tick and the one before to `target/corpus-diff/<name>.json`; CI uploads that folder per
//! architecture. After an intended change, run `UPDATE_CORPUS=1 cargo test --test corpus` and
//! say `Corpus regenerated: <why>` in the PR (ADR-035).

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::input::Input;
use stella_rain_core::replay::{self, Replay, hash_from_hex, hash_to_hex};
use stella_rain_core::validate::parse_and_validate;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/v0")
}

fn diff_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/corpus-diff")
}

/// Re-simulates one case and compares every tick's hash. `Err` is the message the test fails
/// with; a divergence also leaves `<diff_dir>/<name>.json`.
fn check_case(case: &Path, diff_dir: &Path) -> Result<(), String> {
    let name = case.file_name().unwrap().to_string_lossy().into_owned();
    let read = |file: &str| {
        fs::read(case.join(file)).map_err(|e| format!("corpus case `{name}`: {file}: {e}"))
    };
    let stage_bytes = read("stage.json")?;
    let replay_bytes = read("run.replay")?;
    let hashes = String::from_utf8_lossy(&read("hashes.txt")?)
        .lines()
        .map(|l| {
            hash_from_hex(l).ok_or(format!("corpus case `{name}`: hashes.txt: bad hash `{l}`"))
        })
        .collect::<Result<Vec<u64>, String>>()?;

    let stage = parse_and_validate(&stage_bytes)
        .map_err(|e| format!("corpus case `{name}`: stage.json: {e:?}"))?;
    let recorded = Replay::from_bytes(&replay_bytes)
        .map_err(|e| format!("corpus case `{name}`: run.replay: {e}"))?;
    let inputs = recorded
        .unpack_inputs(stage.length_ticks)
        .map_err(|e| format!("corpus case `{name}`: run.replay: {e}"))?;
    if hashes.len() != inputs.len() {
        return Err(format!(
            "corpus case `{name}`: hashes.txt has {} hashes, run.replay has {} ticks",
            hashes.len(),
            inputs.len()
        ));
    }

    let mut engine = Engine::new(&stage).map_err(|e| format!("corpus case `{name}`: {e:?}"))?;
    let mut previous = engine.snapshot();
    for (input, expected) in inputs.iter().zip(&hashes) {
        engine.step(*input);
        let actual = engine.state_hash();
        if actual != *expected {
            let tick = engine.tick();
            let dump = json!({
                "case": name,
                "tick": tick,
                "expected_hash": hash_to_hex(*expected),
                "actual_hash": hash_to_hex(actual),
                "previous": previous,
                "at": engine.snapshot(),
            });
            let path = diff_dir.join(format!("{name}.json"));
            fs::create_dir_all(diff_dir).unwrap();
            fs::write(&path, serde_json::to_vec_pretty(&dump).unwrap()).unwrap();
            return Err(format!(
                "corpus case `{name}` diverges first on tick {tick}: expected {}, got {}; \
                 snapshots of ticks {} and {tick} are in {}",
                hash_to_hex(*expected),
                hash_to_hex(actual),
                tick - 1,
                path.display()
            ));
        }
        previous = engine.snapshot();
    }

    if recorded.final_hash != hash_to_hex(engine.state_hash()) || recorded.ticks != engine.tick() {
        return Err(format!(
            "corpus case `{name}`: run.replay's ticks or final hash do not match hashes.txt"
        ));
    }
    if engine.outcome() == Outcome::Cleared {
        replay::verify_against(&stage_bytes, &replay_bytes, &hashes)
            .map_err(|e| format!("corpus case `{name}`: {e}"))?;
    }
    Ok(())
}

/// A scratch directory under the target directory, emptied first.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("corpus-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes a case's three files from a stage and the inputs to run.
fn write_case(dir: &Path, stage_bytes: &[u8], inputs: &[Input]) {
    let recorded = replay::record(stage_bytes, inputs).unwrap();
    let ran = recorded.unpack_inputs(u32::MAX).unwrap();
    let stage = parse_and_validate(stage_bytes).unwrap();
    let mut engine = Engine::new(&stage).unwrap();
    let mut hashes = String::new();
    for input in &ran {
        engine.step(*input);
        hashes.push_str(&hash_to_hex(engine.state_hash()));
        hashes.push('\n');
    }
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("stage.json"), stage_bytes).unwrap();
    fs::write(dir.join("run.replay"), recorded.to_bytes()).unwrap();
    fs::write(dir.join("hashes.txt"), hashes).unwrap();
}

fn changed_hash_case(name: &str) -> (PathBuf, PathBuf) {
    let dir = scratch(name);
    let case = dir.join("case");
    write_case(&case, &common::stage_bytes(), &common::clearing_inputs());
    (case, dir.join("diff"))
}

/// Replaces the hash on `tick` (1-based) with a different one.
fn change_hash(case: &Path, tick: usize) {
    let path = case.join("hashes.txt");
    let text = fs::read_to_string(&path).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let wrong = hash_from_hex(&lines[tick - 1]).unwrap() ^ 1;
    lines[tick - 1] = hash_to_hex(wrong);
    fs::write(&path, lines.join("\n") + "\n").unwrap();
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
    change_hash(&case, 20);
    let message = check_case(&case, &diff).unwrap_err();
    assert!(message.contains("tick 20"), "{message}");
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
    let path = case.join("hashes.txt");
    let text = fs::read_to_string(&path).unwrap();
    let short: String = text.lines().take(10).map(|l| format!("{l}\n")).collect();
    fs::write(&path, short).unwrap();
    let message = check_case(&case, &diff).unwrap_err();
    assert!(message.contains("10 hashes"), "{message}");
}

#[test]
fn a_hash_that_is_not_hex_fails() {
    let (case, diff) = changed_hash_case("hex");
    fs::write(case.join("hashes.txt"), "not a hash\n").unwrap();
    let message = check_case(&case, &diff).unwrap_err();
    assert!(message.contains("hashes.txt"), "{message}");
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

// --- The cases, built from the test helpers --------------------------------------------------
//
// These are only read by `UPDATE_CORPUS=1`, which writes the files; a normal run reads the files
// and nothing else, so what the device run sees is exactly what CI checked.

type Definition = (&'static str, Vec<u8>, Vec<Input>);

fn bytes(stage: &impl serde::Serialize) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(stage).unwrap();
    bytes.push(b'\n');
    bytes
}

fn firing_for(ticks: u32) -> Vec<Input> {
    (0..ticks).map(|_| common::firing()).collect()
}

/// Agents of every movement type: a companion that orbits the main character and buffs it, a
/// healer that follows it, and an enemy that drifts down while it shoots at the nearest ally.
fn agents() -> Definition {
    let shoot = |selector: &str, cooldown| {
        rule(
            always(),
            json!(selector),
            json!({ "type": "attack",
                    "attack": { "preset": { "id": "aimed_single", "args": [768] } } }),
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
    let stage = stage_full(
        600,
        Some(far_boss()),
        vec![wave(1, 1, 0, shooter, 46_080, 100_000)],
        vec![k, healer],
        vec![],
    );
    let inputs = (0..80)
        .map(|t| {
            if t < 40 {
                common::firing()
            } else {
                common::idle()
            }
        })
        .collect();
    ("agents", bytes(&stage), inputs)
}

/// A boss that lays rings of 24 bullets every 3 ticks and spirals between them, while the main
/// character dodges from side to side.
fn bullets() -> Definition {
    let ring = json!({ "type": "repeat", "count": 20, "interval_ticks": 3, "body": [
        { "type": "ring", "count": 24, "body": [
            { "type": "fire", "speed": 1024, "direction": { "type": "relative",
                                                            "angle": 0 } } ] } ] });
    let spiral = json!({ "type": "repeat", "count": 40, "interval_ticks": 2, "body": [
        { "type": "fire", "speed": 1536, "direction": { "type": "sequential", "step": 1792 } } ] });
    let boss = json!({
        "base": "golem", "hp": 1_000_000, "radius": 8192, "spawn": { "x": 46_080, "y": 40_000 },
        "phases": [{ "timeline": [
            { "type": "attack", "attack": { "inline": [ring] } }, { "type": "wait", "ticks": 70 },
            { "type": "attack", "attack": { "inline": [spiral] } }, { "type": "wait", "ticks": 90 },
            { "type": "loop" } ] }]
    });
    let stage = stage_full(600, Some(boss), vec![], vec![], vec![]);
    let inputs = (0..300)
        .map(|t| Input {
            dx: if (t / 25) % 2 == 0 { 300 } else { -300 },
            fire: true,
            ..Input::default()
        })
        .collect();
    ("bullets", bytes(&stage), inputs)
}

/// The boss of `tests/golden.rs`: a horn that breaks, two phases and a timeline, shot until it
/// falls.
fn boss() -> Definition {
    let horn = json!({ "id": "horn", "asset": "horns_2", "hp": 30,
                       "offset": { "x": 0, "y": 16_000 }, "radius": 6_000 });
    let shot = json!({ "type": "attack",
                       "attack": { "preset": { "id": "aimed_single", "args": [768] } } });
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
                "transition": [{ "type": "clear_bullets" },
                               { "type": "invulnerable_ticks", "ticks": 10 }],
                "timeline": [shot, { "type": "wait", "ticks": 8 }, { "type": "loop" }]
            }
        ]
    });
    let stage = stage_full(600, Some(boss), vec![], vec![], vec![]);
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
                            "attack": { "preset": { "id": "aimed_single", "args": [768] } } }),
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

/// A stage that runs out of ticks with the boss standing.
fn timeout() -> Definition {
    let stage = stage_with(90, Some(far_boss()), vec![]);
    (
        "timeout",
        bytes(&stage),
        (0..90).map(|_| common::idle()).collect(),
    )
}

fn definitions() -> Vec<Definition> {
    vec![
        (
            "clear",
            bytes(&common::clearing_stage_json()),
            common::clearing_inputs(),
        ),
        agents(),
        bullets(),
        boss(),
        fail(),
        timeout(),
    ]
}

/// How a recorded case ends and the most bullets it ever has on screen.
fn what_happens(name: &str) -> (Outcome, u32, usize) {
    let case = corpus_dir().join(name);
    let stage = parse_and_validate(&fs::read(case.join("stage.json")).unwrap()).unwrap();
    let recorded = Replay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
    let mut engine = Engine::new(&stage).unwrap();
    let mut peak = 0;
    for input in recorded.unpack_inputs(stage.length_ticks).unwrap() {
        engine.step(input);
        peak = peak.max(engine.snapshot().bullets.len());
    }
    (engine.outcome(), engine.snapshot().player.hp, peak)
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
    assert!(
        what_happens("bullets").2 >= 300,
        "{} bullets",
        what_happens("bullets").2
    );
    assert_eq!(what_happens("agents").0, Outcome::Running);
}

#[test]
fn the_corpus_matches() {
    if std::env::var_os("UPDATE_CORPUS").is_some() {
        for (name, stage_bytes, inputs) in definitions() {
            write_case(&corpus_dir().join(name), &stage_bytes, &inputs);
        }
    }
    let mut cases: Vec<PathBuf> = fs::read_dir(corpus_dir())
        .expect("tests/corpus/v0 exists")
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    assert!(
        cases.len() >= 5,
        "the corpus has {} cases, want at least 5",
        cases.len()
    );
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|c| check_case(c, &diff_dir()).err())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
