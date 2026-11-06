//! The equipment rules, the reward flags and the weapons' base and tree sets are pack data
//! (ADR-CONT-1, ADR-CONT-2, ADR-CONT-3): read as untrusted bytes, strictly parsed, every number
//! bounded (ADR-SVC-1). The bundled pack holds the placeholders' own values.

use crate::common;
use crate::common::run::trial_setup;
use serde_json::{Value, json};
use stella_rain_core::expertise::{Expertise, MAX_SET_STATS, SetStat};
use stella_rain_core::pack::Pack;
use stella_rain_core::perk_sets::PerkSets;
use stella_rain_core::registry;
use stella_rain_core::rewards::{Rarity, RewardEffect, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::{self, BuildMode, RunSetup};
use stella_rain_core::run_stats::{RunStats, StatEffect};
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

type Edit = Box<dyn Fn(&mut Value)>;
type Rows = Vec<(StatEffect, Vec<u32>)>;

fn rewards_json() -> Value {
    serde_json::from_slice(common::bundled_file("rewards")).unwrap()
}

fn expertise_json() -> Value {
    serde_json::from_slice(common::bundled_file("expertise")).unwrap()
}

fn parse_rewards(v: &Value) -> Result<Rewards, Error> {
    Rewards::parse(v.to_string().as_bytes(), Versions::CURRENT)
}

fn parse_expertise(v: &Value) -> Result<Expertise, Error> {
    Expertise::parse(v.to_string().as_bytes(), Versions::CURRENT)
}

fn refused_at(r: Result<impl std::fmt::Debug, Error>, path: &str) {
    match r {
        Err(Error::Invalid(issues)) => {
            assert!(issues.iter().any(|i| i.path == path), "{path}: {issues:?}")
        }
        other => panic!("{path}: expected a refusal, got {other:?}"),
    }
}

fn rewards_with(edit: impl FnOnce(&mut Value)) -> Value {
    let mut v = rewards_json();
    edit(&mut v);
    v
}

// --- T1: the bundled pack loads each value ------------------------------------------------------

#[test]
fn the_bundled_pack_holds_the_equipment_rules() {
    let rules = Rewards::bundled().equipment().expect("the equipment rules");
    let cats: Vec<(&str, u8)> = rules
        .categories
        .iter()
        .map(|c| (c.id.as_str(), c.slots))
        .collect();
    assert_eq!(cats, [("attack", 5), ("defence", 2), ("mobility", 2)]);
    assert_eq!(rules.rune_slots, 3);
    assert_eq!(
        rules.kills_to_next,
        [10, 20, 35, 55, 80, 110, 145, 185, 230]
    );
    assert_eq!(rules.level_pct, 3);
    let extract: Vec<u32> = rules.rarities.iter().map(|r| r.extract).collect();
    let craft: Vec<u32> = rules.rarities.iter().map(|r| r.craft_price).collect();
    assert_eq!(extract, [5, 10, 20, 40, 80, 160]);
    assert_eq!(craft, [25, 50, 100, 200, 400, 800]);
    assert_eq!(rules.rune_extract, 10);
    assert_eq!(rules.boss_bonus, 50);
    assert_eq!(rules.boss_drops, 1);
    assert_eq!(rules.rune_share_permille, 100);
}

// --- T2: validation refuses a bad value ---------------------------------------------------------

#[test]
fn a_bad_equipment_rule_is_refused_with_its_path() {
    let cases: Vec<(&str, Edit)> = vec![
        (
            "$.equipment.kills_to_next",
            Box::new(|e| e["kills_to_next"] = json!([1, 2, 3, 4, 5, 6, 7, 8])),
        ),
        (
            "$.equipment.kills_to_next[1]",
            Box::new(|e| e["kills_to_next"] = json!([10, 10, 35, 55, 80, 110, 145, 185, 230])),
        ),
        (
            "$.equipment.kills_to_next[0]",
            Box::new(|e| e["kills_to_next"] = json!([0, 20, 35, 55, 80, 110, 145, 185, 230])),
        ),
        (
            "$.equipment.level_pct",
            Box::new(|e| e["level_pct"] = json!(0)),
        ),
        (
            "$.equipment.level_pct",
            Box::new(|e| e["level_pct"] = json!(51)),
        ),
        (
            "$.equipment.categories",
            Box::new(|e| e["categories"] = json!([])),
        ),
        (
            "$.equipment.categories[1].id",
            Box::new(|e| e["categories"][1]["id"] = json!("attack")),
        ),
        (
            "$.equipment.categories[0].slots",
            Box::new(|e| e["categories"][0]["slots"] = json!(0)),
        ),
        (
            "$.equipment.categories[0].slots",
            Box::new(|e| e["categories"][0]["slots"] = json!(17)),
        ),
        (
            "$.equipment.categories",
            Box::new(|e| {
                e["categories"] =
                    json!([{"id": "attack", "slots": 16}, {"id": "defence", "slots": 15}]);
            }),
        ),
        (
            "$.equipment.rune_slots",
            Box::new(|e| e["rune_slots"] = json!(0)),
        ),
        (
            "$.equipment.rarities",
            Box::new(|e| {
                e["rarities"].as_array_mut().unwrap().pop();
            }),
        ),
        (
            "$.equipment.rarities[1].rarity",
            Box::new(|e| e["rarities"][1]["rarity"] = json!("common")),
        ),
        (
            "$.equipment.rarities[0].craft_price",
            Box::new(|e| e["rarities"][0]["craft_price"] = json!(4)),
        ),
        (
            "$.equipment.rune_share_permille",
            Box::new(|e| e["rune_share_permille"] = json!(1001)),
        ),
        (
            "$.equipment.boss_drops",
            Box::new(|e| e["boss_drops"] = json!(9)),
        ),
    ];
    for (path, edit) in cases {
        let v = rewards_with(|v| edit(&mut v["equipment"]));
        refused_at(parse_rewards(&v), path);
    }
}

#[test]
fn a_bad_reward_category_is_refused_with_its_path() {
    let first = |v: &mut Value| {
        v["rewards"][0]
            .as_object_mut()
            .unwrap()
            .insert("category".into(), json!("nowhere"))
    };
    refused_at(
        parse_rewards(&rewards_with(|v| {
            first(v);
        })),
        "$.rewards[0].category",
    );
    let rune = rewards_json()["rewards"]
        .as_array()
        .unwrap()
        .iter()
        .position(|r| r["effect"] == "random_rune")
        .expect("the random rune");
    let v = rewards_with(|v| v["rewards"][rune]["category"] = json!("attack"));
    refused_at(parse_rewards(&v), &format!("$.rewards[{rune}].category"));
    let v = rewards_with(|v| {
        v["rewards"][0]["category"] = json!("attack");
        v["rewards"][0]["deprecated"] = json!(true);
        v["rewards"][0]["equipment_only"] = json!(true);
    });
    refused_at(parse_rewards(&v), "$.rewards[0].equipment_only");
    let v = rewards_with(|v| {
        v.as_object_mut().unwrap().remove("equipment");
    });
    refused_at(parse_rewards(&v), "$.rewards[0].category");
}

// --- T3, T6: the categories and the new stars ----------------------------------------------------

/// The category the placeholder derived from a reward's first effect (`category_of`).
fn derived(effect: RewardEffect) -> Option<&'static str> {
    use RewardEffect::*;
    match effect {
        AttackPct | AttackSpeedPct | RangePct | TypeBonusPct | AttackTypeBonusPct
        | ElementBonusPct | RuneHaste | StageSkill | SkillCooldownPct | Orbit | CritChancePct
        | CritDamagePct | EchoHitPct | KillBurst | BurstDepth | HealPerKills => Some("attack"),
        MaxHp | EvadePct | Regen | EvadeCharge => Some("defence"),
        SpeedPct => Some("mobility"),
        XpPct | PickupRadiusPct | OfferCandidates | Rerolls | Banishes | RandomRune => None,
    }
}

