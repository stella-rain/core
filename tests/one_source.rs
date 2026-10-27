//! Core compiles its content in once, as `Pack::BUNDLED`, and one function reads the versions of
//! every versioned file (CS1a; ADR-CONT-1, ADR-CONT-3, ADR-SVC-1).

// The scans read the crate's tree, which the iOS Simulator run does not have.
#![cfg(not(target_os = "ios"))]

use std::fmt::Debug;
use std::path::{Path, PathBuf};

use stella_rain_core::constellations::{Constellations, MAX_CONSTELLATIONS_BYTES};
use stella_rain_core::difficulties::{Difficulties, MAX_DIFFICULTIES_BYTES};
use stella_rain_core::expertise::{Expertise, MAX_EXPERTISE_BYTES};
use stella_rain_core::match_ups::{MAX_MATCH_UPS_BYTES, MatchUps};
use stella_rain_core::pack::{ContentFiles, Pack, PackError};
use stella_rain_core::perk_sets::{MAX_PERK_SETS_BYTES, PerkSets};
use stella_rain_core::rewards::{MAX_REWARDS_BYTES, Rewards};
use stella_rain_core::rules::{MAX_RULES_BYTES, Rules};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::{self, MAX_SETUP_BYTES};
use stella_rain_core::run_stats::{MAX_RUN_STATS_BYTES, RunStats};
use stella_rain_core::runes::{MAX_RUNES_BYTES, Runes};
use stella_rain_core::stage_skill::{MAX_STAGE_SKILLS_BYTES, StageSkills};
use stella_rain_core::stardust::{MAX_STARDUST_BYTES, Stardust};
use stella_rain_core::starting_unlocks::{MAX_STARTING_UNLOCKS_BYTES, StartingUnlocks};
use stella_rain_core::time_limit::{MAX_TIME_LIMIT_BYTES, TimeLimit};
use stella_rain_core::validate::{self, Error, limits::MAX_STAGE_BYTES};
use stella_rain_core::weapons::{MAX_WEAPONS_BYTES, Weapons};

/// Every `include_bytes!` or `include_str!` of a content or mode file under `dir` (relative to the
/// crate), as `<path>:<literal>`.
fn content_includes(dir: &str) -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk(&root.join(dir), &mut files);
    files.sort();
    let mut found = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        let path = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for line in text.lines() {
            let code = line.split("//").next().unwrap_or("");
            for macro_name in ["include_bytes!(", "include_str!("] {
                let mut rest = code;
                while let Some(at) = rest.find(macro_name) {
                    rest = &rest[at + macro_name.len()..];
                    if let Some(literal) = rest.trim_start().strip_prefix('"')
                        && let Some(end) = literal.find('"')
                        && let literal = &literal[..end]
                        && (literal.contains("content/") || literal.contains("modes/"))
                    {
                        found.push(format!("{path}:{literal}"));
                    }
                }
            }
        }
    }
    found
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn only_the_pack_is_compiled_into_the_library() {
    assert_eq!(
        content_includes("src"),
        ["src/pack.rs:../content/pack.json"]
    );
}

#[test]
fn no_test_compiles_a_content_file_in() {
    assert_eq!(content_includes("tests"), Vec::<String>::new());
}

#[test]
fn one_reader_of_the_versions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    let mut versions = Vec::new();
    let mut helpers = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        let name = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        for _ in text.matches("struct Versions") {
            versions.push(name.clone());
        }
        for _ in text.matches("fn check_versions") {
            helpers.push(name.clone());
        }
    }
    assert_eq!(
        versions,
        ["version.rs"],
        "`struct Versions` outside src/version.rs"
    );
    assert_eq!(helpers, Vec::<String>::new(), "`fn check_versions`");
}

fn bundled(name: &str) -> Vec<u8> {
    ContentFiles::bundled()
        .get(name)
        .unwrap_or_else(|| panic!("no bundled file {name}"))
        .to_vec()
}

/// The first `from` in `bytes` replaced by `to`.
fn replace_first(bytes: &[u8], from: &str, to: &str) -> Vec<u8> {
    let text = std::str::from_utf8(bytes).unwrap();
    // A file may be written without the space after the colon.
    let (from, to) = if text.contains(from) {
        (from.to_string(), to.to_string())
    } else {
        (from.replace(": ", ":"), to.replace(": ", ":"))
    };
    assert!(text.contains(&from), "no {from} in the file");
    text.replacen(&from, &to, 1).into_bytes()
}

