//! The six-rarity rows, the random rune's share per level tier and the type bonus per damage
//! type (D6c, app#336 DS6 item 7; core#264 placeholders 1 to 3). Every ordinary reward family
//! holds all six rarities; a tier of the drop table may set the random rune's draw weight
//! (`random_rune_weight`); and the ten damage types each have a bonus family, which replaces
//! the two group families. The amounts are number set A, a placeholder until Kade picks.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use stella_rain_core::attack::DamageType;
use stella_rain_core::rewards::{MAX_REWARDS, MAX_WEIGHT, Rewards};
use stella_rain_core::stage::Tier;
use stella_rain_core::validate::{self, Error};

use crate::reward_kinds::{near, next_offer, offer, plain, with};

const RARITIES: [&str; 6] = [
    "common",
    "uncommon",
    "rare",
    "epic",
    "legendary",
    "supernova",
];
/// R3 (core#246): how often one run takes a reward of each rarity.
const LIMITS: [u64; 6] = [5, 4, 3, 2, 2, 2];

fn bundled() -> Value {
    serde_json::from_str(include_str!("../content/rewards.json")).expect("a bundled file is JSON")
}

fn rows(file: &Value) -> &Vec<Value> {
    file["rewards"].as_array().unwrap()
}

/// The effect a reward of one effect has, written in the file's released form or as a list.
fn effect_of(r: &Value) -> &Value {
    r.get("effects").map_or(r, |list| &list[0])
}

/// The ordinary rewards (no named star, no stage skill, no random rune) by family: the effect
/// and its damage type, each with its rows in file order as `(rarity, amount, limit, weight)`.
type Rows = Vec<(String, u64, u64, u64)>;

fn families(file: &Value) -> BTreeMap<(String, String), Rows> {
    let mut out: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for r in rows(file) {
        let e = effect_of(r);
        let effect = e["effect"].as_str().unwrap();
        if r.get("named").is_some() || matches!(effect, "stage_skill" | "random_rune") {
            continue;
        }
        let key = (
            effect.to_owned(),
            e.get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        );
        out.entry(key).or_default().push((
            r["rarity"].as_str().unwrap().to_owned(),
            e["amount"].as_u64().unwrap(),
            r["limit"].as_u64().unwrap(),
            r.get("weight").and_then(Value::as_u64).unwrap_or(100),
        ));
    }
    out
}

