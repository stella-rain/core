//! Rewards (ADR-066 §3, ADR-067): what an offer holds is content, read from JSON as untrusted
//! bytes (ADR-020): a size limit before parsing, strict parsing, the current versions only and
//! every field bounded. The bundled file is the default; other bytes run the same way and
//! change the run's content hash.

use serde_json::{Value, json};
use stella_rain_core::id::ContentId;
use stella_rain_core::rewards::{
    MAX_AMOUNT, MAX_REWARDS, MAX_REWARDS_BYTES, MAX_STACK, Rarity, RewardEffect, Rewards,
};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{
    RunRecorder, RunReplayError, content_sha256, content_sha256_with, verify_run, verify_run_with,
};
use stella_rain_core::run_stats::{MAX_DESCRIPTION_CHARS, MAX_TITLE_CHARS, RunStats, StatEffect};
use stella_rain_core::stage_skill::{
    MAX_PULL, MAX_SHIELD, MAX_SKILL_AHEAD, MAX_SKILL_COOLDOWN_TICKS, MAX_SKILL_DURATION_TICKS,
    MAX_SKILL_LEVELS, MAX_SKILL_PCT, MAX_SKILL_RADIUS, MAX_STAGE_SKILLS, SkillKind,
};
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
            .filter(|r| r.effect.stat() == Some(effect))
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
fn the_bundled_rewards_offer_a_stage_skill_of_every_kind() {
    let bundled = Rewards::bundled();
    for kind in SkillKind::ALL {
        let skill = bundled
            .stage_skills()
            .iter()
            .find(|k| k.kind() == kind)
            .unwrap_or_else(|| panic!("no {kind:?} skill"));
        let gives = |r: &stella_rain_core::rewards::Reward| {
            r.effect == RewardEffect::StageSkill && r.skill.as_ref() == Some(&skill.id)
        };
        assert!(
            bundled.rewards().iter().any(gives),
            "no reward gives {:?}",
            skill.id
        );
    }
}

// --- Stage skills (ADR-075) ------------------------------------------------------------------

fn buff_level(cooldown: u32) -> Value {
    json!({ "cooldown_ticks": cooldown,
            "effect": { "type": "buff", "atk_pct": 20, "duration_ticks": 120 } })
}

fn stage_skill(id: &str) -> Value {
    json!({ "id": id, "levels": [buff_level(600)] })
}

/// A file of one stage skill, `aura`, and a reward that gives it.
fn with_skill() -> Value {
    let mut r = reward("aura_reward");
    r["effect"] = json!("stage_skill");
    r["skill"] = json!("aura");
    r["amount"] = json!(1);
    let mut v = file(vec![r]);
    v["stage_skills"] = json!([stage_skill("aura")]);
    v
}

/// `with_skill` with its one level's effect replaced.
fn effect_with(effect: Value) -> Value {
    let mut v = with_skill();
    v["stage_skills"][0]["levels"][0]["effect"] = effect;
    v
}

fn effect_at(field: &str) -> String {
    format!("$.stage_skills[0].levels[0].effect.{field}")
}

#[test]
fn a_stage_skill_reward_names_a_skill_of_the_file() {
    let rewards = parse(&with_skill()).unwrap();
    assert_eq!(rewards.stage_skills().len(), 1);
    assert_eq!(rewards.skill_index(&ContentId("aura".into())), Some(0));
    let mut v = with_skill();
    v["rewards"][0]["skill"] = json!("other");
    invalid_at(&v, "$.rewards[0].skill");
    let mut v = with_skill();
    v["rewards"][0].as_object_mut().unwrap().remove("skill");
    invalid_at(&v, "$.rewards[0].skill");
    // A stat reward names no skill.
    let mut v = with_skill();
    v["rewards"][0]["effect"] = json!("max_hp");
    invalid_at(&v, "$.rewards[0].skill");
    // Its amount is the levels it gives: 1 up to the most a skill has.
    let mut v = with_skill();
    v["rewards"][0]["amount"] = json!(MAX_SKILL_LEVELS);
    assert!(parse(&v).is_ok());
    v["rewards"][0]["amount"] = json!(MAX_SKILL_LEVELS + 1);
    invalid_at(&v, "$.rewards[0].amount");
}

#[test]
fn stage_skill_ids_are_well_formed_and_unique_and_the_list_is_bounded() {
    let mut v = with_skill();
    v["stage_skills"] = json!([stage_skill("aura"), stage_skill("aura")]);
    invalid_at(&v, "$.stage_skills[1].id");
    v["stage_skills"] = json!([stage_skill("aura"), stage_skill("Bad")]);
    invalid_at(&v, "$.stage_skills[1].id");
    let many = |n: usize| {
        let mut v = with_skill();
        let mut skills = vec![stage_skill("aura")];
        skills.extend((1..n).map(|i| stage_skill(&format!("s{i}"))));
        v["stage_skills"] = json!(skills);
        v
    };
    assert!(parse(&many(MAX_STAGE_SKILLS)).is_ok());
    invalid_at(&many(MAX_STAGE_SKILLS + 1), "$.stage_skills");
}

