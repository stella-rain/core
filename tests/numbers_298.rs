//! The numbers of the game design pass of app#298 (Kade's picks of 2026-10-21, "C spiky"), as
//! the bundled content holds them: the runes, the star rewards and their caps, and the first
//! match-up profiles. Each is data in `content/` or `modes/`; none is a constant in code beyond
//! the ceiling the content may not pass (ADR-SVC-1).

use stella_rain_core::version::Versions;

use serde_json::{Value, json};
use stella_rain_core::rewards::{MAX_REWARDS, Rewards};

use crate::common;

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
    for (effect, kinds) in [("rune_haste", 12), ("range_pct", 6), ("type_bonus_pct", 60)] {
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
                "limit": 5 })
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

/// The trial's and the Time Limit's first profiles: the enemies of the decided hit
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

/// The combat picks (ADR-CONT-3, ADR-CONT-4): every regular wave of the Time Limit and the
/// trial holds two weak types, each damage type is some wave's or boss's weakness, and the picks
/// that keep today's value still hold it.
#[test]
fn the_combat_picks_are_applied() {
    use stella_rain_core::attack::DamageType;

    let weak = |e: &Value| e["profile"]["weak"].clone();
    let time_limit = bundled(std::str::from_utf8(common::bundled_file("time_limit")).unwrap());
    let trial = bundled(std::str::from_utf8(common::bundled_file("trial")).unwrap());
    let tutorial = bundled(std::str::from_utf8(common::bundled_file("tutorial")).unwrap());

    let waves = time_limit["waves"].as_array().unwrap();
    for (i, pair) in [
        (2, ["slash", "hydro"]),
        (3, ["strike", "geo"]),
        (4, ["pierce", "anemo"]),
        (6, ["slash", "cryo"]),
        (8, ["strike", "pyro"]),
    ] {
        assert_eq!(weak(&waves[i]["enemy"]), json!(pair), "time_limit wave {i}");
    }
    assert!(
        weak(&waves[5]["enemy"]).is_null(),
        "the tank has no weakness"
    );
    let trial_waves = trial["waves"].as_array().unwrap();
    assert_eq!(weak(&trial_waves[2]["enemy"]), json!(["slash", "hydro"]));
    for (mode, file) in [("time_limit", &time_limit), ("trial", &trial)] {
        for (i, w) in file["waves"].as_array().unwrap().iter().enumerate() {
            if let Some(list) = weak(&w["enemy"]).as_array() {
                assert_eq!(list.len(), 2, "{mode} wave {i}");
            }
        }
    }

    let mut seen: Vec<String> = Vec::new();
    let mut enemies: Vec<&Value> = waves.iter().map(|w| &w["enemy"]).collect();
    enemies.extend(time_limit["bosses"].as_array().into_iter().flatten());
    enemies.extend(time_limit.get("boss"));
    for e in enemies {
        for t in e["profile"]["weak"].as_array().into_iter().flatten() {
            seen.push(t.as_str().unwrap().to_string());
        }
    }
    for t in DamageType::ALL {
        let name = serde_json::to_value(t).unwrap();
        assert!(
            seen.iter().any(|s| name == *s),
            "{name} is nobody's weakness"
        );
    }

    for w in tutorial["waves"].as_array().unwrap() {
        assert_eq!(
            weak(&w["enemy"]),
            json!(["slash"]),
            "the tutorial stays single"
        );
    }
    let ups = bundled(std::str::from_utf8(common::bundled_file("match_ups")).unwrap());
    for k in ["weak_pct", "strong_pct", "resisted_pct"] {
        assert_eq!(ups[k], 20, "{k}");
    }
    assert_eq!(ups["max_type_bonus_pct"], 1000);
    let rewards = bundled(std::str::from_utf8(common::bundled_file("rewards")).unwrap());
    let equipment = &rewards["equipment"];
    for c in equipment["categories"].as_array().unwrap() {
        assert_eq!(c["drop_weight_max"], 100, "{}", c["id"]);
        assert_eq!(c["drop_weight_default"], 0, "{}", c["id"]);
    }
    assert_eq!(equipment["rune_share_permille"], 100);
}

