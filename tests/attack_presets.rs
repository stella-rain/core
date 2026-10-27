//! The attack presets are content (CS1b-1): `content/attack_presets.json` holds what `presets`
//! held as Rust constants, and a pack carries the file.

use crate::common;
use serde_json::{Value, json};
use stella_rain_core::attack_presets::{
    AttackPresets, MAX_ATTACK_PRESETS, MAX_ATTACK_PRESETS_BYTES,
};
use stella_rain_core::pack::{Pack, PackError};
use stella_rain_core::registry::{self, Kind};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay;
use stella_rain_core::validate::{Error, limits::MAX_ATTACK_PHASES};
use stella_rain_core::version::Versions;

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

fn bundled_file() -> Value {
    serde_json::from_slice(common::bundled_file("attack_presets")).unwrap()
}

fn parse(v: &Value) -> Result<AttackPresets, Error> {
    AttackPresets::parse(v.to_string().as_bytes(), Versions::CURRENT)
}

fn refused(v: &Value) -> Vec<String> {
    match parse(v) {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not refused as Invalid: {:?}", other.map(|_| ())),
    }
}

fn index_of(v: &Value, id: &str) -> usize {
    v["presets"]
        .as_array()
        .unwrap()
        .iter()
        .position(|p| p["id"] == id)
        .unwrap()
}

#[test]
fn the_bundled_file_is_what_the_bundled_pack_holds() {
    let p =
        AttackPresets::parse(common::bundled_file("attack_presets"), Versions::CURRENT).unwrap();
    assert_eq!(&p, AttackPresets::bundled());
    let ids: Vec<_> = p.ids().map(|i| i.0.as_str()).collect();
    assert_eq!(
        ids,
        registry_presets()
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>()
    );
    assert!(p.definition("slam").is_some());
    assert!(p.definition("no_such_preset").is_none());
}

#[test]
fn the_ids_arguments_and_phases_are_checked() {
    let mut v = bundled_file();
    v["presets"][0]["id"] = json!("no_such_preset");
    // The preset it replaced is then missing from the file too.
    assert_eq!(refused(&v), ["$.presets[0].id", "$.presets"]);

    let mut v = bundled_file();
    v["presets"][1]["id"] = v["presets"][0]["id"].clone();
    assert!(refused(&v).contains(&"$.presets[1].id".to_string()));

    // `aimed_single` takes one argument.
    let mut v = bundled_file();
    v["presets"][0]["args"] = json!([]);
    assert!(refused(&v).contains(&"$.presets[0].args".to_string()));

    let mut v = bundled_file();
    v["presets"][0]["args"] = json!([{ "name": "Bad" }]);
    assert_eq!(refused(&v), ["$.presets[0].args[0].name"]);

    // `spread_5` reading an argument it does not take.
    let at = index_of(&bundled_file(), "spread_5");
    let mut v = bundled_file();
    v["presets"][at]["phases"][0]["spawn"][0]["body"][0]["wind_up_ticks"] = json!({ "arg": 0 });
    let paths = refused(&v);
    assert!(
        paths
            .iter()
            .all(|p| p.starts_with(&format!("$.presets[{at}].phases"))),
        "{paths:?}"
    );

    // Too many phases.
    let mut v = bundled_file();
    let phase = v["presets"][at]["phases"][0].clone();
    v["presets"][at]["phases"] = json!(vec![phase; MAX_ATTACK_PHASES + 1]);
    assert!(
        refused(&v)
            .iter()
            .any(|p| p.starts_with(&format!("$.presets[{at}].phases")))
    );
}

#[test]
fn every_registered_preset_must_be_present_and_the_count_is_bounded() {
    let mut v = bundled_file();
    let at = index_of(&v, "slam");
    v["presets"].as_array_mut().unwrap().remove(at);
    assert_eq!(refused(&v), ["$.presets"]);

    let mut v = bundled_file();
    v["presets"] = json!([]);
    assert!(refused(&v).contains(&"$.presets".to_string()));

    let mut v = bundled_file();
    let entry = v["presets"][0].clone();
    v["presets"] = json!(vec![entry; MAX_ATTACK_PRESETS + 1]);
    assert!(refused(&v).contains(&"$.presets".to_string()));
}