#[test]
fn a_stage_skill_has_one_to_max_levels_of_one_kind() {
    let levels = |n: usize| {
        let mut v = with_skill();
        v["stage_skills"][0]["levels"] = json!(vec![buff_level(600); n]);
        v
    };
    assert!(parse(&levels(MAX_SKILL_LEVELS)).is_ok());
    invalid_at(&levels(0), "$.stage_skills[0].levels");
    invalid_at(&levels(MAX_SKILL_LEVELS + 1), "$.stage_skills[0].levels");
    let mut v = levels(2);
    v["stage_skills"][0]["levels"][1]["effect"] =
        json!({ "type": "shield", "amount": 5, "duration_ticks": 60 });
    invalid_at(&v, "$.stage_skills[0].levels[1].effect");
}

#[test]
fn a_black_hole_s_numbers_are_bounded() {
    let black_hole = |radius: i64, pull: i64, ticks: i64| {
        effect_with(
            json!({ "type": "black_hole", "radius": radius, "pull": pull, "duration_ticks": ticks }),
        )
    };
    let (r, p, d) = (
        i64::from(MAX_SKILL_RADIUS),
        i64::from(MAX_PULL),
        i64::from(MAX_SKILL_DURATION_TICKS),
    );
    assert!(parse(&black_hole(0, 0, 1)).is_ok());
    assert!(parse(&black_hole(r, p, d)).is_ok());
    invalid_at(&black_hole(-1, p, d), &effect_at("radius"));
    invalid_at(&black_hole(r + 1, p, d), &effect_at("radius"));
    invalid_at(&black_hole(r, -1, d), &effect_at("pull"));
    invalid_at(&black_hole(r, p + 1, d), &effect_at("pull"));
    invalid_at(&black_hole(r, p, 0), &effect_at("duration_ticks"));
    invalid_at(&black_hole(r, p, d + 1), &effect_at("duration_ticks"));
}

#[test]
fn percentages_and_shields_are_bounded() {
    let pct = i64::from(MAX_SKILL_PCT);
    let buff =
        |pct: i64| effect_with(json!({ "type": "buff", "atk_pct": pct, "duration_ticks": 60 }));
    assert!(parse(&buff(pct)).is_ok());
    invalid_at(&buff(0), &effect_at("atk_pct"));
    invalid_at(&buff(pct + 1), &effect_at("atk_pct"));
    let debuff = |pct: i64| {
        effect_with(
            json!({ "type": "debuff_area", "radius": 4096, "vulnerable_pct": pct,
                            "duration_ticks": 60 }),
        )
    };
    assert!(parse(&debuff(pct)).is_ok());
    invalid_at(&debuff(0), &effect_at("vulnerable_pct"));
    invalid_at(&debuff(pct + 1), &effect_at("vulnerable_pct"));
    let shield = |amount: i64| {
        effect_with(json!({ "type": "shield", "amount": amount, "duration_ticks": 60 }))
    };
    assert!(parse(&shield(i64::from(MAX_SHIELD))).is_ok());
    invalid_at(&shield(0), &effect_at("amount"));
    invalid_at(&shield(i64::from(MAX_SHIELD) + 1), &effect_at("amount"));
}

#[test]
fn a_damage_area_s_attack_is_held_to_an_attack_s_budgets() {
    let area = |attack: Value| effect_with(json!({ "type": "damage_area", "attack": attack }));
    let circle = json!([{ "kind": "active", "ticks": 10,
                          "hit_box": { "type": "circle", "radius": 8192 } }]);
    assert!(parse(&area(json!({ "inline": circle }))).is_ok());
    assert!(parse(&area(json!({ "preset": { "id": "slam" } }))).is_ok());
    invalid_at(&area(json!({ "inline": [] })), &effect_at("attack.inline"));
    invalid_at(
        &area(json!({ "preset": { "id": "no_such_attack" } })),
        &effect_at("attack.preset.id"),
    );
}

#[test]
fn the_cooldown_and_the_distance_ahead_are_bounded() {
    let c = i64::from(MAX_SKILL_COOLDOWN_TICKS);
    for (cooldown, ok) in [(1, true), (c, true), (0, false), (c + 1, false)] {
        let mut v = with_skill();
        v["stage_skills"][0]["levels"][0]["cooldown_ticks"] = json!(cooldown);
        if ok {
            assert!(parse(&v).is_ok(), "cooldown {cooldown}");
        } else {
            invalid_at(&v, "$.stage_skills[0].levels[0].cooldown_ticks");
        }
    }
    let a = i64::from(MAX_SKILL_AHEAD);
    for (ahead, ok) in [(0, true), (a, true), (-1, false), (a + 1, false)] {
        let mut v = with_skill();
        v["stage_skills"][0]["ahead"] = json!(ahead);
        if ok {
            assert!(parse(&v).is_ok(), "ahead {ahead}");
        } else {
            invalid_at(&v, "$.stage_skills[0].ahead");
        }
    }
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
