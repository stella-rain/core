//! The range reward: a stat-like star reward, `range_pct`, with an amount, a
//! stack limit and a rarity like the others. Each take adds its amount to how much wider every
//! combo step's hit box is, its radius and offset (angle as written), and the expertise range
//! options add into the same widening; all of it together is held to `MAX_RANGE_PCT`. It stays
//! in the pool until its stack limit, and holding one range reward leaves another offered.
//! Every number here is worked out by hand.

use serde_json::{Value, json};
use stella_rain_core::attack::HitBox;
use stella_rain_core::engine::Engine;
use stella_rain_core::expertise::Expertise;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::rewards::{MAX_RANGE_PCT, RewardEffect, Rewards};
use stella_rain_core::snapshot::AttackView;
use stella_rain_core::version::Versions;

use crate::common;
use crate::common::equip::{Parts, equipped_in};

use common::*;

fn hit_box(v: Value) -> HitBox {
    serde_json::from_value(v).unwrap()
}

/// A common range reward `id`, `pct` percent wider, taken at most `limit` times.
fn range(id: &str, pct: u32, limit: u8) -> Value {
    json!({
        "id": id, "rarity": "common", "effect": "range_pct", "amount": pct, "limit": limit
    })
}

fn rewards(rewards: Vec<Value>) -> Rewards {
    let file = json!({ "rarities": common::rarities(), "rewards": rewards });
    Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT)
        .expect("the test rewards validate")
}

/// The combo step in progress (slot 0).
fn step_attack(e: &Engine) -> Option<AttackView> {
    e.snapshot().players[0]
        .clone()
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
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    e.step(&[
        Input {
            main: true,
            ..Input::default()
        },
        Default::default(),
        Default::default(),
    ]);
    for _ in 0..60 {
        let step = step_attack(e).expect("a combo step started");
        if let Some(h) = step.hit_box {
            return h;
        }
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    panic!("no active phase within 60 ticks")
}

#[test]
fn a_range_reward_is_a_stat_like_effect_bounded_like_the_others() {
    let parsed = rewards(vec![range("reach", MAX_RANGE_PCT, 20)]);
    assert_eq!(parsed.rewards()[0].effect(), RewardEffect::RangePct);
    let bad = |pct: u32, limit: u8| {
        let file =
            json!({ "rarities": common::rarities(), "rewards": [range("reach", pct, limit)] });
        Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT).is_err()
    };
    assert!(bad(0, 1), "an amount of 0");
    assert!(bad(MAX_RANGE_PCT + 1, 1), "an amount past MAX_RANGE_PCT");
    assert!(bad(10, 0), "a limit of 0");
    assert!(bad(10, 21), "a limit past MAX_STACK");
    assert!(!bad(10, 5), "a limit above 1");
}

/// The expertise range and the range reward add into one widening. The weapon tree (equipment
/// mode) gives the range: the first style's first upgrade is a `range_pct` of 20, taken by
/// `TreePick` once 210 points (levels 1 to 6) are seeded. The weapon's sets give no stat, so the
/// numbers are the star's 30 and the upgrade's 20.
#[test]
fn the_range_upgrade_and_the_range_star_add_into_one_widening() {
    // The `giant_sword` as Main: its first step a slash, an arc of 32 pixels 8 pixels ahead after
    // 4 ticks of wind-up. The one enemy is far away: it only gives the stage its points.
    let far = START_Y - 90_000;
    let mut w = wave(
        1,
        1,
        0,
        enemy(1_000_000, hold(START_X, far), json!([])),
        START_X,
        far,
    );
    w["expertise"] = json!({ "hit": 0, "kill": 10 });
    let mut stage = armed(
        stage_full(18_000, Some(far_boss()), vec![w], vec![]),
        json!({ "preset": { "id": "giant_sword" } }),
    );
    stage.sim_version = 2;
    let mut file: Value = serde_json::from_slice(common::bundled_file("expertise")).unwrap();
    file["thresholds"] = json!([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
    for w in file["weapons"].as_array_mut().unwrap() {
        if w["weapon"] != "giant_sword" {
            continue;
        }
        w["base"] = json!([]);
        for style in w["styles"].as_array_mut().unwrap() {
            style["stats"] = json!([]);
        }
        w["styles"][0]["upgrade_1"] =
            json!([{ "id": "wide", "effect": "range_pct", "amount": 20 }]);
    }
    let expertise = Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT)
        .expect("the test expertise");
    let arc = |forward: i32, radius: i32| {
        hit_box(
            json!({ "type": "arc", "offset": { "forward": forward }, "radius": radius,
                        "half_angle": 15_360 }),
        )
    };
    let build = |stars: &[(&str, usize)], points: u32| {
        let file = json!({ "rarities": common::rarities(), "rewards": [range("reach", 30, 2)] });
        let parts = Parts {
            expertise: Some(expertise.clone()),
            expertise_points: points,
            ..Parts::default()
        };
        equipped_in(&stage, file, parts, stars, 1)
    };
    // The step's own arc, with nothing equipped and no points.
    let mut bare = build(&[], 0);
    assert_eq!(swing(&mut bare), arc(2_048, 8_192));

    // The star alone widens it by 30 percent (reach is a mobility star, slot 8).
    let mut e = build(&[("reach", 8)], 210);
    e.step(&[idle(), Default::default(), Default::default()]);
    assert_eq!(e.snapshot().expertise.as_ref().map(|g| g.level), Some(6));
    assert_eq!(e.snapshot().players[0].range_pct, 30);
    assert_eq!(swing(&mut e), arc(2_662, 10_649));

    // The style, then its first upgrade (20 percent): 50 percent, radius and offset half again.
    let me = PlayerIdx::SOLO;
    e.act(me, EquipAction::TreePick { option: 0 })
        .expect("the style");
    e.act(me, EquipAction::TreePick { option: 0 })
        .expect("the upgrade");
    // The star's own 30 stays the player's; the upgrade's 20 is the weapon gauge's.
    assert_eq!(e.snapshot().players[0].range_pct, 30);
    assert_eq!(swing(&mut e), arc(3_072, 12_288));
}

#[test]
fn the_enchantment_effect_is_gone() {
    // Range was the only enchantment; it is the `range_pct` effect now, and an `enchantment`
    // reward no longer parses, nor a reward holding an `enchantment` field.
    let file =
        |reward: Value| json!({ "rarities": common::rarities(), "rewards": [reward] }).to_string();
    let old = json!({
        "id": "reach", "rarity": "common", "effect": "enchantment", "amount": 20, "limit": 2,
        "enchantment": { "type": "range" }
    });
    assert!(Rewards::parse(file(old).as_bytes(), Versions::CURRENT).is_err());
    let mut field = range("reach", 20, 2);
    field["enchantment"] = json!({ "type": "range" });
    assert!(Rewards::parse(file(field).as_bytes(), Versions::CURRENT).is_err());
}
