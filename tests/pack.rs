//! The content pack (ADR-CONT-5, core#148): one JSON file holding the content files, read as
//! untrusted bytes. A pack exported from the bundled files gives the bundled content and the
//! same runs; every other pack is refused before it reaches a run unless each check passes.

use std::fs;
use std::path::Path;
use stella_rain_core::version::{
    NEWEST_READ_SCHEMA_VERSION, SCHEMA_VERSION, SIM_VERSION, Versions,
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use stella_rain_core::pack::{self, ContentFiles, Pack, PackError};
use stella_rain_core::rewards;
use stella_rain_core::rules::Rules;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{self, RunReplay};
use stella_rain_core::run_setup;
use stella_rain_core::star_chart;
use stella_rain_core::stardust::Stardust;
use stella_rain_core::validate::{self, Error};

use crate::common;

fn bundled() -> Vec<u8> {
    Pack::BUNDLED.to_vec()
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

/// `content/pack.json` is generated: `cargo xtask bless`.
#[test]
fn the_bundled_pack_is_the_export_of_the_individual_files() {
    let files = ContentFiles::read_dir(&crate::data_root::data_root().join("content"))
        .expect("content/ and modes/ are readable");
    assert!(
        files.export().expect("the files make a pack") == Pack::BUNDLED,
        "content/pack.json drifted from content/ and modes/; regenerate it with \
         `cargo xtask bless`"
    );
    assert_eq!(
        Pack::bundled().sha256(),
        Sha256::digest(Pack::BUNDLED).as_slice()
    );
}

#[test]
fn read_dir_leaves_out_a_file_that_is_not_there() {
    let dir = std::env::temp_dir().join(format!("read-dir-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("content")).unwrap();
    let files = ContentFiles::read_dir(&dir.join("content")).unwrap();
    for slot in ContentFiles::LAYOUT {
        assert_eq!(files.get(slot.name), None, "{}", slot.name);
    }
    assert!(matches!(files.export(), Err(PackError::Missing { .. })));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_slot_is_where_the_layout_puts_it() {
    let dir = Path::new("c/content");
    let paths: Vec<_> = ContentFiles::LAYOUT
        .iter()
        .map(|s| s.path(dir).to_string_lossy().replace('\\', "/"))
        .collect();
    assert_eq!(paths[0], "c/content/run_stats.json");
    assert_eq!(paths[16], "c/content/../modes/trial.json");
}

/// The files read out of the pack are the files in the tree: `content/` and `modes/`, byte for
/// byte, and the bundled pack has every slot, the optional ones included.
#[test]
fn the_bundled_files_are_the_files_on_disk() {
    let dir = crate::data_root::data_root().join("content");
    for slot in ContentFiles::LAYOUT {
        // The threat colour table is optional and the bundled pack holds none: no file in the
        // tree either (the shipped content names no colour).
        if slot.name == "threat_colours" {
            assert!(slot.optional);
            assert!(ContentFiles::bundled().get(slot.name).is_none());
            assert!(fs::read(slot.path(&dir)).is_err());
            continue;
        }
        let on_disk = fs::read(slot.path(&dir)).expect("the file is in the tree");
        let bundled = ContentFiles::bundled()
            .get(slot.name)
            .unwrap_or_else(|| panic!("the bundled pack lacks {}", slot.name));
        assert!(bundled == on_disk.as_slice(), "{} differs", slot.name);
        assert!(ContentFiles::bundled_file(slot.name) == on_disk.as_slice());
    }
}

#[test]
#[should_panic(expected = "the bundled pack has no file `nope`")]
fn a_name_the_layout_lacks_is_a_bug() {
    ContentFiles::bundled_file("nope");
}

#[test]
fn the_bundled_values_are_the_bundled_files_parsed() {
    assert_eq!(
        Stardust::bundled(),
        &Stardust::parse(ContentFiles::bundled_file("stardust"), Versions::CURRENT).unwrap()
    );
    assert_eq!(
        Rules::bundled(),
        &Rules::parse(ContentFiles::bundled_file("rules"), Versions::CURRENT).unwrap()
    );
}

#[test]
fn the_exported_pack_gives_the_bundled_content() {
    let pack = Pack::from_bytes(&bundled()).expect("the exported pack loads");
    assert_eq!(pack.content(), RunContent::bundled());
    let trial = validate::parse_and_validate(run_setup::mode_stage("trial").unwrap()).unwrap();
    assert_eq!(pack.trial(), &trial);
    assert_eq!(pack.schema_version(), SCHEMA_VERSION);
    assert_eq!(pack.sim_version(), SIM_VERSION);
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
        "\"perk_sets\": {",
        "\"difficulties\": {",
        "\"starting_unlocks\": {",
        "\"constellations\": {",
        "\"stardust\": {",
        "\"star_chart\": {",
        "\"modes\": {\n    \"trial\"",
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

/// Every run case that plays with the bundled files gives the same checkpoints with the
/// exported pack's content, and its replay verifies with it.
#[test]
fn a_run_from_the_pack_hashes_as_one_from_the_bundled_content() {
    let content: RunContent = Pack::from_bytes(&bundled()).unwrap().into();
    let dir = crate::data_root::data_root().join("tests/corpus/current/runs");
    let mut checked = Vec::new();
    for case in ["trial", "easy", "time_limit"] {
        let case_dir = dir.join(case);
        assert!(
            !case_dir.join("mode.json").exists(),
            "{case} has a mode file"
        );
        let replay = fs::read(case_dir.join("run.replay")).unwrap();
        assert!(RunReplay::from_bytes(&replay).is_ok());
        run_replay::verify_run_with(&replay, &content).unwrap_or_else(|e| panic!("{case}: {e}"));
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
    for file in [
        "run_stats",
        "rewards",
        "stage_skills",
        "runes",
        "perk_sets",
        "difficulties",
    ] {
        let mut v = value();
        v.as_object_mut().unwrap().remove(file);
        assert!(matches!(load(&v), Err(PackError::Parse { .. })), "{file}");
    }
}

#[test]
fn an_unknown_pack_version_is_refused() {
    for (field, n) in [
        ("schema_version", NEWEST_READ_SCHEMA_VERSION + 1),
        ("sim_version", SIM_VERSION + 1),
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

/// Each count budget, past its limit by one: refused naming the list and its limit, before
/// the file itself is read (the extra entries are copies, which the file would refuse too).
#[test]
fn each_count_budget_is_enforced() {
    let lists: [(&str, usize); 32] = [
        ("/run_stats/stats", pack::MAX_RUN_STATS),
        ("/rewards/rewards", pack::MAX_REWARDS),
        ("/rewards/rarities", rewards::MAX_RARITIES),
        ("/stage_skills/stage_skills", pack::MAX_STAGE_SKILLS),
        ("/expertise/weapons", pack::MAX_EXPERTISE_WEAPONS),
        ("/expertise/weapons/0/common", pack::MAX_EXPERTISE_COMMON),
        ("/runes/on_hit", pack::MAX_ON_HIT_RUNES),
        ("/runes/damage_over_time", pack::MAX_DAMAGE_OVER_TIME_RUNES),
        ("/perk_sets/perk_sets", pack::MAX_PERK_SETS),
        ("/difficulties/difficulties", pack::MAX_DIFFICULTIES),
        ("/attack_presets/presets", pack::MAX_ATTACK_PRESETS),
        ("/starting_unlocks/weapons", pack::MAX_STARTING_UNLOCKS),
        ("/starting_unlocks/perk_sets", pack::MAX_STARTING_UNLOCKS),
        ("/starting_unlocks/stage_skills", pack::MAX_STARTING_UNLOCKS),
        ("/starting_unlocks/runes", pack::MAX_STARTING_UNLOCKS),
        ("/star_chart/weapons", star_chart::MAX_ITEMS),
        ("/star_chart/perk_sets", star_chart::MAX_ITEMS),
        ("/star_chart/stage_skills", star_chart::MAX_ITEMS),
        ("/star_chart/stats", star_chart::MAX_ITEMS),
        ("/star_chart/stats/0/prices", star_chart::MAX_STAT_LEVELS),
        ("/modes/time_limit/waves", pack::MAX_TIME_LIMIT_WAVES),
        ("/modes/time_limit/boss_ticks", pack::MAX_BOSS_TICKS),
        ("/modes/time_limit/mid_boss_pool", pack::MAX_MID_BOSS_POOL),
        ("/modes/time_limit/elite_table", pack::MAX_ELITE_TABLE_TIERS),
        ("/introduced/run_stats", pack::MAX_RUN_STATS),
        ("/introduced/rewards", pack::MAX_REWARDS),
        ("/introduced/stage_skills", pack::MAX_STAGE_SKILLS),
        ("/introduced/perk_sets", pack::MAX_PERK_SETS),
        ("/introduced/difficulties", pack::MAX_DIFFICULTIES),
        ("/introduced/expertise", pack::MAX_EXPERTISE_WEAPONS),
        ("/introduced/expertise/0/ids", pack::MAX_EXPERTISE_IDS),
        ("/introduced/constellations", pack::MAX_CONSTELLATIONS),
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
    let cases: [(&str, Value, &str); 5] = [
        ("/run_stats/stats/0/step", json!(0), "run_stats"),
        ("/rewards/rewards/0/limit", json!(0), "rewards"),
        ("/match_ups/max_type_bonus_pct", json!(100_000), "match_ups"),
        (
            "/perk_sets/perk_sets/0/effects/0/amount",
            json!(0),
            "perk_sets",
        ),
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

/// The bundled pack with the rewards file as a file made before the equipment rules: no rules,
/// and no reward or perk names a category.
fn without_equipment_rules() -> Value {
    let mut v = value();
    let rewards = v["rewards"].as_object_mut().unwrap();
    rewards.remove("equipment");
    for r in rewards["rewards"].as_array_mut().unwrap() {
        r.as_object_mut().unwrap().remove("category");
    }
    for set in v["perk_sets"]["perk_sets"].as_array_mut().unwrap() {
        let effects = set["effects"].as_array_mut().unwrap();
        effects.retain(|e| e.get("category").is_none());
    }
    v
}

const NO_EQUIPMENT_RULES: &str = "sim_version 2 needs the rewards file's equipment rules";

fn one_issue(result: Result<Pack, PackError>, file: &str) -> validate::Issue {
    match result {
        Err(PackError::File {
            file: f,
            error: Error::Invalid(mut issues),
        }) if f == file => {
            assert_eq!(issues.len(), 1, "{issues:?}");
            issues.remove(0)
        }
        Err(e) => panic!("refused in another way: {e:?}"),
        Ok(_) => panic!("the pack loaded"),
    }
}

#[test]
fn a_sim_version_2_pack_without_equipment_rules_is_refused_with_the_pack() {
    // The fuzz target's invariant: a pack that loads starts a run of each mode, and a
    // run at sim_version 2 needs the rewards file's equipment rules.
    let v = without_equipment_rules();
    let issue = one_issue(load(&v), "rewards");
    assert_eq!(issue.path, "$.equipment");
    assert_eq!(issue.message, NO_EQUIPMENT_RULES);
}

#[test]
fn a_missing_equipment_rule_set_is_refused_once_when_a_perk_names_a_category_too() {
    let mut v = without_equipment_rules();
    let sets = v["perk_sets"]["perk_sets"].as_array_mut().unwrap();
    let effect = sets
        .iter_mut()
        .flat_map(|s| s["effects"].as_array_mut().unwrap().iter_mut())
        .next()
        .unwrap();
    effect["category"] = json!("weapon");
    let issue = one_issue(load(&v), "rewards");
    assert_eq!(issue.message, NO_EQUIPMENT_RULES);
}

#[test]
fn a_sim_version_1_pack_needs_no_equipment_rules() {
    let dir = crate::data_root::data_root().join("tests/corpus/v1/packs");
    let mut loaded = 0;
    for entry in fs::read_dir(dir).expect("the v1 packs") {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        let pack = Pack::from_bytes(&bytes).expect("a v1 pack loads");
        assert!(pack.content().rewards().equipment().is_none());
        loaded += 1;
    }
    assert!(loaded > 0);
}

/// The bundled pack with `normal` running `times` boss times, 3,000 ticks apart.
fn normal_with_boss_times(times: usize) -> (Value, usize) {
    let mut v = value();
    let ticks: Vec<u64> = (1..=times as u64).map(|t| t * 3_000).collect();
    let length = *ticks.last().unwrap();
    let list = v["difficulties"]["difficulties"].as_array_mut().unwrap();
    let at = list
        .iter()
        .position(|d| d["id"] == json!("normal"))
        .unwrap();
    list[at]["length_ticks"] = json!(length);
    list[at]["boss_ticks"] = json!(ticks);
    (v, at)
}

#[test]
fn a_difficulty_with_more_boss_times_than_the_pool_has_bosses_is_refused_with_the_pack() {
    // One mid-boss for each boss time but the last, each pool entry once: the run start refuses
    // more, so the pack does at load.
    let pool = value()["modes"]["time_limit"]["mid_boss_pool"]
        .as_array()
        .unwrap()
        .len();
    let (v, at) = normal_with_boss_times(pool + 2);
    let issue = one_issue(load(&v), "difficulties");
    assert_eq!(issue.path, format!("$.difficulties[{at}].boss_ticks"));
    assert_eq!(
        issue.message,
        "more mid-boss times than the time_limit mode's pool has bosses"
    );
    let (v, _) = normal_with_boss_times(pool + 1);
    load(&v).expect("one boss time more than the pool has mid-bosses is the last, not a mid-boss");
}

/// The bundled pack without the entry `id` of the file `file` (member `list`), and of the
/// pack's `introduced` list of the same name.
fn without_entry(file: &str, list: &str, id: &str) -> Value {
    let mut v = value();
    v[file][list]
        .as_array_mut()
        .unwrap()
        .retain(|e| e["id"] != json!(id));
    v["introduced"][file]
        .as_array_mut()
        .unwrap()
        .retain(|e| e["id"] != json!(id));
    v
}

#[test]
fn a_pack_without_the_default_perk_set_is_refused_with_the_pack() {
    // A run that names no perk set starts with `duelist` (the fuzz invariant's own setup).
    let mut v = without_entry("perk_sets", "perk_sets", run_setup::DEFAULT_PERK_SET);
    // The star chart and the starting unlocks name `duelist` too; both are optional.
    let pack = v.as_object_mut().unwrap();
    pack.remove("star_chart");
    pack.remove("starting_unlocks");
    let issue = one_issue(load(&v), "perk_sets");
    assert_eq!(issue.path, "$.perk_sets");
    assert_eq!(
        issue.message,
        "the default perk set (duelist) is missing: a run that names none starts with it"
    );
}

#[test]
fn a_pack_without_the_default_difficulty_is_refused_with_the_pack() {
    let mut v = without_entry(
        "difficulties",
        "difficulties",
        run_setup::DEFAULT_DIFFICULTY,
    );
    // `hard` opens after `normal`: repoint it so that the missing default is the only issue.
    for d in v["difficulties"]["difficulties"].as_array_mut().unwrap() {
        if d["opens_after"] == json!(run_setup::DEFAULT_DIFFICULTY) {
            d["opens_after"] = json!("easy");
        }
    }
    let issue = one_issue(load(&v), "difficulties");
    assert_eq!(issue.path, "$.difficulties");
    assert_eq!(
        issue.message,
        "the default difficulty (normal) is missing: a run that names none starts with it"
    );
}

#[test]
fn the_default_setup_starts_a_run_of_the_bundled_pack() {
    let pack = load(&value()).expect("the bundled pack loads");
    let setup = run_setup::RunSetup {
        mode: "trial".into(),
        weapon: "giant_sword".into(),
        sim_version: pack.sim_version(),
        ..run_setup::RunSetup::default()
    };
    stella_rain_core::engine::Engine::start_with(&setup, pack.content())
        .expect("a pack that loads starts a run");
}

#[test]
fn a_time_limit_boss_the_trial_stage_would_refuse_is_refused_with_the_pack() {
    // The fuzz target's invariant: a pack that loads starts a run of each mode. A boss
    // definition whose phase waits on a part it does not have is caught at load, not at start.
    let mut v = value();
    let bosses = v["modes"]["time_limit"]["bosses"].as_array_mut().unwrap();
    let core_pair = bosses
        .iter_mut()
        .find(|b| b["base"] == json!("core_pair"))
        .unwrap();
    core_pair["phases"][0]["until"]["part"] = json!("core_middle");
    match load(&v) {
        Err(PackError::File {
            file: "time_limit",
            error: Error::Invalid(issues),
        }) => assert_eq!(issues[0].path, "$.bosses[4].phases[0].until.part"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_id_newer_than_the_pack_is_refused() {
    let mut v = value();
    v["introduced"]["rewards"][0]["sim_version"] = json!(SIM_VERSION + 1);
    let Err(PackError::Introduced(issues)) = load(&v) else {
        panic!("not refused as newer");
    };
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path, "$.introduced.rewards[0].sim_version");
    assert!(
        issues[0]
            .message
            .contains(&format!("newer than the pack's {SIM_VERSION}"))
    );

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
    v["introduced"]["expertise"][0]["weapon"] = json!("giant_sword");
    assert!(matches!(load(&v), Err(PackError::Introduced(_))));
    let mut v = value();
    v["introduced"]["expertise"].as_array_mut().unwrap().pop();
    assert!(matches!(load(&v), Err(PackError::Introduced(_))));
}

#[test]
fn the_bundled_pack_keeps_each_constellation_at_the_version_that_introduced_it() {
    let v = value();
    let listed: Vec<(&str, u64)> = v["introduced"]["constellations"]
        .as_array()
        .expect("the export lists its sets")
        .iter()
        .map(|e| {
            (
                e["id"].as_str().unwrap(),
                e["sim_version"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("summer_triangle", 1),
            ("orions_belt", 1),
            ("big_dipper", 1),
            ("winter_triangle", 1)
        ]
    );
    assert_eq!(
        v["constellations"]["constellations"]
            .as_array()
            .map(Vec::len),
        Some(4)
    );
}

#[test]
fn a_pack_released_without_a_constellation_list_still_loads() {
    // Packs released with the file (core#NS3) have no `introduced.constellations`: they stay
    // valid, their sets simply have no recorded version (ADR-CONT-2).
    let mut v = value();
    v["introduced"]
        .as_object_mut()
        .unwrap()
        .remove("constellations");
    // Without the list a set is not provably released, so one holding an offer effect is
    // refused (fail closed, ADR-SVC-1); the sets without such an effect load.
    assert!(load(&v).is_err());
    for set in v["constellations"]["constellations"]
        .as_array_mut()
        .unwrap()
    {
        for tier in set["tiers"].as_array_mut().unwrap() {
            tier["effects"]
                .as_array_mut()
                .unwrap()
                .retain(|e| e["effect"] != "rerolls" && e["effect"] != "banishes");
        }
    }
    let pack = load(&v).expect("a pack without the list loads");
    assert_eq!(pack.content().constellations().sets().len(), 4);
}

#[test]
fn the_constellation_list_names_each_set_once_and_none_newer_than_the_pack() {
    let mut v = value();
    v["introduced"]["constellations"][0]["id"] = json!("no_such_set");
    assert!(matches!(load(&v), Err(PackError::Introduced(_))));
    let mut v = value();
    v["introduced"]["constellations"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    assert!(matches!(load(&v), Err(PackError::Introduced(_))));
    let mut v = value();
    let first = v["introduced"]["constellations"][0].clone();
    v["introduced"]["constellations"]
        .as_array_mut()
        .unwrap()
        .push(first);
    assert!(matches!(load(&v), Err(PackError::Introduced(_))));
    let mut v = value();
    v["introduced"]["constellations"][1]["sim_version"] = json!(SIM_VERSION + 1);
    let Err(PackError::Introduced(issues)) = load(&v) else {
        panic!("not refused as newer");
    };
    assert_eq!(issues[0].path, "$.introduced.constellations[1].sim_version");
    // A list for a pack with no file names nothing.
    let mut v = value();
    v.as_object_mut().unwrap().remove("constellations");
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
        b"{\"schema_version\":1}",
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
    let (a, b) = (pack.content(), RunContent::bundled());
    // The same files; the pack's bytes, so its hash, are not the bundled pack's.
    assert_eq!(
        a.clone().with_runes(b.runes().clone()),
        b.clone().with_runes(b.runes().clone())
    );
    assert_ne!(a.pack_sha256(), b.pack_sha256());

    // The modes first, the versions last.
    let v = value();
    let mut moved = serde_json::Map::new();
    for key in [
        "modes",
        "runes",
        "introduced",
        "run_stats",
        "rewards",
        "constellations",
    ] {
        moved.insert(key.into(), v[key].clone());
    }
    for key in [
        "perk_sets",
        "difficulties",
        "attack_presets",
        "weapons",
        "rules",
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

/// The pack a caller keeps: the bundled files written compactly, so other bytes, so another hash.
fn kept_pack() -> (Vec<u8>, Pack) {
    let kept = bytes(&value());
    let pack = Pack::from_bytes(&kept).expect("a compact pack loads");
    (kept, pack)
}

fn hex(sha: &[u8]) -> String {
    sha.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn a_run_replay_records_the_sha256_of_the_pack_s_bytes() {
    let replay =
        run_replay::record_run(&common::run::trial_setup(), &crate::run_play::play_run(60))
            .unwrap();
    assert_eq!(replay.content_sha256, hex(&Sha256::digest(Pack::BUNDLED)));
    assert_eq!(replay.content_sha256, hex(&Pack::bundled().sha256()));
    for mode in ["trial", "time_limit"] {
        let sha = run_replay::content_sha256(mode).unwrap();
        assert_eq!(sha, Sha256::digest(Pack::BUNDLED).as_slice());
    }
}

#[test]
fn a_run_replay_recorded_with_a_pack_records_that_pack_s_hash() {
    let (kept, pack) = kept_pack();
    let replay = run_replay::record_run_with_pack(
        &common::run::trial_setup(),
        &pack,
        &crate::run_play::play_run(60),
    )
    .unwrap();
    assert_eq!(replay.content_sha256, hex(&Sha256::digest(&kept)));
    let mut recorder =
        run_replay::RunRecorder::start_with_pack(&common::run::trial_setup(), &pack).unwrap();
    recorder
        .step(&[
            stella_rain_core::input::Input::default(),
            Default::default(),
            Default::default(),
        ])
        .unwrap();
    assert_eq!(
        recorder.replay().content_sha256,
        hex(&Sha256::digest(&kept))
    );
}

#[test]
fn verification_works_with_a_pack_the_caller_supplies() {
    let (_, pack) = kept_pack();
    let bytes = run_replay::record_run_with_pack(
        &common::run::trial_setup(),
        &pack,
        &crate::run_play::play_run(120),
    )
    .unwrap()
    .to_bytes();
    let verified = run_replay::verify_run_with_pack(&bytes, &pack).unwrap();
    assert_eq!(verified.ticks, 120);
    let hashes: Vec<u64> = {
        let replay = RunReplay::from_bytes(&bytes).unwrap();
        let mut engine =
            stella_rain_core::engine::Engine::start_with(&replay.setup, pack.content()).unwrap();
        replay
            .unpack_inputs(u32::MAX)
            .unwrap()
            .into_iter()
            .map(|input| {
                engine.step(&[input, Default::default(), Default::default()]);
                engine.state_hash()
            })
            .collect()
    };
    run_replay::verify_run_against_with_pack(&bytes, &pack, &hashes).unwrap();
}

#[test]
fn verification_refuses_a_replay_that_names_another_pack() {
    let (_, pack) = kept_pack();
    let setup = common::run::trial_setup();
    let play = crate::run_play::play_run(60);
    let bundled_replay = run_replay::record_run(&setup, &play).unwrap().to_bytes();
    let kept_replay = run_replay::record_run_with_pack(&setup, &pack, &play)
        .unwrap()
        .to_bytes();
    assert_eq!(
        run_replay::verify_run_with_pack(&bundled_replay, &pack),
        Err(run_replay::RunReplayError::ContentChanged)
    );
    assert_eq!(
        run_replay::verify_run_with_pack(&kept_replay, Pack::bundled()),
        Err(run_replay::RunReplayError::ContentChanged)
    );
    assert_eq!(
        run_replay::verify_run(&kept_replay),
        Err(run_replay::RunReplayError::ContentChanged)
    );
}

#[test]
fn a_new_reward_is_listed_at_the_pack_version() {
    let new_reward = |v: &mut Value, sim_version: u64| {
        v["rewards"]["rewards"]
            .as_array_mut()
            .expect("rewards")
            .push(json!({
                "id": "star_a", "rarity": "epic", "named": true, "star": "star_a", "limit": 1,
                "effects": [{ "effect": "attack_pct", "amount": 1 }]
            }));
        v["introduced"]["rewards"]
            .as_array_mut()
            .expect("introduced rewards")
            .push(json!({ "id": "star_a", "sim_version": sim_version }));
    };
    let mut v = value();
    new_reward(&mut v, u64::from(SIM_VERSION));
    load(&v).expect("a new ID recorded at the pack's version loads");

    let mut v = value();
    new_reward(&mut v, u64::from(SIM_VERSION) + 1);
    let Err(PackError::Introduced(issues)) = load(&v) else {
        panic!("not refused as newer");
    };
    assert_eq!(issues.len(), 1);
    assert!(
        issues[0]
            .message
            .contains(&format!("newer than the pack's {SIM_VERSION}"))
    );
}

const STYLES: [&str; 6] = [
    "titan_trigger",
    "momentum",
    "breaker",
    "spirit_blade",
    "foresight_counter",
    "judgement_cut",
];

#[test]
fn the_bundled_pack_introduces_every_id_in_version_1() {
    let file: pack::PackFile = serde_json::from_slice(Pack::BUNDLED).unwrap();
    assert_eq!(
        (file.schema_version, file.sim_version),
        (SCHEMA_VERSION, SIM_VERSION)
    );
    let i = &file.introduced;
    let expertise: Vec<_> = i.expertise.iter().flat_map(|w| w.ids.clone()).collect();
    let constellations = i.constellations.clone().unwrap_or_default();
    for (name, list) in [
        ("run_stats", &i.run_stats),
        ("rewards", &i.rewards),
        ("stage_skills", &i.stage_skills),
        ("perk_sets", &i.perk_sets),
        ("difficulties", &i.difficulties),
        ("expertise", &expertise),
        ("elite_types", &i.elite_types),
        ("constellations", &constellations),
    ] {
        assert!(!list.is_empty(), "{name}");
        for e in list {
            // The six equipment-only Rune Haste stars and the 24 IDs of the six styles (a style
            // and its upgrade 1, upgrade 2 and capstone) are the pack's own sim_version 2; the
            // old placeholder style IDs are gone from the table.
            let style = STYLES.iter().any(|s| {
                let id = e.id.as_str();
                id == *s
                    || id
                        .strip_prefix(s)
                        .is_some_and(|r| ["_up_1", "_up_2", "_cap"].contains(&r))
            });
            let want = if e.id.as_str().starts_with("rune_haste_star_") || style {
                2
            } else {
                1
            };
            if name == "expertise" {
                for old in ["crush", "quake", "bulwark", "edge", "tempo", "reach"] {
                    assert!(!e.id.as_str().starts_with(old), "{}", e.id);
                }
            }
            assert_eq!(e.sim_version, want, "{name} {}", e.id);
        }
    }
}

#[test]
fn a_pack_at_version_0_is_refused() {
    // No migration from version 0.
    for (schema_version, sim_version) in [(0, 0), (1, 0), (0, 1)] {
        let mut v = value();
        v["schema_version"] = json!(schema_version);
        v["sim_version"] = json!(sim_version);
        assert_eq!(
            load(&v).err(),
            Some(PackError::UnsupportedVersion {
                schema_version,
                sim_version
            }),
            "{schema_version} {sim_version}"
        );
    }
}

#[test]
fn pack_preset_arguments_outside_bounds_are_refused() {
    let mut v = value();
    let set_arg = |pack_val: &mut Value, n: i32| {
        pack_val["weapons"]["weapons"][0]["trees"][0]["weapon"]["combo"]["main"]["attack"] =
            json!({ "preset": { "id": "aimed_single", "args": [n] } });
    };

    set_arg(&mut v, 15);
    assert!(load(&v).is_ok(), "aimed_single with [15] should load");

    set_arg(&mut v, 0);
    assert!(load(&v).is_ok(), "aimed_single with [0] should load");
    set_arg(&mut v, 600);
    assert!(load(&v).is_ok(), "aimed_single with [600] should load");

    set_arg(&mut v, -1);
    match load(&v) {
        Err(PackError::File {
            file,
            error: Error::Invalid(issues),
        }) => {
            assert_eq!(file, "weapons");
            assert_eq!(issues.len(), 1, "expected 1 issue for N = -1");
            assert!(
                issues[0].path.ends_with(".attack.preset.args[0]"),
                "path was {}",
                issues[0].path
            );
            assert_eq!(
                issues[0].message,
                "preset aimed_single argument 0 (wind_up_ticks) is -1, below its minimum 0"
            );
        }
        other => panic!("expected File Invalid error for N = -1, got {other:?}"),
    }

    set_arg(&mut v, 601);
    match load(&v) {
        Err(PackError::File {
            file,
            error: Error::Invalid(issues),
        }) => {
            assert_eq!(file, "weapons");
            assert_eq!(issues.len(), 1, "expected 1 issue for N = 601");
            assert!(
                issues[0].path.ends_with(".attack.preset.args[0]"),
                "path was {}",
                issues[0].path
            );
            assert_eq!(
                issues[0].message,
                "preset aimed_single argument 0 (wind_up_ticks) is 601, above its maximum 600"
            );
        }
        other => panic!("expected File Invalid error for N = 601, got {other:?}"),
    }
}

/// The offer effects are deprecated: a pack of `sim_version` 2 refuses them in content that is
/// new at 2 and keeps them in released content (ADR-CONT-2, ADR-REPL-2, ADR-SVC-1).
mod offer_effects {
    use super::*;

    fn new_star(v: &mut Value, id: &str, effects: Value) {
        v["rewards"]["rewards"]
            .as_array_mut()
            .expect("rewards")
            .push(json!({
                "id": id, "rarity": "epic", "named": true, "star": id, "limit": 1,
                "effects": effects
            }));
        v["introduced"]["rewards"]
            .as_array_mut()
            .expect("introduced rewards")
            .push(json!({ "id": id, "sim_version": SIM_VERSION }));
    }

    /// The paths of the issues the pack is refused with in `file`.
    fn refused(v: &Value, file: &str) -> Vec<String> {
        match load(v) {
            Err(PackError::File {
                file: f,
                error: Error::Invalid(issues),
            }) if f == file => issues.into_iter().map(|i| i.path).collect(),
            other => panic!("not refused in {file}: {:?}", other.map(|_| ())),
        }
    }

    fn index(v: &Value, file: &str, id: &str) -> usize {
        v[file][file]
            .as_array()
            .unwrap()
            .iter()
            .position(|e| e["id"] == id)
            .unwrap()
    }

    #[test]
    fn a_new_reward_with_an_offer_effect_is_refused() {
        for effect in ["offer_candidates", "rerolls", "banishes"] {
            let mut v = value();
            new_star(
                &mut v,
                "new_offer",
                json!([{ "effect": effect, "amount": 1 }]),
            );
            let i = index(&v, "rewards", "new_offer");
            assert_eq!(
                refused(&v, "rewards"),
                [format!("$.rewards[{i}].effects[0].effect")],
                "{effect}"
            );
        }
        let mut v = value();
        new_star(
            &mut v,
            "new_offer",
            json!([{ "effect": "attack_pct", "amount": 1 }, { "effect": "rerolls", "amount": 1 }]),
        );
        let i = index(&v, "rewards", "new_offer");
        assert_eq!(
            refused(&v, "rewards"),
            [format!("$.rewards[{i}].effects[1].effect")]
        );
    }

    #[test]
    fn a_new_perk_set_with_an_offer_effect_is_refused() {
        for effect in ["extra_rerolls", "extra_banishes"] {
            let mut v = value();
            v["perk_sets"]["perk_sets"]
                .as_array_mut()
                .unwrap()
                .push(json!({ "id": "new_perk", "effects": [{ "effect": effect, "amount": 1 }] }));
            v["introduced"]["perk_sets"]
                .as_array_mut()
                .unwrap()
                .push(json!({ "id": "new_perk", "sim_version": SIM_VERSION }));
            let i = index(&v, "perk_sets", "new_perk");
            assert_eq!(
                refused(&v, "perk_sets"),
                [format!("$.perk_sets[{i}].effects[0].effect")],
                "{effect}"
            );
            v["perk_sets"]["perk_sets"][i]["effects"] =
                json!([{ "effect": "max_hp", "amount": 1 }]);
            load(&v).expect("the same set without the offer effect loads");
        }
    }

    fn with_pair(effect: &str, listed: bool) -> Value {
        let mut v = value();
        for id in ["star_a", "star_b"] {
            new_star(&mut v, id, json!([{ "effect": "attack_pct", "amount": 1 }]));
        }
        v["constellations"] = json!({ "constellations": [{
            "id": "pair", "name": "The Pair", "members": ["star_a", "star_b"],
            "tiers": [{ "threshold": 2, "effects": [{ "effect": effect, "amount": 1 }] }]
        }]});
        let introduced = v["introduced"].as_object_mut().unwrap();
        if listed {
            introduced.insert(
                "constellations".into(),
                json!([{ "id": "pair", "sim_version": SIM_VERSION }]),
            );
        } else {
            introduced.remove("constellations");
        }
        v
    }

    #[test]
    fn a_new_constellation_with_an_offer_effect_is_refused() {
        for listed in [true, false] {
            for effect in ["rerolls", "banishes"] {
                let v = with_pair(effect, listed);
                assert_eq!(
                    refused(&v, "constellations"),
                    ["$.constellations[0].tiers[0].effects[0].effect"],
                    "{effect}, listed {listed}"
                );
            }
            load(&with_pair("attack_pct", listed)).expect("without an offer effect it loads");
        }
    }

    #[test]
    fn released_content_and_frozen_packs_keep_their_offer_effects() {
        let pack = Pack::bundled();
        let rewards = pack.content().rewards();
        let polaris = rewards
            .rewards()
            .iter()
            .find(|r| r.id.as_str() == "star_polaris");
        assert!(polaris.is_some());
        let v = value();
        for (list, id) in [
            ("rewards", "star_polaris"),
            ("perk_sets", "scholar"),
            ("constellations", "big_dipper"),
        ] {
            let found = v["introduced"][list]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == id)
                .unwrap_or_else(|| panic!("{id} is listed"));
            assert_eq!(found["sim_version"], 1, "{id}");
        }
        let text = String::from_utf8(Pack::BUNDLED.to_vec()).unwrap();
        for effect in ["\"rerolls\"", "\"extra_rerolls\"", "\"banishes\""] {
            assert!(text.contains(effect), "{effect} stays in released content");
        }

        let dir = crate::data_root::data_root().join("tests/corpus/v1/packs");
        let mut seen = 0;
        let mut with_rerolls = 0;
        for entry in fs::read_dir(dir).unwrap() {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            Pack::from_bytes(&bytes).expect("a frozen pack loads");
            seen += 1;
            if String::from_utf8_lossy(&bytes).contains("star_polaris")
                && String::from_utf8_lossy(&bytes).contains("\"rerolls\"")
            {
                with_rerolls += 1;
            }
        }
        assert!(seen > 0 && with_rerolls > 0);
    }

    #[test]
    fn star_polaris_is_deprecated_and_the_only_new_flag() {
        let all = rewards::Rewards::bundled();
        let polaris = all
            .rewards()
            .iter()
            .position(|r| r.id.as_str() == "star_polaris")
            .unwrap();
        assert!(all.rewards()[polaris].deprecated);
        let index = u16::try_from(polaris).unwrap();
        assert!(!all.default_pool(2).contains(&index));
        assert!(all.default_pool(1).contains(&index));
        assert_eq!(all.rewards().iter().filter(|r| r.deprecated).count(), 7);
    }
}

#[test]
fn the_layout_has_the_catalogue_after_stardust() {
    assert_eq!(ContentFiles::LAYOUT.len(), 19);
    let names: Vec<&str> = ContentFiles::LAYOUT.iter().map(|s| s.name).collect();
    let at = |n: &str| names.iter().position(|x| *x == n).unwrap();
    assert_eq!(at("star_chart"), at("stardust") + 1);
}

/// A hold: an inline loop of one active phase and an inline release.
fn inline_hold() -> Value {
    let active = json!({ "kind": "active", "ticks": 4, "type": "strike",
        "hit_box": { "type": "circle", "offset": { "forward": 0, "right": 0 }, "radius": 8192 } });
    json!({ "hold": { "loop": { "inline": [active.clone()] },
        "release": { "inline": [active] }, "max_ticks": 60 } })
}

fn inline_charge() -> Value {
    let slash = json!({ "preset": { "id": "slash" } });
    json!({ "charge": { "levels": [{ "at_ticks": 20, "attack": slash }], "max_ticks": 60 } })
}

fn grant_kind(step: Value, grant: Value) -> Value {
    json!({ "type": "replace_step", "step": step, "grant": grant })
}

/// The pack with `kinds` on the Hammer's first style (the style level's own kinds).
fn pack_with_hammer_kinds(kinds: Value) -> Value {
    let mut v = value();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_hammer")
        .expect("the Hammer's expertise");
    w["styles"][0]["kinds"] = kinds;
    w["styles"][0]["upgrade_1"][0]["effect"] = json!("attack_pct");
    v
}

fn refused_in_expertise(v: &Value) -> Vec<String> {
    match load(v) {
        Err(PackError::File {
            file: "expertise",
            error: Error::Invalid(issues),
        }) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("expected an expertise refusal, got {:?}", other.map(|_| ())),
    }
}

/// T3: a style's grants are checked against the weapon's base tree, in the pack.
#[test]
fn a_style_grant_is_checked_against_the_base_tree() {
    // A charge on AAA replaces the Hammer's hold there; a hold on the first A is the Hammer's
    // inline attack, which ends in a spawn-free recovery.
    let aaa = json!(["main", "main", "main"]);
    assert!(
        load(&pack_with_hammer_kinds(json!([grant_kind(
            aaa,
            inline_charge()
        )])))
        .is_ok()
    );
    let a = json!(["main"]);
    assert!(
        load(&pack_with_hammer_kinds(json!([grant_kind(
            a,
            inline_hold()
        )])))
        .is_ok()
    );
    // Not a step of the tree.
    let far = json!(vec!["smash"; 7]);
    let paths = refused_in_expertise(&pack_with_hammer_kinds(json!([grant_kind(
        far,
        inline_charge()
    )])));
    assert!(
        paths
            .iter()
            .any(|p| p == "$.weapons[0].styles[0].kinds[0].step"),
        "{paths:?}"
    );
    // A step that is none of the tree's: depth 3 of smash from neutral.
    let none = json!(["smash", "main"]);
    let paths = refused_in_expertise(&pack_with_hammer_kinds(json!([grant_kind(
        none,
        inline_charge()
    )])));
    assert!(
        paths
            .iter()
            .any(|p| p == "$.weapons[0].styles[0].kinds[0].step"),
        "{paths:?}"
    );
}

/// T3: a hold on a step whose attack does not end in a spawn-free recovery is refused.
#[test]
fn a_granted_hold_needs_a_step_that_ends_in_a_spawn_free_recovery() {
    let mut v = pack_with_hammer_kinds(json!([grant_kind(json!(["main"]), inline_hold())]));
    assert_eq!(v["weapons"]["weapons"][0]["id"], "giant_hammer");
    v["weapons"]["weapons"][0]["trees"][0]["weapon"]["combo"]["main"]["attack"] = json!({
        "inline": [
            { "kind": "wind_up", "ticks": 6 },
            { "kind": "active", "ticks": 4, "type": "strike",
              "hit_box": { "type": "circle", "offset": { "forward": 0, "right": 0 }, "radius": 8192 } }
        ]
    });
    let paths = refused_in_expertise(&v);
    assert!(
        paths
            .iter()
            .any(|p| p.starts_with("$.weapons[0].styles[0].kinds[0]")),
        "{paths:?}"
    );
}

/// T3: a charge whose level attack names an unregistered preset is refused; a weapon with no
/// tree in the weapons file is skipped, not an error.
#[test]
fn a_grant_with_an_unknown_preset_is_refused() {
    let bad = json!({ "charge": { "levels": [{ "at_ticks": 20,
        "attack": { "preset": { "id": "no_such_preset" } } }], "max_ticks": 60 } });
    let paths = refused_in_expertise(&pack_with_hammer_kinds(json!([grant_kind(
        json!(["main", "main", "main"]),
        bad
    )])));
    assert!(
        paths
            .iter()
            .any(|p| p.starts_with("$.weapons[0].styles[0].kinds[0]")),
        "{paths:?}"
    );
}