#[test]
fn an_unknown_field_a_version_field_and_an_oversized_file_are_refused() {
    let mut v = bundled_file();
    v["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let mut v = bundled_file();
    v["presets"][0]["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let mut v = bundled_file();
    v["schema_version"] = json!(0);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let big = vec![b' '; MAX_ATTACK_PRESETS_BYTES + 1];
    assert!(matches!(
        AttackPresets::parse(&big, Versions::CURRENT),
        Err(Error::TooLarge { .. })
    ));
}

#[test]
fn no_message_repeats_the_file_s_text() {
    let mut v = bundled_file();
    v["presets"][0]["id"] = json!("zz_unregistered_marker");
    v["presets"][1]["args"] = json!([{ "name": "Zz Marker" }]);
    match parse(&v) {
        Err(Error::Invalid(issues)) => {
            for i in issues {
                assert!(!i.message.contains("marker") && !i.message.contains("Marker"));
                assert!(!i.path.contains("marker"));
            }
        }
        other => panic!("{:?}", other.map(|_| ())),
    }
}

#[test]
fn a_pack_without_the_member_or_with_a_bad_one_is_refused() {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    assert!(
        v.as_object_mut()
            .unwrap()
            .remove("attack_presets")
            .is_some()
    );
    assert!(matches!(
        Pack::from_bytes(&serde_json::to_vec(&v).unwrap()),
        Err(PackError::Parse { .. })
    ));

    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    v["attack_presets"]["presets"][0]["id"] = json!("no_such_preset");
    match Pack::from_bytes(&serde_json::to_vec(&v).unwrap()) {
        Err(e @ PackError::File { .. }) => {
            assert!(matches!(
                &e,
                PackError::File {
                    file: "attack_presets",
                    error: Error::Invalid(_)
                }
            ));
            assert!(
                e.to_string().contains("$.attack_presets.presets[0].id"),
                "{e}"
            );
        }
        other => panic!("{:?}", other.map(|_| ())),
    }
}

#[test]
fn a_run_made_with_other_presets_does_not_verify_against_this_pack() {
    let a = RunContent::bundled().clone();
    let mut v = bundled_file();
    let at = index_of(&v, "slam");
    v["presets"][at]["phases"][0]["ticks"] = json!(24);
    let b = a.clone().with_attack_presets(parse(&v).unwrap());
    assert_ne!(
        run_replay::content_sha256_with("trial", &a),
        run_replay::content_sha256_with("trial", &b)
    );
    assert_ne!(a.pack_sha256(), None);
    assert_eq!(b.pack_sha256(), None);
    // And differently from another file replaced alone.
    let c = a.clone().with_runes(a.runes().clone());
    assert_ne!(
        run_replay::content_sha256_with("trial", &b),
        run_replay::content_sha256_with("trial", &c)
    );
}

#[test]
fn a_pack_whose_weapon_holds_on_a_preset_of_the_wrong_shape_is_refused() {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    v["weapons"]["weapons"][0]["trees"][0]["weapon"]["combo"]["main"]["hold"] = json!({
        "loop": { "preset": { "id": "thrust" } },
        "release": { "preset": { "id": "slash" } },
        "max_ticks": 60
    });
    match Pack::from_bytes(&serde_json::to_vec(&v).unwrap()) {
        Err(e @ PackError::File { .. }) => {
            assert!(matches!(
                &e,
                PackError::File {
                    file: "weapons",
                    ..
                }
            ));
            assert!(
                e.to_string()
                    .contains("$.weapons.weapons[0].trees[0].weapon.combo.main.hold.loop"),
                "{e}"
            );
        }
        other => panic!("{:?}", other.map(|_| ())),
    }
}
