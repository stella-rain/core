//! The six rarities (ADR-REPL-5): every effect that scales with rarity has a reward
//! at each of them, the new amounts follow the ladder from the common reward (rounded down), the
//! supernova reward takes a bigger amount and no bigger stack limit than legendary, every
//! difficulty offers each rarity it allows, the top drop tier weighs them all, and an offer at
//! supernova falls to legendary once the supernova rewards are taken. Every number here is worked
//! out by hand.

use serde_json::{Value, json};
use stella_rain_core::difficulties::Difficulties;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::rewards::{MAX_AMOUNT, MAX_HP_AMOUNT, Rarity, Reward, RewardEffect, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::version::Versions;

use crate::common;

/// The effects that scale with rarity, from the common reward's amount (the ladder's base).
const FAMILIES: [RewardEffect; 7] = [
    RewardEffect::MaxHp,
    RewardEffect::AttackPct,
    RewardEffect::AttackSpeedPct,
    RewardEffect::EvadePct,
    RewardEffect::SpeedPct,
    RewardEffect::XpPct,
    RewardEffect::Regen,
];

/// The rarities that come with the six-rarity pool: uncommon, epic and supernova.
const NEW: [Rarity; 3] = [Rarity::Uncommon, Rarity::Epic, Rarity::Supernova];

/// The bundled reward of `effect` at `rarity`.
fn bundled(effect: RewardEffect, rarity: Rarity) -> &'static Reward {
    Rewards::bundled()
        .rewards()
        .iter()
        .find(|r| r.effect() == effect && r.rarity == rarity)
        .unwrap_or_else(|| panic!("no {effect:?} reward at {rarity:?}"))
}

/// The amount the ladder gives a new rarity, from the common reward's `common`: uncommon
/// `× 1.5`, epic `× 3.5` (both rounded down) and supernova `× 8`.
fn ladder(rarity: Rarity, common: u32) -> u32 {
    match rarity {
        Rarity::Uncommon => common * 3 / 2,
        Rarity::Epic => common * 7 / 2,
        Rarity::Supernova => common * 8,
        _ => unreachable!("only the new rarities are on the ladder"),
    }
}

/// The stack limit each new rarity gets: uncommon 4, epic 2 and supernova 2.
fn limit_of(rarity: Rarity) -> u8 {
    match rarity {
        Rarity::Uncommon => 4,
        Rarity::Epic => 2,
        Rarity::Supernova => 2,
        _ => unreachable!("only the new rarities have a ladder limit"),
    }
}

#[test]
fn every_scaling_effect_has_a_reward_at_all_six_rarities() {
    for effect in FAMILIES {
        for rarity in Rarity::ALL {
            let found = Rewards::bundled()
                .rewards()
                .iter()
                .filter(|r| !r.named && r.effect() == effect && r.rarity == rarity)
                .count();
            assert_eq!(found, 1, "{effect:?} at {rarity:?}");
        }
    }
}

#[test]
fn the_new_amounts_follow_the_ladder_from_the_common_reward() {
    for effect in FAMILIES {
        let common = bundled(effect, Rarity::Common).amount().unwrap();
        for rarity in NEW {
            let r = bundled(effect, rarity);
            assert_eq!(
                r.amount(),
                Some(ladder(rarity, common)),
                "{effect:?} {rarity:?}"
            );
            assert_eq!(r.limit, Some(limit_of(rarity)), "{effect:?} {rarity:?}");
        }
    }
}

#[test]
fn every_new_amount_is_inside_its_bound() {
    for effect in FAMILIES {
        let bound = match effect {
            RewardEffect::MaxHp | RewardEffect::Regen => MAX_HP_AMOUNT,
            _ => MAX_AMOUNT,
        };
        for rarity in Rarity::ALL {
            let amount = bundled(effect, rarity).amount().unwrap();
            assert!(
                (1..=bound).contains(&amount),
                "{effect:?} {rarity:?}: {amount}"
            );
        }
    }
}

