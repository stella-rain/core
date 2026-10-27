//! Rewards (ADR-079 §3, ADR-067): what an offer holds is content, read from JSON as untrusted
//! bytes (ADR-020): a size limit before parsing, strict parsing, the current versions only and
//! every field bounded. The bundled file is the default; other bytes run the same way and
//! change the run's content hash.

use serde_json::{Value, json};
use stella_rain_core::id::ContentId;
use stella_rain_core::rewards::{
    MAX_AMOUNT, MAX_HP_AMOUNT, MAX_ORBIT_COOLDOWN_TICKS, MAX_ORBIT_TURN, MAX_ORBITS, MAX_ORBS,
    MAX_RANGE_PCT, MAX_REWARDS, MAX_REWARDS_BYTES, MAX_STACK, Rarity, RewardEffect, Rewards,
};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{
    RunRecorder, RunReplayError, content_sha256, content_sha256_with, verify_run, verify_run_with,
};
use stella_rain_core::run_stats::{MAX_DESCRIPTION_CHARS, MAX_TITLE_CHARS, RunStats, StatEffect};
use stella_rain_core::stage_skill::{MAX_SKILL_LEVELS, StageSkills};
use stella_rain_core::validate::Error;
use stella_rain_core::validate::limits::MAX_EVADE_CHARGES;
use stella_rain_core::version::Versions;

use crate::common;

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
    json!({ "rewards": rewards })
}

fn parse_at(v: &Value, versions: Versions) -> Result<Rewards, Error> {
    Rewards::parse(v.to_string().as_bytes(), versions)
}

