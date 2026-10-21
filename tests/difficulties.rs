//! The difficulties file (ADR-110): each difficulty a content ID with the percents its enemies
//! are scaled by, the range of rarities its drops may have, its Stardust time factor and,
//! optionally, its own length, boss times and level table. Read from JSON as untrusted bytes
//! (ADR-020). The bundled file is the default; other bytes run the same way and change the run's
//! content hash.

use serde_json::{Value, json};
use stella_rain_core::difficulties::{
    DEFAULT_DIFFICULTY, Difficulties, MAX_DIFFICULTIES, MAX_DIFFICULTIES_BYTES, MAX_PCT, scale_pct,
};
use stella_rain_core::rewards::{Rarity, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{content_sha256, content_sha256_with};
use stella_rain_core::stage::LevelTable;
use stella_rain_core::time_limit::{MAX_BOSS_TIMES, MAX_LENGTH_TICKS};
use stella_rain_core::validate::Error;

fn difficulty(id: &str) -> Value {
    json!({
        "id": id, "title": "Level", "description": "A level",
        "hp_pct": 100, "damage_pct": 100,
        "rarity_min": "common", "rarity_max": "legendary",
        "stardust_start_pct": 100, "stardust_end_pct": 150,
    })
}

fn file(list: Vec<Value>) -> Value {
    json!({ "schema_version": 0, "sim_version": 0, "difficulties": list })
}

fn one(change: impl FnOnce(&mut Value)) -> Value {
    let mut d = difficulty("normal");
    change(&mut d);
    file(vec![d])
}

fn parse(v: &Value) -> Result<Difficulties, Error> {
    Difficulties::parse(v.to_string().as_bytes())
}

fn paths(e: Error) -> Vec<String> {
    match e {
        Error::Invalid(issues) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not a validation error: {other:?}"),
    }
}

fn bundled(id: &str) -> &'static stella_rain_core::difficulties::Difficulty {
    Difficulties::bundled()
        .get(&id.into())
        .expect("a bundled difficulty")
}

#[test]
fn the_bundled_file_holds_the_four_difficulties_with_kades_numbers() {
    let ids: Vec<&str> = Difficulties::bundled()
        .list()
        .iter()
        .map(|d| d.id.0.as_str())
        .collect();
    assert_eq!(ids, ["easy", "normal", "hard", "rainy"]);
    let numbers = |id: &str| {
        let d = bundled(id);
        (
            d.hp_pct,
            d.damage_pct,
            d.rarity_min,
            d.rarity_max,
            d.stardust_start_pct,
            d.stardust_end_pct,
        )
    };
    use Rarity::*;
    assert_eq!(numbers("easy"), (70, 75, Common, Rare, 100, 150));
    assert_eq!(numbers("normal"), (100, 100, Common, Legendary, 100, 150));
    assert_eq!(numbers("hard"), (150, 125, Common, Supernova, 100, 150));
    assert_eq!(numbers("rainy"), (200, 150, Uncommon, Supernova, 200, 400));
    assert_eq!(DEFAULT_DIFFICULTY, "normal");
}

#[test]
fn only_easy_overrides_the_mode() {
    let easy = bundled("easy");
    assert_eq!(easy.length_ticks, Some(10 * 60 * 60));
    assert_eq!(easy.boss_ticks.as_deref(), Some(&[18_000, 36_000][..]));
    assert_eq!(easy.level_table, Some(LevelTable::SHORT));
    for id in ["normal", "hard", "rainy"] {
        let d = bundled(id);
        assert_eq!(
            (d.length_ticks, &d.boss_ticks, d.level_table),
            (None, &None, None)
        );
    }
}

#[test]
fn scaling_truncates_toward_zero_and_saturates() {
    assert_eq!(scale_pct(1_001, 70), 700);
    assert_eq!(scale_pct(7, 100), 7);
    assert_eq!(scale_pct(u32::MAX, MAX_PCT), u32::MAX);
}

#[test]
fn the_rarity_range_is_clamped_to_the_rarities_rewards_have() {
    // Rewards have every rarity today (tests/rarities.rs), so each range is the file's own.
    let rewards = Rewards::bundled();
    use Rarity::*;
    assert_eq!(bundled("easy").rarity_range(rewards), (Common, Rare));
    assert_eq!(bundled("normal").rarity_range(rewards), (Common, Legendary));
    assert_eq!(bundled("hard").rarity_range(rewards), (Common, Supernova));
    assert_eq!(
        bundled("rainy").rarity_range(rewards),
        (Uncommon, Supernova)
    );
}

#[test]
fn a_changed_file_changes_the_content_hash() {
    let mut v: Value = serde_json::from_slice(Difficulties::BUNDLED).unwrap();
    v["difficulties"][2]["hp_pct"] = json!(151);
    let bytes = v.to_string();
    let other = Difficulties::parse(bytes.as_bytes()).unwrap();
    assert_ne!(other.sha256(), Difficulties::bundled().sha256());
    let content = RunContent::bundled().clone().with_difficulties(other);
    assert_ne!(
        content_sha256_with("time_limit", &content),
        content_sha256("time_limit")
    );
}

#[test]
fn a_file_over_the_size_limit_is_refused_before_parsing() {
    let big = vec![b' '; MAX_DIFFICULTIES_BYTES + 1];
    assert!(matches!(
        Difficulties::parse(&big),
        Err(Error::TooLarge { .. })
    ));
}

#[test]
fn versions_and_unknown_fields_are_refused() {
    let mut v = one(|_| {});
    v["sim_version"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::UnsupportedVersion { .. })));
    let v = one(|d| d["extra"] = json!(1));
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
}