/// What a file reader says to the bytes of another game, a short file and a file over its limit.
fn refuses<E: Debug + PartialEq>(
    name: &str,
    bytes: &[u8],
    limit: usize,
    read: impl Fn(&[u8]) -> Result<(), E>,
    too_large: impl Fn(usize, usize) -> E,
    parse: impl Fn(usize, usize) -> E,
    unsupported: impl Fn(u32, u32) -> E,
) {
    assert_eq!(
        read(&replace_first(
            bytes,
            "\"sim_version\": 0",
            "\"sim_version\": 1"
        )),
        Err(unsupported(0, 1)),
        "{name}: sim_version 1"
    );
    assert_eq!(
        read(&replace_first(
            bytes,
            "\"schema_version\": 0",
            "\"schema_version\": 7"
        )),
        Err(unsupported(7, 0)),
        "{name}: schema_version 7"
    );
    assert_eq!(
        read(b"{\"schema_version\": 0}"),
        Err(parse(1, 21)),
        "{name}: no sim_version"
    );
    let mut padded = bytes.to_vec();
    padded.resize(limit + 1, b' ');
    assert_eq!(
        read(&padded),
        Err(too_large(limit + 1, limit)),
        "{name}: over the limit"
    );
}

/// As `refuses`, for a reader whose error is `validate::Error`.
fn refuses_file(name: &str, bytes: &[u8], limit: usize, read: impl Fn(&[u8]) -> Result<(), Error>) {
    refuses(
        name,
        bytes,
        limit,
        read,
        |size, limit| Error::TooLarge { size, limit },
        |line, column| Error::Parse { line, column },
        |schema_version, sim_version| Error::UnsupportedVersion {
            schema_version,
            sim_version,
        },
    );
}

#[test]
fn every_versioned_file_refuses_another_game_the_same_way() {
    let content = RunContent::bundled();
    let file = |name: &str| bundled(name);
    refuses_file("run_stats", &file("run_stats"), MAX_RUN_STATS_BYTES, |b| {
        RunStats::parse(b).map(|_| ())
    });
    refuses_file("rewards", &file("rewards"), MAX_REWARDS_BYTES, |b| {
        Rewards::parse(b).map(|_| ())
    });
    refuses_file(
        "stage_skills",
        &file("stage_skills"),
        MAX_STAGE_SKILLS_BYTES,
        |b| StageSkills::parse(b).map(|_| ()),
    );
    refuses_file("expertise", &file("expertise"), MAX_EXPERTISE_BYTES, |b| {
        Expertise::parse(b).map(|_| ())
    });
    refuses_file("match_ups", &file("match_ups"), MAX_MATCH_UPS_BYTES, |b| {
        MatchUps::parse(b).map(|_| ())
    });
    refuses_file("runes", &file("runes"), MAX_RUNES_BYTES, |b| {
        Runes::parse(b).map(|_| ())
    });
    refuses_file("perk_sets", &file("perk_sets"), MAX_PERK_SETS_BYTES, |b| {
        PerkSets::parse(b).map(|_| ())
    });
    refuses_file(
        "difficulties",
        &file("difficulties"),
        MAX_DIFFICULTIES_BYTES,
        |b| Difficulties::parse(b).map(|_| ()),
    );
    refuses_file("weapons", &file("weapons"), MAX_WEAPONS_BYTES, |b| {
        Weapons::parse(b).map(|_| ())
    });
    refuses_file("rules", &file("rules"), MAX_RULES_BYTES, |b| {
        Rules::parse(b).map(|_| ())
    });
    refuses_file(
        "starting_unlocks",
        &file("starting_unlocks"),
        MAX_STARTING_UNLOCKS_BYTES,
        |b| {
            StartingUnlocks::parse_with(
                b,
                content.perk_sets(),
                content.stage_skills(),
                content.runes(),
                content.weapons(),
            )
            .map(|_| ())
        },
    );
    refuses_file(
        "constellations",
        &file("constellations"),
        MAX_CONSTELLATIONS_BYTES,
        |b| Constellations::parse_with(b, content.rewards()).map(|_| ()),
    );
    refuses_file("stardust", &file("stardust"), MAX_STARDUST_BYTES, |b| {
        Stardust::parse(b).map(|_| ())
    });
    refuses_file(
        "time_limit",
        &file("time_limit"),
        MAX_TIME_LIMIT_BYTES,
        |b| TimeLimit::parse(b).map(|_| ()),
    );
    refuses_file("stage", &file("trial"), MAX_STAGE_BYTES, |b| {
        validate::parse_and_validate(b).map(|_| ())
    });
    let setup = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/run_setup/trial.json"),
    )
    .unwrap();
    refuses_file("run_setup", &setup, MAX_SETUP_BYTES, |b| {
        run_setup::parse_and_validate(b).map(|_| ())
    });
    refuses(
        "pack",
        Pack::BUNDLED,
        Pack::MAX_BYTES,
        |b| Pack::from_bytes(b).map(|_| ()),
        |size, limit| PackError::TooLarge { size, limit },
        |line, column| PackError::Parse { line, column },
        |schema_version, sim_version| PackError::UnsupportedVersion {
            schema_version,
            sim_version,
        },
    );
}