fn parse(v: &Value) -> Result<Rewards, Error> {
    parse_at(v, Versions::CURRENT)
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
fn the_bundled_rewards_hold_every_effect_at_its_rarities() {
    let rewards = Rewards::bundled().rewards();
    for effect in StatEffect::ALL {
        let rarities: Vec<Rarity> = rewards
            .iter()
            .filter(|r| !r.named && r.effect().stat() == Some(effect))
            .map(|r| r.rarity)
            .collect();
        // The three released rarities come first, then the six-rarity ladder's new ones
        // (tests/rarities.rs).
        assert_eq!(
            rarities,
            [
                Rarity::Common,
                Rarity::Rare,
                Rarity::Legendary,
                Rarity::Uncommon,
                Rarity::Epic,
                Rarity::Supernova,
            ],
            "{effect:?}"
        );
    }
    assert!(rewards.iter().all(|r| !r.title.is_empty()));
    // The pack's rewards are the file's, with the sets of the constellations file beside them.
    let file = Rewards::parse(common::bundled_file("rewards"), Versions::CURRENT).unwrap();
    assert_eq!(file.rewards(), Rewards::bundled().rewards());
    assert_eq!(file.sha256(), Rewards::bundled().sha256());
}

#[test]
fn the_bundled_rewards_have_the_stack_limit_of_their_rarity() {
    // app#336: how many times one run takes an ordinary reward, by rarity.
    // (The random rune has none: it leaves the offers once every rune is held.)
    for reward in Rewards::bundled()
        .rewards()
        .iter()
        .filter(|r| !r.named && !r.is_random_rune())
    {
        let limit = match reward.rarity {
            Rarity::Common => 5,
            Rarity::Uncommon => 4,
            Rarity::Rare => 3,
            Rarity::Epic | Rarity::Legendary | Rarity::Supernova => 2,
        };
        assert_eq!(
            reward.limit,
            Some(limit),
            "{} ({:?})",
            reward.id.0,
            reward.rarity
        );
    }
}

#[test]
fn the_bundled_rewards_offer_every_bundled_stage_skill() {
    let bundled = Rewards::bundled();
    assert_eq!(bundled.stage_skills(), StageSkills::bundled());
    for skill in StageSkills::bundled().skills() {
        let gives = |r: &stella_rain_core::rewards::Reward| {
            r.effect() == RewardEffect::StageSkill && r.effects[0].skill.as_ref() == Some(&skill.id)
        };
        assert!(
            bundled.rewards().iter().any(gives),
            "no reward gives {:?}",
            skill.id
        );
    }
}

// --- Stage skills (ADR-075, ADR-098) ---------------------------------------------------------

/// A stage skills file of `aura` and `ward`.
fn skills() -> StageSkills {
    let skill = |id: &str| {
        json!({ "id": id, "icon": "icon_war_cry", "title": "Aura", "description": "",
                "levels": [{ "cooldown_ticks": 600, "effects": [
                    { "type": "shield", "amount": 5, "duration_ticks": 60 }
                ] }] })
    };
    let file = json!({ "stage_skills": [skill("aura"), skill("ward")] });
    StageSkills::parse(file.to_string().as_bytes(), Versions::CURRENT).unwrap()
}

/// A file of one reward that gives `ward`.
fn with_skill() -> Value {
    let mut r = reward("ward_reward");
    r["effect"] = json!("stage_skill");
    r["skill"] = json!("ward");
    r["amount"] = json!(1);
    file(vec![r])
}

fn parse_with_skills(v: &Value) -> Result<Rewards, Error> {
    Rewards::parse_with(v.to_string().as_bytes(), Versions::CURRENT, &skills())
}

fn invalid_with_skills_at(v: &Value, path: &str) {
    match parse_with_skills(v) {
        Err(Error::Invalid(issues)) => {
            assert!(issues.iter().any(|i| i.path == path), "{path}: {issues:?}")
        }
        other => panic!("{path}: expected invalid, got {other:?}"),
    }
}

#[test]
fn a_stage_skill_reward_names_a_skill_of_the_stage_skills_file() {
    let rewards = parse_with_skills(&with_skill()).unwrap();
    assert_eq!(rewards.stage_skills(), &skills());
    assert_eq!(
        rewards.stage_skills().index_of(&ContentId("ward".into())),
        Some(1)
    );
    // Not in the bundled skills.
    invalid_at(&with_skill(), "$.rewards[0].effects[0].skill");
    let mut v = with_skill();
    v["rewards"][0]["skill"] = json!("other");
    invalid_with_skills_at(&v, "$.rewards[0].effects[0].skill");
    let mut v = with_skill();
    v["rewards"][0].as_object_mut().unwrap().remove("skill");
    invalid_with_skills_at(&v, "$.rewards[0].effects[0].skill");
    // A stat reward names no skill.
    let mut v = with_skill();
    v["rewards"][0]["effect"] = json!("max_hp");
    invalid_with_skills_at(&v, "$.rewards[0].effects[0].skill");
    // Its amount is the levels it gives: 1 up to the most a skill has.
    let mut v = with_skill();
    v["rewards"][0]["amount"] = json!(MAX_SKILL_LEVELS);
    assert!(parse_with_skills(&v).is_ok());
    v["rewards"][0]["amount"] = json!(MAX_SKILL_LEVELS + 1);
    invalid_with_skills_at(&v, "$.rewards[0].effects[0].amount");
}

#[test]
fn the_rewards_file_no_longer_holds_the_skills() {
    let mut v = with_skill();
    v["stage_skills"] = json!([]);
    assert!(matches!(parse_with_skills(&v), Err(Error::Parse { .. })));
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
        Rewards::parse(&big, Versions::CURRENT),
        Err(Error::TooLarge {
            size: MAX_REWARDS_BYTES + 1,
            limit: MAX_REWARDS_BYTES
        })
    );
    let mut padded = file(vec![reward("might")]).to_string().into_bytes();
    padded.resize(MAX_REWARDS_BYTES, b' ');
    assert!(
        Rewards::parse(&padded, Versions::CURRENT).is_ok(),
        "at the limit it parses"
    );
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
    v["rewards"][0]["extra"] = json!(1);
    assert_eq!(
        parse_at(
            &v,
            Versions {
                schema_version: 0,
                sim_version: 1
            }
        ),
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
        // An unnamed reward stacks: its limit starts at 2.
        ("limit", 2, i64::from(MAX_STACK)),
    ] {
        let path = match field {
            "amount" => "$.rewards[0].effects[0].amount".to_string(),
            _ => format!("$.rewards[0].{field}"),
        };
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

#[test]
fn a_hit_point_amount_has_the_scaled_bound_and_the_others_keep_theirs() {
    // ADR-103: hit points are scaled by 100, a percentage is not.
    assert_eq!(MAX_HP_AMOUNT, 5_000 * 100);
    let hp = |amount: u32| {
        let mut v = one_with("effect", json!("max_hp"));
        v["rewards"][0]["amount"] = json!(amount);
        v
    };
    assert!(parse(&hp(MAX_HP_AMOUNT)).is_ok());
    invalid_at(&hp(MAX_HP_AMOUNT + 1), "$.rewards[0].effects[0].amount");
    invalid_at(
        &one_with("amount", json!(MAX_AMOUNT + 1)),
        "$.rewards[0].effects[0].amount",
    );
}

#[test]
fn an_extra_charge_reward_adds_one_to_the_most_charges_and_names_no_skill() {
    let charge = |amount: u32| {
        let mut v = one_with("effect", json!("evade_charge"));
        v["rewards"][0]["amount"] = json!(amount);
        v
    };
    let rewards = parse(&charge(1)).expect("one charge");
    assert_eq!(rewards.rewards()[0].effect(), RewardEffect::EvadeCharge);
    assert_eq!(RewardEffect::EvadeCharge.stat(), None);
    assert!(parse(&charge(u32::from(MAX_EVADE_CHARGES))).is_ok());
    invalid_at(&charge(0), "$.rewards[0].effects[0].amount");
    invalid_at(
        &charge(u32::from(MAX_EVADE_CHARGES) + 1),
        "$.rewards[0].effects[0].amount",
    );
    let mut named = charge(1);
    named["rewards"][0]["skill"] = json!("black_hole");
    invalid_at(&named, "$.rewards[0].effects[0].skill");
}

/// A range reward `id`, `amount` percent wider, taken at most 5 times.
fn range(id: &str, amount: u32) -> Value {
    json!({
        "id": id, "rarity": "rare", "effect": "range_pct", "amount": amount, "limit": 5,
        "title": "Reach", "description": "Every combo step reaches further"
    })
}

#[test]
fn a_range_reward_is_no_stat_and_widens_within_bounds() {
    let one = |amount: u32| file(vec![range("reach", amount)]);
    let rewards = parse(&one(25)).expect("a range reward");
    assert_eq!(rewards.rewards()[0].effect(), RewardEffect::RangePct);
    assert_eq!(RewardEffect::RangePct.stat(), None);
    assert!(parse(&one(MAX_RANGE_PCT)).is_ok());
    invalid_at(&one(0), "$.rewards[0].effects[0].amount");
    invalid_at(&one(MAX_RANGE_PCT + 1), "$.rewards[0].effects[0].amount");

    let mut most = one(25);
    most["rewards"][0]["limit"] = json!(MAX_STACK);
    assert!(parse(&most).is_ok());
    let mut named = one(25);
    named["rewards"][0]["skill"] = json!("black_hole");
    invalid_at(&named, "$.rewards[0].effects[0].skill");
    let mut old = one(25);
    old["rewards"][0]["enchantment"] = json!({ "type": "range" });
    assert!(matches!(parse(&old), Err(Error::Parse { .. })));
}

/// An orbit reward `id`: `amount` orbs of a one-phase attack 40 pixels out, a quarter turn a
/// second, every 2 seconds.
fn orbit(id: &str, amount: u32) -> Value {
    json!({
        "id": id, "rarity": "rare", "effect": "orbit", "amount": amount, "limit": 3,
        "title": "Orbit", "description": "Orbs turn around you",
        "orbit": {
            "attack": { "inline": [
                { "kind": "active", "ticks": 60, "damage": 5,
                  "hit_box": { "type": "circle", "offset": { "forward": 10_240 }, "radius": 1_024 } }
            ] },
            "turn_per_tick": 384,
            "cooldown_ticks": 120
        }
    })
}

#[test]
fn an_orbit_reward_names_its_orbit_and_adds_one_to_the_most_orbs() {
    let one = |amount: u32| file(vec![orbit("ring", amount)]);
    let rewards = parse(&one(1)).expect("one orb");
    assert_eq!(rewards.rewards()[0].effect(), RewardEffect::Orbit);
    assert_eq!(RewardEffect::Orbit.stat(), None);
    let held = rewards.rewards()[0].orbit().expect("its orbit");
    assert_eq!((held.turn_per_tick, held.cooldown_ticks), (384, 120));
    assert!(parse(&one(u32::from(MAX_ORBS))).is_ok());
    invalid_at(&one(0), "$.rewards[0].effects[0].amount");
    invalid_at(
        &one(u32::from(MAX_ORBS) + 1),
        "$.rewards[0].effects[0].amount",
    );

    let mut bare = one(1);
    bare["rewards"][0].as_object_mut().unwrap().remove("orbit");
    invalid_at(&bare, "$.rewards[0].effects[0].orbit");
    let mut stray = one_with("orbit", orbit("x", 1)["orbit"].clone());
    stray["rewards"][0]["effect"] = json!("attack_pct");
    invalid_at(&stray, "$.rewards[0].effects[0].orbit");
    let mut named = one(1);
    named["rewards"][0]["skill"] = json!("black_hole");
    invalid_at(&named, "$.rewards[0].effects[0].skill");
}

#[test]
fn an_orbit_s_turn_cooldown_and_attack_are_bounded() {
    let with = |field: &str, value: Value| {
        let mut r = orbit("ring", 1);
        r["orbit"][field] = value;
        file(vec![r])
    };
    for turn in [-MAX_ORBIT_TURN, 0, MAX_ORBIT_TURN] {
        assert!(parse(&with("turn_per_tick", json!(turn))).is_ok(), "{turn}");
    }
    for turn in [-MAX_ORBIT_TURN - 1, MAX_ORBIT_TURN + 1] {
        invalid_at(
            &with("turn_per_tick", json!(turn)),
            "$.rewards[0].effects[0].orbit.turn_per_tick",
        );
    }
    assert!(parse(&with("cooldown_ticks", json!(1))).is_ok());
    assert!(parse(&with("cooldown_ticks", json!(MAX_ORBIT_COOLDOWN_TICKS))).is_ok());
    for ticks in [0, MAX_ORBIT_COOLDOWN_TICKS + 1] {
        invalid_at(
            &with("cooldown_ticks", json!(ticks)),
            "$.rewards[0].effects[0].orbit.cooldown_ticks",
        );
    }
    let unknown = with(
        "attack",
        json!({ "preset": { "id": "no_such_attack", "args": [] } }),
    );
    match parse(&unknown) {
        Err(Error::Invalid(issues)) => assert!(
            issues
                .iter()
                .any(|i| i.path.starts_with("$.rewards[0].effects[0].orbit.attack")),
            "{issues:?}"
        ),
        other => panic!("expected invalid, got {other:?}"),
    }
}

#[test]
fn a_rewards_file_holds_at_most_four_orbits() {
    let orbits = |n: usize| file((0..n).map(|i| orbit(&format!("ring_{i}"), 1)).collect());
    assert!(parse(&orbits(MAX_ORBITS)).is_ok());
    invalid_at(
        &orbits(MAX_ORBITS + 1),
        &format!("$.rewards[{MAX_ORBITS}].effects[0].effect"),
    );
}

fn stronger() -> Rewards {
    let mut v: Value = serde_json::from_slice(common::bundled_file("rewards")).unwrap();
    v["rewards"][0]["amount"] = json!(9);
    Rewards::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap()
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

/// The bundled stage skills with the first skill's first cooldown a tick shorter.
fn quicker_skills() -> StageSkills {
    let mut v: Value = serde_json::from_slice(common::bundled_file("stage_skills")).unwrap();
    let c = v["stage_skills"][0]["levels"][0]["cooldown_ticks"]
        .as_u64()
        .unwrap();
    v["stage_skills"][0]["levels"][0]["cooldown_ticks"] = json!(c - 1);
    StageSkills::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap()
}

#[test]
fn the_content_hash_covers_the_stage_skills() {
    let rewards = Rewards::parse_with(
        common::bundled_file("rewards"),
        Versions::CURRENT,
        &quicker_skills(),
    )
    .unwrap();
    assert_eq!(rewards.rewards(), Rewards::bundled().rewards());
    assert_eq!(rewards.sha256(), Rewards::bundled().sha256());
    let other = RunContent::new(RunStats::bundled().clone(), rewards);
    assert_eq!(other.stage_skills(), &quicker_skills());
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

#[test]
fn vigour_is_shown_hit_points_stored_times_100() {
    let amounts: Vec<(&str, u32)> = Rewards::bundled()
        .rewards()
        .iter()
        .filter(|r| r.id.0.starts_with("max_hp_"))
        .map(|r| (r.id.0.as_str(), r.amount().unwrap()))
        .collect();
    assert_eq!(
        amounts,
        [
            ("max_hp_common", 20_000),
            ("max_hp_rare", 50_000),
            ("max_hp_legendary", 100_000),
            ("max_hp_uncommon", 30_000),
            ("max_hp_epic", 70_000),
            ("max_hp_supernova", 160_000),
        ]
    );
}

// --- Effect lists, named stars and weights (app#336) -------------------------------------------

/// A reward whose `effects` list holds `effects`, with the single-effect fields left out.
fn bundle(id: &str, effects: Vec<Value>) -> Value {
    json!({
        "id": id, "rarity": "epic", "effects": effects, "limit": 2,
        "title": "Bundle", "description": ""
    })
}

fn pct(effect: &str, amount: u32) -> Value {
    json!({ "effect": effect, "amount": amount })
}

/// A named reward: one epic effect, limit 1, called `star`.
fn named(id: &str, star: &str) -> Value {
    let mut r = bundle(id, vec![pct("attack_pct", 10)]);
    r["limit"] = json!(1);
    r["named"] = json!(true);
    r["star"] = json!(star);
    r
}

fn message_at(v: &Value, path: &str) -> String {
    match parse(v) {
        Err(Error::Invalid(issues)) => issues
            .iter()
            .find(|i| i.path == path)
            .unwrap_or_else(|| panic!("{path}: {issues:?}"))
            .message
            .clone(),
        other => panic!("{path}: expected invalid, got {other:?}"),
    }
}

#[test]
fn a_reward_holds_a_list_of_one_to_four_effects() {
    let ok = file(vec![bundle(
        "quad",
        vec![
            pct("attack_pct", 10),
            pct("speed_pct", 5),
            pct("xp_pct", 5),
            pct("evade_pct", 5),
        ],
    )]);
    let rewards = parse(&ok).expect("four effects parse");
    let back = serde_json::to_value(&rewards.rewards()[0]).unwrap();
    assert_eq!(back["effects"].as_array().map(Vec::len), Some(4), "{back}");
    invalid_at(&file(vec![bundle("none", vec![])]), "$.rewards[0].effects");
    let five = (0..5).map(|_| pct("attack_pct", 1)).collect();
    invalid_at(&file(vec![bundle("five", five)]), "$.rewards[0].effects");
}

#[test]
fn the_single_effect_form_parses_to_a_list_of_one() {
    let rewards = parse(&file(vec![reward("might")])).expect("the released form parses");
    let back = serde_json::to_value(&rewards.rewards()[0]).unwrap();
    assert_eq!(
        back["effects"],
        json!([{ "effect": "attack_pct", "amount": 10 }]),
        "{back}"
    );
    assert!(
        back.get("effect").is_none() && back.get("amount").is_none(),
        "{back}"
    );
    // The released form and the list form together, or neither, are not a reward.
    let mut both = reward("might");
    both["effects"] = json!([pct("attack_pct", 10)]);
    assert!(parse(&file(vec![both])).is_err());
    let mut neither = reward("might");
    neither.as_object_mut().unwrap().remove("effect");
    assert!(parse(&file(vec![neither])).is_err());
}

#[test]
fn every_entry_of_a_list_is_checked_as_a_single_effect_is() {
    let bad = |entry: Value, path: &str| {
        invalid_at(
            &file(vec![bundle("b", vec![pct("attack_pct", 1), entry])]),
            path,
        )
    };
    bad(pct("attack_pct", 0), "$.rewards[0].effects[1].amount");
    bad(
        pct("attack_pct", MAX_AMOUNT + 1),
        "$.rewards[0].effects[1].amount",
    );
    bad(
        json!({ "effect": "stage_skill", "amount": 1 }),
        "$.rewards[0].effects[1].skill",
    );
    bad(
        json!({ "effect": "attack_pct", "amount": 1, "skill": "x" }),
        "$.rewards[0].effects[1].skill",
    );
    bad(pct("type_bonus_pct", 5), "$.rewards[0].effects[1].type");
}

#[test]
fn the_caps_that_sum_hold_over_a_whole_bundle() {
    let sum = |effect: &str, half: u32, over: u32| {
        let ok = file(vec![bundle(
            "b",
            vec![pct(effect, half), pct(effect, half)],
        )]);
        assert!(parse(&ok).is_ok(), "{effect}: {half} twice");
        let bad = file(vec![bundle(
            "b",
            vec![pct(effect, half), pct(effect, over)],
        )]);
        invalid_at(&bad, "$.rewards[0].effects");
    };
    sum("range_pct", MAX_RANGE_PCT / 2, MAX_RANGE_PCT / 2 + 1);
    sum("rune_haste", 150, 151);
    sum("max_hp", MAX_HP_AMOUNT / 2, MAX_HP_AMOUNT / 2 + 1);
    sum("attack_pct", MAX_AMOUNT / 2, MAX_AMOUNT / 2 + 1);
}

#[test]
fn a_bundle_holds_at_most_one_random_rune_and_at_most_one_orbit() {
    let rune = json!({ "effect": "random_rune" });
    // With another effect it is a reward with a limit, like any bundle.
    let with_haste = bundle("b", vec![rune.clone(), pct("rune_haste", 50)]);
    let rewards = parse(&file(vec![with_haste])).expect("a random rune and a haste parse");
    assert!(!rewards.rewards()[0].is_random_rune());
    assert_eq!(rewards.rewards()[0].limit, Some(2));
    // Alone it keeps its own rules: no limit.
    let mut alone = bundle("r", vec![rune.clone()]);
    alone.as_object_mut().unwrap().remove("limit");
    assert!(parse(&file(vec![alone])).is_ok());
    // A second one in the same reward is refused.
    invalid_at(
        &file(vec![bundle(
            "b",
            vec![rune.clone(), pct("attack_pct", 1), rune],
        )]),
        "$.rewards[0].effects",
    );
    let entry = {
        let o = orbit("o", 1);
        json!({ "effect": "orbit", "amount": 1, "orbit": o["orbit"] })
    };
    let two = vec![entry.clone(), entry];
    invalid_at(&file(vec![bundle("b", two)]), "$.rewards[0].effects");
}

#[test]
fn a_named_reward_is_one_of_a_kind_and_epic_or_above() {
    let ok = file(vec![named("vega_star", "Vega")]);
    let rewards = parse(&ok).expect("a named epic reward with limit 1 parses");
    let back = serde_json::to_value(&rewards.rewards()[0]).unwrap();
    assert_eq!(
        (back["named"].clone(), back["star"].clone()),
        (json!(true), json!("Vega"))
    );
    for rarity in ["epic", "legendary", "supernova"] {
        let mut r = named("vega_star", "Vega");
        r["rarity"] = json!(rarity);
        assert!(parse(&file(vec![r])).is_ok(), "{rarity}");
    }
    let mut stacking = named("vega_star", "Vega");
    stacking["limit"] = json!(2);
    assert!(message_at(&file(vec![stacking]), "$.rewards[0].limit").contains("named"));
    for rarity in ["common", "uncommon", "rare"] {
        let mut r = named("vega_star", "Vega");
        r["rarity"] = json!(rarity);
        assert!(
            message_at(&file(vec![r]), "$.rewards[0].rarity").contains("named"),
            "{rarity}"
        );
    }
}

#[test]
fn a_named_reward_has_a_star_name_and_an_unnamed_one_has_none_and_stacks() {
    let mut nameless = named("vega_star", "Vega");
    nameless.as_object_mut().unwrap().remove("star");
    invalid_at(&file(vec![nameless]), "$.rewards[0].star");
    let mut stray = reward("might");
    stray["star"] = json!("Vega");
    invalid_at(&file(vec![stray]), "$.rewards[0].star");
    let mut empty = named("vega_star", "");
    invalid_at(&file(vec![empty.clone()]), "$.rewards[0].star");
    empty["star"] = json!("x".repeat(MAX_TITLE_CHARS + 1));
    invalid_at(&file(vec![empty]), "$.rewards[0].star");
    // An unnamed reward stacks: limit 2 or more.
    let mut single = reward("might");
    single["limit"] = json!(1);
    assert!(message_at(&file(vec![single]), "$.rewards[0].limit").contains("unnamed"));
    let mut explicit = reward("might");
    explicit["named"] = json!(false);
    assert!(parse(&file(vec![explicit])).is_ok());
}

#[test]
fn star_names_are_unique_and_a_file_holds_at_most_32_named_rewards() {
    let two = file(vec![named("one", "Vega"), named("two", "Vega")]);
    invalid_at(&two, "$.rewards[1].star");
    let many = |n: usize| {
        file(
            (0..n)
                .map(|i| named(&format!("star_{i}"), &format!("Star {i}")))
                .collect(),
        )
    };
    assert!(parse(&many(32)).is_ok());
    invalid_at(&many(33), "$.rewards");
}

#[test]
fn a_weight_is_1_to_1000_and_defaults_to_100() {
    for weight in [1, 100, 1000] {
        assert!(
            parse(&one_with("weight", json!(weight))).is_ok(),
            "{weight}"
        );
    }
    for weight in [0, 1001] {
        invalid_at(&one_with("weight", json!(weight)), "$.rewards[0].weight");
    }
    let rewards = parse(&file(vec![reward("might")])).unwrap();
    let back = serde_json::to_value(&rewards.rewards()[0]).unwrap();
    assert!(
        back.get("weight").is_none() || back["weight"] == json!(100),
        "{back}"
    );
}
