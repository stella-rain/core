//! Healing (ADR-120 is not yet written, so this file cites none): a boss kill heals a quarter of
//! the main character's most hit points, never above it; a regular kill drops a heal pickup with
//! a chance of 1 in 150 from the run's generator, and taking the pickup heals 300 hit points,
//! never above the most; a regen reward adds its amount a second at its rarity (10, 30 and 60 for
//! common, rare and legendary). Hit points are in the stage's scale (ADR-103): 100 is one shown
//! hit point. Every number here is worked out by hand.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::rewards::{MAX_HP_AMOUNT, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::snapshot::OfferOption;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate;

use crate::common;

use common::*;

/// 32 pixels above the main character: inside the first step's hit box, and a kill there drops
/// its pickup where the main character can reach it.
const NEAR_Y: i32 = START_Y - 8_192;

/// The most hit points of the test main character.
const MAX_HP: u32 = 1_000;
/// The main character's entity ID.
const PLAYER: u32 = 0;

/// The bundled healing of `trial.json`'s numbers: a quarter for a boss, one pickup in 150 that
/// heals 300, at most 5 on the map.
fn heal_json(boss_pct: u16, drop_one_in: u32, cap: u16) -> Value {
    json!({
        "boss_pct": boss_pct, "drop_one_in": drop_one_in, "amount": 300, "cap": cap,
        "pickup_radius": 12_288
    })
}

/// `stage` healing with `heal`, a stage with levels and stars (so a kill's star offers open,
/// taken from the pool of `rarity`), the main character at `MAX_HP`.
fn healing(mut stage: Stage, heal: Value) -> Stage {
    stage.heal = Some(serde_json::from_value(heal).expect("the heal config parses"));
    stage.player.hp = MAX_HP;
    stage
}

/// The rewards file with `list` in it.
fn rewards(list: Vec<Value>) -> Rewards {
    let file = json!({ "schema_version": 0, "sim_version": 0, "rewards": list });
    Rewards::parse(file.to_string().as_bytes()).expect("the test rewards validate")
}

/// A common-or-other regen reward of `amount` hit points a second, taken at most `limit` times.
fn regen(id: &str, rarity: &str, amount: u32, limit: u8) -> Value {
    json!({
        "id": id, "rarity": rarity, "effect": "regen", "amount": amount, "limit": limit,
        "title": "Regeneration", "description": ""
    })
}

/// Levels of 1000 experience and stars of `rarity` only, which every kill drops, all taken.
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

/// A shooter at `NEAR_Y` that fires once at the main character (and the weapon kills it at
/// `NEAR_Y` if it is `hp` or fewer): one bullet of 100 hit points hurts it. A pickup is then
/// taken or not as the stage's radius allows.
fn shooter(hp: u32) -> Value {
    let rules = json!([attack_rule(always(), 10_000)]);
    wave(
        1,
        1,
        0,
        enemy(hp, hold(START_X, NEAR_Y), rules),
        START_X,
        NEAR_Y,
    )
}

