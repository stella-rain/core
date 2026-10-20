//! The content pack (ADR-080, core#148): one JSON file holding the content files, read as
//! untrusted bytes. A pack exported from the bundled files gives the bundled content and the
//! same runs; every other pack is refused before it reaches a run unless each check passes.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use stella_rain_core::expertise::Expertise;
use stella_rain_core::match_ups::MatchUps;
use stella_rain_core::pack::{self, Pack, PackError};
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{self, RunReplay};
use stella_rain_core::run_setup;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::runes::Runes;
use stella_rain_core::stage_skill::StageSkills;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::{self, Error};

fn bundled() -> Vec<u8> {
    pack::export_bundled()
}

fn value() -> Value {
    serde_json::from_slice(&bundled()).expect("the exported pack is JSON")
}

fn bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap()
}

fn load(v: &Value) -> Result<Pack, PackError> {
    Pack::from_bytes(&bytes(v))
}

/// `content/pack.json` is generated: `cargo run --bin content-pack export > content/pack.json`.
#[test]
fn the_bundled_pack_is_the_export_of_the_individual_files() {
    assert!(
        Pack::BUNDLED == bundled().as_slice(),
        "content/pack.json drifted from the individual files; regenerate it with \
         `cargo run --bin content-pack export > content/pack.json`"
    );
    assert_eq!(
        Pack::bundled().sha256(),
        Sha256::digest(Pack::BUNDLED).as_slice()
    );
}

#[test]
fn the_bundled_content_is_the_individual_files() {
    let content = RunContent::bundled();
    assert_eq!(
        content.run_stats().sha256(),
        RunStats::parse(RunStats::BUNDLED).unwrap().sha256()
    );
    assert_eq!(
        content.rewards().sha256(),
        Rewards::parse(Rewards::BUNDLED).unwrap().sha256()
    );
    assert_eq!(
        content.stage_skills().sha256(),
        StageSkills::parse(StageSkills::BUNDLED).unwrap().sha256()
    );
    assert_eq!(
        content.expertise().sha256(),
        Expertise::parse(Expertise::BUNDLED).unwrap().sha256()
    );
    assert_eq!(
        content.match_ups().sha256(),
        MatchUps::parse(MatchUps::BUNDLED).unwrap().sha256()
    );
    assert_eq!(
        content.runes().sha256(),
        Runes::parse(Runes::BUNDLED).unwrap().sha256()
    );
    assert_eq!(
        content.time_limit().sha256(),
        TimeLimit::parse(TimeLimit::BUNDLED).unwrap().sha256()
    );
}

#[test]
fn the_exported_pack_gives_the_bundled_content() {
    let pack = Pack::from_bytes(&bundled()).expect("the exported pack loads");
    assert_eq!(pack.content(), RunContent::bundled());
    let trial = validate::parse_and_validate(run_setup::mode_stage("trial").unwrap()).unwrap();
    assert_eq!(pack.trial(), &trial);
    assert_eq!(pack.schema_version(), 0);
    assert_eq!(pack.sim_version(), 0);
    let sha: [u8; 32] = Sha256::digest(bundled()).into();
    assert_eq!(pack.sha256(), sha);
}

#[test]
fn the_exported_pack_has_its_fields_in_order() {
    let text = String::from_utf8(bundled()).unwrap();
    let keys = [
        "\"schema_version\"",
        "\"sim_version\"",
        "\"introduced\"",
        "\"run_stats\"",
        "\"rewards\"",
        "\"stage_skills\"",
        "\"expertise\"",
        "\"match_ups\"",
        "\"runes\"",
        "\"modes\"",
    ];
    let at: Vec<usize> = keys.iter().map(|k| text.find(k).unwrap()).collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{at:?}");
    assert!(text.ends_with("}\n"));
}

#[test]
fn a_saved_pack_loads_as_the_same_content() {
    // The same files written compactly: other bytes, so another pack hash, the same content.
    let pack = load(&value()).expect("a compact pack loads");
    assert_ne!(
        pack.sha256(),
        Pack::from_bytes(&bundled()).unwrap().sha256()
    );
    assert_eq!(
        pack.content().run_stats().stats(),
        RunContent::bundled().run_stats().stats()
    );
    assert_eq!(
        pack.content().rewards().rewards(),
        RunContent::bundled().rewards().rewards()
    );
}

