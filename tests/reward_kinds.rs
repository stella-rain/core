//! The named stars' reward kinds that do not change combat (core#NS2a, app#336): `pickup_radius_pct`,
//! `skill_cooldown_pct`, `heal_per_kills`, `offer_candidates`, and the reward-given `rerolls`
//! and `banishes`. A kind is bounded like the others (ADR-020): its amount, and for
//! `heal_per_kills` its `kills`, and no other effect takes `kills`.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::rewards::{
    MAX_PICKUP_RADIUS_PCT, MAX_SKILL_COOLDOWN_PCT, RewardEffect, Rewards,
};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::snapshot::OfferOption;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::Error;
use stella_rain_core::validate::limits::MAX_HEAL_AMOUNT;
use stella_rain_core::version::Versions;

use crate::common::*;

pub(crate) fn entry(effect: &str, amount: u32) -> Value {
    json!({ "effect": effect, "amount": amount })
}

pub(crate) fn file_of(effects: Vec<Value>) -> Value {
    json!({ "rarities": crate::common::rarities(), "rewards": [{
        "id": "kind", "rarity": "rare", "effects": effects, "limit": 3
    }] })
}

pub(crate) fn parse(v: &Value) -> Result<Rewards, Error> {
    Rewards::parse(v.to_string().as_bytes(), Versions::CURRENT)
}

pub(crate) fn invalid_at(v: &Value, path: &str) {
    match parse(v) {
        Err(Error::Invalid(issues)) => {
            assert!(issues.iter().any(|i| i.path == path), "{path}: {issues:?}")
        }
        other => panic!("{path}: expected invalid, got {other:?}"),
    }
}

pub(crate) const AMOUNT: &str = "$.rewards[0].effects[0].amount";

#[test]
fn the_new_kinds_parse_with_no_stat_and_are_bounded() {
    let cases: [(&str, RewardEffect, u32); 5] = [
        (
            "pickup_radius_pct",
            RewardEffect::PickupRadiusPct,
            MAX_PICKUP_RADIUS_PCT,
        ),
        (
            "skill_cooldown_pct",
            RewardEffect::SkillCooldownPct,
            MAX_SKILL_COOLDOWN_PCT,
        ),
        ("offer_candidates", RewardEffect::OfferCandidates, 1),
        ("rerolls", RewardEffect::Rerolls, 10),
        ("banishes", RewardEffect::Banishes, 10),
    ];
    for (name, kind, most) in cases {
        let ok =
            parse(&file_of(vec![entry(name, most)])).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(ok.rewards()[0].effect(), kind, "{name}");
        assert_eq!(kind.stat(), None, "{name}");
        invalid_at(&file_of(vec![entry(name, 0)]), AMOUNT);
        invalid_at(&file_of(vec![entry(name, most + 1)]), AMOUNT);
        let mut none = file_of(vec![entry(name, 1)]);
        none["rewards"][0]["effects"][0]
            .as_object_mut()
            .unwrap()
            .remove("amount");
        invalid_at(&none, AMOUNT);
    }
}

#[test]
fn the_per_pick_bounds_are_200_50_1_and_10() {
    assert_eq!(MAX_PICKUP_RADIUS_PCT, 200);
    assert_eq!(MAX_SKILL_COOLDOWN_PCT, 50);
}

#[test]
fn a_bundle_cannot_add_past_a_kinds_bound() {
    let two = file_of(vec![entry("rerolls", 6), entry("rerolls", 5)]);
    invalid_at(&two, "$.rewards[0].effects");
    assert!(parse(&file_of(vec![entry("rerolls", 5), entry("banishes", 5)])).is_ok());
}

fn heal(amount: u32, kills: Option<u32>) -> Value {
    let mut e = entry("heal_per_kills", amount);
    if let Some(k) = kills {
        e["kills"] = json!(k);
    }
    e
}

#[test]
fn heal_per_kills_holds_an_amount_and_a_kill_interval() {
    let ok = parse(&file_of(vec![heal(MAX_HEAL_AMOUNT, Some(100))])).expect("the largest");
    assert_eq!(ok.rewards()[0].effect(), RewardEffect::HealPerKills);
    assert_eq!(ok.rewards()[0].effects[0].kills, Some(100));
    assert!(parse(&file_of(vec![heal(1, Some(1))])).is_ok());
    invalid_at(&file_of(vec![heal(0, Some(5))]), AMOUNT);
    invalid_at(&file_of(vec![heal(MAX_HEAL_AMOUNT + 1, Some(5))]), AMOUNT);
    let kills = "$.rewards[0].effects[0].kills";
    invalid_at(&file_of(vec![heal(100, None)]), kills);
    invalid_at(&file_of(vec![heal(100, Some(0))]), kills);
    invalid_at(&file_of(vec![heal(100, Some(101))]), kills);
    let mut stray = entry("rerolls", 1);
    stray["kills"] = json!(5);
    invalid_at(&file_of(vec![stray]), kills);
}

// The effects in a short run.

/// 32 pixels above the main character: inside the pickup circle (48 pixels).
pub(crate) const NEAR_Y: i32 = START_Y - 8_192;
/// 60 pixels above it: outside the circle, inside the weapon's reach.
const FAR_Y: i32 = START_Y - 15_360;

