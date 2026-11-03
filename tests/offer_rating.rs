//! `Engine::offer_ratings`: each candidate of an open star offer rated with the rules' own
//! formulas (ADR-SIM-1, ADR-REPL-5): its categories, the expected final-DPS gain in basis
//! points, the set tier it reaches and the stats it changes. A read-only query: nothing the
//! hash covers moves (ADR-SIM-3).

use serde_json::{Value, json};
use stella_rain_core::constellations::Constellations;
use stella_rain_core::engine::Engine;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::snapshot::{Category, OfferRating, StatChange, StatKind};
use stella_rain_core::stage::{RarityWeights, Stage, Tier};
use stella_rain_core::version::{SIM_VERSION, Versions};

use crate::common::*;
use crate::reward_kinds::{NEAR_Y, entry, kind, next_offer, offer, starry, take};

/// Atk 1000, 100 hit points, the weapon `phases` (one step, a circle that reaches the enemies),
/// and enough enemies for a dozen offers, every star epic.
fn stage_of(phases: Value) -> Stage {
    let waves = vec![wave(
        1,
        14,
        6,
        enemy(10, hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    )];
    let weapon = json!({ "inline": {
        "combo": { "main": { "attack": { "inline": phases } } },
        "window_ticks": 30,
        "assist_range": 0
    } });
    let mut stage = starry(armed(
        stage_full(18_000, Some(far_boss()), waves, vec![]),
        weapon,
    ));
    stage.sim_version = SIM_VERSION;
    stage.player.atk = 1000;
    stage.player.hp = 100;
    stage.stars.as_mut().expect("stars").tiers = vec![Tier {
        from_level: 1,
        weights: RarityWeights {
            epic: 1,
            ..RarityWeights::default()
        },
        random_rune_weight: None,
    }];
    stage
}

/// One active phase of `ticks` ticks, damage from the attack stat, reaching the enemies.
fn swing(ticks: u32) -> Value {
    json!([{ "kind": "active", "ticks": ticks,
             "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 16_384 } }])
}

fn epic(id: &str, effects: Vec<Value>) -> Value {
    let mut r = kind(id, effects, 1_000, 2);
    r["rarity"] = json!("epic");
    r
}

fn named(id: &str, effects: Vec<Value>) -> Value {
    let mut r = epic(id, effects);
    r["named"] = json!(true);
    r["limit"] = json!(1);
    r["star"] = json!(id);
    r
}

fn engine_of(stage: &Stage, list: Vec<Value>, sets: Option<Value>) -> Engine {
    let file = json!({ "rarities": rarities(), "rewards": list });
    let rewards = Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT)
        .expect("the test rewards validate");
    let rewards = match sets {
        Some(v) => {
            let c =
                Constellations::parse_with(v.to_string().as_bytes(), Versions::CURRENT, &rewards)
                    .expect("the test sets validate");
            rewards.with_constellations(c)
        }
        None => rewards,
    };
    Engine::new_with(
        stage,
        &RunContent::new(RunStats::bundled().clone(), rewards),
    )
    .expect("the test stage is valid")
}

/// A run that took `held` (in order, one offer each) and stands at an open offer.
fn scene(phases: Value, list: Vec<Value>, sets: Option<Value>, held: &[&str]) -> Engine {
    let mut e = engine_of(&stage_of(phases), list, sets);
    next_offer(&mut e);
    for id in held {
        take(&mut e, id);
        next_offer(&mut e);
    }
    e
}

fn rating(e: &Engine, id: &str) -> OfferRating {
    let at = offer(e)
        .candidates
        .iter()
        .position(|c| c.reward.as_str() == id)
        .unwrap_or_else(|| panic!("{id} is not offered"));
    let all = e.offer_ratings(PlayerIdx::SOLO);
    assert_eq!(all.len(), offer(e).candidates.len(), "one per candidate");
    all[at].clone()
}

fn change(stat: StatKind, before: u32, after: u32) -> StatChange {
    StatChange {
        stat,
        before,
        after,
    }
}

fn attack(amount: u32) -> Value {
    entry("attack_pct", amount)
}

// --- A: the worked example -----------------------------------------------------------------

#[test]
fn thirty_percent_attack_and_a_twenty_star_is_fifteen_point_three_eight_percent() {
    let list = vec![
        epic("pre", vec![attack(30)]),
        epic("star", vec![attack(20)]),
    ];
    let e = scene(swing(10), list, None, &["pre"]);
    let r = rating(&e, "star");
    assert_eq!(r.dps_bp, Some(1538));
    assert_eq!(r.categories, vec![Category::Attack]);
    assert_eq!(r.set_tier, None);
    assert_eq!(r.changes, vec![change(StatKind::AttackPct, 30, 50)]);
}

// --- B: sets -------------------------------------------------------------------------------

fn tier(threshold: u8, effects: Vec<Value>) -> Value {
    json!({ "threshold": threshold, "effects": effects })
}

fn set(id: &str, members: &[&str], tiers: Vec<Value>) -> Value {
    json!({ "id": id, "name": format!("The {id}"), "members": members, "tiers": tiers })
}

fn sets(v: Value) -> Option<Value> {
    Some(json!({ "constellations": [v] }))
}

#[test]
fn a_pick_that_completes_a_tier_counts_the_sets_bonus() {
    let pair = || set("pair", &["m1", "m2"], vec![tier(2, vec![attack(10)])]);
    let list = vec![
        named("m1", vec![entry("max_hp", 1)]),
        named("m2", vec![attack(10)]),
    ];
    // M1 held: M2 completes the pair, its own 10 and the tier's 10 on a base of 100.
    let e = scene(swing(10), list.clone(), sets(pair()), &["m1"]);
    let r = rating(&e, "m2");
    assert_eq!(r.dps_bp, Some(2000));
    assert_eq!(r.set_tier, Some(1));
    // Nothing held: the pick is 1 of 2, no tier.
    let e = scene(swing(10), list, sets(pair()), &[]);
    let r = rating(&e, "m2");
    assert_eq!(r.dps_bp, Some(1000));
    assert_eq!(r.set_tier, None);
}

#[test]
fn the_full_tier_adds_only_the_difference() {
    let trio = set(
        "trio",
        &["m1", "m2", "m3"],
        vec![tier(2, vec![attack(10)]), tier(3, vec![attack(15)])],
    );
    let hp = || vec![entry("max_hp", 1)];
    let list = vec![
        named("m1", hp()),
        named("m2", hp()),
        named("m3", vec![attack(5)]),
    ];
    let e = scene(swing(10), list, sets(trio), &["m1", "m2"]);
    let r = rating(&e, "m3");
    assert_eq!(r.set_tier, Some(2));
    // Tier 2's 10 is held; the pick adds its own 5 and the 5 that tier 3 adds to tier 2.
    assert_eq!(r.changes, vec![change(StatKind::AttackPct, 10, 20)]);
}

#[test]
fn a_tier_counts_only_what_it_adds_to_the_tier_below() {
    let regen = || entry("regen", 5);
    let pair = set(
        "pair",
        &["m1", "m2"],
        vec![
            tier(1, vec![attack(10)]),
            tier(2, vec![attack(10), regen()]),
        ],
    );
    let list = vec![
        named("m1", vec![entry("max_hp", 1)]),
        named("m2", vec![entry("max_hp", 1)]),
    ];
    let e = scene(swing(10), list, sets(pair), &["m1"]);
    let r = rating(&e, "m2");
    // Tier 2 adds only the regeneration to the attack tier 1 already gave.
    assert_eq!(r.set_tier, Some(2));
    assert_eq!(r.categories, vec![Category::Defence]);
    assert_eq!(r.dps_bp, None);
}

// --- C: first in its category --------------------------------------------------------------

#[test]
fn the_first_crit_and_the_first_attack_star_are_rated_against_nothing() {
    let list = vec![
        epic("crit", vec![entry("crit_chance_pct", 10)]),
        epic("atk", vec![attack(20)]),
    ];
    let e = scene(swing(10), list, None, &[]);
    // (90 * 1000 + 2000 * 10) = 110_000 against 100_000.
    assert_eq!(rating(&e, "crit").dps_bp, Some(1000));
    assert_eq!(rating(&e, "atk").dps_bp, Some(2000));
}

// --- D: stars that are not damage ----------------------------------------------------------

#[test]
fn a_star_that_is_not_damage_has_no_gain_but_shows_its_category_and_change() {
    let list = vec![
        epic("hp", vec![entry("max_hp", 20)]),
        epic("xp", vec![entry("xp_pct", 10)]),
        epic("speed", vec![entry("speed_pct", 10)]),
    ];
    let e = scene(swing(10), list, None, &[]);
    let hp = rating(&e, "hp");
    assert_eq!(hp.dps_bp, None);
    assert_eq!(hp.categories, vec![Category::Defence]);
    assert_eq!(hp.changes, vec![change(StatKind::MaxHp, 100, 120)]);
    assert_eq!(rating(&e, "xp").categories, vec![Category::Growth]);
    assert_eq!(rating(&e, "speed").categories, vec![Category::Mobility]);
    let list = vec![
        epic("mixed", vec![attack(20), entry("max_hp", 10)]),
        epic("plain", vec![attack(20)]),
    ];
    let e = scene(swing(10), list, None, &[]);
    let mixed = rating(&e, "mixed");
    assert_eq!(mixed.categories, vec![Category::Attack, Category::Defence]);
    assert_eq!(mixed.dps_bp, rating(&e, "plain").dps_bp);
    assert_eq!(mixed.dps_bp, Some(2000));
}

// --- E: caps -------------------------------------------------------------------------------

#[test]
fn a_term_already_at_its_cap_gains_nothing() {
    let list = vec![
        epic("p1", vec![entry("crit_chance_pct", 40)]),
        epic("p2", vec![entry("crit_chance_pct", 40)]),
        epic("more", vec![entry("crit_chance_pct", 10)]),
    ];
    let e = scene(swing(10), list, None, &["p1", "p2"]);
    let r = rating(&e, "more");
    assert_eq!(r.dps_bp, Some(0));
    // The held 80 is held to 75, so the pick moves nothing.
    assert_eq!(r.changes, vec![]);
}

// --- F: the weapon's mix and attack speed ----------------------------------------------------

#[test]
fn a_type_bonus_is_rated_on_the_weapons_own_mix() {
    let pyro_star = || {
        epic(
            "pyro",
            vec![json!({"effect": "type_bonus_pct", "type": "pyro", "amount": 50})],
        )
    };
    let mixed = json!([
        { "kind": "active", "ticks": 5, "type": "pyro",
          "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 16_384 } },
        { "kind": "active", "ticks": 5,
          "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 16_384 } }
    ]);
    // 1500 + 1000 against 2000.
    let e = scene(mixed, vec![pyro_star()], None, &[]);
    assert_eq!(rating(&e, "pyro").dps_bp, Some(2500));
    // A weapon with no pyro phase gains nothing.
    let e = scene(swing(10), vec![pyro_star()], None, &[]);
    assert_eq!(rating(&e, "pyro").dps_bp, Some(0));
}

#[test]
fn attack_speed_is_rated_on_the_rounded_ticks() {
    let list = vec![epic("fast", vec![entry("attack_speed_pct", 30)])];
    let e = scene(swing(100), list, None, &[]);
    // 100 ticks become (100 * 100 + 65) / 130 = 77, not 100 / 1.3.
    let bp = (100 * 10_000 + 77 / 2) / 77 - 10_000;
    assert_eq!(rating(&e, "fast").dps_bp, Some(bp));
    assert_eq!(bp, 2987);
}

// --- H: read-only --------------------------------------------------------------------------

#[test]
fn rating_changes_nothing_and_repeats() {
    let list = vec![
        epic("a", vec![attack(20)]),
        epic("b", vec![entry("max_hp", 20)]),
        epic("c", vec![entry("crit_chance_pct", 10)]),
    ];
    let e = scene(swing(10), list, None, &[]);
    let (hash, snapshot) = (e.state_hash(), e.snapshot());
    let first = e.offer_ratings(PlayerIdx::SOLO);
    assert_eq!(first.len(), 3);
    assert_eq!(e.offer_ratings(PlayerIdx::SOLO), first);
    assert_eq!(e.state_hash(), hash);
    assert_eq!(e.snapshot(), snapshot);
}

// --- I: the API ----------------------------------------------------------------------------

#[test]
fn no_offer_a_missing_player_or_version_1_gives_nothing() {
    let list = vec![epic("a", vec![attack(20)]), epic("b", vec![attack(10)])];
    // No offer open.
    let fresh = engine_of(&stage_of(swing(10)), list.clone(), None);
    assert!(fresh.offer_ratings(PlayerIdx::SOLO).is_empty());
    // An offer open, for a slot outside the party.
    let e = scene(swing(10), list.clone(), None, &[]);
    assert!(!e.offer_ratings(PlayerIdx::SOLO).is_empty());
    assert!(e.offer_ratings(PlayerIdx::new(1).unwrap()).is_empty());
    // Version 1 is frozen: no rating.
    let mut stage = stage_of(swing(10));
    stage.sim_version = 1;
    let mut v1 = engine_of(&stage, list, None);
    next_offer(&mut v1);
    assert!(v1.offer_ratings(PlayerIdx::SOLO).is_empty());
}