/// `count` enemies of 10 hp at `NEAR_Y`, one every 6 ticks from tick 1, standing still.
fn targets(count: u16) -> Value {
    wave(
        1,
        count,
        6,
        enemy(10, hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    )
}

fn hp(e: &Engine) -> u32 {
    e.snapshot().player.hp
}

/// Steps `e` until `stop` holds, pressing the Main weapon every 6 ticks (`firing`); returns the
/// events of the ticks it ran.
fn run(
    e: &mut Engine,
    max_ticks: u32,
    stop: impl FnMut(&Engine) -> bool,
) -> Vec<(u32, DomainEvent)> {
    fire_until(e, idle(), max_ticks, stop)
}

/// The amounts the main character was healed by, in order.
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

fn is_death(ev: &DomainEvent) -> bool {
    matches!(ev, DomainEvent::Died { .. })
}

/// The boss (entity 1, after the main character) died: its `Died` event is in the last tick.
fn boss_dead(e: &Engine) -> bool {
    e.events()
        .iter()
        .any(|ev| matches!(ev, DomainEvent::Died { entity } if entity.0 == 1))
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
    assert!(validate::validate(&stage(config(25, 150, 300, 5))).is_ok());
    assert!(
        validate::validate(&stage(config(100, 1, 1, 1))).is_ok(),
        "the least of each"
    );
    assert!(
        validate::validate(&stage(config(101, 150, 300, 5))).is_err(),
        "over 100 percent"
    );
    assert!(
        validate::validate(&stage(config(25, 0, 300, 5))).is_err(),
        "a chance of 1 in 0"
    );
    assert!(
        validate::validate(&stage(config(25, 150, 0, 5))).is_err(),
        "a heal of 0"
    );
    assert!(
        validate::validate(&stage(config(25, 150, 300, 0))).is_err(),
        "no pickups at all"
    );
    assert!(
        validate::validate(&stage(config(25, 150, 300, 17))).is_err(),
        "past the hard cap"
    );
}

#[test]
fn a_regen_reward_is_bounded_like_the_max_hit_points_reward() {
    let parses = |amount: u32| {
        let file = json!({ "schema_version": 0, "sim_version": 0,
                           "rewards": [regen("regen", "common", amount, 5)] });
        Rewards::parse(file.to_string().as_bytes()).is_ok()
    };
    assert!(parses(10), "the common amount");
    assert!(parses(MAX_HP_AMOUNT), "the most hit points a second");
    assert!(!parses(0), "no regeneration");
    assert!(!parses(MAX_HP_AMOUNT + 1), "past the most");
}

// Boss kill: a quarter of the most hit points, never above them.

/// The boss of `hp` at `NEAR_Y`, which the weapon kills, with the shooter hurting the main
/// character once before it.
fn boss_stage(boss_hp: u32, heal: Value) -> Stage {
    let boss = boss_at(boss_hp, NEAR_Y);
    let waves = vec![shooter(10_000)];
    healing(stage_full(1_800, Some(boss), waves, vec![]), heal)
}

#[test]
fn a_boss_kill_heals_a_quarter_of_the_most_hit_points() {
    let mut e = Engine::new(&boss_stage(10, heal_json(25, 150, 5))).unwrap();
    // The shooter's bullet hurts the main character before the boss falls.
    let seen = run(&mut e, 600, boss_dead);
    assert!(boss_dead(&e), "the boss falls within 600 ticks");
    let boss_heals = healed(&seen);
    // 25 percent of 1000 is 250; the bullet's 100 is the only damage taken, so the heal is 250
    // and it lands at 1000 (never above) once the hit points were 900.
    assert_eq!(
        boss_heals,
        vec![250],
        "one boss kill, one heal of a quarter"
    );
    assert!(hp(&e) <= MAX_HP, "never above the most");
}

#[test]
fn a_boss_kill_heals_nothing_in_a_stage_without_healing() {
    let mut stage = boss_stage(10, heal_json(25, 150, 5));
    stage.heal = None;
    let mut e = Engine::new(&stage).unwrap();
    let seen = run(&mut e, 600, boss_dead);
    assert!(healed(&seen).is_empty(), "no heal without the config");
}

// Heal pickups: a regular kill drops one with the run's draw, taking one heals 300.

#[test]
fn a_regular_kill_drops_a_pickup_at_one_in_one_and_it_heals_on_touch() {
    // One in one: every regular kill drops, so every kill's pickup is taken where it fell.
    let stage = healing(
        starry(
            stage_full(600, Some(far_boss()), vec![targets(3)], vec![]),
            "common",
        ),
        heal_json(25, 1, 5),
    );
    let mut e = Engine::new(&stage).unwrap();
    let seen = run(&mut e, 300, |e| e.tick() >= 200);
    let drops = count(&seen, is_drop);
    assert!(drops >= 1, "a kill dropped a pickup");
    assert!(hp(&e) <= MAX_HP, "never above the most");
}

#[test]
fn a_pickup_heals_300_and_never_above_the_most() {
    // The shooter hurts the main character to 900 first; the kill at NEAR_Y drops a pickup the
    // main character takes at once. 900 + 300 is 1200, so it heals 100 to the most.
    let stage = healing(
        starry(
            stage_full(600, Some(far_boss()), vec![shooter(10), targets(1)], vec![]),
            "common",
        ),
        heal_json(25, 1, 5),
    );
    let mut e = Engine::new(&stage).unwrap();
    let seen = run(&mut e, 300, |e| {
        e.snapshot().player.hp < 1_000 && e.tick() > 30
    });
    let pickup_heals = healed(&seen);
    assert!(!pickup_heals.is_empty(), "the pickup was taken");
    assert_eq!(pickup_heals[0], 100, "healed up to the most, not by 300");
    assert_eq!(hp(&e), MAX_HP, "at the most, never above");
}

#[test]
fn the_pickup_draw_is_one_in_150_and_the_same_per_seed() {
    let stage = healing(
        starry(
            stage_full(
                3_000,
                Some(far_boss()),
                vec![targets(100), targets(20)],
                vec![],
            ),
            "common",
        ),
        heal_json(25, 150, 16),
    );
    let mut a = Engine::new(&stage).unwrap();
    let mut b = Engine::new(&stage).unwrap();
    let mut kills = 0_usize;
    let mut drops = 0_usize;
    for _ in 0..1_500 {
        a.step(firing(idle(), a.tick()));
        b.step(firing(idle(), b.tick()));
        assert_eq!(a.state_hash(), b.state_hash(), "the same hash every tick");
        kills += a.events().iter().filter(|ev| is_death(ev)).count();
        drops += a.events().iter().filter(|ev| is_drop(ev)).count();
    }
    // 120 regular kills at 1 in 150 are most likely none or one, and at most a handful: the
    // draw is per regular kill, so a dozen would be far off.
    assert!(kills <= 120, "no more kills than enemies");
    assert!(drops <= kills, "a pickup per kill at most");
    assert!(drops <= 3, "1 in 150 of {kills} kills, not {drops}");
}

#[test]
fn the_pickups_on_the_map_stay_within_the_cap() {
    // The pickup circle is 1 pixel, so none is taken: the map fills to its cap and stops.
    let mut heal = heal_json(25, 1, 2);
    heal["pickup_radius"] = json!(1);
    let stage = healing(
        starry(
            stage_full(600, Some(far_boss()), vec![targets(6)], vec![]),
            "common",
        ),
        heal,
    );
    let mut e = Engine::new(&stage).unwrap();
    run(&mut e, 300, |e| e.tick() >= 200);
    assert_eq!(e.snapshot().heal_pickups.len(), 2, "held to the cap");
    assert_eq!(hp(&e), MAX_HP, "no heal from a pickup not taken");
}

// Regen: a regen reward adds its amount to the perk counter, a second at a time.

/// A stage where one kill drops a star of `rarity` taken at once, after the shooter hurts the
/// main character: then the reward `regen` is offered, taken and held for a second.
fn regen_offer(rarity: &str, amount: u32) -> Engine {
    let stage = starry(
        stage_full(
            1_200,
            Some(far_boss()),
            vec![shooter(10), targets(1)],
            vec![],
        ),
        rarity,
    );
    let list = vec![regen("regen", rarity, amount, 5)];
    let content = RunContent::new(RunStats::bundled().clone(), rewards(list));
    Engine::new_with(&stage, &content).expect("the test stage is valid")
}

#[test]
fn a_regen_reward_heals_its_amount_each_second_at_each_rarity() {
    for (rarity, amount) in [("common", 10), ("rare", 30), ("legendary", 60)] {
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
