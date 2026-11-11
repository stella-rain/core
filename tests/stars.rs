//! Falling stars and their offers (ADR-REPL-5): a kill may drop a star, with pity;
//! taking it opens an offer at the end of the tick and stops the world; the offer holds three
//! candidates at the star's rarity, a reroll of each, a skip and a banish of each. A star's
//! rarity follows the player's level. Only a stage with `levels` and `stars` has them.

use serde_json::{Value, json};
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::rewards::{Rarity, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{self, Error, limits};
use stella_rain_core::version::Versions;

use crate::common;

use common::*;

/// Far above the main character: out of its pickup circle.
const FAR_Y: i32 = 100_000;
/// 32 pixels above it: inside the pickup circle, so a star dropped here is taken at once.
const NEAR_Y: i32 = START_Y - 8_192;

/// `count` enemies of 10 hp (one shot each), one every 6 ticks from tick 1, holding at `y`
/// straight above the main character, in the line of its Main weapon's shots.
fn targets(at_tick: u32, count: u16, y: i32) -> Value {
    wave(
        at_tick,
        count,
        6,
        enemy(10, hold(START_X, y), json!([])),
        START_X,
        y,
    )
}

/// The stars of the tests: every kill drops one, all common, the map holds 8, the pickup circle
/// is 48 pixels, one reroll and one banish; `over` replaces fields.
fn stars(over: Value) -> Value {
    let mut v = json!({
        "drop_per_mille": 1000, "pity": 1, "cap": 8, "pickup_radius": 12_288,
        "rerolls": 1, "banishes": 1,
        "tiers": [{ "from_level": 1, "weights": { "common": 1 } }],
    });
    for (k, val) in over.as_object().unwrap() {
        v[k] = val.clone();
    }
    v
}

/// A stage with levels that need `xp` each and `stars`, and `waves`.
fn starry(xp: &[u32], stars: Value, waves: Vec<Value>) -> Stage {
    let mut stage = stage_full(18_000, Some(far_boss()), waves, vec![]);
    stage.levels = Some(serde_json::from_value(json!({ "xp": xp })).expect("levels parse"));
    stage.stars = Some(serde_json::from_value(stars).expect("stars parse"));
    stage
}

/// A rewards file of `(id, rarity, effect, amount, limit)`.
fn rewards(list: &[(&str, &str, &str, u32, u8)]) -> Rewards {
    let rewards: Vec<Value> = list
        .iter()
        .map(|(id, rarity, effect, amount, limit)| {
            json!({ "id": id, "rarity": rarity, "effect": effect, "amount": amount,
                    "limit": limit })
        })
        .collect();
    let file = json!({ "rarities": common::rarities(), "rewards": rewards });
    Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT)
        .expect("the test rewards validate")
}

/// `n` common rewards `c0`, `c1`, ..., each one more hit point, five times a run.
fn commons(n: usize) -> Rewards {
    let ids: Vec<String> = (0..n).map(|i| format!("c{i}")).collect();
    let list: Vec<(&str, &str, &str, u32, u8)> = ids
        .iter()
        .map(|id| (id.as_str(), "common", "max_hp", 1, 5))
        .collect();
    rewards(&list)
}

fn with(stage: &Stage, rewards: Rewards) -> Engine {
    let content = RunContent::new(RunStats::bundled().clone(), rewards);
    Engine::new_with(stage, &content).expect("the test stage is valid")
}

/// Steps with `input`, firing, until an offer is open, returning the events of the ticks run.
fn until_offer(e: &mut Engine, input: stella_rain_core::input::Input) -> Vec<(u32, DomainEvent)> {
    let events = fire_until(e, input, 1200, Engine::offer_open);
    assert!(e.offer_open(), "no offer within 1200 ticks");
    events
}

fn deaths(events: &[(u32, DomainEvent)]) -> usize {
    events
        .iter()
        .filter(|(_, ev)| matches!(ev, DomainEvent::Died { .. }))
        .count()
}

// --- Drops -------------------------------------------------------------------------------------

#[test]
fn a_kill_drops_a_star_where_the_enemy_fell() {
    let mut e = with(
        &starry(&[1000], stars(json!({})), vec![targets(1, 1, FAR_Y)]),
        commons(3),
    );
    let events = fire_until(&mut e, idle(), 600, |e| !e.snapshot().stars.is_empty());
    let s = e.snapshot();
    assert_eq!(s.stars.len(), 1);
    let star = s.stars[0];
    assert_eq!((star.at.x.0, star.at.y.0), (START_X, FAR_Y));
    assert_eq!(star.rarity, Rarity::Common);
    assert!(
        events.iter().any(|(_, ev)| *ev
            == DomainEvent::StarDropped {
                star: star.id,
                rarity: Rarity::Common
            }),
        "{events:?}"
    );
    assert!(!e.offer_open(), "a star far away waits on the map");
}