#[test]
fn the_number_of_difficulties_is_bounded() {
    assert_eq!(paths(parse(&file(vec![])).unwrap_err()), ["$.difficulties"]);
    let many = |n: usize| file((0..n).map(|i| difficulty(&format!("d{i}"))).collect());
    assert!(parse(&many(MAX_DIFFICULTIES)).is_ok());
    assert_eq!(
        paths(parse(&many(MAX_DIFFICULTIES + 1)).unwrap_err()),
        ["$.difficulties"]
    );
}

#[test]
fn ids_are_well_formed_and_unique() {
    let v = file(vec![difficulty("Bad Id")]);
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.difficulties[0].id"]);
    let v = file(vec![difficulty("a"), difficulty("a")]);
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.difficulties[1].id"]);
}

#[test]
fn percents_are_bounded() {
    for field in [
        "hp_pct",
        "damage_pct",
        "stardust_start_pct",
        "stardust_end_pct",
    ] {
        let at_most = one(|d| {
            d[field] = json!(MAX_PCT);
            d["stardust_end_pct"] = json!(MAX_PCT);
        });
        assert!(parse(&at_most).is_ok(), "{field}");
        for bad in [0, MAX_PCT + 1] {
            let v = one(|d| d[field] = json!(bad));
            assert_eq!(
                paths(parse(&v).unwrap_err()),
                [format!("$.difficulties[0].{field}")],
                "{field} {bad}"
            );
        }
    }
}

#[test]
fn the_stardust_factor_does_not_fall_over_the_run() {
    let v = one(|d| {
        d["stardust_start_pct"] = json!(200);
        d["stardust_end_pct"] = json!(199);
    });
    assert_eq!(
        paths(parse(&v).unwrap_err()),
        ["$.difficulties[0].stardust_end_pct"]
    );
}

#[test]
fn the_rarity_range_does_not_run_backwards() {
    let v = one(|d| {
        d["rarity_min"] = json!("epic");
        d["rarity_max"] = json!("rare");
    });
    assert_eq!(
        paths(parse(&v).unwrap_err()),
        ["$.difficulties[0].rarity_max"]
    );
    let v = one(|d| d["rarity_min"] = json!("mythic"));
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let v = one(|d| {
        d["rarity_min"] = json!("rare");
        d["rarity_max"] = json!("rare");
    });
    assert!(parse(&v).is_ok());
}

#[test]
fn a_length_comes_with_its_boss_times() {
    let v = one(|d| d["length_ticks"] = json!(600));
    assert_eq!(
        paths(parse(&v).unwrap_err()),
        ["$.difficulties[0].boss_ticks"]
    );
    let v = one(|d| d["boss_ticks"] = json!([600]));
    assert_eq!(
        paths(parse(&v).unwrap_err()),
        ["$.difficulties[0].length_ticks"]
    );
    let v = one(|d| {
        d["length_ticks"] = json!(600);
        d["boss_ticks"] = json!([300, 600]);
    });
    assert!(parse(&v).is_ok());
}

#[test]
fn boss_times_obey_the_mode_files_rules() {
    let with = |length: u32, times: Value| {
        one(|d| {
            d["length_ticks"] = json!(length);
            d["boss_ticks"] = times;
        })
    };
    let bad = |v: Value| paths(parse(&v).unwrap_err());
    assert_eq!(
        bad(with(0, json!([0]))),
        [
            "$.difficulties[0].length_ticks",
            "$.difficulties[0].boss_ticks[0]"
        ]
    );
    assert_eq!(
        bad(with(MAX_LENGTH_TICKS + 1, json!([MAX_LENGTH_TICKS + 1]))),
        ["$.difficulties[0].length_ticks"]
    );
    assert_eq!(bad(with(600, json!([]))), ["$.difficulties[0].boss_ticks"]);
    let many: Vec<u32> = (1..=MAX_BOSS_TIMES as u32 + 1).collect();
    assert_eq!(bad(with(9, json!(many))), ["$.difficulties[0].boss_ticks"]);
    assert_eq!(
        bad(with(600, json!([0, 600]))),
        ["$.difficulties[0].boss_ticks[0]"]
    );
    assert_eq!(
        bad(with(600, json!([300, 300, 600]))),
        ["$.difficulties[0].boss_ticks[1]"]
    );
    assert_eq!(
        bad(with(600, json!([300, 601]))),
        ["$.difficulties[0].boss_ticks[1]"]
    );
    assert_eq!(
        bad(with(600, json!([300, 500]))),
        ["$.difficulties[0].boss_ticks[1]"]
    );
    let many: Vec<u32> = (1..=MAX_BOSS_TIMES as u32).collect();
    assert!(parse(&with(MAX_BOSS_TIMES as u32, json!(many))).is_ok());
}

#[test]
fn the_level_table_obeys_the_stages_rules() {
    let v = one(|d| d["level_table"] = json!({ "style": 4, "upgrades": [4, 6], "capstone": 8 }));
    assert_eq!(
        paths(parse(&v).unwrap_err()),
        ["$.difficulties[0].level_table"]
    );
    let v = one(|d| d["level_table"] = json!({ "style": 2, "upgrades": [4, 6], "capstone": 11 }));
    assert_eq!(
        paths(parse(&v).unwrap_err()),
        ["$.difficulties[0].level_table"]
    );
    let v = one(|d| d["level_table"] = json!({ "style": 2, "upgrades": [4, 6], "capstone": 8 }));
    assert!(parse(&v).is_ok());
}

#[test]
fn text_is_bounded() {
    let v = one(|d| d["title"] = json!(""));
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.difficulties[0].title"]);
    let v = one(|d| d["description"] = json!("x".repeat(500)));
    assert_eq!(
        paths(parse(&v).unwrap_err()),
        ["$.difficulties[0].description"]
    );
}
