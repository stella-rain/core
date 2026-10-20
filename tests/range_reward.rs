//! The range reward (ADR-099 point 6): a stat-like star reward, `range_pct`, with an amount, a
//! stack limit and a rarity like the others. Each take adds its amount to how much wider every
//! combo step's hit box is, its radius and offset (angle as written), and the expertise range
//! options add into the same widening; all of it together is held to `MAX_RANGE_PCT`. It stays
//! in the pool until its stack limit, and holding one range reward leaves another offered.
//! Every number here is worked out by hand.

use serde_json::{Value, json};
use stella_rain_core::attack::HitBox;
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::expertise::Expertise;
use stella_rain_core::input::Input;
use stella_rain_core::rewards::{MAX_RANGE_PCT, RewardEffect, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::snapshot::{AttackView, OfferOption, WeaponRole};
use stella_rain_core::stage::Stage;

use crate::common;

use common::*;

/// 32 pixels above the main character: inside the pickup circle, so a star dropped here is
/// taken at once, and inside the first step's hit box.
const NEAR_Y: i32 = START_Y - 8_192;

/// A Main weapon of one step: a circle of 32 pixels 4 pixels ahead, one active tick of 10
/// damage and 4 ticks of recovery.
fn weapon() -> Value {
    json!({ "inline": {
        "combo": { "main": { "attack": { "inline": [
            { "kind": "active", "ticks": 1, "damage": 10, "hit_box": circle(1_024, 8_192) },
            { "kind": "recovery", "ticks": 4 }
        ] } } },
        "window_ticks": 30,
        "assist_range": 0
    } })
}

fn circle(forward: i32, radius: i32) -> Value {
    json!({ "type": "circle", "offset": { "forward": forward }, "radius": radius })
}

fn hit_box(v: Value) -> HitBox {
    serde_json::from_value(v).unwrap()
}

/// Levels of 1000 experience and stars that every kill drops as common, taken at once.
fn starry(stage: Stage) -> Stage {
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

/// The weapon above and `count` enemies of 10 hp (one step each), one every 6 ticks from tick
/// 1, at `NEAR_Y`.
fn targets(count: u16) -> Stage {
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

/// A common range reward `id`, `pct` percent wider, taken at most `limit` times.
fn range(id: &str, pct: u32, limit: u8) -> Value {
    json!({
        "id": id, "rarity": "common", "effect": "range_pct", "amount": pct, "limit": limit,
        "title": "Reach", "description": ""
    })
}

fn might() -> Value {
    json!({
        "id": "might", "rarity": "common", "effect": "attack_pct", "amount": 5, "limit": 5,
        "title": "Might", "description": ""
    })
}

fn rewards(rewards: Vec<Value>) -> Rewards {
    let file = json!({ "schema_version": 0, "sim_version": 0, "rewards": rewards });
    Rewards::parse(file.to_string().as_bytes()).expect("the test rewards validate")
}

fn with(stage: &Stage, list: Vec<Value>) -> Engine {
    let content = RunContent::new(RunStats::bundled().clone(), rewards(list));
    Engine::new_with(stage, &content).expect("the test stage is valid")
}

/// Takes the open offer's candidate that is `reward`.
fn take(e: &mut Engine, reward: &str) {
    let offer = e.snapshot().progress.unwrap().offer.expect("an open offer");
    let candidate = offer
        .candidates
        .iter()
        .position(|c| c.reward.0 == reward)
        .unwrap_or_else(|| panic!("{reward} is not offered"));
    let option = OfferOption::Take {
        candidate: u8::try_from(candidate).unwrap(),
    };
    let index = offer.options.iter().position(|o| *o == option).unwrap();
    e.choose(u32::try_from(index).unwrap()).unwrap();
}

fn offered(e: &Engine) -> Vec<String> {
    let offer = e.snapshot().progress.unwrap().offer.expect("an open offer");
    let mut ids: Vec<String> = offer
        .candidates
        .iter()
        .map(|c| c.reward.0.clone())
        .collect();
    ids.sort();
    ids
}

/// Fires until the next offer opens.
fn next_offer(e: &mut Engine) {
    fire_until(e, idle(), 600, Engine::offer_open);
    assert!(e.offer_open(), "no offer within 600 ticks");
}

/// The combo step in progress (slot 0).
fn step_attack(e: &Engine) -> Option<AttackView> {
    e.snapshot()
        .player
        .attacks
        .into_iter()
        .find(|a| a.slot == 0)
}

/// Waits for the combo step in progress to end, presses `main` for a tick, then waits out the
/// step's wind-up: the hit box of its active phase.
fn swing(e: &mut Engine) -> HitBox {
    for _ in 0..60 {
        if step_attack(e).is_none() {
            break;
        }
        e.step(idle());
    }
    e.step(Input {
        main: true,
        ..Input::default()
    });
    for _ in 0..60 {
        let step = step_attack(e).expect("a combo step started");
        if let Some(h) = step.hit_box {
            return h;
        }
        e.step(idle());
    }
    panic!("no active phase within 60 ticks")
}

#[test]
fn a_range_reward_is_a_stat_like_effect_bounded_like_the_others() {
    let parsed = rewards(vec![range("reach", MAX_RANGE_PCT, 20)]);
    assert_eq!(parsed.rewards()[0].effect, RewardEffect::RangePct);
    let bad = |pct: u32, limit: u8| {
        let file = json!({ "schema_version": 0, "sim_version": 0,
                           "rewards": [range("reach", pct, limit)] });
        Rewards::parse(file.to_string().as_bytes()).is_err()
    };
    assert!(bad(0, 1), "an amount of 0");
    assert!(bad(MAX_RANGE_PCT + 1, 1), "an amount past MAX_RANGE_PCT");
    assert!(bad(10, 0), "a limit of 0");
    assert!(bad(10, 21), "a limit past MAX_STACK");
    assert!(!bad(10, 5), "a limit above 1");
}

#[test]
fn a_range_reward_taken_several_times_sums_up_to_the_clamp() {
    let mut e = with(&targets(3), vec![range("reach", 40, 3)]);
    e.step(idle());
    e.step(idle());
    assert_eq!(swing(&mut e), hit_box(circle(1_024, 8_192)));
    next_offer(&mut e);
    take(&mut e, "reach");
    assert_eq!(e.snapshot().player.range_pct, 40);

    // 40 percent: radius and offset 1.4 times.
    assert_eq!(swing(&mut e), hit_box(circle(1_433, 11_468)));
    next_offer(&mut e);
    take(&mut e, "reach");
    assert_eq!(e.snapshot().player.range_pct, 80);

    // 80 percent.
    assert_eq!(swing(&mut e), hit_box(circle(1_843, 14_745)));
    next_offer(&mut e);
    take(&mut e, "reach");
    assert_eq!(e.snapshot().player.range_pct, 120, "the sum as taken");

    // 120 percent taken, 100 applied: twice as large.
    assert_eq!(swing(&mut e), hit_box(circle(2_048, 16_384)));
}

#[test]
fn a_range_reward_stays_in_the_pool_until_its_stack_limit() {
    let mut e = with(&targets(3), vec![range("reach", 10, 2), might()]);
    next_offer(&mut e);
    assert_eq!(offered(&e), ["might", "reach"]);
    take(&mut e, "reach");
    next_offer(&mut e);
    assert_eq!(offered(&e), ["might", "reach"], "taken once of two");
    take(&mut e, "reach");
    next_offer(&mut e);
    assert_eq!(offered(&e), ["might"], "at its limit");
}

#[test]
fn holding_a_range_reward_leaves_another_offered() {
    let list = vec![range("reach", 10, 3), range("reach_more", 20, 3), might()];
    let mut e = with(&targets(2), list);
    next_offer(&mut e);
    assert_eq!(offered(&e), ["might", "reach", "reach_more"]);
    take(&mut e, "reach");
    next_offer(&mut e);
    assert_eq!(offered(&e), ["might", "reach", "reach_more"]);
    take(&mut e, "reach_more");
    assert_eq!(e.snapshot().player.range_pct, 30);
}

/// The expertise option `id` of `effect` and `amount`, common.
fn option(id: &str, effect: &str, amount: u32) -> Value {
    json!({ "id": id, "effect": effect, "amount": amount, "title": "Option", "description": "" })
}

#[test]
fn the_expertise_range_option_and_the_range_reward_add_into_one_widening() {
    // The `blade` as Main: its first step a slash, an arc of 32 pixels 8 pixels ahead after 4
    // ticks of wind-up; the `maul` as Sub. A kill of 10 points reaches expertise level 1, whose
    // offer opens before the kill's star's.
    let mut w = wave(
        1,
        1,
        0,
        enemy(1, hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    );
    w["expertise"] = json!({ "hit": 0, "kill": 10 });
    let stage = starry(armed(
        stage_full(18_000, Some(far_boss()), vec![w], vec![]),
        json!({ "preset": { "id": "blade" } }),
    ));
    let mut file: Value = serde_json::from_slice(Expertise::BUNDLED).unwrap();
    file["thresholds"] = json!([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
    for w in file["weapons"].as_array_mut().unwrap() {
        w["common"] = json!([option("wide", "range_pct", 20)]);
    }
    let expertise = Expertise::parse(file.to_string().as_bytes()).expect("the test expertise");
    let content = RunContent::new(
        RunStats::bundled().clone(),
        rewards(vec![range("reach", 30, 1)]),
    )
    .with_expertise(expertise);
    let mut e = Engine::new_with(&stage, &content).expect("the test stage is valid");
    let arc = |forward: i32, radius: i32| {
        hit_box(
            json!({ "type": "arc", "offset": { "forward": forward }, "radius": radius,
                        "half_angle": 15_360 }),
        )
    };

    e.step(idle());
    e.step(idle());
    assert_eq!(swing(&mut e), arc(2_048, 8_192));
    for _ in 0..30 {
        if e.offer_open() {
            break;
        }
        e.step(idle());
    }
    assert!(e.offer_open(), "the expertise offer");
    e.choose(0).unwrap();
    assert_eq!(
        e.events().first(),
        Some(&DomainEvent::ExpertiseOptionTaken {
            weapon: WeaponRole::Main,
            option: "wide".into(),
            taken: 1,
        })
    );
    assert!(e.offer_open(), "the star's offer");
    take(&mut e, "reach");
    assert_eq!(e.snapshot().player.range_pct, 30);

    // 20 + 30 percent: radius and offset half again.
    assert_eq!(swing(&mut e), arc(3_072, 12_288));
}

#[test]
fn the_enchantment_effect_is_gone() {
    // Range was the only enchantment; it is the `range_pct` effect now, and an `enchantment`
    // reward no longer parses, nor a reward holding an `enchantment` field.
    let file = |reward: Value| {
        json!({ "schema_version": 0, "sim_version": 0, "rewards": [reward] }).to_string()
    };
    let old = json!({
        "id": "reach", "rarity": "common", "effect": "enchantment", "amount": 20, "limit": 1,
        "title": "Reach", "description": "", "enchantment": { "type": "range" }
    });
    assert!(Rewards::parse(file(old).as_bytes()).is_err());
    let mut field = range("reach", 20, 1);
    field["enchantment"] = json!({ "type": "range" });
    assert!(Rewards::parse(file(field).as_bytes()).is_err());
}
