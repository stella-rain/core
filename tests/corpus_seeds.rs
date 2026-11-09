//! The seed search of the solo run cases (ADR-SIM-7, ADR-SIM-3). Each case is played in equipment
//! mode from a world seed and a building bot's seed, written to a scratch tree under `target/`
//! and checked by the promise its name makes: the assertion code the corpus tests use, run on the
//! candidate through `catch_unwind`. The first pair that delivers is the case's seed. No other
//! predicate decides: a case with no hit is reported, never changed.

use crate::common::run::trial_setup;
use crate::corpus::{
    RunDefinition, assert_elites, assert_five_kinds, assert_longest_run, assert_match_ups,
    assert_short_run_at, assert_stage_skill, case_content, elite_reach_at,
    equipment_run_definition, replay_run_case_at, write_atomic,
};
use crate::corpus_pins::{write_case_json, write_pack_file};
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use stella_rain_core::content_hash::to_hex;
use stella_rain_core::engine::Outcome;
use stella_rain_core::pack::Pack;
use stella_rain_core::run_replay;
use stella_rain_core::validate::limits::MAX_TICKS;

/// The case's promise on the written candidate `case` (named `name`): the corpus tests' assertions.
fn promise(name: &str, case: &Path) {
    match name {
        "tutorial" => {
            let bytes = fs::read(case.join("run.replay")).unwrap();
            let v = run_replay::verify_run_with(&bytes, &case_content(case).unwrap()).unwrap();
            assert_eq!(v.outcome, Outcome::Cleared, "the outcome of `{name}`");
        }
        "trial" => {
            let run = replay_run_case_at(case, name, Outcome::Failed);
            assert_five_kinds(&run.kinds);
            assert_match_ups(run.match_ups);
        }
        "stage_skill" => assert_stage_skill(&replay_run_case_at(case, name, Outcome::Failed).used),
        "time_limit" => assert_longest_run(case),
        "time_limit_kill" => assert_short_run_at(case, name, Outcome::Cleared, None, false),
        "time_limit_overtime" => {
            assert_short_run_at(case, name, Outcome::Failed, Some(600), true);
        }
        "elites" => assert_elites(&elite_reach_at(case, name)),
        // `easy` and `rainy` name no outcome: the run only has to verify.
        _ => {
            let bytes = fs::read(case.join("run.replay")).unwrap();
            run_replay::verify_run_with(&bytes, &case_content(case).unwrap()).unwrap();
        }
    }
}

/// Writes the case `def` into the scratch tree `tree` (`runs/<name>/`, `packs/`), as the bless
/// writes a run case, and returns its folder.
fn write_candidate(tree: &Path, def: &RunDefinition) -> PathBuf {
    let (name, setup, pack_bytes, commands) = def;
    let dir = tree.join("runs").join(name);
    fs::create_dir_all(&dir).unwrap();
    let (pack, sha) = match pack_bytes {
        Some(bytes) => (
            Pack::from_bytes(bytes).unwrap(),
            write_pack_file(tree, bytes).unwrap(),
        ),
        None => (Pack::bundled().clone(), to_hex(&Pack::bundled().sha256())),
    };
    write_case_json(&dir, &sha).unwrap();
    let recorded = run_replay::record_run_with_pack(setup, &pack, commands).unwrap();
    write_atomic(&dir.join("run.replay"), &recorded.to_bytes());
    dir
}

/// One pair's result: `case world bot outcome ticks hit|miss[: first failing assertion]`.
struct Pair {
    hit: bool,
    line: String,
}

/// Plays `name` from (`world`, `bot`), writes it under `tree`, runs `check` on it.
fn run_pair(tree: &Path, name: &str, world: u64, bot: u64, check: &dyn Fn(&Path)) -> Pair {
    let def = equipment_run_definition(name, world, bot);
    let dir = write_candidate(tree, &def);
    let bytes = fs::read(dir.join("run.replay")).unwrap();
    let v = run_replay::verify_run_with(&bytes, &case_content(&dir).unwrap()).unwrap();
    let verdict = catch_unwind(AssertUnwindSafe(|| check(&dir)));
    let head = format!("{name} {world} {bot} {:?} {}", v.outcome, v.ticks);
    match verdict {
        Ok(()) => Pair {
            hit: true,
            line: format!("{head} hit"),
        },
        Err(e) => {
            let why = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_default();
            let why = why.lines().next().unwrap_or_default().to_string();
            Pair {
                hit: false,
                line: format!("{head} miss: {why}"),
            }
        }
    }
}

fn scratch(test: &str) -> PathBuf {
    let tree = crate::data_root::target_dir()
        .join("corpus-search")
        .join(test);
    fs::create_dir_all(&tree).unwrap();
    tree
}

