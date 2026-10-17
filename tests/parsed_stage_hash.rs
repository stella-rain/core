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
    r#""main":{"preset":{"id":"blade"}},"#,
    r#""sub":{"inline":{"bonus":{"type":"pull","distance":0,"ticks":1}}},"#,
    r#""evade":{"distance":12288,"ticks":8,"invulnerable_ticks":10,"cooldown_ticks":40}},"#,
    r#""companions":[],"waves":[],"#,
    r#""boss":{"base":"golem","hp":20,"radius":8192,"spawn":{"x":46080,"y":121856},"parts":[],"#,
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
        "347b5f4555fa473aa502173358ce5c525dd2fd1a359799e579750a1406d58f08"
    );
}

#[test]
fn every_corpus_stage_keeps_its_hash() {
    // Recorded from `core`: these cover the nested types the small stage above does not have
    // (agents, rules, attacks, boss parts and phases).
    for (bytes, expected) in [
        (
            &include_bytes!("corpus/v0/agents/stage.json")[..],
            "284f4745f05ad5115463e4f404f683968fde671a5235ee9905c3158ea3e09139",
        ),
        (
            &include_bytes!("corpus/v0/boss/stage.json")[..],
            "96b97cd380953f7f9e3ac7707b48e2a012163f72cdb57b4fedf6dcd70c8c70a5",
        ),
        (
            &include_bytes!("corpus/v0/bullets/stage.json")[..],
            "72462c899852572fcd2a33399a9e5a83ffa02a6a951bec64f435c3ab4f0bef1a",
        ),
        (
            &include_bytes!("corpus/v0/fail/stage.json")[..],
            "eb64183001705d029ee31578f6a341b86f7af5b16d70f155ad322d79cc3fe955",
        ),
        (
            &include_bytes!("corpus/v0/timeout/stage.json")[..],
            "743587186a8eb63fc9530e17891fff93985196a2819be42bc7c40ff8850c1107",
        ),
    ] {
        assert_eq!(hash_of(bytes), expected);
    }
}

#[test]
fn formatting_and_key_order_do_not_change_the_hash() {
    let reformatted = br#"{"title":"Test Stage",
        "boss":{"phases":[{}],"spawn":{"y":121856,"x":46080},"radius":8192,"hp":20,"base":"golem"},
        "player":{"sub":{"inline":{"bonus":{"ticks":1,"distance":0,"type":"pull"}}},
                  "main":{"preset":{"id":"blade"}},"atk":10,"hp":3,"base":"pilot_a",
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