/// Every run case that plays with the bundled files gives the same hash every tick with the
/// exported pack's content, and its replay verifies with it.
#[test]
fn a_run_from_the_pack_hashes_as_one_from_the_bundled_content() {
    let content: RunContent = Pack::from_bytes(&bundled()).unwrap().into();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/runs/v0");
    let mut checked = Vec::new();
    for case in ["trial", "stage_skill", "time_limit"] {
        let case_dir = dir.join(case);
        assert!(
            !case_dir.join("mode.json").exists(),
            "{case} has a mode file"
        );
        let replay = fs::read(case_dir.join("run.replay")).unwrap();
        let hashes: Vec<u64> = fs::read_to_string(case_dir.join("hashes.txt"))
            .unwrap()
            .lines()
            .map(|l| u64::from_str_radix(l, 16).unwrap())
            .collect();
        assert!(RunReplay::from_bytes(&replay).is_ok());
        run_replay::verify_run_against_with(&replay, &content, &hashes)
            .unwrap_or_else(|e| panic!("{case}: {e}"));
        checked.push(case);
    }
    assert_eq!(checked.len(), 3);
}

#[test]
fn a_pack_at_the_byte_limit_loads_and_one_past_it_is_not_parsed() {
    let mut at = bundled();
    at.resize(Pack::MAX_BYTES, b' ');
    assert!(Pack::from_bytes(&at).is_ok());
    // Not JSON at all: the size is refused before any parsing.
    let past = vec![b'x'; Pack::MAX_BYTES + 1];
    assert_eq!(
        Pack::from_bytes(&past).unwrap_err(),
        PackError::TooLarge {
            size: Pack::MAX_BYTES + 1,
            limit: Pack::MAX_BYTES,
        }
    );
}

#[test]
fn an_unknown_field_is_refused() {
    let mut v = value();
    v["extra"] = json!(1);
    assert!(matches!(load(&v), Err(PackError::Parse { .. })));
    let mut v = value();
    v["modes"]["extra"] = json!({});
    assert!(matches!(load(&v), Err(PackError::Parse { .. })));
    let mut v = value();
    v["introduced"]["extra"] = json!([]);
    assert!(matches!(load(&v), Err(PackError::Parse { .. })));
    let mut v = value();
    v["run_stats"]["stats"][0]["extra"] = json!(1);
    assert!(matches!(
        load(&v),
        Err(PackError::File {
            file: "run_stats",
            error: Error::Parse { .. }
        })
    ));
}

#[test]
fn a_missing_file_is_refused() {
    for file in ["run_stats", "rewards", "stage_skills", "runes"] {
        let mut v = value();
        v.as_object_mut().unwrap().remove(file);
        assert!(matches!(load(&v), Err(PackError::Parse { .. })), "{file}");
    }
}

#[test]
fn an_unknown_pack_version_is_refused() {
    for (field, n) in [
        ("schema_version", 1),
        ("sim_version", 1),
        ("sim_version", 7),
    ] {
        let mut v = value();
        v[field] = json!(n);
        assert!(
            matches!(load(&v), Err(PackError::UnsupportedVersion { .. })),
            "{field} {n}"
        );
    }
    // A version that is not even a u32.
    let mut v = value();
    v["sim_version"] = json!(-1);
    assert!(matches!(load(&v), Err(PackError::Parse { .. })));
}

#[test]
fn a_file_of_another_version_is_refused() {
    let mut v = value();
    v["runes"]["sim_version"] = json!(1);
    assert!(matches!(
        load(&v),
        Err(PackError::File {
            file: "runes",
            error: Error::UnsupportedVersion { .. }
        })
    ));
}

