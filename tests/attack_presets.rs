//! The attack presets are content (CS1b-1): `content/attack_presets.json` holds what `presets`
//! held as Rust constants, and a pack carries the file.

use serde_json::Value;
use stella_rain_core::pack::Pack;
use stella_rain_core::registry::{self, Kind};

/// Every registry preset's phases as `presets::definition` gave them before the move
/// (`{id: phases}`).
const BEFORE: &str = include_str!("fixtures/attack_presets_before.json");

fn registry_presets() -> Vec<(&'static str, u8)> {
    registry::ENTRIES
        .iter()
        .filter(|e| e.kind == Kind::Preset)
        .map(|e| (e.id, e.args))
        .collect()
}

#[test]
fn the_pack_lists_every_registry_preset_with_today_s_phases() {
    let pack: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let list = pack["attack_presets"]["presets"]
        .as_array()
        .expect("the pack has an attack_presets member with a presets list");
    let before: Value = serde_json::from_str(BEFORE).unwrap();
    let registered = registry_presets();
    assert_eq!(registered.len(), 9);
    let ids: Vec<&str> = list.iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        registered.iter().map(|(id, _)| *id).collect::<Vec<_>>()
    );
    for (preset, (id, arity)) in list.iter().zip(&registered) {
        assert_eq!(preset["phases"], before[*id], "{id}");
        assert_eq!(
            preset["args"].as_array().unwrap().len(),
            usize::from(*arity),
            "{id}"
        );
    }
    let on_disk = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/content/attack_presets.json"
    ))
    .expect("content/attack_presets.json");
    let file: Value = serde_json::from_slice(&on_disk).unwrap();
    assert_eq!(file, pack["attack_presets"]);
}

/// Removed with the Rust definitions.
#[test]
fn the_fixture_is_today_s_presets() {
    let before: Value = serde_json::from_str(BEFORE).unwrap();
    for (id, _) in registry_presets() {
        let now = serde_json::to_value(stella_rain_core::presets::definition(id).unwrap()).unwrap();
        assert_eq!(now, before[id], "{id}");
    }
}
