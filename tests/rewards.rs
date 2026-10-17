//! Rewards (ADR-066 §3, ADR-067): what an offer holds is content, read from JSON as untrusted
//! bytes (ADR-020): a size limit before parsing, strict parsing, the current versions only and
//! every field bounded. The bundled file is the default; other bytes run the same way and
//! change the run's content hash.

use serde_json::{Value, json};
use stella_rain_core::id::ContentId;
use stella_rain_core::rewards::{
    MAX_AMOUNT, MAX_REWARDS, MAX_REWARDS_BYTES, MAX_STACK, Rarity, Rewards,
};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{
    RunRecorder, RunReplayError, content_sha256, content_sha256_with, verify_run, verify_run_with,
};
use stella_rain_core::run_stats::{MAX_DESCRIPTION_CHARS, MAX_TITLE_CHARS, RunStats, StatEffect};
use stella_rain_core::validate::Error;

mod common;

use common::run::{run_script, trial_setup};

fn reward(id: &str) -> Value {
    json!({
        "id": id,
        "rarity": "rare",
        "effect": "attack_pct",
        "amount": 10,
        "limit": 3,
        "title": "Might",
        "description": "Hit harder"
    })
}

fn file(rewards: Vec<Value>) -> Value {
    json!({ "schema_version": 0, "sim_version": 0, "rewards": rewards })
}

fn parse(v: &Value) -> Result<Rewards, Error> {
    Rewards::parse(v.to_string().as_bytes())
}

/// One valid reward with `field` set to `value`.
fn one_with(field: &str, value: Value) -> Value {
    let mut r = reward("might");
    r[field] = value;
    file(vec![r])
}

fn invalid_at(v: &Value, path: &str) {
    match parse(v) {
        Err(Error::Invalid(issues)) => {
            assert!(issues.iter().any(|i| i.path == path), "{path}: {issues:?}")
        }
        other => panic!("{path}: expected invalid, got {other:?}"),
    }
}

#[test]
fn the_bundled_rewards_hold_every_effect_at_three_rarities() {
    let rewards = Rewards::bundled().rewards();
    for effect in StatEffect::ALL {
        let rarities: Vec<Rarity> = rewards
            .iter()
            .filter(|r| r.effect == effect)
            .map(|r| r.rarity)
            .collect();
        assert_eq!(
            rarities,
            [Rarity::Common, Rarity::Rare, Rarity::Legendary],
            "{effect:?}"
        );
    }
    assert!(rewards.iter().all(|r| !r.title.is_empty()));
    assert_eq!(
        Rewards::parse(Rewards::BUNDLED).as_ref(),
        Ok(Rewards::bundled())
    );
}

#[test]
fn a_valid_file_parses_in_its_order() {
    let rewards = parse(&file(vec![reward("b"), reward("a")])).unwrap();
    let ids: Vec<&ContentId> = rewards.rewards().iter().map(|r| &r.id).collect();
    assert_eq!(ids, [&ContentId("b".into()), &ContentId("a".into())]);
    assert_eq!(rewards.index_of(&ContentId("a".into())), Some(1));
    assert_eq!(rewards.index_of(&ContentId("c".into())), None);
}

#[test]
fn a_file_over_the_limit_is_not_parsed() {
    let big = vec![b' '; MAX_REWARDS_BYTES + 1];
    assert_eq!(
        Rewards::parse(&big),
        Err(Error::TooLarge {
            size: MAX_REWARDS_BYTES + 1,
            limit: MAX_REWARDS_BYTES
        })
    );
    let mut padded = file(vec![reward("might")]).to_string().into_bytes();
    padded.resize(MAX_REWARDS_BYTES, b' ');
    assert!(Rewards::parse(&padded).is_ok(), "at the limit it parses");
}

