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
    r#""shot":{"preset":{"id":"twin_shot","args":[]}},"#,
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
        "6aca6a164e8b2efc1632b5f48caadf6967a033e904bad51bba427cdd552e4cf0"
    );
}

#[test]
fn every_corpus_stage_keeps_its_hash() {
    // Recorded from `core`: these cover the nested types the small stage above does not have
    // (agents, rules, attacks, boss parts and phases).
    for (bytes, expected) in [
        (
            &include_bytes!("corpus/v0/agents/stage.json")[..],
            "7d75d53dc13aa3cf8d887bafb5ff740a5ffcac53eb8b9df7cab64b1f889bb242",
        ),
        (
            &include_bytes!("corpus/v0/boss/stage.json")[..],
            "6e7c9cc8812d557468a66497cfac11122f51f3ca5ad587bde4e14638df0f577e",
        ),
        (
            &include_bytes!("corpus/v0/bullets/stage.json")[..],
            "bc9cf994278771b80284a8d5189caf62cdec8f90018ec385f10fc0d2d7556af6",
        ),
        (
            &include_bytes!("corpus/v0/fail/stage.json")[..],
            "0085a2a43d6a3cdae570e7e25e3de817725ba720b5c5f715dcae117f1c69d8e5",
        ),
        (
            &include_bytes!("corpus/v0/timeout/stage.json")[..],
            "868fd7548c7c5eed7b24949d32488a406ef20be8cc0ef7b45a23e66a670721f2",
        ),
    ] {
        assert_eq!(hash_of(bytes), expected);
    }
}

#[test]
fn formatting_and_key_order_do_not_change_the_hash() {
    let reformatted = br#"{"title":"Test Stage",
        "boss":{"phases":[{}],"spawn":{"y":25600,"x":46080},"radius":8192,"hp":20,"base":"golem"},
        "player":{"shot":{"preset":{"args":[],"id":"twin_shot"}},"atk":10,"hp":3,"base":"pilot_a",
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