/// A Main weapon of one step, a circle of 64 pixels, 10 damage.
pub(crate) fn weapon() -> Value {
    json!({ "inline": {
        "combo": { "main": { "attack": { "inline": [
            { "kind": "active", "ticks": 1, "damage": 10,
              "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 16_384 } },
            { "kind": "recovery", "ticks": 4 }
        ] } } },
        "window_ticks": 30,
        "assist_range": 0
    } })
}

/// Levels of 1000 experience and stars that every kill drops as common.
pub(crate) fn starry(stage: Stage) -> Stage {
    let mut stage = stage;
    stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).unwrap());
    stage.stars = Some(
        serde_json::from_value(json!({
            "drop_per_mille": 1000, "pity": 1, "cap": 8, "pickup_radius": 12_288,
            "rerolls": 0, "banishes": 0,
            "tiers": [{ "from_level": 1, "weights": { "common": 1 } }],
        }))
        .unwrap(),
    );
    stage
}

/// `count` enemies of 10 hp, one every 6 ticks from tick 1, at `NEAR_Y`.
pub(crate) fn near(count: u16) -> Stage {
    let waves = vec![wave(
        1,
        count,
        6,
        enemy(10, hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    )];
    starry(armed(
        stage_full(18_000, Some(far_boss()), waves, vec![]),
        weapon(),
    ))
}

pub(crate) fn plain(id: &str, weight: u32) -> Value {
    json!({ "id": id, "rarity": "common", "effect": "attack_pct", "amount": 1, "limit": 5,
            "weight": weight })
}

pub(crate) fn kind(id: &str, effects: Vec<Value>, weight: u32, limit: u8) -> Value {
    json!({ "id": id, "rarity": "common", "effects": effects, "limit": limit,
            "weight": weight })
}

pub(crate) fn with(stage: &Stage, list: Vec<Value>) -> Engine {
    let file = json!({ "rarities": crate::common::rarities(), "rewards": list });
    let rewards = Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT)
        .expect("the test rewards validate");
    let content = RunContent::new(RunStats::bundled().clone(), rewards);
    Engine::new_with(stage, &content).expect("the test stage is valid")
}

pub(crate) fn take(e: &mut Engine, reward: &str) {
    let offer = e.snapshot().progress.unwrap().offer.expect("an open offer");
    let candidate = offer
        .candidates
        .iter()
        .position(|c| c.reward.as_str() == reward)
        .unwrap_or_else(|| panic!("{reward} is not offered"));
    let option = OfferOption::Take {
        candidate: u8::try_from(candidate).unwrap(),
    };
    let index = offer.options.iter().position(|o| *o == option).unwrap();
    e.choose(u32::try_from(index).unwrap()).unwrap();
}

pub(crate) fn next_offer(e: &mut Engine) {
    fire_until(e, idle(), 600, Engine::offer_open);
    assert!(e.offer_open(), "no offer within 600 ticks");
}

pub(crate) fn offer(e: &Engine) -> stella_rain_core::snapshot::OfferView {
    e.snapshot().progress.unwrap().offer.expect("an open offer")
}

#[test]
fn rerolls_and_banishes_are_added_to_the_runs_counters() {
    let list = vec![
        kind(
            "more",
            vec![entry("rerolls", 2), entry("banishes", 3)],
            1_000,
            2,
        ),
        plain("a", 1),
        plain("b", 1),
        plain("c", 1),
    ];
    let mut e = with(&near(4), list);
    next_offer(&mut e);
    assert_eq!((offer(&e).rerolls_left, offer(&e).banishes_left), (0, 0));
    take(&mut e, "more");
    next_offer(&mut e);
    assert_eq!((offer(&e).rerolls_left, offer(&e).banishes_left), (2, 3));
    // A second pick adds again.
    take(&mut e, "more");
    next_offer(&mut e);
    assert_eq!((offer(&e).rerolls_left, offer(&e).banishes_left), (4, 6));
}

#[test]
fn an_offer_candidates_reward_makes_the_offer_four_and_never_more() {
    let wide = |id: &str| kind(id, vec![entry("offer_candidates", 1)], 1_000, 2);
    let mut list = vec![wide("wide_a"), wide("wide_b")];
    list.extend(["a", "b", "c", "d", "e"].map(|id| plain(id, 1)));
    let mut e = with(&near(4), list);
    next_offer(&mut e);
    assert_eq!(offer(&e).candidates.len(), 3);
    take(&mut e, "wide_a");
    next_offer(&mut e);
    assert_eq!(offer(&e).candidates.len(), 4);
    take(&mut e, "wide_b");
    next_offer(&mut e);
    assert_eq!(offer(&e).candidates.len(), 4, "at most 4 in all");
}

#[test]
fn a_pickup_radius_reward_takes_a_star_that_was_out_of_reach() {
    // A star at 32 pixels is taken at once; a second enemy 60 pixels away leaves its star on
    // the map, 48 pixels being the circle, unless 30 percent more (62.4) has been taken.
    let waves = vec![
        wave(
            1,
            1,
            0,
            enemy(10, hold(START_X, NEAR_Y), json!([])),
            START_X,
            NEAR_Y,
        ),
        wave(
            300,
            1,
            0,
            enemy(10, hold(START_X, FAR_Y), json!([])),
            START_X,
            FAR_Y,
        ),
    ];
    let stage = starry(armed(
        stage_full(18_000, Some(far_boss()), waves, vec![]),
        weapon(),
    ));
    let list = vec![
        kind("reach", vec![entry("pickup_radius_pct", 30)], 1_000, 5),
        plain("might", 1),
    ];
    let outcome = |reward: &str| {
        let mut e = with(&stage, list.clone());
        next_offer(&mut e);
        take(&mut e, reward);
        fire_until(&mut e, idle(), 500, |_| false);
        (e.offer_open(), e.snapshot().stars.len())
    };
    assert_eq!(
        outcome("might"),
        (false, 1),
        "out of reach stays on the map"
    );
    assert_eq!(outcome("reach"), (true, 0), "30 percent more reaches it");
}
