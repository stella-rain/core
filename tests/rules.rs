//! The pack's `rules.json` (DIR1, DS103): the crowd target and the cap's headroom, the first
//! numbers of the simulation that live in the pack. The cap is derived from them and never
//! stored; the file is a required slot of the pack, and the derived content hash covers it.

use crate::common;
use serde_json::{Value, json};
use stella_rain_core::pack::{ContentFiles, Pack, PackError};
use stella_rain_core::rules::{MAX_RULES_BYTES, Rules};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::content_sha256_with;
use stella_rain_core::validate::{Error, limits};
use stella_rain_core::version::Versions;

fn file(crowd_target: u64, cap_headroom_pct: u64) -> Vec<u8> {
    json!({ "crowd_target": crowd_target, "cap_headroom_pct": cap_headroom_pct })
        .to_string()
        .into_bytes()
}

fn paths(result: Result<Rules, Error>) -> Vec<String> {
    match result {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not refused as invalid: {other:?}"),
    }
}

#[test]
fn the_bundled_file_gives_a_target_of_200_and_a_cap_of_260() {
    let rules = Rules::bundled();
    assert_eq!(rules.crowd_target(), 200);
    assert_eq!(rules.cap_headroom_pct(), 30);
    assert_eq!(rules.enemy_cap_for(200), 260);
    assert_eq!(
        Rules::parse(common::bundled_file("rules"), Versions::CURRENT).unwrap(),
        *rules
    );
}

#[test]
fn the_limits_are_the_engine_ceiling_and_the_most_any_target_can_be() {
    assert_eq!(limits::MAX_ENEMY_CAP, 500);
    assert_eq!(limits::MAX_CROWD_TARGET, 400);
}

#[test]
fn the_headroom_is_25_to_30_percent() {
    for bad in [24, 31] {
        assert_eq!(
            paths(Rules::parse(&file(200, bad), Versions::CURRENT)),
            ["$.cap_headroom_pct"]
        );
    }
    for good in [25, 30] {
        assert!(Rules::parse(&file(200, good), Versions::CURRENT).is_ok());
    }
}

#[test]
fn the_target_keeps_its_cap_within_the_ceiling() {
    for bad in [0, 385] {
        assert_eq!(
            paths(Rules::parse(&file(bad, 30), Versions::CURRENT)),
            ["$.crowd_target"]
        );
    }
    // 384 * 130 / 100 = 499.
    assert_eq!(
        Rules::parse(&file(384, 30), Versions::CURRENT)
            .unwrap()
            .enemy_cap_for(384),
        499
    );
    // 400 * 125 / 100 = 500.
    assert_eq!(
        Rules::parse(&file(400, 25), Versions::CURRENT)
            .unwrap()
            .enemy_cap_for(400),
        500
    );
    assert_eq!(
        paths(Rules::parse(&file(401, 25), Versions::CURRENT)),
        ["$.crowd_target"]
    );
}

#[test]
fn an_override_is_clamped_so_the_cap_stays_within_the_ceiling() {
    let rules = Rules::bundled();
    assert_eq!(rules.enemy_cap_for(100), 130);
    assert_eq!(rules.enemy_cap_for(384), 499);
    assert_eq!(rules.enemy_cap_for(400), 499);
    assert_eq!(rules.enemy_cap_for(u16::MAX), 499);
    let tight = Rules::parse(&file(200, 25), Versions::CURRENT).unwrap();
    assert_eq!(tight.enemy_cap_for(400), 500);
}

#[test]
fn unknown_fields_other_versions_and_big_files_are_refused() {
    let mut v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    v["extra"] = 1.into();
    assert!(matches!(
        Rules::parse(v.to_string().as_bytes(), Versions::CURRENT),
        Err(Error::Parse { .. })
    ));
    let v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    assert!(matches!(
        Rules::parse(
            v.to_string().as_bytes(),
            Versions {
                schema_version: 0,
                sim_version: 1
            }
        ),
        Err(Error::UnsupportedVersion { .. })
    ));
    let mut v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    v["sim_version"] = 0.into();
    assert!(matches!(
        Rules::parse(v.to_string().as_bytes(), Versions::CURRENT),
        Err(Error::Parse { .. })
    ));
    let mut big = common::bundled_file("rules").to_vec();
    big.resize(MAX_RULES_BYTES + 1, b' ');
    assert!(matches!(
        Rules::parse(&big, Versions::CURRENT),
        Err(Error::TooLarge { .. })
    ));
}

#[test]
fn the_pack_has_seventeen_slots_and_rules_is_required() {
    assert_eq!(ContentFiles::LAYOUT.len(), 17);
    let slot = ContentFiles::LAYOUT
        .iter()
        .find(|s| s.name == "rules")
        .expect("a rules slot");
    assert!(!slot.optional && !slot.in_modes);
}

#[test]
fn a_pack_without_rules_is_refused() {
    // Exporting names the slot.
    let bundled = ContentFiles::bundled();
    let mut files = ContentFiles::default();
    for slot in ContentFiles::LAYOUT {
        if slot.name != "rules" {
            files.insert(slot.name, bundled.get(slot.name).unwrap().to_vec());
        }
    }
    assert!(matches!(
        files.export(),
        Err(PackError::Missing { file: "rules" })
    ));
    // Reading a pack that lacks it fails closed.
    let mut pack: Value = serde_json::from_slice(&bundled.export().unwrap()).unwrap();
    pack.as_object_mut().unwrap().remove("rules");
    assert!(Pack::from_bytes(pack.to_string().as_bytes()).is_err());
}

#[test]
fn the_bundled_pack_holds_the_bundled_file_and_round_trips() {
    let bytes = ContentFiles::bundled().export().unwrap();
    let pack = Pack::from_bytes(&bytes).unwrap();
    assert_eq!(pack.content().rules(), Rules::bundled());
    let again = ContentFiles::from_pack(&bytes).unwrap();
    assert_eq!(again.get("rules").unwrap(), common::bundled_file("rules"));
    assert_eq!(again.export().unwrap(), bytes);
}

#[test]
fn replacing_the_rules_file_changes_the_derived_content_hash() {
    let base = RunContent::bundled().clone();
    let changed = base
        .clone()
        .with_rules(Rules::parse(&file(150, 30), Versions::CURRENT).unwrap());
    assert_eq!(changed.rules().crowd_target(), 150);
    for mode in ["trial", "time_limit", "tutorial"] {
        assert_ne!(
            content_sha256_with(mode, &base.clone().with_rules(Rules::bundled().clone())),
            content_sha256_with(mode, &changed),
            "{mode}"
        );
    }
}
