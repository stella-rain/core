//! Healing: a boss kill heals a quarter of the main character's most hit points, never above it;
//! a regular kill drops a heal pickup with a chance of 1 in 150 from the run's generator, and
//! taking the pickup heals 300 shown hit points, never above the most; a regen reward adds its
//! amount a second at its rarity (10, 15, 30, 35, 60 and 80 shown hit points from common up to
//! supernova). Kade's numbers are shown hit points; the files hold them in the stage's scale
//! (ADR-103): `DAMAGE_SCALE` (100) is one shown hit point. Every number here is worked out by hand.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::rewards::{MAX_HP_AMOUNT, Rarity, RewardEffect, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::snapshot::OfferOption;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate;
use stella_rain_core::validate::limits::{DAMAGE_SCALE, MAX_HEAL_AMOUNT, MAX_PLAYER_HP};

use crate::common;

use common::*;

/// 32 pixels above the main character: inside the Main weapon's circle, and inside a pickup's
/// circle from where the main character stands.
const NEAR_Y: i32 = START_Y - 8_192;

/// The most hit points of the test main character.
const MAX_HP: u32 = 1_000;

/// What a pickup heals: 300 shown hit points (Kade, app#336), as the bundled `trial` mode has.
const PICKUP: u32 = 300 * DAMAGE_SCALE;

/// The main character's entity ID.
const PLAYER: u32 = 0;

/// A Main weapon of one step: a circle 8 pixels ahead of the main character, one active tick of
/// 10 damage and 4 ticks of recovery (as `tests/range_reward.rs`).
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

/// The heal config: `boss_pct` percent for a boss, one pickup in `drop_one_in` that heals
/// `PICKUP`, at most `cap` on the map, taken within 12 288 of the main character.
fn heal_json(boss_pct: u16, drop_one_in: u32, cap: u16) -> Value {
    json!({
        "boss_pct": boss_pct, "drop_one_in": drop_one_in, "amount": PICKUP, "cap": cap,
        "pickup_radius": 12_288
    })
}

/// `stage` with the Main weapon above, the main character at `MAX_HP` and `heal` as its healing.
fn healing(stage: Stage, heal: Value) -> Stage {
    let mut stage = armed(stage, weapon());
    stage.heal = Some(serde_json::from_value(heal).expect("the heal config parses"));
    stage.player.hp = MAX_HP;
    stage
}

/// Levels of 1000 experience and stars of `rarity` only, which every kill drops, all taken.
/// Every star's offer freezes the world until one is chosen, so only the regen tests use it.
fn starry(stage: Stage, rarity: &str) -> Stage {
    let mut stage = stage;
    stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).unwrap());
    stage.stars = Some(
        serde_json::from_value(json!({
            "drop_per_mille": 1000, "pity": 1, "cap": 8, "pickup_radius": 12_288,
            "rerolls": 0, "banishes": 0,
            "tiers": [{ "from_level": 1, "weights": { rarity: 1 } }],
        }))
        .unwrap(),
    );
    stage
}

/// The rewards file with `list` in it.
fn rewards(list: Vec<Value>) -> Rewards {
    let file = json!({ "schema_version": 0, "sim_version": 0, "rewards": list });
    Rewards::parse(file.to_string().as_bytes()).expect("the test rewards validate")
}

/// A regen reward of `amount` hit points a second at `rarity`, taken at most `limit` times.
fn regen(id: &str, rarity: &str, amount: u32, limit: u8) -> Value {
    json!({
        "id": id, "rarity": rarity, "effect": "regen", "amount": amount, "limit": limit,
        "title": "Regeneration", "description": ""
    })
}

/// A shooter 40 000 pixels to the side of the main character, out of the Main weapon's reach,
/// that fires once at the first tick: its bullet hurts the main character by 100 hit points.
/// Its 10 000 hit points keep the weapon from killing it.
fn shooter() -> Value {
    let rules = json!([attack_rule(always(), 10_000)]);
    let (x, y) = (START_X + 40_000, START_Y);
    wave(1, 1, 0, enemy(10_000, hold(x, y), rules), x, y)
}

/// As `shooter`, with a bullet of `damage`: enough to leave room for a heal of shown size.
fn heavy_shooter(damage: u32) -> Value {
    let attack = json!({ "inline": [{ "kind": "active", "ticks": 1, "damage": damage, "spawn": [
        { "type": "zone", "shape": { "type": "circle", "radius": 3_072 }, "anchor": "target", "wind_up_ticks": 0, "active_ticks": 2 }
    ] }] });
    let rules = json!([{ "when": always(), "target": "player",
                         "do": { "type": "attack", "attack": attack },
                         "cooldown_ticks": 10_000 }]);
    let (x, y) = (START_X + 40_000, START_Y);
    wave(1, 1, 0, enemy(10_000, hold(x, y), rules), x, y)
}

