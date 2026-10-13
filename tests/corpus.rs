//! The replay corpus (ADR-019): `tests/corpus/v0/<name>/` holds a `stage.json`, a `run.replay`
//! and `hashes.txt` (the state hash after every tick, one lowercase hex hash a line). Each case
//! is re-simulated and every tick compared, so a change that moves any hash fails on the first
//! tick it moved, on x86_64 and on the arm64 runner alike. The files are plain, so the device
//! run can read them too (ADR-054).
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
    (0..ticks).map(|_| common::idle()).collect()
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
    // The Sub weapon is the `needle` preset's: five bullets in a fan along the facing.
    let stage = armed(
        stage_full(
            600,
            Some(far_boss()),
            vec![wave(1, 1, 0, shooter, 46_080, 100_000)],
            vec![k, healer],
        ),
        weapon(
            json!({ "preset": { "id": "aimed_single", "args": [2048] } }),
            json!("facing"),
            6,
        ),
        json!({ "preset": { "id": "needle" } }),
    );
    // Standing still for 40 ticks, shooting up at the enemy, then walking right, which turns
    // the Main weapon's shots away from it.
    let inputs = (0..80)
        .map(|t| if t < 40 { idle() } else { moving(64, 0) })
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
    // Both weapons are the `blaster` preset's: the twin shot along the facing, and one bullet
    // at the nearest enemy, the boss; its charge and its Sub skill are used too.
    let blaster = json!({ "preset": { "id": "blaster" } });
    let stage = armed_full(
        stage_full(600, Some(boss), vec![], vec![]),
        blaster.clone(),
        blaster,
    );
    // Half speed from side to side, turning to face up the field once a second so that the Main
    // weapon does not only shoot sideways, and evading twice.
    // Charging from tick 20 for 34 ticks (a perfect release at level 2), from tick 180 until
    // the evade on tick 230 breaks it, and the Sub skill pressed on ticks 40 and 260.
    let inputs = (0..300)
        .map(|t| {
            let input = match t {
                _ if t % 60 == 0 => moving(0, -1),
                100 | 230 => Input {
                    evade: true,
                    ..moving(50, 0)
                },
                _ if (t / 25) % 2 == 0 => moving(50, 0),
                _ => moving(-50, 0),
            };
            Input {
                charge: (20..54).contains(&t) || (180..240).contains(&t),
                sub: t == 40 || t == 260,
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
    // The Sub weapon aims at the horn every 12 ticks: a bullet 5 degrees off the line to it,
    // then one aimed again 3 ticks later. Once the horn breaks it has nothing to aim at and
    // waits.
    let at_horn = json!({ "inline": spawned(json!([
        { "type": "fire", "speed": 2048, "direction": { "type": "relative", "angle": 1280 } },
        { "type": "wait", "ticks": 3 },
        { "type": "fire", "speed": 2048, "direction": { "type": "aimed", "offset": 0 } }
    ])) });
    let mut stage = stage_full(600, Some(boss), vec![], vec![]);
    stage.player.sub =
        serde_json::from_value(sub_ref(weapon(at_horn, json!({ "part": "horn" }), 12))).unwrap();
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
/// stands still, then walks from side to side. Nobody fires a bullet.
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
    // The weapons do nothing here, so that no bullet flies.
    let stage = quiet(stage_full(600, Some(far_boss()), waves, vec![guard]));
    let inputs = (0..300)
        .map(|t| match t {
            0..150 => idle(),
            _ if (t / 40) % 2 == 0 => moving(33, 0),
            _ => moving(-33, 0),
        })
        .collect();
    ("melee", bytes(&stage), inputs)
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
        .map(|t| match t {
            0..60 => moving(127, 0),
            60..100 => moving(0, -127),
            100..220 => moving(-127, 0),
            _ => moving(-64, 64),
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
        landscape(),
        melee(),
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
    // Hit boxes alone: no bullet ever flies, yet the main character is hurt and enemies die.
    let (outcome, hp, peak) = what_happens("melee");
    assert_eq!((outcome, peak), (Outcome::Running, 0));
    assert!(hp < 99, "the swings land: {hp} hp left");
    assert!(deaths("melee") >= 2, "the companion cleaves");
    // The map, not the portrait field: the corner holds the main character, and shots and
    // enemies get past where the portrait field's margin would have removed them.
    let (corner, farthest) = landscape_reach();
    assert!(corner, "the main character reaches the top-right corner");
    assert!(farthest > 92_160 + 16_384, "x {farthest}");
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