#[test]
fn the_old_rewards_carry_the_category_their_first_effect_gave() {
    let rewards = Rewards::bundled().rewards();
    let old: Vec<_> = rewards
        .iter()
        .filter(|r| !r.id.as_str().starts_with("rune_haste_star_"))
        .collect();
    assert_eq!(old.len(), 135);
    for r in old {
        let want = derived(r.effect());
        assert_eq!(
            r.category.as_ref().map(|c| c.as_str()),
            want,
            "{}",
            r.id.as_str()
        );
        assert!(!r.equipment_only, "{}", r.id.as_str());
    }
}

#[test]
fn the_new_rune_haste_stars_replace_the_old_six() {
    let rewards = Rewards::bundled();
    for rarity in Rarity::ALL {
        let name = format!("{rarity:?}").to_lowercase();
        let old = &rewards.rewards()[rewards
            .index_of(&format!("rune_haste_{name}").as_str().into())
            .expect("old")];
        let new = &rewards.rewards()[rewards
            .index_of(&format!("rune_haste_star_{name}").as_str().into())
            .expect("new")];
        assert!(old.deprecated && !old.equipment_only);
        assert!(new.equipment_only && !new.deprecated);
        assert_eq!(new.category.as_ref().map(|c| c.as_str()), Some("attack"));
        assert_eq!(
            (new.effects.clone(), new.limit),
            (old.effects.clone(), old.limit)
        );
        let pack: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
        let introduced = |id: &str| {
            pack["introduced"]["rewards"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == id)
                .map(|e| e["sim_version"].as_u64().unwrap())
        };
        assert_eq!(introduced(new.id.as_str()), Some(2));
        assert_eq!(introduced(old.id.as_str()), Some(1));
    }
}

