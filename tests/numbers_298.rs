//! The numbers of the game design pass of app#298 (Kade's picks of 2026-10-21, "C spiky"), as
//! the bundled content holds them: the runes, the star rewards and their caps, and the first
//! match-up profiles. Each is data in `content/` or `modes/`; none is a constant in code beyond
//! the ceiling the content may not pass (ADR-020, ADR-110).

use std::collections::BTreeSet;
use stella_rain_core::version::Versions;

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::rewards::{MAX_REWARDS, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::snapshot::OfferOption;

use crate::common;
use crate::common::run::{run_input, trial_setup};

fn bundled(text: &str) -> Value {
    serde_json::from_str(text).expect("a bundled file is JSON")
}

fn rune<'a>(file: &'a Value, list: &str, id: &str) -> &'a Value {
    file[list]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["rune"] == id)
        .unwrap_or_else(|| panic!("{id} in {list}"))
}

#[test]
fn the_runes_have_the_decided_numbers() {
    let file = bundled(std::str::from_utf8(common::bundled_file("runes")).unwrap());
    let on_hit = |id: &str| rune(&file, "on_hit", id);
    let e = on_hit("explosion");
    assert_eq!(
        (&e["cooldown_ticks"], &e["damage"]),
        (&json!(90), &json!(2500))
    );
    assert_eq!(e["attack"][0]["hit_box"]["radius"], 12_288);
    let t = on_hit("thunder");
    assert_eq!(
        (&t["cooldown_ticks"], &t["damage"]),
        (&json!(60), &json!(3000))
    );
    let b = on_hit("boomerang");
    assert_eq!(
        (&b["cooldown_ticks"], &b["damage"]),
        (&json!(120), &json!(1800))
    );
    let h = on_hit("homing_missiles");
    assert_eq!(
        (&h["cooldown_ticks"], &h["damage"], &h["missiles"]),
        (&json!(150), &json!(1000), &json!(4))
    );
    for id in ["bleed", "poison", "burn"] {
        let d = rune(&file, "damage_over_time", id);
        assert_eq!(
            (&d["interval_ticks"], &d["damage"], &d["max_stacks"]),
            (&json!(20), &json!(300), &json!(6)),
            "{id}"
        );
    }
}

fn reward<'a>(file: &'a Value, id: &str) -> &'a Value {
    file["rewards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("reward {id}"))
}

#[test]
fn the_star_rewards_have_the_decided_amounts() {
    let file = bundled(std::str::from_utf8(common::bundled_file("rewards")).unwrap());
    let amounts = |prefix: &str, rarities: &[&str]| -> Vec<Value> {
        rarities
            .iter()
            .map(|r| reward(&file, &format!("{prefix}_{r}"))["amount"].clone())
            .collect()
    };
    let three = ["common", "rare", "legendary"];
    assert_eq!(
        amounts("attack_speed", &three),
        [8, 20, 40].map(Value::from)
    );
    // The rarities between and above keep their ratio to the decided three (a placeholder
    // until the numbers pass for them).
    assert_eq!(
        amounts("attack_speed", &["uncommon", "epic", "supernova"]),
        [12, 28, 64].map(Value::from)
    );
    // Rune Haste, range and the type bonus hold the decided three; the six-rarity rows around
    // them are tested in `rarity_rows_d6c`.
    assert_eq!(
        amounts("rune_haste", &three),
        [25, 50, 100].map(Value::from)
    );
    assert_eq!(amounts("range", &three), [15, 30, 50].map(Value::from));
    assert_eq!(
        amounts("type_bonus_slash", &three),
        [25, 50, 80].map(Value::from)
    );
    assert_eq!(file["max_rune_haste_pct"], 500);
}

#[test]
fn the_run_stats_and_match_ups_hold_their_caps() {
    let stats = bundled(std::str::from_utf8(common::bundled_file("run_stats")).unwrap());
    assert_eq!(stats["max_attack_speed_pct"], 500);
    let ups = bundled(std::str::from_utf8(common::bundled_file("match_ups")).unwrap());
    assert_eq!(ups["max_type_bonus_pct"], 1000);
}

