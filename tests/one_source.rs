//! Core compiles its content in once, as `Pack::BUNDLED`, and one function reads the versions of
//! every versioned file (CS1a; ADR-CONT-1, ADR-CONT-3, ADR-SVC-1).

// The scans read the crate's tree, which the iOS Simulator run does not have.
#![cfg(not(target_os = "ios"))]

use std::fmt::Debug;
use std::path::{Path, PathBuf};
use stella_rain_core::version::{SCHEMA_VERSION, SIM_VERSION, Versions};

use stella_rain_core::attack_presets::{AttackPresets, MAX_ATTACK_PRESETS_BYTES};
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
fn only_the_pack_and_text_are_compiled_into_the_library() {
    assert_eq!(
        content_includes("src"),
        [
            "src/pack.rs:../content/pack.json",
            "src/text.rs:../content/text.en.json",
        ]
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
    let newer = SIM_VERSION + 1;
    assert_eq!(
        read(&replace_first(
            bytes,
            &format!("\"sim_version\": {SIM_VERSION}"),
            &format!("\"sim_version\": {newer}")
        )),
        Err(unsupported(SCHEMA_VERSION, newer)),
        "{name}: sim_version {newer}"
    );
    assert_eq!(
        read(&replace_first(
            bytes,
            &format!("\"schema_version\": {SCHEMA_VERSION}"),
            "\"schema_version\": 7"
        )),
        Err(unsupported(7, SIM_VERSION)),
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

/// A pack's content files have no version of their own; the pack's header is the only one. The
/// stages in `modes/` are stages and keep theirs (CS1a-2).
#[test]
fn no_content_file_has_its_own_version() {
    let content_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("content");
    for slot in ContentFiles::LAYOUT {
        let bytes = std::fs::read(slot.path(&content_dir)).unwrap();
        let file: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let object = file.as_object().unwrap();
        let is_stage = matches!(slot.name, "trial" | "tutorial");
        for field in ["schema_version", "sim_version"] {
            if is_stage {
                let current = if field == "schema_version" {
                    SCHEMA_VERSION
                } else {
                    SIM_VERSION
                };
                assert_eq!(
                    object.get(field),
                    Some(&current.into()),
                    "{} {field}",
                    slot.name
                );
            } else {
                assert!(!object.contains_key(field), "{} has a {field}", slot.name);
            }
        }
    }
}

/// What a content file's reader says when it is given the versions of another game, a file
/// with a version of its own and a file over its limit. A content file has no versions of its
/// own: it is read at the pack's (CS1a-2).
fn refuses_content(
    name: &str,
    bytes: &[u8],
    limit: usize,
    read: impl Fn(&[u8], Versions) -> Result<(), Error>,
) {
    let at = |schema_version, sim_version| Versions {
        schema_version,
        sim_version,
    };
    let unsupported = |schema_version, sim_version| {
        Err(Error::UnsupportedVersion {
            schema_version,
            sim_version,
        })
    };
    assert_eq!(
        read(bytes, at(0, 1)),
        unsupported(0, 1),
        "{name}: sim_version 1"
    );
    assert_eq!(
        read(bytes, at(7, 0)),
        unsupported(7, 0),
        "{name}: schema_version 7"
    );
    // A version in the file is an unknown field, on the line after the opening brace.
    let with_own = replace_first(bytes, "{\n", "{\n  \"schema_version\": 0,\n");
    assert_eq!(
        read(&with_own, Versions::CURRENT),
        Err(Error::Parse {
            line: 2,
            column: 18
        }),
        "{name}: a version of its own"
    );
    let mut padded = bytes.to_vec();
    padded.resize(limit + 1, b' ');
    assert_eq!(
        read(&padded, Versions::CURRENT),
        Err(Error::TooLarge {
            size: limit + 1,
            limit
        }),
        "{name}: over the limit"
    );
}

/// A content file's reader, given its bytes and the versions to read them at.
type Reader = fn(&[u8], Versions) -> Result<(), Error>;

#[test]
fn every_versioned_file_refuses_another_game_the_same_way() {
    let content = RunContent::bundled();
    let file = |name: &str| bundled(name);
    let rows: [(&str, usize, Reader); 12] = [
        ("run_stats", MAX_RUN_STATS_BYTES, |b, v| {
            RunStats::parse(b, v).map(|_| ())
        }),
        ("rewards", MAX_REWARDS_BYTES, |b, v| {
            Rewards::parse(b, v).map(|_| ())
        }),
        ("stage_skills", MAX_STAGE_SKILLS_BYTES, |b, v| {
            StageSkills::parse(b, v).map(|_| ())
        }),
        ("expertise", MAX_EXPERTISE_BYTES, |b, v| {
            Expertise::parse(b, v).map(|_| ())
        }),
        ("match_ups", MAX_MATCH_UPS_BYTES, |b, v| {
            MatchUps::parse(b, v).map(|_| ())
        }),
        ("runes", MAX_RUNES_BYTES, |b, v| {
            Runes::parse(b, v).map(|_| ())
        }),
        ("perk_sets", MAX_PERK_SETS_BYTES, |b, v| {
            PerkSets::parse(b, v).map(|_| ())
        }),
        ("difficulties", MAX_DIFFICULTIES_BYTES, |b, v| {
            Difficulties::parse(b, v).map(|_| ())
        }),
        ("attack_presets", MAX_ATTACK_PRESETS_BYTES, |b, v| {
            AttackPresets::parse(b, v).map(|_| ())
        }),
        ("weapons", MAX_WEAPONS_BYTES, |b, v| {
            Weapons::parse(b, v).map(|_| ())
        }),
        ("rules", MAX_RULES_BYTES, |b, v| {
            Rules::parse(b, v).map(|_| ())
        }),
        ("stardust", MAX_STARDUST_BYTES, |b, v| {
            Stardust::parse(b, v).map(|_| ())
        }),
    ];
    for (name, limit, read) in rows {
        refuses_content(name, &file(name), limit, read);
    }
    refuses_content(
        "time_limit",
        &file("time_limit"),
        MAX_TIME_LIMIT_BYTES,
        |b, v| TimeLimit::parse(b, v).map(|_| ()),
    );
    refuses_content(
        "starting_unlocks",
        &file("starting_unlocks"),
        MAX_STARTING_UNLOCKS_BYTES,
        |b, v| {
            StartingUnlocks::parse_with(
                b,
                v,
                content.perk_sets(),
                content.stage_skills(),
                content.runes(),
                content.weapons(),
            )
            .map(|_| ())
        },
    );
    refuses_content(
        "constellations",
        &file("constellations"),
        MAX_CONSTELLATIONS_BYTES,
        |b, v| Constellations::parse_with(b, v, content.rewards()).map(|_| ()),
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