// --- T5: the default pool -----------------------------------------------------------------------

#[test]
fn the_default_pool_is_a_rule() {
    let rewards = Rewards::bundled();
    let id_at = |p: u16| rewards.rewards()[usize::from(p)].id.as_str().to_string();
    let equipment: Vec<String> = rewards
        .default_pool(BuildMode::Equipment)
        .into_iter()
        .map(id_at)
        .collect();
    for r in rewards.rewards() {
        let id = r.id.as_str().to_string();
        let want = r.category.is_some() && !r.deprecated;
        assert_eq!(equipment.contains(&id), want, "{id}");
    }
    assert!(equipment.contains(&"rune_haste_star_rare".to_string()));
    assert!(!equipment.contains(&"rune_haste_rare".to_string()));
    assert!(!equipment.contains(&"random_rune".to_string()));
    let offers = rewards.default_pool(BuildMode::Offers);
    let want: Vec<u16> = (0..rewards.rewards().len())
        .filter(|&i| !rewards.rewards()[i].equipment_only)
        .map(|i| u16::try_from(i).unwrap())
        .collect();
    assert_eq!(offers, want);
    assert_eq!(offers.len(), 135);
}

fn setup(mode: BuildMode, pool: Option<&[&str]>) -> RunSetup {
    RunSetup {
        mode: "tutorial".into(),
        build_mode: mode,
        star_pool: pool.map(|p| p.iter().map(|s| (*s).into()).collect()),
        ..trial_setup()
    }
}

#[test]
fn an_explicit_pool_may_name_a_deprecated_star_but_not_an_equipment_only_one_in_offers() {
    let content = RunContent::bundled();
    let ok = setup(
        BuildMode::Equipment,
        Some(&["rune_haste_common", "rune_haste_star_rare"]),
    );
    assert!(run_setup::validate_in(&ok, content).is_ok());
    let bad = setup(
        BuildMode::Offers,
        Some(&["rune_haste_common", "rune_haste_star_rare"]),
    );
    refused_at(run_setup::validate_in(&bad, content), "$.star_pool[1]");
}

