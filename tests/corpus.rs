//! The replay corpus (ADR-019): `tests/corpus/v0/<name>/` holds a `stage.json`, a `run.replay`
//! and `hashes.txt` (the state hash after every tick, one lowercase hex hash a line). Each case
//! is re-simulated and every tick compared, so a change that moves any hash fails on the first
//! tick it moved, on x86_64 and on the arm64 runner alike. The files are plain, so the device
//! run can read them too (ADR-054).
//!
//! On a mismatch the failure names the first diverging tick and writes the snapshots of that
//! tick and the one before to `target/corpus-diff/<name>.json`; CI uploads that folder per
//! architecture. After an intended change, run
//! `UPDATE_CORPUS=1 cargo test --test integration corpus::` and say `Corpus regenerated: <why>`
//! in the PR (ADR-035).
//!
//! Runs of the launch game (ADR-079) are kept apart, in `tests/corpus/runs/v0/<name>/`: a
//! `run.replay` (a run replay: setup, content hash, inputs and commands) and `hashes.txt` (the
//! hash after every tick, before the commands between it and the next). They need no stage file, because the
//! setup names a mode bundled in `core`; the stage cases' folders each hold one.

use crate::common;

use std::fs;
use std::path::{Path, PathBuf};

use common::run::play_run_with;
use common::*;
use serde_json::{Value, json};
use stella_rain_core::attack::MatchUp;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::replay::{self, Replay, hash_from_hex, hash_to_hex};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{self, Command, CommandKind, RunReplay, TickCommand};
use stella_rain_core::run_setup::{self, RunSetup};
use stella_rain_core::snapshot::{EntityKind, OfferOption, Snapshot};
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::parse_and_validate;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/v0")
}