/// The economy picks (ADR-CONT-3, ADR-CONT-4): the Star Chart prices in tenths of Stardust, the
/// Lumine levels, the Starlight craft price at three times the extract, the rune extract and the
/// boss bonus and drops; the Stardust shares and bank are kept.
#[test]
fn the_economy_picks_are_applied() {
    let chart = bundled(std::str::from_utf8(common::bundled_file("star_chart")).unwrap());
    let num = |v: &Value| v.as_u64().unwrap();
    let nums = |v: &Value| -> Vec<u64> { v.as_array().unwrap().iter().map(num).collect() };
    let priced = |list: &str, price: u64| {
        let items = chart[list].as_array().unwrap();
        for item in items {
            assert_eq!(num(&item["price"]), price, "{list} {}", item["id"]);
        }
        items.len() as u64 * price
    };
    let mut total = 0;
    assert_eq!(chart["weapons"].as_array().unwrap().len(), 1);
    total += priced("weapons", 12000);
    assert_eq!(chart["perk_sets"].as_array().unwrap().len(), 3);
    total += priced("perk_sets", 8000);
    assert_eq!(chart["stage_skills"].as_array().unwrap().len(), 4);
    total += priced("stage_skills", 6000);
    assert_eq!(num(&chart["starting_rune"]), 6000);
    total += 6000;
    let stats = chart["stats"].as_array().unwrap();
    assert_eq!(stats.len(), 5);
    for s in stats {
        assert_eq!(
            nums(&s["prices"]),
            [600, 1200, 1800, 2400, 3000],
            "{}",
            s["id"]
        );
        total += nums(&s["prices"]).iter().sum::<u64>();
    }
    let expertise = nums(&chart["expertise_multiplier"]);
    assert_eq!(expertise, [3000, 6000, 9000, 12000, 15000]);
    total += expertise.iter().sum::<u64>();
    assert_eq!(total, 156_000);
    let lumine = nums(&chart["lumine_levels"]);
    assert_eq!(
        lumine,
        (1..=10).map(|k| 10_000 * k).collect::<Vec<u64>>(),
        "Lumine level k costs 1,000 x k shown"
    );
    assert_eq!(lumine.iter().sum::<u64>(), 550_000);

    let rewards = bundled(std::str::from_utf8(common::bundled_file("rewards")).unwrap());
    let equipment = &rewards["equipment"];
    let rarities = equipment["rarities"].as_array().unwrap();
    let extract: Vec<u64> = rarities.iter().map(|r| num(&r["extract"])).collect();
    let craft: Vec<u64> = rarities.iter().map(|r| num(&r["craft_price"])).collect();
    assert_eq!(extract, [5, 10, 20, 40, 80, 160]);
    assert_eq!(craft, [15, 30, 60, 120, 240, 480]);
    for (c, e) in craft.iter().zip(&extract) {
        assert_eq!(*c, 3 * e, "a craft costs three times the extract");
    }
    assert_eq!(equipment["rune_extract"], 20);
    assert_eq!(equipment["boss_bonus"], 150);
    assert_eq!(equipment["boss_drops"], 2);
    assert_eq!(equipment["rune_share_permille"], 100);
    let lumine_cost: Vec<u64> = rarities.iter().map(|r| num(&r["lumine_cost"])).collect();
    assert_eq!(lumine_cost, [1, 2, 3, 4, 6, 10]);

    // The Stardust shares and the bank are kept for both modes.
    let stardust = bundled(std::str::from_utf8(common::bundled_file("stardust")).unwrap());
    let modes = stardust["modes"].as_array().unwrap();
    assert_eq!(modes.len(), 2);
    for m in modes {
        assert_eq!(m["bank"], true, "{}", m["id"]);
        assert_eq!(
            (
                m["pay_pct"].clone(),
                m["bank_pct"].clone(),
                m["bank_cap"].clone()
            ),
            (json!(50), json!(50), json!(1000)),
            "{}",
            m["id"]
        );
    }
}