// --- T17: the layout and an equipment run on a pack without rules --------------------------------

#[test]
fn an_equipment_run_needs_the_pack_to_hold_the_rules_but_an_offers_run_does_not() {
    let bare = rewards_with(|v| {
        v.as_object_mut().unwrap().remove("equipment");
        for r in v["rewards"].as_array_mut().unwrap() {
            r.as_object_mut().unwrap().remove("category");
        }
    });
    let rewards = parse_rewards(&bare).expect("a pack made before the rules still reads");
    assert!(rewards.equipment().is_none());
    let content = RunContent::new(RunStats::bundled().clone(), rewards);
    let refused = run_setup::validate_in(&setup(BuildMode::Equipment, None), &content);
    refused_at(refused, "$.build_mode");
    assert!(run_setup::validate_in(&setup(BuildMode::Offers, None), &content).is_ok());
}

// --- T14: the Growth three and the other stats get no value --------------------------------------

#[test]
fn no_bundled_set_names_a_growth_or_pickup_effect() {
    for set in Expertise::bundled().weapons().iter().flat_map(|w| {
        w.base
            .iter()
            .chain(w.styles.iter().flat_map(|s| s.stats.iter()))
    }) {
        assert!(
            !matches!(
                set.effect,
                StatEffect::PickupRadiusPct
                    | StatEffect::StardustPct
                    | StatEffect::Rerolls
                    | StatEffect::Banishes
                    | StatEffect::OfferCandidates
            ),
            "{:?}",
            set.effect
        );
    }
    assert_eq!(StatEffect::ALL.len(), 13);
}

// --- perk sets ----------------------------------------------------------------------------------

#[test]
fn the_bundled_perk_sets_weigh_drops_by_category() {
    use stella_rain_core::perk_sets::PerkEffect;
    let sets = PerkSets::bundled();
    let drop_weight = |id: &str| {
        let set = &sets.sets()[sets.index_of(&id.into()).unwrap()];
        set.effects
            .iter()
            .find(|e| e.effect == PerkEffect::DropWeight)
            .map(|e| (e.category.map(|c| c.as_str().to_string()), e.amount))
    };
    assert_eq!(drop_weight("duelist"), Some((Some("attack".into()), 25)));
    assert_eq!(drop_weight("vanguard"), Some((Some("defence".into()), 25)));
    assert_eq!(drop_weight("scout"), Some((Some("mobility".into()), 25)));
    assert_eq!(drop_weight("scholar"), None);
}

#[test]
fn a_perk_category_is_required_for_drop_weight_only_and_must_exist() {
    let bytes = common::bundled_file("perk_sets");
    let mut v: Value = serde_json::from_slice(bytes).unwrap();
    let at = |v: &Value| {
        let sets = v["perk_sets"].as_array().unwrap();
        let s = sets.iter().position(|s| s["id"] == "duelist").unwrap();
        let e = sets[s]["effects"]
            .as_array()
            .unwrap()
            .iter()
            .position(|e| e["effect"] == "drop_weight")
            .unwrap();
        (s, e)
    };
    let (s, e) = at(&v);
    v["perk_sets"][s]["effects"][e]
        .as_object_mut()
        .unwrap()
        .remove("category");
    refused_at(
        PerkSets::parse(v.to_string().as_bytes(), Versions::CURRENT),
        &format!("$.perk_sets[{s}].effects[{e}].category"),
    );
    let mut v: Value = serde_json::from_slice(bytes).unwrap();
    v["perk_sets"][s]["effects"][0]["category"] = json!("attack");
    refused_at(
        PerkSets::parse(v.to_string().as_bytes(), Versions::CURRENT),
        &format!("$.perk_sets[{s}].effects[0].category"),
    );
}