fn runs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/runs/v0")
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
        if engine.state_hash() != *expected {
            return Err(divergence(&name, *expected, &engine, &previous, diff_dir));
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

/// Reads `hashes.txt` of a case.
fn read_hashes(case: &Path, name: &str) -> Result<Vec<u64>, String> {
    let text = fs::read(case.join("hashes.txt"))
        .map_err(|e| format!("corpus case `{name}`: hashes.txt: {e}"))?;
    String::from_utf8_lossy(&text)
        .lines()
        .map(|l| {
            hash_from_hex(l).ok_or(format!("corpus case `{name}`: hashes.txt: bad hash `{l}`"))
        })
        .collect()
}

/// Applies one command between ticks, as a replay's verification does.
fn apply(engine: &mut Engine, c: &TickCommand) -> Result<(), String> {
    match c.kind {
        CommandKind::Choose => engine.choose(c.index).map_err(|e| format!("{e:?}")),
        CommandKind::Spend => engine.spend(c.index).map_err(|e| format!("{e:?}")),
    }
}

/// Re-simulates one run case (ADR-079), taking its commands between the ticks, and compares
/// every tick's hash; then the run replay must verify against the same hashes.
fn check_run_case(case: &Path, diff_dir: &Path) -> Result<(), String> {
    let name = case.file_name().unwrap().to_string_lossy().into_owned();
    let fail = |e: &dyn std::fmt::Display| format!("corpus case `{name}`: {e}");
    let replay_bytes = fs::read(case.join("run.replay")).map_err(|e| fail(&e))?;
    let hashes = read_hashes(case, &name)?;
    let recorded = RunReplay::from_bytes(&replay_bytes).map_err(|e| fail(&e))?;
    let content = case_content(case).map_err(|e| fail(&e))?;
    let stage =
        run_setup::stage_with(&recorded.setup, content.time_limit()).map_err(|e| fail(&e))?;
    let inputs = recorded
        .unpack_inputs(run_setup::max_replay_ticks(
            &recorded.setup,
            &stage,
            content.time_limit(),
        ))
        .map_err(|e| fail(&e))?;
    let commands = recorded.unpack_commands().map_err(|e| fail(&e))?;
    if hashes.len() != inputs.len() {
        return Err(fail(&format!(
            "hashes.txt has {} hashes, run.replay has {} ticks",
            hashes.len(),
            inputs.len()
        )));
    }

    let mut engine = Engine::start_with(&recorded.setup, &content).map_err(|e| fail(&e))?;
    let mut next = commands.iter().peekable();
    for (input, expected) in inputs.iter().zip(&hashes) {
        while let Some(c) = next.next_if(|c| c.tick == engine.tick()) {
            apply(&mut engine, c).map_err(|e| fail(&e))?;
        }
        let previous = engine.snapshot();
        engine.step(*input);
        if engine.state_hash() != *expected {
            return Err(divergence(&name, *expected, &engine, &previous, diff_dir));
        }
    }
    run_replay::verify_run_against_with(&replay_bytes, &content, &hashes).map_err(|e| fail(&e))?;
    Ok(())
}

/// The content a run case plays with: the bundled files, and the `time_limit` mode file of its
/// own when the case has a `mode.json` (a short mode, so that a run ends by the final boss's
/// kill or by the overtime without 20 minutes of play).
fn case_content(case: &Path) -> Result<RunContent, String> {
    match fs::read(case.join("mode.json")) {
        Ok(bytes) => {
            let mode = TimeLimit::parse(&bytes).map_err(|e| e.to_string())?;
            Ok(RunContent::bundled().clone().with_time_limit(mode))
        }
        Err(_) => Ok(RunContent::bundled().clone()),
    }
}

/// The hash after every tick of a run replay, before the commands that follow it.
fn run_hashes(replay: &RunReplay, content: &RunContent) -> Vec<u64> {
    let mut engine = Engine::start_with(&replay.setup, content).unwrap();
    let commands = replay.unpack_commands().unwrap();
    let mut next = commands.iter().peekable();
    let mut hashes = Vec::new();
    for input in replay.unpack_inputs(u32::MAX).unwrap() {
        while let Some(c) = next.next_if(|c| c.tick == engine.tick()) {
            apply(&mut engine, c).unwrap();
        }
        engine.step(input);
        hashes.push(engine.state_hash());
    }
    hashes
}

/// Writes a run case's files from a setup and the commands to run, and the mode file it plays
/// with if it has one.
fn write_run_case(dir: &Path, setup: &RunSetup, mode: Option<&str>, commands: &[Command]) {
    fs::create_dir_all(dir).unwrap();
    let content = match mode {
        Some(json) => {
            fs::write(dir.join("mode.json"), json).unwrap();
            case_content(dir).unwrap()
        }
        None => RunContent::bundled().clone(),
    };
    let recorded = run_replay::record_run_with(setup, &content, commands).unwrap();
    let hashes: String = run_hashes(&recorded, &content)
        .into_iter()
        .map(|h| hash_to_hex(h) + "\n")
        .collect();
    fs::write(dir.join("run.replay"), recorded.to_bytes()).unwrap();
    fs::write(dir.join("hashes.txt"), hashes).unwrap();
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

// --- Run cases (ADR-079) -----------------------------------------------------------------------

fn changed_run_case(name: &str) -> (PathBuf, PathBuf) {
    let dir = scratch(&format!("run-{name}"));
    let case = dir.join("case");
    let point = common::run::first_point_tick();
    write_run_case(
        &case,
        &common::run::trial_setup(),
        None,
        &common::run::play_run(point + 20),
    );
    (case, dir.join("diff"))
}

#[test]
fn an_unchanged_run_case_passes() {
    let (case, diff) = changed_run_case("unchanged");
    assert_eq!(check_run_case(&case, &diff), Ok(()));
    assert!(!diff.exists(), "a passing case writes no diff");
}

#[test]
fn a_changed_hash_in_a_run_case_fails_naming_its_tick() {
    let (case, diff) = changed_run_case("tick-n");
    let after_point = common::run::first_point_tick() as usize + 1;
    change_hash(&case, after_point);
    let message = check_run_case(&case, &diff).unwrap_err();
    assert!(
        message.contains(&format!("tick {after_point}")),
        "{message}"
    );
    let dump: Value = serde_json::from_slice(&fs::read(diff.join("case.json")).unwrap()).unwrap();
    assert_eq!(dump["tick"], json!(after_point));
}

#[test]
fn a_run_case_whose_spend_changed_fails() {
    let (case, diff) = changed_run_case("spend");
    let path = case.join("run.replay");
    let mut replay = RunReplay::from_bytes(&fs::read(&path).unwrap()).unwrap();
    let mut commands = replay.unpack_commands().unwrap();
    let spend = commands
        .iter()
        .position(|c| c.kind == CommandKind::Spend)
        .expect("the run spends a point");
    assert_ne!(commands[spend].index, 0);
    commands[spend].index = 0;
    replay.commands = run_replay::pack_commands(&commands);
    fs::write(&path, replay.to_bytes()).unwrap();
    // The tick after the spend differs from the one recorded.
    let message = check_run_case(&case, &diff).unwrap_err();
    let after_point = commands[spend].tick + 1;
    assert!(
        message.contains(&format!("tick {after_point}")),
        "{message}"
    );
}

#[test]
fn a_run_case_with_hashes_that_are_not_one_per_tick_fails() {
    let (case, diff) = changed_run_case("count");
    let path = case.join("hashes.txt");
    let text = fs::read_to_string(&path).unwrap();
    let short: String = text.lines().take(10).map(|l| format!("{l}\n")).collect();
    fs::write(&path, short).unwrap();
    let message = check_run_case(&case, &diff).unwrap_err();
    assert!(message.contains("10 hashes"), "{message}");
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
/// slash, thrust) and `sub` on 40 and 48 (two smashes from the root, each a slash that pulls).
fn combos(t: u32, input: Input) -> Input {
    let c = t % 60;
    Input {
        main: matches!(c, 0 | 8 | 16),
        sub: c == 40 || c == 48,
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
    // The test stage's Main weapon: one bullet along the facing, fired every 6 ticks.
    let stage = stage_full(
        600,
        Some(far_boss()),
        vec![wave(1, 1, 0, shooter, 46_080, 100_000)],
        vec![k, healer],
    );
    // Standing still for 40 ticks, shooting up at the enemy, then walking right, which turns
    // the Main weapon's shots away from it.
    let inputs = (0..80)
        .map(|t| firing(if t < 40 { idle() } else { moving(64, 0) }, t))
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
            { "type": "attack", "attack": { "inline": spawned(json!([ring])) } },
            { "type": "wait", "ticks": 70 },
            { "type": "attack", "attack": { "inline": spawned(json!([spiral])) } },
            { "type": "wait", "ticks": 90 },
            { "type": "loop" } ] }]
    });
    // The weapons are the `giant_sword` and the `giant_hammer`: the giant_sword's combos by hand, with the giant_hammer's
    // pull on the steps reached by `sub`. The boss stands far out of reach.
    let stage = armed(
        stage_full(600, Some(boss), vec![], vec![]),
        json!({ "preset": { "id": "giant_sword" } }),
    );
    // Half speed from side to side, turning to face up the field once a second, and evading
    // twice. `main` on ticks 0 and 8 (slash, slash), then on 16 held to 68 (the thrust charged
    // to level 2, 52 ticks, released inside its perfect window: a slam); `sub` on 100 and 108
    // (thrust, slash); `main` on 133, during the evade of tick 130 (the dash-strike); `main` on
    // 170 and 178, then on 186 held until the evade of tick 230 breaks the charge; and the
    // switches and the stage skill pressed now and then. The boss stays out of the giant_sword's
    // assist range, so the switches find no target here (`assist` is the case that does).
    let inputs = (0..300)
        .map(|t| {
            let input = match t {
                _ if t % 60 == 0 => moving(0, -1),
                130 | 230 => Input {
                    evade: true,
                    ..moving(50, 0)
                },
                _ if (t / 25) % 2 == 0 => moving(50, 0),
                _ => moving(-50, 0),
            };
            Input {
                main: matches!(t, 0 | 8 | 133 | 170 | 178)
                    || (16..68).contains(&t)
                    || (186..240).contains(&t),
                sub: t == 100 || t == 108,
                skill: t % 50 == 7,
                assist: (t / 70) % 2 == 1,
                auto: (t / 90) % 2 == 1,
                ..input
            }
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
    // The Main weapon fires up the field every 6 ticks, through the horn, then at the boss.
    let stage = stage_full(600, Some(boss), vec![], vec![]);
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

/// Melee (ADR-055): three enemies trail the main character and swing at it, a wind-up, an arc
/// and a recovery, rooted while they wind up and swing; a companion orbiting it cleaves them
/// with a wide arc that hits again every 2 ticks, through their defence. The main character
/// stands still, then walks from side to side, playing the `giant_sword`'s combos, whose smashes
/// pull what they hit. Nobody fires a bullet.
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

/// Aim assist and auto mode (ADR-072, ADR-073) with the giant_sword, whose assist range is 160
/// pixels: three bats to the left and an elite to the right, all within it, and a boss far up
/// the field. Auto mode alone for 100 ticks (the elite first, then the bats as they fall);
/// combos by hand with assist for 100; then assist on while the main character walks up to the
/// boss, pressing `main` twice and holding it from the third step's start for a charge (which
/// halves its speed), with auto mode on again from tick 262, until the boss is the target.
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
    let inputs = (0..320)
        .map(|t| match t {
            0..100 => Input {
                auto: true,
                ..idle()
            },
            100..200 => Input {
                assist: true,
                ..combos(t, idle())
            },
            _ => Input {
                main: t == 205 || t == 225 || t == 240 || (250..285).contains(&t),
                assist: true,
                auto: t >= 262,
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
                    "attack": { "preset": { "id": "aimed_single", "args": [768] } } }),
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
            wave(1, 4, 25, crosser.clone(), width / 2, 30_000),
            wave(60, 4, 25, crosser, width / 2 - 100_000, 8_000),
        ],
        vec![],
    ))
    .unwrap();
    stage["map"] = json!({ "width": width, "height": height,
                           "start": { "x": width - 30_720, "y": 20_000 } });
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