/// Searches `name` over world seeds 1..=20 x bot seeds 1..=20 (world outer, bot inner), prints a
/// line per pair and stops at the first hit. Returns it.
fn search(tree: &Path, name: &str) -> Option<(u64, u64)> {
    for world in 1..=20 {
        for bot in 1..=20 {
            let pair = run_pair(tree, name, world, bot, &|dir| promise(name, dir));
            println!("{}", pair.line);
            if pair.hit {
                println!("SEED {name} world {world} bot {bot}");
                return Some((world, bot));
            }
        }
    }
    println!("NOHIT {name}");
    None
}

#[test]
#[ignore = "a seed search, run by hand"]
fn search_corpus_run_seeds() {
    let tree = scratch("search");
    let quiet = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    // The cases whose seed the search finds.
    let mut missing = Vec::new();
    for name in [
        "time_limit_kill",
        "stage_skill",
        "elites",
        "trial",
        "time_limit",
    ] {
        if search(&tree, name).is_none() {
            missing.push(name);
        }
    }
    // The cases that keep the setup they have (the offers-era seed, bot seed 1).
    let keep = trial_setup().seed;
    for name in ["tutorial", "time_limit_overtime", "easy", "rainy"] {
        let pair = run_pair(&tree, name, keep, 1, &|dir| promise(name, dir));
        println!("{}", pair.line);
        if !pair.hit {
            missing.push(name);
        }
    }
    std::panic::set_hook(quiet);
    println!("TICKS {MAX_TICKS}");
    assert!(missing.is_empty(), "no seed delivers: {missing:?}");
}

#[test]
fn the_search_reports_a_hit_and_a_miss() {
    let tree = scratch("fast");
    for name in ["tutorial", "time_limit_overtime", "time_limit_kill"] {
        let hit = run_pair(&tree, name, 1, 1, &|dir| promise(name, dir));
        assert!(hit.hit, "{}", hit.line);
    }
    // The same run under a promise it does not keep is a miss that says which assertion failed.
    let miss = run_pair(&tree, "tutorial", 1, 1, &|dir| {
        let bytes = fs::read(dir.join("run.replay")).unwrap();
        let v = run_replay::verify_run_with(&bytes, &case_content(dir).unwrap()).unwrap();
        assert_eq!(v.outcome, Outcome::Failed, "the outcome of `tutorial`");
    });
    assert!(!miss.hit, "{}", miss.line);
    assert!(
        miss.line
            .contains("miss: assertion `left == right` failed: the outcome of `tutorial`"),
        "{}",
        miss.line
    );
}

// The seven movable party cases (ADR-SIM-7, ADR-SIM-3).

/// The seven party cases the search moves to equipment mode.
const PARTY_NAMES: [&str; 7] = [
    "two_time_limit",
    "three_time_limit",
    "three_tutorial",
    "join_two",
    "leave_hole",
    "two_revive",
    "three_revive",
];

/// Plays `name` from (`world`, `bot`) in memory and checks its promise: one line
/// `case world bot outcome ticks hit|miss: <first missed predicate>`.
fn party_pair(name: &'static str, world: u64, bot: u64) -> Pair {
    let replay = crate::run_play::party_case(name, world, bot)
        .play()
        .replay();
    let miss = crate::party_run::party_promise_miss(name, &replay);
    let head = match run_replay::verify_run(&replay.to_bytes()) {
        Ok(v) => format!("{name} {world} {bot} {:?} {}", v.outcome, v.ticks),
        Err(e) => format!("{name} {world} {bot} unverified {e:?}"),
    };
    match miss {
        None => Pair {
            hit: true,
            line: format!("{head} hit"),
        },
        Some(why) => Pair {
            hit: false,
            line: format!("{head} miss: {why}"),
        },
    }
}

/// Searches a party case over world seeds 1..=20 x bot seeds 1..=20 (world outer, bot inner),
/// prints a line per pair and stops at the first hit.
fn search_party(name: &'static str) -> Option<(u64, u64)> {
    for world in 1..=20 {
        for bot in 1..=20 {
            let pair = party_pair(name, world, bot);
            println!("{}", pair.line);
            if pair.hit {
                println!("SEED {name} world {world} bot {bot}");
                return Some((world, bot));
            }
        }
    }
    println!("NOHIT {name}");
    None
}

/// `cargo test --release --locked --test integration search_corpus_party_seeds -- --ignored
/// --nocapture`.
#[test]
#[ignore = "a seed search, run by hand"]
fn search_corpus_party_seeds() {
    let quiet = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let missing: Vec<_> = PARTY_NAMES
        .into_iter()
        .filter(|name| search_party(name).is_none())
        .collect();
    std::panic::set_hook(quiet);
    assert!(missing.is_empty(), "no seed delivers: {missing:?}");
}