// --- T9, T10: the sets ---------------------------------------------------------------------------

/// `floor(max * n / 10)` for Lv1..Lv10.
fn lv(max: u32) -> Vec<u32> {
    (1..=10).map(|n| max * n / 10).collect()
}

fn stat(effect: StatEffect, max: u32) -> (StatEffect, Vec<u32>) {
    (effect, lv(max))
}

fn rows(set: &[SetStat]) -> Rows {
    set.iter().map(|s| (s.effect, s.by_level.clone())).collect()
}

#[test]
fn the_bundled_sets_are_the_placeholder_values() {
    use StatEffect::*;
    let ex = Expertise::bundled();
    let base = vec![
        stat(AttackPct, 100),
        stat(AttackSpeedPct, 100),
        stat(MaxHp, 100_000),
    ];
    let trees: [(&str, &str, Rows); 6] = [
        (
            "giant_sword",
            "edge",
            vec![
                stat(AttackPct, 500),
                stat(AttackSpeedPct, 300),
                stat(SkillCooldownPct, 100),
                stat(RangePct, 200),
            ],
        ),
        (
            "giant_sword",
            "tempo",
            vec![
                stat(AttackPct, 330),
                stat(AttackSpeedPct, 1000),
                stat(SkillCooldownPct, 200),
                stat(RangePct, 500),
            ],
        ),
        (
            "giant_sword",
            "reach",
            vec![
                stat(AttackPct, 300),
                stat(AttackSpeedPct, 300),
                stat(SkillCooldownPct, 100),
                stat(RangePct, 800),
            ],
        ),
        (
            "giant_hammer",
            "crush",
            vec![
                stat(AttackPct, 600),
                stat(AttackSpeedPct, 200),
                stat(SkillCooldownPct, 100),
                stat(RangePct, 200),
            ],
        ),
        (
            "giant_hammer",
            "quake",
            vec![
                stat(AttackPct, 300),
                stat(AttackSpeedPct, 200),
                stat(SkillCooldownPct, 200),
                stat(RangePct, 800),
            ],
        ),
        (
            "giant_hammer",
            "bulwark",
            vec![
                stat(AttackPct, 200),
                stat(SkillCooldownPct, 200),
                stat(MaxHp, 40_000),
                stat(EvadePct, 200),
            ],
        ),
    ];
    for weapon in ["giant_sword", "giant_hammer"] {
        let w = ex.weapon(&weapon.into()).unwrap();
        assert_eq!(rows(&w.base), base, "{weapon} base");
    }
    for (weapon, style, want) in trees {
        let w = ex.weapon(&weapon.into()).unwrap();
        let s = w.styles.iter().find(|s| s.id.as_str() == style).unwrap();
        assert_eq!(rows(&s.stats), want, "{weapon}/{style}");
    }
    let tempo = &ex.weapon(&"giant_sword".into()).unwrap().styles[1];
    let at5: Vec<u32> = tempo.stats.iter().map(|s| s.by_level[4]).collect();
    assert_eq!(at5, [165, 500, 100, 250]);
}