#[test]
fn a_stage_without_stars_drops_none() {
    let mut stage = starry(&[1000], stars(json!({})), vec![targets(1, 3, FAR_Y)]);
    stage.stars = None;
    let mut e = with(&stage, commons(3));
    let events = fire_until(&mut e, idle(), 600, |_| false);
    assert_eq!(deaths(&events), 3);
    assert!(e.snapshot().stars.is_empty());
}

/// The kills, counted from 1, that dropped a star, over `ticks` ticks of `input`.
fn dropping_kills(e: &mut Engine, ticks: u32, input: stella_rain_core::input::Input) -> Vec<usize> {
    let mut kills = 0;
    let mut out = Vec::new();
    for _ in 0..ticks {
        e.step(&[
            firing(input, e.tick()),
            Default::default(),
            Default::default(),
        ]);
        for ev in e.events() {
            match ev {
                DomainEvent::Died { .. } => kills += 1,
                DomainEvent::StarDropped { .. } => out.push(kills),
                _ => {}
            }
        }
    }
    out
}

#[test]
fn pity_drops_a_star_after_that_many_kills_without_one() {
    let st = stars(json!({ "drop_per_mille": 0, "pity": 3, "cap": 64 }));
    let mut e = with(
        &starry(&[1000], st, vec![targets(1, 12, FAR_Y)]),
        commons(3),
    );
    assert_eq!(dropping_kills(&mut e, 600, idle()), [4, 8, 12]);
}

#[test]
fn a_star_s_rarity_follows_the_player_s_level() {
    // Level 2 at 3 experience; the third kill reaches it, so its star is drawn from level 2's
    // tier, the one after the kill's experience.
    let st = stars(json!({ "tiers": [
        { "from_level": 1, "weights": { "common": 1 } },
        { "from_level": 2, "weights": { "legendary": 1 } } ] }));
    let mut e = with(&starry(&[3], st, vec![targets(1, 4, FAR_Y)]), commons(3));
    fire_until(&mut e, idle(), 600, |e| e.snapshot().stars.len() == 4);
    let rarities: Vec<Rarity> = e.snapshot().stars.iter().map(|s| s.rarity).collect();
    use Rarity::{Common, Legendary};
    assert_eq!(rarities, [Common, Common, Legendary, Legendary]);
}

#[test]
fn rarities_are_drawn_by_their_weights() {
    let st = stars(json!({ "cap": 64,
        "tiers": [{ "from_level": 1, "weights": { "common": 3, "rare": 1 } }] }));
    let mut e = with(
        &starry(&[1000], st, vec![targets(1, 60, FAR_Y)]),
        commons(3),
    );
    fire_until(&mut e, idle(), 1200, |e| e.snapshot().stars.len() == 60);
    let rare = e
        .snapshot()
        .stars
        .iter()
        .filter(|s| s.rarity == Rarity::Rare)
        .count();
    // 15 expected; a seeded draw, so the number is fixed, but any weighting bug moves it far.
    assert!((7..=25).contains(&rare), "{rare} rare of 60");
}

// --- Taking a star -----------------------------------------------------------------------------

#[test]
fn taking_a_star_stops_the_world_at_the_end_of_the_tick() {
    let mut e = with(
        &starry(&[1000], stars(json!({})), vec![targets(1, 1, NEAR_Y)]),
        commons(4),
    );
    let events = until_offer(&mut e, idle());
    let tick = e.tick();
    let this_tick: Vec<&DomainEvent> = events
        .iter()
        .filter(|(t, _)| *t == tick)
        .map(|(_, ev)| ev)
        .collect();
    assert!(
        matches!(this_tick[..], [.., DomainEvent::StarTaken { .. }]),
        "{this_tick:?}"
    );
    assert!(e.offer_open(), "the offer is open");
    assert!(e.snapshot().stars.is_empty(), "the star was taken");
    let hash = e.state_hash();
    for _ in 0..10 {
        e.step(&[
            firing(idle(), e.tick()),
            Default::default(),
            Default::default(),
        ]);
        assert_eq!(
            (e.tick(), e.state_hash()),
            (tick, hash),
            "the world is stopped"
        );
        assert!(e.events().is_empty());
    }
}