pub(crate) fn definitions() -> Vec<Definition> {
    vec![
        (
            "clear",
            bytes(&common::clearing_stage_json()),
            common::clearing_inputs(),
        ),
        agents(),
        assist(),
        bullets(),
        boss(),
        evade_charges(),
        fail(),
        landscape(),
        melee(),
        timeout(),
    ]
}

/// The run cases: the trial mode played until the main character falls, spending its points
/// and answering its offers on the way, with every kind of option; and the same from another
/// seed, whose run takes a stage skill from an elite's star and uses it.
fn run_definitions() -> Vec<(&'static str, RunSetup, Option<String>, Vec<Command>)> {
    use common::run::{
        KILL_SEED, STAGE_SKILL_SEED, TRIAL_LENGTH, play_run, play_run_from, time_limit_setup,
        trial_setup,
    };
    let stage_skill = RunSetup {
        seed: STAGE_SKILL_SEED,
        ..trial_setup()
    };
    let stage_skill_commands = play_run_from(&stage_skill, TRIAL_LENGTH);
    // The time_limit mode's waves around the main character, played until it falls (it does
    // before the first boss time; the bosses are in `tests/time_limit_run.rs`).
    let time_limit = time_limit_setup();
    let time_limit_commands = play_run_from(&time_limit, 25_000);
    // Two short modes with no waves and the final boss at tick 60, the only boss: in the first
    // the overtime is long enough for the main character to kill it (it does, at tick 8098);
    // in the second it ends at tick 600 with the boss alive.
    let (kill_mode, kill_content) = short_time_limit(12_000);
    let kill_setup = RunSetup {
        seed: KILL_SEED,
        ..time_limit_setup()
    };
    let kill_commands = play_run_with(&kill_setup, &kill_content, 20_060, boss_input);
    let (overtime_mode, overtime_content) = short_time_limit(540);
    let kill_setup_2 = time_limit_setup();
    let overtime_commands = play_run_with(&kill_setup_2, &overtime_content, 600, boss_input);
    // The elites run: every elite type, the cap of 5 reached, and each elite's event; its own
    // mode file, below.
    let (elites_mode, elites_content) = elites_time_limit();
    let elites_setup = time_limit_setup();
    let elites_script = EliteScript::default();
    let elites_commands = play_run_with(&elites_setup, &elites_content, ELITES_TICKS, |e, t| {
        elites_script.input(e, t)
    });
    vec![
        ("trial", trial_setup(), None, play_run(TRIAL_LENGTH)),
        ("stage_skill", stage_skill, None, stage_skill_commands),
        ("time_limit", time_limit, None, time_limit_commands),
        (
            "time_limit_kill",
            kill_setup,
            Some(kill_mode),
            kill_commands,
        ),
        (
            "time_limit_overtime",
            kill_setup_2,
            Some(overtime_mode),
            overtime_commands,
        ),
        ("elites", elites_setup, Some(elites_mode), elites_commands),
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
/// 100, which 100% makes a hit of 200 (the bundled 25% makes 125, `tests/elite_pack_leader.rs`).
fn elites_time_limit() -> (String, RunContent) {
    let mut v: Value = serde_json::from_slice(TimeLimit::BUNDLED).unwrap();
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
    let json = serde_json::to_string_pretty(&v).unwrap() + "\n";
    let content = RunContent::bundled()
        .clone()
        .with_time_limit(TimeLimit::parse(json.as_bytes()).unwrap());
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
        common::run::avoid_bullets(engine, wanted, false)
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
            return common::run::run_input_for(engine, tick, &[EntityKind::Boss], u32::MAX);
        }
        let me = s.player.at;
        let enemies = || s.entities.iter().filter(|e| e.kind == EntityKind::Enemy);
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
                && e.asset.0 != "golem"
                && (Some(e.id) == self.mimic_revealed.get()
                    || !(e.asset.0 == "fairy_a" && e.hp <= 12_000))
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
                Some(_) => match nearest(&|e| !e.elite && !e.disguised) {
                    Some(escort) => return go(escort.at, FIGHT_NEAR, true),
                    None => self.leader_met.set(true),
                },
                None => self.leader_met.set(true),
            }
        }

        // Once the cap has held a minute back: the goblin, the shrine, the mimic, the rest.
        if (21_660..25_100).contains(&tick) {
            if !self.goblin_met.get() {
                match nearest(&|e| e.asset.0 == "fairy_a" && e.hp <= 12_000 && e.elite) {
                    Some(g) if d2(g.at) > 40 * 40 * 65_536 => return go(g.at, 40 * 256, false),
                    _ => self.goblin_met.set(true),
                }
            }
            if !self.shrine_touched.get() {
                if let Some(shrine) = nearest(&|e| e.asset.0 == "golem") {
                    return go(shrine.at, 0, false);
                }
            }
            if let Some(guardian) = nearest(&|e| !e.elite && !e.disguised) {
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
        match nearest(&|e| e.elite && e.asset.0 != "golem") {
            Some(e) if d2(e.at) < 240 * 240 * 65_536 => away(e.at),
            _ => Input::default(),
        }
    }
}

