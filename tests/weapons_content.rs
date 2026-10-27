//! The weapon trees are content (WP1): `content/weapons.json` holds what `presets` held as Rust
//! constants, and a pack carries the file.

use crate::common;
use serde_json::{Value, json};
use stella_rain_core::pack::{Pack, PackError};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay;
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;
use stella_rain_core::weapons::{MAX_TREES, MAX_WEAPONS, Weapons};

/// Every tree of every weapon as `presets::weapon_tree` gave it before the move
/// (`{weapon: {tree: definition}}`).
const BEFORE: &str = include_str!("fixtures/weapons_before.json");

#[test]
fn the_bundled_weapons_file_holds_the_hammer_and_sword_trees_as_before() {
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/content/weapons.json"))
        .expect("content/weapons.json");
    let file: Value = serde_json::from_slice(&bytes).unwrap();
    let before: Value = serde_json::from_str(BEFORE).unwrap();
    let mut now = serde_json::Map::new();
    for weapon in file["weapons"].as_array().expect("a weapons list") {
        let mut trees = serde_json::Map::new();
        for tree in weapon["trees"].as_array().expect("a trees list") {
            trees.insert(
                tree["id"].as_str().unwrap().to_string(),
                tree["weapon"].clone(),
            );
        }
        now.insert(weapon["id"].as_str().unwrap().to_string(), trees.into());
    }
    assert_eq!(Value::Object(now), before);
}

#[test]
fn a_pack_without_weapons_is_refused() {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    assert!(v.as_object_mut().unwrap().remove("weapons").is_some());
    let bytes = serde_json::to_vec(&v).unwrap();
    assert!(Pack::from_bytes(&bytes).is_err());
}

fn bundled_file() -> Value {
    serde_json::from_slice(
        &std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/content/weapons.json")).unwrap(),
    )
    .unwrap()
}

fn refused(v: &Value) -> Vec<String> {
    match Weapons::parse(v.to_string().as_bytes(), Versions::CURRENT) {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not refused as Invalid: {:?}", other.map(|_| ())),
    }
}

#[test]
fn the_bundled_file_is_what_the_bundled_pack_holds() {
    let w = Weapons::parse(common::bundled_file("weapons"), Versions::CURRENT).unwrap();
    assert_eq!(&w, Weapons::bundled());
    let ids: Vec<_> = w.ids().map(|i| i.0.as_str()).collect();
    assert_eq!(ids, ["giant_hammer", "giant_sword"]);
}

#[test]
fn the_counts_are_bounded_and_ids_are_unique_and_registered() {
    let mut v = bundled_file();
    v["weapons"][1]["id"] = json!("giant_hammer");
    assert_eq!(refused(&v), ["$.weapons[1].id"]);
    let mut v = bundled_file();
    v["weapons"][0]["id"] = json!("no_such_weapon");
    assert_eq!(refused(&v), ["$.weapons[0].id"]);
    let mut v = bundled_file();
    v["weapons"][0]["trees"] = json!([]);
    assert_eq!(refused(&v), ["$.weapons[0].trees"]);
    let mut v = bundled_file();
    let tree = v["weapons"][0]["trees"][0].clone();
    v["weapons"][0]["trees"] = json!(vec![tree; MAX_TREES + 1]);
    assert!(refused(&v).contains(&"$.weapons[0].trees".to_string()));
    let mut v = bundled_file();
    v["weapons"] = json!([]);
    assert_eq!(refused(&v), ["$.weapons"]);
    let mut v = bundled_file();
    let entry = v["weapons"][0].clone();
    v["weapons"] = json!(vec![entry; MAX_WEAPONS + 1]);
    assert!(refused(&v).contains(&"$.weapons".to_string()));
}

#[test]
fn a_tree_is_checked_as_a_stage_s_inline_weapon_is() {
    let mut v = bundled_file();
    v["weapons"][0]["trees"][0]["weapon"]["combo"]["main"]["attack"] =
        json!({"preset": {"id": "no_such_preset"}});
    let paths = refused(&v);
    assert_eq!(paths.len(), 1, "{paths:?}");
    assert!(
        paths[0].starts_with("$.weapons[0].trees[0].weapon.combo"),
        "{paths:?}"
    );
    let mut v = bundled_file();
    v["weapons"][0]["trees"][0]["id"] = json!("Not A Tree");
    assert_eq!(refused(&v), ["$.weapons[0].trees[0].id"]);
}

#[test]
fn a_pack_with_more_weapons_than_the_budget_is_refused_before_its_files_are_read() {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let entry = v["weapons"]["weapons"][0].clone();
    v["weapons"]["weapons"] = json!(vec![entry; MAX_WEAPONS + 1]);
    match Pack::from_bytes(&serde_json::to_vec(&v).unwrap()) {
        Err(PackError::Budget { path, .. }) => assert_eq!(path, "$.weapons.weapons"),
        other => panic!("{:?}", other.map(|_| ())),
    }
}

#[test]
fn a_run_made_with_other_weapons_does_not_verify_against_this_pack() {
    let a = RunContent::bundled().clone();
    let mut v = bundled_file();
    v["weapons"][0]["trees"][0]["weapon"]["window_ticks"] = json!(21);
    let b = a
        .clone()
        .with_weapons(Weapons::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap());
    assert_ne!(
        run_replay::content_sha256_with("trial", &a),
        run_replay::content_sha256_with("trial", &b)
    );
}