#[test]
fn a_bad_set_is_refused_with_its_path() {
    let sets = "$.weapons[0].styles[1].stats";
    let cases: Vec<(String, Edit)> = vec![
        (
            format!("{sets}[0].by_level"),
            Box::new(|s| {
                s[0]["by_level"].as_array_mut().unwrap().pop();
            }),
        ),
        (
            format!("{sets}[0].by_level"),
            Box::new(|s| s[0]["by_level"].as_array_mut().unwrap().push(json!(999))),
        ),
        (
            format!("{sets}[0].by_level[2]"),
            Box::new(|s| s[0]["by_level"] = json!([10, 20, 15, 40, 50, 60, 70, 80, 90, 100])),
        ),
        (
            format!("{sets}[0].by_level[9]"),
            Box::new(|s| s[0]["by_level"] = json!([1, 2, 3, 4, 5, 6, 7, 8, 9, 1001])),
        ),
        (
            format!("{sets}[3].by_level[9]"),
            Box::new(|s| s[3]["by_level"] = json!([1, 2, 3, 4, 5, 6, 7, 8, 9, 1001])),
        ),
        (
            format!("{sets}[1].effect"),
            Box::new(|s| s[1]["effect"] = json!("attack_pct")),
        ),
        (
            format!("{sets}[1].effect"),
            Box::new(|s| s[1]["effect"] = json!("xp_pct")),
        ),
        (
            format!("{sets}[1].effect"),
            Box::new(|s| s[1]["effect"] = json!("speed_pct")),
        ),
        (
            format!("{sets}[1].effect"),
            Box::new(|s| s[1]["effect"] = json!("stardust_pct")),
        ),
        (
            format!("{sets}[1].effect"),
            Box::new(|s| s[1]["effect"] = json!("pickup_radius_pct")),
        ),
        (
            format!("{sets}[1].effect"),
            Box::new(|s| s[1]["effect"] = json!("rerolls")),
        ),
        (
            sets.to_string(),
            Box::new(|s| {
                for _ in 0..5 {
                    let r = s[0].clone();
                    s.as_array_mut().unwrap().push(r);
                }
            }),
        ),
    ];
    for (path, edit) in cases {
        let mut v = expertise_json();
        edit(&mut v["weapons"][0]["styles"][1]["stats"]);
        refused_at(parse_expertise(&v), &path);
    }
    assert_eq!(MAX_SET_STATS, 8);
}

#[test]
fn a_hit_point_set_may_reach_the_hit_point_bound_and_a_set_may_be_empty() {
    use stella_rain_core::rewards::MAX_HP_AMOUNT;
    let mut v = expertise_json();
    let hp = json!({"effect": "max_hp", "by_level": [40_000, 40_000, 40_000, 40_000, 40_000, 40_000, 40_000, 40_000, 40_000, MAX_HP_AMOUNT]});
    v["weapons"][0]["styles"][0]["stats"] = json!([hp]);
    assert!(parse_expertise(&v).is_ok());
    let mut v = expertise_json();
    let hp =
        json!({"effect": "max_hp", "by_level": [1, 2, 3, 4, 5, 6, 7, 8, 9, MAX_HP_AMOUNT + 1]});
    v["weapons"][0]["styles"][0]["stats"] = json!([hp]);
    refused_at(
        parse_expertise(&v),
        "$.weapons[0].styles[0].stats[0].by_level[9]",
    );
    let mut v = expertise_json();
    v["weapons"][0]["styles"][0]
        .as_object_mut()
        .unwrap()
        .remove("stats");
    v["weapons"][0].as_object_mut().unwrap().remove("base");
    let ex = parse_expertise(&v).expect("a pack made before the keys reads");
    assert!(ex.weapons()[0].base.is_empty() && ex.weapons()[0].styles[0].stats.is_empty());
}

#[test]
fn the_style_effect_and_amount_stay_required() {
    let mut v = expertise_json();
    v["weapons"][0]["styles"][0]
        .as_object_mut()
        .unwrap()
        .remove("amount");
    assert!(parse_expertise(&v).is_err());
}

// --- T15: a kept pack still reads ----------------------------------------------------------------

#[test]
fn a_file_made_before_the_keys_existed_writes_the_same_bytes() {
    let mut v = rewards_json();
    v.as_object_mut().unwrap().remove("equipment");
    for r in v["rewards"].as_array_mut().unwrap() {
        for k in ["category", "deprecated", "equipment_only"] {
            r.as_object_mut().unwrap().remove(k);
        }
    }
    assert!(parse_rewards(&v).is_ok());
    assert!(registry::is_well_formed("rune_haste_star_supernova"));
}
