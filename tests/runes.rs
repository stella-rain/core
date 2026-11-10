//! Runes: a closed list built into `core`, held by the Main
//! weapon each at most once, and the random rune, a star reward that shows as a hidden
//! candidate and draws its rune from the run's generator only when it is picked, among the
//! runes not held. Once every rune is held it leaves the pool. What explosion, thunder,
//! boomerang, homing missiles and bleed do is `explosion.rs`'s, `thunder.rs`'s,
//! `boomerang.rs`'s, `homing_missiles.rs`'s and `bleed.rs`'s; poison and burn are bleed's kind,
//! `poison_burn.rs`'s.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::rewards::{RewardEffect, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::rune::{MAX_RUNES, Rune, RuneKind};
use stella_rain_core::stage::Stage;
use stella_rain_core::version::Versions;

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
            { "kind": "active", "ticks": 1, "damage": 10,
              "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 8_192 } },
            { "kind": "recovery", "ticks": 4 }
        ] } } },
        "window_ticks": 30,
        "assist_range": 0
    } })
}

/// `count` enemies of 10 hp (one step each), one every 6 ticks from tick 1, at `NEAR_Y`; every
/// kill drops a common star, taken at once; `rerolls` and `banishes` for the run.
fn targets(count: u16, rerolls: u8, banishes: u8) -> Stage {
    let waves = vec![wave(
        1,
        count,
        6,
        enemy(10, hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    )];
    let mut stage = armed(
        stage_full(18_000, Some(far_boss()), waves, vec![]),
        weapon(),
    );
    stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).unwrap());
    stage.stars = Some(
        serde_json::from_value(json!({
            "drop_per_mille": 1000, "pity": 1, "cap": 8, "pickup_radius": 12_288,
            "rerolls": rerolls, "banishes": banishes,
            "tiers": [{ "from_level": 1, "weights": { "common": 1 } }],
        }))
        .unwrap(),
    );
    stage
}

/// A common random rune.
fn random_rune() -> Value {
    json!({
        "id": "random_rune", "rarity": "common", "effect": "random_rune"
    })
}

/// A common reward `id` that changes nothing in the test stages: an evade charge, while the
/// stage's evade has no more charges to give.
fn inert(id: &str) -> Value {
    inert_limited(id, 20)
}

/// `inert`, taken at most `limit` times.
fn inert_limited(id: &str, limit: u8) -> Value {
    json!({
        "id": id, "rarity": "common", "effect": "evade_charge", "amount": 1, "limit": limit
    })
}

fn file(rewards: Vec<Value>) -> String {
    json!({ "rarities": common::rarities(), "rewards": rewards }).to_string()
}

fn rewards(list: Vec<Value>) -> Rewards {
    Rewards::parse(file(list).as_bytes(), Versions::CURRENT).expect("the test rewards validate")
}

fn content(list: Vec<Value>) -> RunContent {
    RunContent::new(RunStats::bundled().clone(), rewards(list))
}

fn with(stage: &Stage, list: Vec<Value>) -> Engine {
    Engine::new_with(stage, &content(list)).expect("the test stage is valid")
}

/// Fires until the next offer opens.
fn next_offer(e: &mut Engine) {
    fire_until(e, idle(), 600, Engine::offer_open);
    assert!(e.offer_open(), "no offer within 600 ticks");
}

#[test]
fn the_runes_are_a_closed_list_of_two_kinds() {
    assert_eq!(
        Rune::ALL,
        [
            Rune::Explosion,
            Rune::Thunder,
            Rune::Boomerang,
            Rune::HomingMissiles,
            Rune::Bleed,
            Rune::Poison,
            Rune::Burn
        ]
    );
    assert_eq!(MAX_RUNES, Rune::ALL.len());
    assert_eq!(Rune::Explosion.kind(), RuneKind::OnHit);
    assert_eq!(Rune::Thunder.kind(), RuneKind::OnHit);
    assert_eq!(Rune::Boomerang.kind(), RuneKind::OnHit);
    assert_eq!(Rune::HomingMissiles.kind(), RuneKind::OnHit);
    assert_eq!(Rune::Bleed.kind(), RuneKind::DamageOverTime);
    assert_eq!(Rune::Poison.kind(), RuneKind::DamageOverTime);
    assert_eq!(Rune::Burn.kind(), RuneKind::DamageOverTime);
    assert_eq!(serde_json::to_value(Rune::Explosion).unwrap(), "explosion");
    assert_eq!(serde_json::to_value(Rune::Thunder).unwrap(), "thunder");
    assert_eq!(serde_json::to_value(Rune::Boomerang).unwrap(), "boomerang");
    assert_eq!(
        serde_json::to_value(Rune::HomingMissiles).unwrap(),
        "homing_missiles"
    );
    assert_eq!(serde_json::to_value(Rune::Bleed).unwrap(), "bleed");
    assert_eq!(serde_json::to_value(Rune::Poison).unwrap(), "poison");
    assert_eq!(serde_json::to_value(Rune::Burn).unwrap(), "burn");
}

#[test]
fn a_random_rune_has_a_rarity_and_no_amount_or_limit() {
    let parsed = rewards(vec![random_rune()]);
    let r = &parsed.rewards()[0];
    assert_eq!(r.effect(), RewardEffect::RandomRune);
    assert_eq!((r.amount(), r.limit), (None, None));
    let refused = |field: &str, value: Value| {
        let mut reward = random_rune();
        reward[field] = value;
        let err = Rewards::parse(file(vec![reward]).as_bytes(), Versions::CURRENT).unwrap_err();
        let at = if field == "amount" {
            "effects[0].amount"
        } else {
            field
        };
        format!("{err:?}").contains(&format!("$.rewards[0].{at}"))
    };
    assert!(refused("amount", json!(1)), "an amount");
    assert!(refused("limit", json!(1)), "a limit");
}

#[test]
fn every_other_reward_still_needs_its_amount_and_limit() {
    for (field, path) in [("amount", "effects[0].amount"), ("limit", "limit")] {
        let mut reward = inert("dash");
        reward.as_object_mut().unwrap().remove(field);
        let err = Rewards::parse(file(vec![reward]).as_bytes(), Versions::CURRENT).unwrap_err();
        assert!(
            format!("{err:?}").contains(&format!("$.rewards[0].{path}")),
            "{field} missing: {err:?}"
        );
    }
}

#[test]
fn the_hidden_candidate_names_no_rune() {
    let mut e = with(&targets(1, 0, 0), vec![random_rune(), inert("dash")]);
    next_offer(&mut e);
    let snapshot = e.snapshot();
    let offer = snapshot.progress.as_ref().unwrap().offer.as_ref().unwrap();
    for c in &offer.candidates {
        assert_eq!(
            c.hidden,
            c.reward.as_str() == "random_rune",
            "{}",
            c.reward.as_str()
        );
    }
    assert!(snapshot.players[0].runes.is_empty(), "nothing drawn yet");
    let text = serde_json::to_string(&snapshot).unwrap();
    assert!(text.contains("\"hidden\":true"));
    for rune in [
        "explosion",
        "thunder",
        "boomerang",
        "homing_missiles",
        "bleed",
        "poison",
        "burn",
    ] {
        assert!(!text.contains(rune), "no rune in the snapshot");
    }
}
