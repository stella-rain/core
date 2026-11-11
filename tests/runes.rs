//! Runes: a closed list built into `core`, held by the Main
//! weapon each at most once, and the random rune, a star reward that shows as a hidden
//! candidate and draws its rune from the run's generator only when it is picked, among the
//! runes not held. Once every rune is held it leaves the pool. What explosion, thunder,
//! boomerang, homing missiles and bleed do is `explosion.rs`'s, `thunder.rs`'s,
//! `boomerang.rs`'s, `homing_missiles.rs`'s and `bleed.rs`'s; poison and burn are bleed's kind,
//! `poison_burn.rs`'s.

use serde_json::{Value, json};
use stella_rain_core::rewards::{RewardEffect, Rewards};
use stella_rain_core::rune::{MAX_RUNES, Rune, RuneKind};
use stella_rain_core::version::Versions;

use crate::common;

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
