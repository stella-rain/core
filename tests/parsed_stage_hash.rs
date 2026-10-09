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
    r#""shot":{"preset":{"id":"twin_shot","args":[]}},"skills":[]},"companions":[],"waves":[],"#,
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
        "79af423b38d2d3f18bd79d2c8cca8d3b60721ca65238cf1ef03821e39989e570"
    );
}

#[test]
fn every_corpus_stage_keeps_its_hash() {
    // Recorded from `core`: these cover the nested types the small stage above does not have
    // (agents, rules, attacks, boss parts and phases).
    for (bytes, expected) in [
        (
            &include_bytes!("corpus/v0/agents/stage.json")[..],
            "458975f6a021cc1d5a2e80b4c8b9d1b5880339a0b2ac7c25aab93cd93315c3ee",
        ),
        (
            &include_bytes!("corpus/v0/boss/stage.json")[..],
            "c33e5b41f3ecbe40ec0ade426124e90c5a1a6ea46e1cddaee5a56ddca8b89372",
        ),
        (
            &include_bytes!("corpus/v0/bullets/stage.json")[..],
            "3f138809887203c9d8815aa7da5ea3318a05e4124eb0e4f2924fe19a7187e716",
        ),
        (
            &include_bytes!("corpus/v0/fail/stage.json")[..],
            "9b92d230ea04d1125c7965dd284eb38bca8c0a1cf944f9fe83d297df449854f7",
        ),
        (
            &include_bytes!("corpus/v0/timeout/stage.json")[..],
            "151560a57ac41ab2255b6eeb71035a9ec032e0b5684c24408164d55387e95121",
        ),
    ] {
        assert_eq!(hash_of(bytes), expected);
    }
}

#[test]
fn formatting_and_key_order_do_not_change_the_hash() {
    let reformatted = br#"{"title":"Test Stage",
        "boss":{"phases":[{}],"spawn":{"y":25600,"x":46080},"radius":8192,"hp":20,"base":"golem"},
        "player":{"shot":{"preset":{"args":[],"id":"twin_shot"}},"atk":10,"hp":3,"base":"pilot_a"},
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