/// Each count budget, past its limit by one: refused naming the list and its limit, before
/// the file itself is read (the extra entries are copies, which the file would refuse too).
#[test]
fn each_count_budget_is_enforced() {
    let lists: [(&str, usize); 15] = [
        ("/run_stats/stats", pack::MAX_RUN_STATS),
        ("/rewards/rewards", pack::MAX_REWARDS),
        ("/stage_skills/stage_skills", pack::MAX_STAGE_SKILLS),
        ("/expertise/weapons", pack::MAX_EXPERTISE_WEAPONS),
        ("/expertise/weapons/0/common", pack::MAX_EXPERTISE_COMMON),
        ("/runes/on_hit", pack::MAX_ON_HIT_RUNES),
        ("/runes/damage_over_time", pack::MAX_DAMAGE_OVER_TIME_RUNES),
        ("/modes/time_limit/waves", pack::MAX_TIME_LIMIT_WAVES),
        ("/modes/time_limit/boss_ticks", pack::MAX_BOSS_TICKS),
        ("/modes/time_limit/mid_boss_pool", pack::MAX_MID_BOSS_POOL),
        ("/introduced/run_stats", pack::MAX_RUN_STATS),
        ("/introduced/rewards", pack::MAX_REWARDS),
        ("/introduced/stage_skills", pack::MAX_STAGE_SKILLS),
        ("/introduced/expertise", pack::MAX_EXPERTISE_WEAPONS),
        ("/introduced/expertise/0/ids", pack::MAX_EXPERTISE_IDS),
    ];
    for (pointer, limit) in lists {
        let mut v = value();
        let list = v.pointer_mut(pointer).unwrap().as_array_mut().unwrap();
        let first = list[0].clone();
        list.resize(limit + 1, first);
        match load(&v) {
            Err(PackError::Budget {
                path,
                limit: l,
                found,
            }) => {
                assert_eq!(
                    path,
                    format!("${}", pointer.replace('/', ".")).replace(".0", "[0]")
                );
                assert_eq!((l, found), (limit, limit + 1), "{pointer}");
            }
            other => panic!("{pointer}: {other:?}"),
        }
    }
}

#[test]
fn a_bound_of_a_file_is_enforced_inside_a_pack() {
    let cases: [(&str, Value, &str); 4] = [
        ("/run_stats/stats/0/step", json!(0), "run_stats"),
        ("/rewards/rewards/0/limit", json!(0), "rewards"),
        ("/match_ups/max_type_bonus_pct", json!(100_000), "match_ups"),
        ("/modes/time_limit/length_ticks", json!(0), "time_limit"),
    ];
    for (pointer, bad, file) in cases {
        let mut v = value();
        *v.pointer_mut(pointer).unwrap() = bad;
        match load(&v) {
            Err(PackError::File {
                file: f,
                error: Error::Invalid(issues),
            }) => {
                assert_eq!(f, file, "{pointer}");
                assert!(!issues.is_empty());
            }
            other => panic!("{pointer}: {other:?}"),
        }
    }
    // The trial stage is checked as a stage.
    let mut v = value();
    v["modes"]["trial"]["length_ticks"] = json!(0);
    assert!(matches!(
        load(&v),
        Err(PackError::File {
            file: "trial",
            error: Error::Invalid(_)
        })
    ));
}

#[test]
fn a_reward_needs_a_stage_skill_of_the_same_pack() {
    let mut v = value();
    let skills = v["stage_skills"]["stage_skills"].as_array_mut().unwrap();
    skills.retain(|s| s["id"] != json!("aegis"));
    v["introduced"]["stage_skills"]
        .as_array_mut()
        .unwrap()
        .retain(|s| s["id"] != json!("aegis"));
    assert!(matches!(
        load(&v),
        Err(PackError::File {
            file: "rewards",
            error: Error::Invalid(_)
        })
    ));
}

#[test]
fn an_id_newer_than_the_pack_is_refused() {
    let mut v = value();
    v["introduced"]["rewards"][0]["sim_version"] = json!(1);
    let Err(PackError::Introduced(issues)) = load(&v) else {
        panic!("not refused as newer");
    };
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path, "$.introduced.rewards[0].sim_version");
    assert!(issues[0].message.contains("newer than the pack's 0"));

    let mut v = value();
    v["introduced"]["expertise"][1]["ids"][2]["sim_version"] = json!(3);
    let Err(PackError::Introduced(issues)) = load(&v) else {
        panic!("not refused as newer");
    };
    assert_eq!(
        issues[0].path,
        "$.introduced.expertise[1].ids[2].sim_version"
    );
}

