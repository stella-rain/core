//! The weapon trees are content (WP1): `content/weapons.json` holds what `presets` held as Rust
//! constants, and a pack carries the file.

use serde_json::Value;
use stella_rain_core::pack::{self, Pack};

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
    let mut v: Value = serde_json::from_slice(&pack::export_bundled()).unwrap();
    assert!(v.as_object_mut().unwrap().remove("weapons").is_some());
    let bytes = serde_json::to_vec(&v).unwrap();
    assert!(Pack::from_bytes(&bytes).is_err());
}