fn type_name(ty: DamageType) -> String {
    serde_json::to_value(ty)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn every_ordinary_family_has_exactly_the_six_rarities() {
    for (family, found) in families(&bundled()) {
        let got: Vec<&str> = found.iter().map(|f| f.0.as_str()).collect();
        let mut sorted = got.clone();
        sorted.sort_by_key(|r| RARITIES.iter().position(|x| x == r));
        assert_eq!(sorted, RARITIES, "{family:?}");
    }
}

fn amounts(file: &Value, effect: &str, ty: &str) -> Vec<u64> {
    let found = &families(file)[&(effect.to_owned(), ty.to_owned())];
    RARITIES
        .iter()
        .map(|r| found.iter().find(|f| f.0 == *r).unwrap().1)
        .collect()
}

#[test]
fn rune_haste_range_and_the_type_bonuses_hold_number_set_a() {
    let file = bundled();
    assert_eq!(amounts(&file, "rune_haste", ""), [25, 38, 50, 70, 100, 160]);
    assert_eq!(amounts(&file, "range_pct", ""), [15, 23, 30, 42, 50, 80]);
    for ty in DamageType::ALL {
        assert_eq!(
            amounts(&file, "type_bonus_pct", &type_name(ty)),
            [25, 38, 50, 70, 80, 128],
            "{ty:?}"
        );
    }
    for (family, found) in families(&file) {
        if matches!(
            family.0.as_str(),
            "rune_haste" | "range_pct" | "type_bonus_pct"
        ) {
            let limits: Vec<u64> = RARITIES
                .iter()
                .map(|r| found.iter().find(|f| f.0 == *r).unwrap().2)
                .collect();
            assert_eq!(limits, LIMITS, "{family:?}");
        }
    }
}

#[test]
fn there_is_one_type_bonus_family_per_damage_type_and_no_group_family() {
    let file = bundled();
    let all = families(&file);
    for ty in DamageType::ALL {
        let found = &all[&("type_bonus_pct".to_owned(), type_name(ty))];
        assert_eq!(found.len(), 6, "{ty:?}");
        // Ten families together are offered about as often as one ordinary family.
        assert!(found.iter().all(|f| f.3 == 10), "{ty:?}");
    }
    let effects: Vec<&str> = rows(&file)
        .iter()
        .flat_map(|r| {
            r.get("effects")
                .map_or(vec![r], |l| l.as_array().unwrap().iter().collect())
        })
        .map(|e| e["effect"].as_str().unwrap())
        .collect();
    for gone in ["attack_type_bonus_pct", "element_bonus_pct"] {
        assert!(!effects.contains(&gone), "{gone} is no longer in content");
    }
    // 75 - 6 + 60 + 6 rows.
    assert_eq!(rows(&file).len(), 135);
}

// --- The random rune's weight per tier -----------------------------------------------------------

/// How many of `seeds` offers hold the random rune first, in a stage whose one tier has
/// `tier_weight`, with the random rune weighing `rune_weight` and one other common 1.
fn rune_first(tier_weight: Option<u16>, rune_weight: u32, seeds: u32) -> u32 {
    let mut first = 0;
    for seed in 0..seeds {
        let mut stage = near(2);
        stage.seed = seed;
        stage.stars.as_mut().unwrap().tiers[0].random_rune_weight = tier_weight;
        let list = vec![
            json!({ "id": "rune", "rarity": "common", "effect": "random_rune",
                    "weight": rune_weight, "title": "Rune", "description": "" }),
            plain("other", 1),
        ];
        let mut e = with(&stage, list);
        next_offer(&mut e);
        first += u32::from(offer(&e).candidates[0].reward.0 == "rune");
    }
    first
}

#[test]
fn a_tier_sets_the_random_runes_draw_weight_while_it_applies() {
    // The reward weighs 1 against 1: about half. The tier's 1000 makes it nearly always first.
    let own = rune_first(None, 1, 40);
    assert!((8..=32).contains(&own), "{own} of 40 at weight 1 against 1");
    let raised = rune_first(Some(1_000), 1, 40);
    assert!(raised >= 38, "{raised} of 40 at the tier's 1000 against 1");
}

#[test]
fn a_tier_without_the_field_uses_the_rewards_own_weight() {
    // The reward weighs 1000 against 1: nearly always first; a tier of 1 takes it back to half.
    let own = rune_first(None, 1_000, 40);
    assert!(own >= 38, "{own} of 40 at the reward's 1000 against 1");
    let lowered = rune_first(Some(1), 1_000, 40);
    assert!(
        (8..=32).contains(&lowered),
        "{lowered} of 40 at the tier's 1"
    );
}

fn stage_with_tier_field(value: Value) -> Vec<String> {
    let mut stage = near(2);
    let mut tier = serde_json::to_value(&stage.stars.as_ref().unwrap().tiers[0]).unwrap();
    tier["random_rune_weight"] = value;
    stage.stars.as_mut().unwrap().tiers = vec![serde_json::from_value(tier).unwrap()];
    match validate::validate(&stage) {
        Ok(()) => Vec::new(),
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        Err(e) => panic!("{e:?}"),
    }
}

#[test]
fn the_tiers_random_rune_weight_is_validated_like_a_weight() {
    let path = "$.stars.tiers[0].random_rune_weight";
    assert!(stage_with_tier_field(json!(1)).is_empty());
    assert!(stage_with_tier_field(json!(MAX_WEIGHT)).is_empty());
    assert_eq!(stage_with_tier_field(json!(0)), [path]);
    assert_eq!(stage_with_tier_field(json!(MAX_WEIGHT + 1)), [path]);
}

#[test]
fn a_tier_without_the_field_round_trips_to_the_same_bytes() {
    let old = r#"{"from_level":3,"weights":{"common":70,"uncommon":15,"rare":30,"epic":0,"legendary":0,"supernova":0}}"#;
    let tier: Tier = serde_json::from_str(old).unwrap();
    assert_eq!(tier.random_rune_weight, None);
    assert_eq!(serde_json::to_string(&tier).unwrap(), old);
}

// --- The bundled tiers ---------------------------------------------------------------------------

#[test]
fn the_bundled_modes_give_the_random_rune_five_fifteen_and_twenty_five_percent() {
    // The commons besides the rune weigh 9 x 100 + 10 x 10 = 1000; w / (1000 + w) is 5.03,
    // 14.97 and 24.99 percent at 53, 176 and 333.
    let rewards = Rewards::bundled();
    let others: u32 = rewards
        .rewards()
        .iter()
        .filter(|r| r.rarity == stella_rain_core::rewards::Rarity::Common && !r.is_random_rune())
        .map(|r| u32::from(r.weight))
        .sum();
    assert_eq!(others, 1_000);
    for text in [
        include_str!("../modes/trial.json"),
        include_str!("../modes/tutorial.json"),
    ] {
        let mode: Value = serde_json::from_str(text).unwrap();
        let weights: Vec<Value> = mode["stars"]["tiers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["random_rune_weight"].clone())
            .collect();
        assert_eq!(weights, [json!(53), json!(176), json!(333)]);
    }
}

// --- The most rewards ----------------------------------------------------------------------------

#[test]
fn a_rewards_file_may_hold_192_rewards_and_not_one_more() {
    assert_eq!(MAX_REWARDS, 192);
    let parse = |n: usize| {
        let list: Vec<Value> = (0..n)
            .map(|i| {
                json!({ "id": format!("r{i}"), "rarity": "common", "effect": "attack_pct",
                        "amount": 5, "limit": 5, "title": "T", "description": "" })
            })
            .collect();
        let file = json!({ "schema_version": 0, "sim_version": 0, "rewards": list });
        Rewards::parse(file.to_string().as_bytes())
    };
    assert!(parse(192).is_ok());
    assert!(parse(193).is_err());
}

#[test]
fn the_bundled_file_fits_the_size_limit_with_room() {
    let size = include_bytes!("../content/rewards.json").len();
    assert!(
        size <= stella_rain_core::rewards::MAX_REWARDS_BYTES,
        "{size}"
    );
}