#[test]
fn supernova_takes_a_bigger_amount_and_no_bigger_limit_than_legendary() {
    // Legendary and supernova both stack twice (app#336): the bigger amount is what sets
    // supernova apart, never a bigger limit.
    for effect in FAMILIES {
        let sup = bundled(effect, Rarity::Supernova);
        let leg = bundled(effect, Rarity::Legendary);
        assert!(sup.amount() > leg.amount(), "{effect:?}");
        assert!(sup.limit <= leg.limit, "{effect:?}");
        assert_eq!(sup.limit, Some(2), "{effect:?}");
    }
}

#[test]
fn every_difficulty_offers_each_rarity_it_allows() {
    let rewards = Rewards::bundled();
    for d in Difficulties::bundled().list() {
        let range = d.rarity_range(rewards);
        assert_eq!(range, (d.rarity_min, d.rarity_max), "{}", d.id.as_str());
        for rarity in Rarity::ALL {
            if (d.rarity_min..=d.rarity_max).contains(&rarity) {
                assert!(
                    rewards.rewards().iter().any(|r| r.rarity == rarity),
                    "{} has no {rarity:?} reward",
                    d.id.as_str()
                );
            }
        }
    }
}

#[test]
fn the_top_drop_tier_of_trial_and_tutorial_weighs_every_rarity() {
    for stage in [
        RunContent::bundled().trial(),
        RunContent::bundled().tutorial(),
    ] {
        let tiers = &stage.stars.as_ref().expect("a stage with stars").tiers;
        let top = tiers.last().expect("a tier");
        for rarity in Rarity::ALL {
            assert!(top.weights.get(rarity) > 0, "{rarity:?} at the top tier");
        }
    }
}

#[test]
fn the_bundled_rewards_still_parse_and_bound_their_new_limits() {
    let bad = |limit: u8| {
        let mut v: Value = serde_json::from_slice(common::bundled_file("rewards")).unwrap();
        let rewards = v["rewards"].as_array_mut().unwrap();
        let r = rewards
            .iter_mut()
            .find(|r| r["id"] == "attack_supernova")
            .unwrap();
        r["limit"] = json!(limit);
        Rewards::parse(v.to_string().as_bytes(), Versions::CURRENT).is_err()
    };
    assert!(bad(0), "a limit of 0");
    assert!(bad(21), "a limit past MAX_STACK");
    assert!(!bad(2), "the bundled limit of 2");
}

/// A run setup on `difficulty`, the `time_limit` mode with the bundled content.
fn setup(difficulty: &str) -> RunSetup {
    RunSetup {
        schema_version: 1,
        sim_version: 1,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        weapon_tree: None,
        seed: 5,
        star_pool: None,
        extra_rerolls: 0,
        upgrades: Vec::new(),
        perk_set: "duelist".into(),
        difficulty: difficulty.into(),
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
        stat_priority: Vec::new(),
        ..RunSetup::default()
    }
}

/// The rarities of the stars that drop in a scripted `time_limit` run on `difficulty`.
fn star_rarities(difficulty: &str, ticks: u32) -> Vec<Rarity> {
    let mut e = Engine::start(&setup(difficulty)).unwrap();
    let mut out = Vec::new();
    for t in 0..ticks {
        if e.outcome() != Outcome::Running {
            break;
        }
        let input = crate::common::run::run_input(&e, t);
        e.step(&[input, Default::default(), Default::default()]);
        for ev in e.events() {
            if let DomainEvent::StarDropped { rarity, .. } = ev {
                out.push(*rarity);
            }
        }
    }
    out
}

#[test]
fn stars_drop_only_inside_each_difficultys_range_with_all_six_rarities_in_the_pool() {
    let easy = star_rarities("easy", 3_000);
    assert!(!easy.is_empty());
    assert!(easy.iter().all(|r| *r <= Rarity::Rare), "{easy:?}");
    let normal = star_rarities("normal", 3_000);
    assert!(normal.iter().all(|r| *r <= Rarity::Legendary), "{normal:?}");
    let hard = star_rarities("hard", 3_000);
    assert!(hard.iter().all(|r| *r <= Rarity::Supernova), "{hard:?}");
    let rainy = star_rarities("rainy", 3_000);
    assert!(!rainy.is_empty());
    assert!(rainy.iter().all(|r| *r >= Rarity::Uncommon), "{rainy:?}");
}