/// `count` enemies of 10 hp at `NEAR_Y`, one every 6 ticks from `at_tick`, standing still. The
/// shooter's bullet takes about 20 ticks to arrive, so the targets wait for it.
fn targets(at_tick: u32, count: u16) -> Value {
    wave(
        at_tick,
        count,
        6,
        enemy(10, hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    )
}

/// A stage with a boss of `boss_hp` at `NEAR_Y` and `shooters` shooters, each hurting the main
/// character by 100 on the first tick.
fn boss_stage_with(shooters: usize, boss_hp: u32, heal: Value) -> Stage {
    let boss = boss_at(boss_hp, NEAR_Y);
    let waves = (0..shooters).map(|_| shooter()).collect();
    healing(stage_full(1_800, Some(boss), waves, vec![]), heal)
}

fn hp(e: &Engine) -> u32 {
    e.snapshot().player.hp
}

/// Steps `e` until `stop` holds, pressing the Main weapon every 6 ticks (`firing`); returns the
/// events of the ticks it ran, with their tick.
fn run(
    e: &mut Engine,
    max_ticks: u32,
    stop: impl FnMut(&Engine) -> bool,
) -> Vec<(u32, DomainEvent)> {
    fire_until(e, idle(), max_ticks, stop)
}

/// The amounts the main character was healed by, in order, on every tick run.
fn healed(seen: &[(u32, DomainEvent)]) -> Vec<u32> {
    seen.iter()
        .filter_map(|(_, ev)| match ev {
            DomainEvent::Healed { target, amount } if target.0 == PLAYER => Some(*amount),
            _ => None,
        })
        .collect()
}

fn count(seen: &[(u32, DomainEvent)], kind: fn(&DomainEvent) -> bool) -> usize {
    seen.iter().filter(|(_, ev)| kind(ev)).count()
}

fn is_drop(ev: &DomainEvent) -> bool {
    matches!(ev, DomainEvent::HealDropped { .. })
}

/// The boss (entity 1, after the main character) died on the last step.
fn boss_dead(e: &Engine) -> bool {
    e.events()
        .iter()
        .any(|ev| matches!(ev, DomainEvent::Died { entity, .. } if entity.0 == 1))
}

// Validation: the caps of the heal config and of the regen amount (ADR-020).

#[test]
fn a_heal_config_is_bounded_like_the_other_content() {
    let stage = |heal: Value| {
        let mut stage = stage_full(600, Some(far_boss()), vec![], vec![]);
        stage.heal = Some(serde_json::from_value(heal).expect("the heal config parses"));
        stage
    };
    let config = |boss_pct: u16, drop_one_in: u32, amount: u32, cap: u16| {
        json!({ "boss_pct": boss_pct, "drop_one_in": drop_one_in, "amount": amount,
                "cap": cap, "pickup_radius": 6_144 })
    };
    assert!(validate::validate(&stage(config(25, 150, PICKUP, 5))).is_ok());
    assert!(
        validate::validate(&stage(config(100, 1, 1, 1))).is_ok(),
        "the least of each"
    );
    assert!(
        validate::validate(&stage(config(101, 150, PICKUP, 5))).is_err(),
        "over 100 percent"
    );
    assert!(
        validate::validate(&stage(config(25, 0, PICKUP, 5))).is_err(),
        "a chance of 1 in 0"
    );
    assert!(
        validate::validate(&stage(config(25, 150, 0, 5))).is_err(),
        "a heal of 0"
    );
    assert!(
        validate::validate(&stage(config(25, 150, PICKUP, 0))).is_err(),
        "no pickups at all"
    );
    assert!(
        validate::validate(&stage(config(25, 150, PICKUP, 17))).is_err(),
        "past the hard cap"
    );
    assert!(
        validate::validate(&stage(config(25, 150, MAX_HEAL_AMOUNT, 5))).is_ok(),
        "the most a pickup heals"
    );
    assert!(
        validate::validate(&stage(config(25, 150, MAX_HEAL_AMOUNT + 1, 5))).is_err(),
        "past the most a pickup heals"
    );
}

#[test]
fn a_regen_reward_is_bounded_like_the_max_hit_points_reward() {
    let parses = |amount: u32| {
        let file = json!({ "schema_version": 0, "sim_version": 0,
                           "rewards": [regen("regen", "common", amount, 5)] });
        Rewards::parse(file.to_string().as_bytes()).is_ok()
    };
    assert!(parses(10 * DAMAGE_SCALE), "the common amount");
    assert!(parses(80 * DAMAGE_SCALE), "the supernova amount");
    assert!(parses(MAX_HP_AMOUNT), "the most hit points a second");
    assert!(!parses(0), "no regeneration");
    assert!(!parses(MAX_HP_AMOUNT + 1), "past the most");
}

// Boss kill: a quarter of the most hit points, never above them.

#[test]
fn a_boss_kill_heals_a_quarter_of_the_most_hit_points_as_far_as_the_most_allows() {
    // The shooters' bullets hurt the main character once: the invulnerability after a hit
    // absorbs the rest. The heal is 25 percent of the most (250) as far as the most leaves room
    // for the hit points it had before the heal.
    let mut e = engine(&boss_stage_with(3, 100, heal_json(25, 150, 5)));
    let seen = run(&mut e, 600, boss_dead);
    assert!(boss_dead(&e), "the boss falls within 600 ticks");
    let amount = healed(&seen)[0];
    let before = hp(&e) - amount;
    assert_eq!(amount, 250.min(MAX_HP - before));
    assert_eq!(hp(&e), (before + 250).min(MAX_HP), "never above the most");
}

#[test]
fn a_boss_kill_heals_up_to_the_most_hit_points_and_no_further() {
    // One bullet: 900 on the kill's tick; 250 more would be 1150, so it heals 100 to the most.
    let mut e = engine(&boss_stage_with(1, 100, heal_json(25, 150, 5)));
    let seen = run(&mut e, 600, boss_dead);
    assert_eq!(healed(&seen), vec![100]);
    assert_eq!(hp(&e), MAX_HP, "never above the most");
}

#[test]
fn a_boss_kill_heals_nothing_in_a_stage_without_healing() {
    let mut stage = boss_stage_with(3, 100, heal_json(25, 150, 5));
    stage.heal = None;
    let mut e = engine(&stage);
    let seen = run(&mut e, 600, boss_dead);
    assert!(boss_dead(&e), "the boss falls within 600 ticks");
    assert!(healed(&seen).is_empty(), "no heal without the config");
}

// Heal pickups: a regular kill drops one with the run's draw, and taking one heals 300.

#[test]
fn a_regular_kill_drops_a_pickup_at_one_in_one_and_the_first_heals_on_touch() {
    // One in one: every regular kill drops, and each pickup is taken where it fell. The bullet
    // hurts the main character to 900 first, so the first pickup heals 100 to the most.
    let stage = healing(
        stage_full(
            600,
            Some(far_boss()),
            vec![shooter(), targets(60, 3)],
            vec![],
        ),
        heal_json(25, 1, 5),
    );
    let mut e = engine(&stage);
    let seen = run(&mut e, 300, |e| e.tick() >= 200);
    assert_eq!(count(&seen, is_drop), 3, "every regular kill drops one");
    assert_eq!(healed(&seen)[0], 100, "the first pickup heals to the most");
    assert_eq!(hp(&e), MAX_HP, "never above the most");
}

#[test]
fn a_pickup_heals_300_shown_hit_points_as_far_as_the_most_allows() {
    // The kill at `NEAR_Y` drops a pickup the main character takes at once, on the tick the
    // shooter's bullet hurts it to 900: 300 shown more would be far past 1000, so it heals 100
    // to the most.
    let stage = healing(
        stage_full(
            600,
            Some(far_boss()),
            vec![shooter(), targets(60, 1)],
            vec![],
        ),
        heal_json(25, 1, 5),
    );
    let mut e = engine(&stage);
    let seen = run(&mut e, 300, |e| e.tick() >= 90);
    assert_eq!(healed(&seen), vec![100]);
    assert_eq!(hp(&e), MAX_HP, "at the most, never above");
}

#[test]
fn a_pickup_heals_far_more_than_300_raw_hit_points() {
    // At the most a stage allows (99 shown), a bullet of 90 shown hit points leaves 9 shown:
    // the pickup's 300 shown hit points heal all 90 back, to the most and no further.
    let mut stage = healing(
        stage_full(
            600,
            Some(far_boss()),
            vec![heavy_shooter(90 * DAMAGE_SCALE), targets(60, 1)],
            vec![],
        ),
        heal_json(25, 1, 5),
    );
    stage.player.hp = MAX_PLAYER_HP;
    let mut e = engine(&stage);
    let seen = run(&mut e, 300, |e| e.tick() >= 90);
    assert_eq!(healed(&seen), vec![90 * DAMAGE_SCALE]);
    assert_eq!(hp(&e), MAX_PLAYER_HP, "at the most, never above");
}

#[test]
fn the_bundled_heal_pickup_heals_300_shown_hit_points() {
    let content = RunContent::bundled();
    let trial = content.trial().heal.as_ref().expect("the trial mode heals");
    assert_eq!(trial.amount, 300 * DAMAGE_SCALE);
    assert_eq!(trial.boss_pct, 25, "a percent of the most: unscaled");
    if let Some(heal) = &content.tutorial().heal {
        assert_eq!(heal.amount, 300 * DAMAGE_SCALE, "tutorial");
    }
}

#[test]
fn the_pickup_draw_is_one_in_150_and_the_same_per_seed() {
    // 120 regular kills at 1 in 150: most likely none, and a dozen would be far off.
    let stage = healing(
        stage_full(
            3_000,
            Some(far_boss()),
            vec![targets(1, 100), targets(1, 20)],
            vec![],
        ),
        heal_json(25, 150, 16),
    );
    let mut a = engine(&stage);
    let mut b = engine(&stage);
    let mut kills = 0_usize;
    let mut drops = 0_usize;
    for _ in 0..1_500 {
        a.step(firing(idle(), a.tick()));
        b.step(firing(idle(), b.tick()));
        assert_eq!(a.state_hash(), b.state_hash(), "the same hash every tick");
        kills += a
            .events()
            .iter()
            .filter(|ev| matches!(ev, DomainEvent::Died { .. }))
            .count();
        drops += a.events().iter().filter(|ev| is_drop(ev)).count();
    }
    assert_eq!(kills, 120, "every enemy is killed");
    assert!(drops <= 3, "1 in 150 of {kills} kills, not {drops}");
}

#[test]
fn the_pickups_on_the_map_stay_within_the_cap() {
    // The pickup circle is 1 pixel, so none is taken: the map fills to its cap and stops.
    let mut heal = heal_json(25, 1, 2);
    heal["pickup_radius"] = json!(1);
    let stage = healing(
        stage_full(600, Some(far_boss()), vec![targets(1, 6)], vec![]),
        heal,
    );
    let mut e = engine(&stage);
    let seen = run(&mut e, 300, |e| e.tick() >= 200);
    assert_eq!(e.snapshot().heal_pickups.len(), 2, "held to the cap");
    assert_eq!(count(&seen, is_drop), 2, "the kills past the cap drop none");
    assert!(healed(&seen).is_empty(), "no heal from a pickup not taken");
}

// Regen: a regen reward adds its amount to the perk counter, a second at a time.

/// A stage where one kill drops a star of `rarity`, taken at once, after the shooter hurts the
/// main character from 99 shown hit points to 9: then the reward `regen` is offered and taken.
fn regen_offer(rarity: &str, amount: u32) -> Engine {
    let stage = armed(
        starry(
            stage_full(
                1_200,
                Some(far_boss()),
                vec![heavy_shooter(90 * DAMAGE_SCALE), targets(60, 1)],
                vec![],
            ),
            rarity,
        ),
        weapon(),
    );
    let mut stage = stage;
    stage.player.hp = MAX_PLAYER_HP;
    let list = vec![regen("regen", rarity, amount, 5)];
    let content = RunContent::new(RunStats::bundled().clone(), rewards(list));
    Engine::new_with(&stage, &content).expect("the test stage is valid")
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

#[test]
fn a_regen_reward_heals_its_amount_each_second_at_each_rarity() {
    for (rarity, amount) in bundled_regen() {
        let rarity = format!("{rarity:?}").to_lowercase();
        let rarity = rarity.as_str();
        let mut e = regen_offer(rarity, amount);
        run(&mut e, 300, |e| e.offer_open());
        assert!(e.offer_open(), "{rarity}: an offer opens after the kill");
        take(&mut e, "regen");
        let before = hp(&e);
        // One second is 60 ticks: exactly one regeneration in any 60 ticks.
        run(&mut e, 60, |_| false);
        assert_eq!(
            hp(&e) - before,
            amount,
            "{rarity}: {amount} hit points a second"
        );
    }
}

/// The bundled regen rewards' rarities and amounts, from common up.
fn bundled_regen() -> Vec<(Rarity, u32)> {
    let mut all: Vec<(Rarity, u32)> = Rewards::bundled()
        .rewards()
        .iter()
        .filter(|r| r.effect() == RewardEffect::Regen)
        .map(|r| (r.rarity, r.amount().unwrap()))
        .collect();
    all.sort_by_key(|(rarity, _)| rarity.index());
    all
}

#[test]
fn the_bundled_regen_rewards_give_shown_hit_points_a_second() {
    // Kade's 10 / 30 / 60 shown for common / rare / legendary, and the ladder of core#236
    // (×1.5, ×3.5, ×8 of common) for uncommon / epic / supernova.
    let shown = [10, 15, 30, 35, 60, 80].map(|n| n * DAMAGE_SCALE);
    let expected: Vec<(Rarity, u32)> = Rarity::ALL.iter().copied().zip(shown).collect();
    assert_eq!(bundled_regen(), expected);
    assert!(
        expected.iter().all(|(_, a)| *a <= MAX_HP_AMOUNT),
        "within the cap"
    );
}
