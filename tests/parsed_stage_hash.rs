//! The parsed-stage hash (ADR-046): SHA-256 of a domain tag followed by `core`'s compact JSON
//! of the validated stage, fields in declared order, the stage `id` left out. The moderation
//! blocklist stores it, so the bytes and the vectors below are frozen from the freeze of
//! ADR-035: a test that fails here after the freeze means every stored hash has changed.

use serde_json::{Value, json};
use stella_rain_core::content_hash::to_hex;
use stella_rain_core::parsed_stage_hash::{parsed_stage_hash, parsed_stage_hash_input};
use stella_rain_core::validate::parse_and_validate;

const CLEAR: &[u8] = include_bytes!("corpus/v0/clear/stage.json");

/// What is hashed for `corpus/v0/clear/stage.json`, written out by hand.
const CLEAR_INPUT: &str = concat!(
    "stella-rain/parsed-stage/v1\n",
    r#"{"schema_version":0,"sim_version":0,"title":"Test Stage","description":"","seed":7,"#,
    r#""length_ticks":600,"player":{"base":"pilot_a","hp":3,"atk":10,"#,
    r#""main":{"preset":{"id":"blaster"}},"#,
    r#""sub":{"inline":{"#,
    r#""auto":{"attack":{"inline":[{"kind":"recovery","ticks":1}]},"targeting":"facing","#,
    r#""cooldown_ticks":1},"#,
    r#""skill":{"attack":{"inline":[{"kind":"recovery","ticks":1}]},"targeting":"facing","#,
    r#""cooldown_ticks":1}}},"#,
    r#""evade":{"distance":12288,"ticks":8,"invulnerable_ticks":10,"cooldown_ticks":40}},"#,
    r#""companions":[],"waves":[],"#,
    r#""boss":{"base":"golem","hp":20,"radius":8192,"spawn":{"x":46080,"y":25600},"parts":[],"#,
    r#""phases":[{"transition":[],"timeline":[],"rules":[]}]}}"#,
);

fn hash_of(bytes: &[u8]) -> String {
    to_hex(&parsed_stage_hash(&parse_and_validate(bytes).unwrap()))
}

fn edited(edit: impl FnOnce(&mut Value)) -> String {
    let mut stage: Value = serde_json::from_slice(CLEAR).unwrap();
    edit(&mut stage);
    hash_of(stage.to_string().as_bytes())
}

#[test]
fn the_hashed_bytes_are_the_tag_and_compact_json_in_declared_order_without_the_id() {
    let stage = parse_and_validate(CLEAR).unwrap();
    assert_eq!(
        String::from_utf8(parsed_stage_hash_input(&stage)).unwrap(),
        CLEAR_INPUT
    );
}

#[test]
fn matches_the_adr_046_vector() {
    // SHA-256 of `CLEAR_INPUT`, computed with `sha256sum`, not with `core`.
    assert_eq!(
        hash_of(CLEAR),
        "d9d67c41ae9e864dd13f192b11f15964e0196489ffc1dcfb61b4bbfead8e5c91"
    );
}

#[test]
fn every_corpus_stage_keeps_its_hash() {
    // Recorded from `core`: these cover the nested types the small stage above does not have
    // (agents, rules, attacks, boss parts and phases).
    for (bytes, expected) in [
        (
            &include_bytes!("corpus/v0/agents/stage.json")[..],
            "c5e3e04276ba999225c8647babe91891fb78204ba30fa94eddd6a64290819a4b",
        ),
        (
            &include_bytes!("corpus/v0/boss/stage.json")[..],
            "d5a83c726b81c2e4af33fcdb2cc1b8dd28697f5f732b34872d8deb879610778b",
        ),
        (
            &include_bytes!("corpus/v0/bullets/stage.json")[..],
            "8f8d383d2a4c8027159504e613ae92e5eb64bfed3cfc73a25ab9f042ff6bc1dd",
        ),
        (
            &include_bytes!("corpus/v0/fail/stage.json")[..],
            "e3ac5fbe6592a9f05040db531b514d7cde53b99a40289210fd410f98a2dd1d74",
        ),
        (
            &include_bytes!("corpus/v0/timeout/stage.json")[..],
            "6332d4564546f1dfdb23daeb3bf0a203c89619e1e7a509814435e56c81253544",
        ),
    ] {
        assert_eq!(hash_of(bytes), expected);
    }
}

#[test]
fn formatting_and_key_order_do_not_change_the_hash() {
    let reformatted = br#"{"title":"Test Stage",
        "boss":{"phases":[{}],"spawn":{"y":25600,"x":46080},"radius":8192,"hp":20,"base":"golem"},
        "player":{"sub":{"inline":{
                    "skill":{"cooldown_ticks":1,"targeting":"facing",
                             "attack":{"inline":[{"ticks":1,"kind":"recovery"}]}},
                    "auto":{"cooldown_ticks":1,"targeting":"facing",
                            "attack":{"inline":[{"ticks":1,"kind":"recovery"}]}}}},
                  "main":{"preset":{"id":"blaster"}},"atk":10,"hp":3,"base":"pilot_a",
                  "evade":{"cooldown_ticks":40,"invulnerable_ticks":10,"ticks":8,"distance":12288}},
        "length_ticks":600,   "seed":7,
        "id":"test-stage","sim_version":0,"schema_version":0}
"#;
    assert_ne!(&reformatted[..], CLEAR);
    assert_eq!(hash_of(reformatted), hash_of(CLEAR));
}

#[test]
fn a_default_written_out_does_not_change_the_hash() {
    let base = hash_of(CLEAR);
    assert_eq!(base, edited(|s| s["description"] = json!("")));
    assert_eq!(base, edited(|s| s["waves"] = json!([])));
    assert_eq!(base, edited(|s| s["boss"]["parts"] = json!([])));
}

#[test]
fn another_id_does_not_change_the_hash() {
    assert_eq!(hash_of(CLEAR), edited(|s| s["id"] = json!("a-copy")));
}

#[test]
fn any_other_change_of_content_changes_the_hash() {
    let base = hash_of(CLEAR);
    assert_ne!(base, edited(|s| s["title"] = json!("Test Stage 2")));
    assert_ne!(base, edited(|s| s["description"] = json!("x")));
    assert_ne!(base, edited(|s| s["seed"] = json!(8)));
    assert_ne!(base, edited(|s| s["length_ticks"] = json!(601)));
    assert_ne!(base, edited(|s| s["player"]["hp"] = json!(4)));
    assert_ne!(base, edited(|s| s["boss"]["spawn"]["x"] = json!(46081)));
}