#[test]
fn every_id_a_pack_introduces_is_listed_once() {
    // Missing.
    let mut v = value();
    v["introduced"]["run_stats"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    assert!(matches!(load(&v), Err(PackError::Introduced(_))));
    // Listed twice.
    let mut v = value();
    let list = v["introduced"]["stage_skills"].as_array_mut().unwrap();
    let first = list[0].clone();
    list.push(first);
    assert!(matches!(load(&v), Err(PackError::Introduced(_))));
    // Not in the file.
    let mut v = value();
    v["introduced"]["rewards"][0]["id"] = json!("no_such_reward");
    assert!(matches!(load(&v), Err(PackError::Introduced(_))));
    // A weapon the expertise does not have, and a weapon left out.
    let mut v = value();
    v["introduced"]["expertise"][0]["weapon"] = json!("maul");
    assert!(matches!(load(&v), Err(PackError::Introduced(_))));
    let mut v = value();
    v["introduced"]["expertise"].as_array_mut().unwrap().pop();
    assert!(matches!(load(&v), Err(PackError::Introduced(_))));
}

#[test]
fn refusals_never_repeat_the_pack() {
    let marker = "zq_secret_marker";
    let mut cases = Vec::new();
    let mut v = value();
    v[marker] = json!(marker);
    cases.push(bytes(&v));
    let mut v = value();
    v["introduced"]["rewards"][0]["id"] = json!(marker);
    cases.push(bytes(&v));
    let mut v = value();
    v["run_stats"]["stats"][0]["id"] = json!(marker);
    v["run_stats"]["stats"][0]["title"] = json!("");
    cases.push(bytes(&v));
    let mut v = value();
    v["run_stats"]["stats"][0][marker] = json!(marker);
    cases.push(bytes(&v));
    for case in cases {
        let error = Pack::from_bytes(&case).unwrap_err();
        let shown = format!("{error} {error:?}");
        assert!(!shown.contains("zq_"), "{shown}");
    }
}

#[test]
fn garbage_is_refused() {
    for garbage in [
        &b""[..],
        b"{",
        b"null",
        b"[]",
        b"\xff\xfe",
        b"{\"schema_version\":0}",
    ] {
        assert!(Pack::from_bytes(garbage).is_err());
    }
}

/// The pack finds each file's bytes itself: a key written with escapes, members in another
/// order and other whitespace find the same files, with the same bytes.
#[test]
fn a_file_is_found_whatever_the_pack_s_layout() {
    let text = String::from_utf8(bundled()).unwrap();
    let escaped = text
        .replacen("\"run_stats\": {", "\"run\\u005fstats\" :\t{", 1)
        .replacen("\"time_limit\": {", "\"time\\u005flimit\"\n:{", 1);
    assert_ne!(escaped, text);
    let pack = Pack::from_bytes(escaped.as_bytes()).expect("escaped keys load");
    assert_eq!(pack.content(), RunContent::bundled());

    // The modes first, the versions last.
    let v = value();
    let mut moved = serde_json::Map::new();
    for key in ["modes", "runes", "introduced", "run_stats", "rewards"] {
        moved.insert(key.into(), v[key].clone());
    }
    for key in [
        "expertise",
        "match_ups",
        "stage_skills",
        "sim_version",
        "schema_version",
    ] {
        moved.insert(key.into(), v[key].clone());
    }
    let pack = load(&Value::Object(moved)).expect("another order loads");
    assert_eq!(
        pack.content().stage_skills().skills(),
        RunContent::bundled().stage_skills().skills()
    );
}

#[test]
fn a_file_given_twice_is_refused() {
    let text = String::from_utf8(bundled()).unwrap();
    let twice = text.replacen("\"run_stats\": {", "\"runes\": {}, \"run_stats\": {", 1);
    assert!(matches!(
        Pack::from_bytes(twice.as_bytes()),
        Err(PackError::Parse { .. })
    ));
}