// --- The boss offer ----------------------------------------------------------------------------

/// A stage with stars whose boss, of `hp`, waits straight above the main character, in the line
/// of its Main weapon's shots; the drop table's one tier weighs common and rare.
fn boss_stage(hp: u32) -> Stage {
    let st =
        stars(json!({ "tiers": [{ "from_level": 1, "weights": { "common": 5, "rare": 1 } }] }));
    let mut stage = starry(&[1000], st, vec![]);
    stage.boss = Some(serde_json::from_value(boss_at(hp, FAR_Y)).expect("boss parses"));
    stage
}

/// Three common rewards and three rare ones.
fn mixed() -> Rewards {
    rewards(&[
        ("c0", "common", "max_hp", 1, 5),
        ("c1", "common", "max_hp", 1, 5),
        ("c2", "common", "max_hp", 1, 5),
        ("r0", "rare", "max_hp", 1, 5),
        ("r1", "rare", "max_hp", 1, 5),
        ("r2", "rare", "max_hp", 1, 5),
    ])
}

/// A stage without a boss whose one enemy's kill drops a star that is taken at once.
fn clearing_star_stage() -> Stage {
    // The enemy comes on tick 2, so that the stage may end before it (a wave must come before
    // the end), and the weapon kills it the tick it comes.
    let mut stage = starry(&[1000], stars(json!({})), vec![targets(2, 1, NEAR_Y)]);
    stage.boss = None;
    stage
}

/// The tick on which the star of `clearing_star_stage` is taken, which is also its clear.
fn clearing_star_tick() -> u32 {
    let mut e = with(&clearing_star_stage(), commons(4));
    let events = fire_until(&mut e, idle(), 600, |e| e.outcome() != Outcome::Running);
    assert_eq!(e.outcome(), Outcome::Cleared);
    let taken = events
        .iter()
        .find(|(_, ev)| matches!(ev, DomainEvent::StarTaken { .. }))
        .expect("the star is taken");
    assert_eq!(taken.0, e.tick(), "taken on the tick of the clear");
    taken.0
}

#[test]
fn a_fail_on_the_tick_a_star_is_taken_opens_no_offer() {
    let mut stage = clearing_star_stage();
    stage.boss = Some(serde_json::from_value(far_boss()).expect("boss parses"));
    stage.length_ticks = clearing_star_tick();
    let mut e = with(&stage, commons(4));
    let events = fire_until(&mut e, idle(), 600, |e| e.outcome() != Outcome::Running);
    assert_eq!(e.outcome(), Outcome::Failed);
    assert!(
        events
            .iter()
            .any(|(_, ev)| matches!(ev, DomainEvent::StarTaken { .. }))
    );
    assert!(!e.offer_open());
}

#[test]
fn a_boss_kill_in_a_stage_without_stars_opens_nothing() {
    let mut stage = boss_stage(10);
    stage.stars = None;
    let mut e = with(&stage, mixed());
    fire_until(&mut e, idle(), 600, |e| e.outcome() != Outcome::Running);
    assert_eq!(e.outcome(), Outcome::Cleared);
    assert!(!e.offer_open());
}

// --- Refusals ----------------------------------------------------------------------------------

// --- The validator -----------------------------------------------------------------------------

/// The paths of the issues the validator finds with `stars` in a stage with levels.
fn issues(stars: Value) -> Vec<String> {
    match validate::validate(&starry(&[10], stars, vec![])) {
        Ok(()) => Vec::new(),
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        Err(e) => panic!("{e:?}"),
    }
}

#[test]
fn stars_need_levels() {
    let mut stage = starry(&[10], stars(json!({})), vec![]);
    stage.levels = None;
    let Err(Error::Invalid(issues)) = validate::validate(&stage) else {
        panic!("stars without levels validate");
    };
    assert_eq!(issues[0].path, "$.stars");
}