#[test]
fn the_new_rewards_are_ordinary_ones_the_random_rune_apart() {
    let file = bundled(std::str::from_utf8(common::bundled_file("rewards")).unwrap());
    for (effect, kinds) in [("rune_haste", 6), ("range_pct", 6), ("type_bonus_pct", 60)] {
        let found: Vec<&Value> = file["rewards"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["effect"] == effect)
            .collect();
        assert_eq!(found.len(), kinds, "{effect}");
        for r in found {
            assert!(
                r["limit"].as_u64().unwrap() >= 2,
                "{r}: ordinary rewards stack"
            );
            assert!(r.get("named").is_none(), "{r}");
        }
    }
    let rune: Vec<&Value> = file["rewards"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["effect"] == "random_rune")
        .collect();
    assert_eq!(rune.len(), 1);
    assert_eq!(rune[0]["rarity"], "common");
    assert!(
        rune[0].get("limit").is_none(),
        "it leaves once every rune is held"
    );
    assert!(Rewards::bundled().rewards().len() <= MAX_REWARDS);
}

#[test]
fn a_rewards_file_may_hold_the_most_rewards_and_not_one_more() {
    let one = |i: usize| {
        json!({ "id": format!("r{i}"), "rarity": "common", "effect": "attack_pct", "amount": 5,
                "limit": 5, "title": "T", "description": "" })
    };
    let parse = |n: usize| {
        let list: Vec<Value> = (0..n).map(one).collect();
        let file = json!({ "rarities": common::rarities(), "rewards": list });
        Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT)
    };
    assert_eq!(MAX_REWARDS, 192);
    assert!(parse(MAX_REWARDS).is_ok());
    assert!(parse(MAX_REWARDS + 1).is_err());
}

/// The trial's and the Time Limit's first profiles (ADR-100): the enemies of the decided hit
/// points that held the old profile now hold the decided one.
#[test]
fn the_first_match_up_profiles_are_the_decided_ones() {
    for (mode, text) in [
        (
            "trial",
            std::str::from_utf8(common::bundled_file("trial")).unwrap(),
        ),
        (
            "time_limit",
            std::str::from_utf8(common::bundled_file("time_limit")).unwrap(),
        ),
    ] {
        let file = bundled(text);
        let mut enemies: Vec<&Value> = file["waves"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| &w["enemy"])
            .collect();
        enemies.extend(
            file["elite_types"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| &e["enemy"]),
        );
        enemies.extend(file["bosses"].as_array().into_iter().flatten());
        enemies.extend(file.get("boss"));
        for e in enemies {
            let profile = &e["profile"];
            match e["hp"].as_u64() {
                Some(3000) => assert_eq!(profile["weak"], json!(["slash", "electro"]), "{mode}"),
                Some(24_000) => assert_eq!(profile["resist"], "elements", "{mode}"),
                Some(300_000) => {
                    assert_eq!(profile["weak"], json!(["strike", "pyro"]), "{mode}");
                    assert_eq!(profile["resist"], "physical", "{mode}");
                }
                _ => {}
            }
        }
    }
}

/// Every offer a run of the trial opens over `seeds`, taking the first candidate each time:
/// the IDs of the rewards offered.
fn offered(seeds: std::ops::Range<u64>, ticks: u32) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    for seed in seeds {
        let setup = RunSetup {
            seed,
            ..trial_setup()
        };
        let mut e = Engine::start_with(&setup, RunContent::bundled()).unwrap();
        for t in 0..ticks {
            while let Some(offer) = e.snapshot().progress.and_then(|p| p.offer) {
                seen.extend(offer.candidates.iter().map(|c| c.reward.0.clone()));
                let take = offer
                    .options
                    .iter()
                    .position(|o| matches!(o, OfferOption::Take { .. }))
                    .unwrap();
                e.choose(u32::try_from(take).unwrap()).unwrap();
            }
            if e.offer_open() {
                break;
            }
            let input = run_input(&e, t);
            e.step(input);
        }
    }
    seen
}

#[test]
fn a_run_can_be_offered_each_new_reward_kind() {
    let seen = offered(1..25, 2400);
    for id in ["random_rune", "rune_haste_common", "range_common"] {
        assert!(seen.contains(id), "{id} is offered in some run: {seen:?}");
    }
    // Ten families weigh 10 each, so which damage type comes first is a matter of the seeds.
    assert!(
        seen.iter().any(|id| id.starts_with("type_bonus_")),
        "a type bonus is offered in some run: {seen:?}"
    );
}