/// A `time_limit` mode with no waves, the final boss at tick 60 and `overtime` ticks of overtime,
/// as its file and as the content a run reads it with.
fn short_time_limit(overtime: u32) -> (String, RunContent) {
    let mut v: Value = serde_json::from_slice(TimeLimit::BUNDLED).unwrap();
    v["length_ticks"] = json!(60);
    v["boss_ticks"] = json!([60]);
    v["overtime_ticks"] = json!(overtime);
    v["waves"] = json!([]);
    v["elite_table"] =
        json!([{ "start_tick": 0, "weights": [{ "elite": "elite_low", "weight": 1 }] }]);
    let json = serde_json::to_string_pretty(&v).unwrap() + "\n";
    let content = RunContent::bundled()
        .clone()
        .with_time_limit(TimeLimit::parse(json.as_bytes()).unwrap());
    (json, content)
}

/// The run's buttons going for the boss as well as the enemies, getting out of the way of
/// bullets throughout.
fn boss_input(engine: &Engine, tick: u32) -> Input {
    common::run::run_input_for(
        engine,
        tick,
        &[EntityKind::Enemy, EntityKind::Boss],
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

/// A run case re-simulated from its file, which must verify.
fn replay_run_case(name: &str) -> RunCase {
    let bytes = fs::read(runs_dir().join(name).join("run.replay")).unwrap();
    let verified = run_replay::verify_run(&bytes).unwrap();
    assert_eq!(
        verified.outcome,
        Outcome::Failed,
        "the main character falls in `{name}`"
    );
    let replay = RunReplay::from_bytes(&bytes).unwrap();
    let commands = replay.unpack_commands().unwrap();
    assert!(
        commands.iter().any(|c| c.kind == CommandKind::Spend),
        "points are spent in `{name}`: {commands:?}"
    );
    assert_eq!(verified.commands, u32::try_from(commands.len()).unwrap());
    let mut engine = Engine::start(&replay.setup).unwrap();
    let mut next = commands.iter().peekable();
    let (mut kinds, mut used, mut match_ups) = (Vec::new(), Vec::new(), [0; 4]);
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
            } else {
                engine.spend(c.index).unwrap();
            }
        }
        engine.step(input);
        for e in engine.events() {
            match e {
                DomainEvent::StageSkillUsed { skill, level } => {
                    used.push((skill.0.clone(), *level));
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
                _ => {}
            }
        }
    }
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
    // The time_limit run ends with the main character falling, after spends.
    replay_run_case("time_limit");
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
        used.len() >= 2 && used.iter().all(|u| *u == ("black_hole".into(), 1)),
        "the black hole is used more than once: {used:?}"
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
            .filter(|e| e.kind == EntityKind::Enemy && (e.elite || e.disguised))
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
                        elite_kind(&v.asset.0, v.hp, v.aura_radius.is_some()),
                        tick,
                    ));
                }
                DomainEvent::EliteLeft { elite } => r.left.push((*elite, tick)),
                DomainEvent::ShrineActivated { elite } => r.activated.push(*elite),
                DomainEvent::Died { entity } => r.died.push(*entity),
                DomainEvent::MimicRevealed { elite } => r.revealed.push(*elite),
                DomainEvent::PlayerHit { hp_left } => {
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
    // A leader's aura raises a hit: an escort shot of 100 lands as 200 (100 * 200 / 100). Before
    // the middle grade's elite comes (minute 8, the first to shoot for 200) nothing else deals
    // 200.
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
    assert!(skipped.contains(&"mimic"), "{skipped:?}");
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
    // Hit boxes alone: no bullet ever flies, yet the main character is hurt and enemies die.
    let (outcome, hp, peak) = what_happens("melee");
    assert_eq!((outcome, peak), (Outcome::Running, 0));
    assert!(hp < 9_900, "the swings land: {hp} hp left");
    assert!(deaths("melee") >= 2, "the companion cleaves");
    // The map, not the portrait field: the corner holds the main character, and shots and
    // enemies get past where the portrait field's margin would have removed them.
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
/// tick, and the largest x any bullet or enemy reaches.
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
        let xs = s.bullets.iter().map(|b| b.at.x.0);
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
    if std::env::var_os("UPDATE_CORPUS").is_some() {
        // The stages are written at design values and scaled here (ADR-103).
        for (name, stage_bytes, inputs) in definitions() {
            let scaled = common::scaled_bytes(&stage_bytes, common::SCALE);
            write_case(&corpus_dir().join(name), &scaled, &inputs);
        }
        for (name, setup, mode, commands) in run_definitions() {
            write_run_case(&runs_dir().join(name), &setup, mode.as_deref(), &commands);
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
    let mut runs: Vec<PathBuf> = fs::read_dir(runs_dir())
        .expect("tests/corpus/runs/v0 exists")
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