#[test]
fn every_star_field_is_bounded() {
    use limits::*;
    let cases: [(&str, i64, i64); 6] = [
        ("drop_per_mille", 0, i64::from(MAX_DROP_PER_MILLE)),
        ("pity", 1, i64::from(MAX_PITY)),
        ("cap", 1, i64::from(MAX_STARS_ON_MAP)),
        ("pickup_radius", 1, i64::from(MAX_PICKUP_RADIUS)),
        ("rerolls", 0, i64::from(MAX_REROLLS)),
        ("banishes", 0, i64::from(MAX_BANISHES)),
    ];
    for (field, lo, hi) in cases {
        let path = format!("$.stars.{field}");
        assert_eq!(
            issues(stars(json!({ field: lo }))),
            Vec::<String>::new(),
            "{field} {lo}"
        );
        assert_eq!(
            issues(stars(json!({ field: hi }))),
            Vec::<String>::new(),
            "{field} {hi}"
        );
        assert_eq!(
            issues(stars(json!({ field: hi + 1 }))),
            std::slice::from_ref(&path),
            "{field} {hi}+1"
        );
        if lo > 0 {
            assert_eq!(
                issues(stars(json!({ field: lo - 1 }))),
                [path],
                "{field} {lo}-1"
            );
        }
    }
}

fn tiers(n: usize) -> Value {
    let tiers: Vec<Value> = (0..n)
        .map(|i| json!({ "from_level": i + 1, "weights": { "common": 1 } }))
        .collect();
    json!(tiers)
}

#[test]
fn the_drop_table_has_one_to_sixteen_tiers_in_level_order_from_level_1() {
    assert_eq!(
        issues(stars(json!({ "tiers": tiers(limits::MAX_TIERS) }))),
        Vec::<String>::new()
    );
    assert_eq!(
        issues(stars(json!({ "tiers": tiers(limits::MAX_TIERS + 1) }))),
        ["$.stars.tiers"]
    );
    assert_eq!(issues(stars(json!({ "tiers": [] }))), ["$.stars.tiers"]);
    let w = json!({ "common": 1 });
    let late_start = json!([{ "from_level": 2, "weights": w }]);
    assert_eq!(
        issues(stars(json!({ "tiers": late_start }))),
        ["$.stars.tiers[0].from_level"]
    );
    let same = json!([{ "from_level": 1, "weights": w }, { "from_level": 1, "weights": w }]);
    assert_eq!(
        issues(stars(json!({ "tiers": same }))),
        ["$.stars.tiers[1].from_level"]
    );
    let top = limits::MAX_TIER_LEVEL;
    for (level, ok) in [(top, true), (top + 1, false)] {
        let t = json!([{ "from_level": 1, "weights": w }, { "from_level": level, "weights": w }]);
        let found = issues(stars(json!({ "tiers": t })));
        assert_eq!(found.is_empty(), ok, "level {level}: {found:?}");
    }
}

#[test]
fn a_tier_weighs_something_and_no_rarity_weighs_too_much() {
    let tier = |weights: Value| json!({ "tiers": [{ "from_level": 1, "weights": weights }] });
    let max = limits::MAX_RARITY_WEIGHT;
    assert_eq!(
        issues(stars(tier(json!({ "supernova": max })))),
        Vec::<String>::new()
    );
    assert_eq!(
        issues(stars(tier(json!({ "rare": max + 1 })))),
        ["$.stars.tiers[0].weights"]
    );
    assert_eq!(issues(stars(tier(json!({})))), ["$.stars.tiers[0].weights"]);
}

// --- Weights -----------------------------------------------------------------------------------

// --- Named stars -------------------------------------------------------------------------------

#[test]
fn every_bundled_mode_takes_a_star_a_sword_step_away_without_moving() {
    // A star lands where its enemy fell, inside the hit circle that killed it: up to 64 pixels
    // from a player who stands and swings the main combo (its largest step). A pickup circle
    // smaller than that leaves the stars on the map until the player walks over them. 64 pixels
    // is also `limits::MAX_PICKUP_RADIUS`.
    use stella_rain_core::fixed::UNITS_PER_PX;
    use stella_rain_core::pack::Pack;
    let step = 64 * UNITS_PER_PX;
    let content = Pack::bundled().content();
    let modes = [
        ("time_limit", Some(content.time_limit().stars()), None),
        (
            "trial",
            Pack::bundled().trial().stars.as_ref(),
            Pack::bundled().trial().heal.as_ref(),
        ),
        ("tutorial", content.tutorial().stars.as_ref(), None),
    ];
    for (mode, stars, heal) in modes {
        let stars = stars.unwrap_or_else(|| panic!("{mode} has stars"));
        assert!(
            stars.pickup_radius.0 >= step,
            "{mode}: star pickup radius {} px is under a sword step",
            stars.pickup_radius.0 / UNITS_PER_PX
        );
        if let Some(heal) = heal {
            assert!(
                heal.pickup_radius.0 >= step,
                "{mode}: heal pickup radius {} px is under a sword step",
                heal.pickup_radius.0 / UNITS_PER_PX
            );
        }
    }
}