#[test]
fn only_the_rewards_format_parses() {
    for bad in [
        json!("not an object"),
        json!({ "schema_version": 0, "sim_version": 0 }),
        {
            let mut v = file(vec![reward("might")]);
            v["extra"] = json!(1);
            v
        },
        one_with("extra", json!(1)),
        one_with("effect", json!("no_such_effect")),
        one_with("rarity", json!("mythic")),
        one_with("amount", json!(-1)),
        one_with("limit", json!(256)),
    ] {
        assert!(
            matches!(parse(&bad), Err(Error::Parse { .. })),
            "{bad}: {:?}",
            parse(&bad)
        );
    }
}

#[test]
fn a_newer_file_is_unsupported() {
    let mut v = file(vec![reward("might")]);
    v["sim_version"] = json!(1);
    v["rewards"][0]["extra"] = json!(1);
    assert_eq!(
        parse(&v),
        Err(Error::UnsupportedVersion {
            schema_version: 0,
            sim_version: 1
        })
    );
}

#[test]
fn one_to_max_rewards() {
    invalid_at(&file(vec![]), "$.rewards");
    let most: Vec<Value> = (0..MAX_REWARDS).map(|i| reward(&format!("r{i}"))).collect();
    assert!(parse(&file(most.clone())).is_ok());
    let mut too_many = most;
    too_many.push(reward("one_more"));
    invalid_at(&file(too_many), "$.rewards");
}

#[test]
fn ids_are_well_formed_and_unique() {
    invalid_at(&file(vec![reward("Not An ID")]), "$.rewards[0].id");
    invalid_at(
        &file(vec![reward("might"), reward("might")]),
        "$.rewards[1].id",
    );
}

#[test]
fn amounts_limits_and_texts_are_bounded() {
    for (field, lo, hi) in [
        ("amount", 1, i64::from(MAX_AMOUNT)),
        ("limit", 1, i64::from(MAX_STACK)),
    ] {
        let path = format!("$.rewards[0].{field}");
        assert!(parse(&one_with(field, json!(lo))).is_ok(), "{field} {lo}");
        assert!(parse(&one_with(field, json!(hi))).is_ok(), "{field} {hi}");
        invalid_at(&one_with(field, json!(lo - 1)), &path);
        invalid_at(&one_with(field, json!(hi + 1)), &path);
    }
    invalid_at(&one_with("title", json!("")), "$.rewards[0].title");
    let long = |n: usize| json!("x".repeat(n));
    assert!(parse(&one_with("title", long(MAX_TITLE_CHARS))).is_ok());
    invalid_at(
        &one_with("title", long(MAX_TITLE_CHARS + 1)),
        "$.rewards[0].title",
    );
    assert!(parse(&one_with("description", long(MAX_DESCRIPTION_CHARS))).is_ok());
    invalid_at(
        &one_with("description", long(MAX_DESCRIPTION_CHARS + 1)),
        "$.rewards[0].description",
    );
    invalid_at(
        &one_with("description", json!("a\u{7}b")),
        "$.rewards[0].description",
    );
}

fn stronger() -> Rewards {
    let mut v: Value = serde_json::from_slice(Rewards::BUNDLED).unwrap();
    v["rewards"][0]["amount"] = json!(9);
    Rewards::parse(v.to_string().as_bytes()).unwrap()
}

#[test]
fn the_content_hash_covers_the_rewards() {
    let other = RunContent::new(RunStats::bundled().clone(), stronger());
    assert_eq!(
        content_sha256_with("trial", RunContent::bundled()),
        content_sha256("trial")
    );
    assert_ne!(
        content_sha256_with("trial", &other),
        content_sha256("trial")
    );
}

#[test]
fn a_run_with_other_rewards_verifies_only_with_those_rewards() {
    let content = RunContent::new(RunStats::bundled().clone(), stronger());
    let mut recorder = RunRecorder::start_with(&trial_setup(), &content).unwrap();
    for t in 0..50 {
        recorder.step(run_script(t)).unwrap();
    }
    let bytes = recorder.replay().to_bytes();
    assert!(verify_run_with(&bytes, &content).is_ok());
    assert_eq!(verify_run(&bytes), Err(RunReplayError::ContentChanged));
}
