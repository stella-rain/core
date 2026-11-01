//! The parsed-stage hash (ADR-046): SHA-256 of a domain tag followed by `core`'s compact JSON
//! of the validated stage, fields in declared order, the stage `id` left out. The moderation
//! blocklist stores it, so the bytes and the vectors below are frozen from the freeze of
//! ADR-035: a test that fails here after the freeze means every stored hash has changed.

use serde_json::{Value, json};
use stella_rain_core::content_hash::to_hex;
use stella_rain_core::parsed_stage_hash::{parsed_stage_hash, parsed_stage_hash_input};
use stella_rain_core::validate::parse_and_validate;

const CLEAR: &[u8] = include_bytes!("corpus/v1/stages/clear/stage.json");

/// What is hashed for `corpus/v1/stages/clear/stage.json`, written out by hand.
const CLEAR_INPUT: &str = concat!(
    "stella-rain/parsed-stage/v1\n",
    r#"{"schema_version":1,"sim_version":1,"title":"Test Stage","description":"","seed":7,"#,
    r#""length_ticks":600,"player":{"base":"pilot_a","hp":300,"atk":1000,"#,
    r#""weapon":{"preset":{"id":"giant_sword"}},"#,
    r#""evade":{"distance":12288,"ticks":8,"invulnerable_ticks":10,"cooldown_ticks":40}},"#,
    r#""companions":[],"waves":[],"#,
    r#""boss":{"base":"golem","hp":2000,"radius":8192,"spawn":{"x":46080,"y":121856},"parts":[],"#,
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
        "0abb5cdb2a35706fb0c8d95727685e559eb33ecdf513324678a6d973983674bc"
    );
}

#[test]
fn every_corpus_stage_keeps_its_hash() {
    // Recorded from `core`: these cover the nested types the small stage above does not have
    // (agents, rules, attacks, boss parts and phases).
    for (bytes, expected) in [
        (
            &include_bytes!("corpus/v1/stages/agents/stage.json")[..],
            "b7327105943407c1e23da365e9ff2987de08d14d9fddb11fccd048c4e919a92a",
        ),
        (
            &include_bytes!("corpus/v1/stages/boss/stage.json")[..],
            "c8cdfe291b329605bf0d507d16701f654f521441dcc1c86263dd6d507ca08cf7",
        ),
        (
            &include_bytes!("corpus/v1/stages/fail/stage.json")[..],
            "937a10822b9294e0487ade13057a9cd3d76c2c6203dc9f17f9a1e6e00397ba5f",
        ),
        (
            &include_bytes!("corpus/v1/stages/timeout/stage.json")[..],
            "b02533a271d5ce3abc6564a47c49bfa9c39942c709a1c4adaeffe8279cf12dc6",
        ),
    ] {
        assert_eq!(hash_of(bytes), expected);
    }
}

#[test]
fn formatting_and_key_order_do_not_change_the_hash() {
    let reformatted = br#"{"title":"Test Stage",
        "boss":{"phases":[{}],"spawn":{"y":121856,"x":46080},"radius":8192,"hp":2000,"base":"golem"},
        "player":{"weapon":{"preset":{"id":"giant_sword"}},"atk":1000,"hp":300,"base":"pilot_a",
                  "evade":{"cooldown_ticks":40,"invulnerable_ticks":10,"ticks":8,"distance":12288}},
        "length_ticks":600,   "seed":7,
        "id":"test-stage","sim_version":1,"schema_version":1}
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
