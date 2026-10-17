//! Falling stars and their offers (ADR-066 §2, §3, ADR-068): a kill may drop a star, with pity;
//! taking it opens an offer at the end of the tick and stops the world; the offer holds three
//! candidates at the star's rarity, a reroll of each, a skip and a banish of each. A star's
//! rarity follows the player's level. Only a stage with `levels` and `stars` has them.

use serde_json::{Value, json};
use stella_rain_core::engine::{ChoiceError, Engine, Outcome, SpendError};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::ContentId;
use stella_rain_core::rewards::{Rarity, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::snapshot::{OfferKind, OfferOption, OfferView};
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{self, Error, limits};

mod common;

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
                    "limit": limit, "title": "Reward", "description": "" })
        })
        .collect();
    let file = json!({ "schema_version": 0, "sim_version": 0, "rewards": rewards });
    Rewards::parse(file.to_string().as_bytes()).expect("the test rewards validate")
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

fn offer(e: &Engine) -> OfferView {
    e.snapshot()
        .progress
        .expect("a stage with levels")
        .offer
        .expect("an open offer")
}

fn candidates(e: &Engine) -> Vec<String> {
    offer(e)
        .candidates
        .into_iter()
        .map(|c| c.reward.0)
        .collect()
}

/// The index of `option` in the open offer.
fn index_of(e: &Engine, option: OfferOption) -> u32 {
    let i = offer(e)
        .options
        .iter()
        .position(|o| *o == option)
        .unwrap_or_else(|| panic!("{option:?} is not offered: {:?}", offer(e).options));
    u32::try_from(i).unwrap()
}

/// Steps with `input` until an offer is open, returning the events of the ticks run.
fn until_offer(e: &mut Engine, input: stella_rain_core::input::Input) -> Vec<(u32, DomainEvent)> {
    let events = run_until(e, input, 1200, Engine::offer_open);
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
    let events = run_until(&mut e, idle(), 600, |e| !e.snapshot().stars.is_empty());
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
    let events = run_until(&mut e, idle(), 600, |_| false);
    assert_eq!(deaths(&events), 3);
    assert!(e.snapshot().stars.is_empty());
}

/// The kills, counted from 1, that dropped a star, over `ticks` ticks of `input`.
fn dropping_kills(e: &mut Engine, ticks: u32, input: stella_rain_core::input::Input) -> Vec<usize> {
    let mut kills = 0;
    let mut out = Vec::new();
    for _ in 0..ticks {
        e.step(input);
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
fn a_full_map_drops_no_star_and_the_pity_does_not_count_the_kill() {
    // The second kill fills the map; kills 3 to 6 drop nothing and count for nothing. Once the
    // star is taken, the pity starts where it was left: the 7th kill misses, the 8th drops.
    let st = stars(json!({ "drop_per_mille": 0, "pity": 1, "cap": 1 }));
    let stage = starry(
        &[1000],
        st,
        vec![targets(1, 6, FAR_Y), targets(600, 2, FAR_Y)],
    );
    let mut e = with(&stage, commons(3));
    assert_eq!(dropping_kills(&mut e, 300, idle()), [2]);
    assert_eq!(e.snapshot().stars.len(), 1);
    until_offer(&mut e, moving(0, -127));
    e.choose(index_of(&e, OfferOption::Skip)).unwrap();
    assert!(e.snapshot().stars.is_empty());
    assert_eq!(dropping_kills(&mut e, 600, idle()), [2]);
}

#[test]
fn a_star_s_rarity_follows_the_player_s_level() {
    // Level 2 at 3 experience; the third kill reaches it, so its star is drawn from level 2's
    // tier, the one after the kill's experience.
    let st = stars(json!({ "tiers": [
        { "from_level": 1, "weights": { "common": 1 } },
        { "from_level": 2, "weights": { "legendary": 1 } } ] }));
    let mut e = with(&starry(&[3], st, vec![targets(1, 4, FAR_Y)]), commons(3));
    run_until(&mut e, idle(), 600, |e| e.snapshot().stars.len() == 4);
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
    run_until(&mut e, idle(), 1200, |e| e.snapshot().stars.len() == 60);
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
fn taking_a_star_opens_its_offer_at_the_end_of_the_tick_and_stops_the_world() {
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
        matches!(
            this_tick[..],
            [
                ..,
                DomainEvent::StarTaken { .. },
                DomainEvent::OfferOpened { .. }
            ]
        ),
        "{this_tick:?}"
    );
    let o = offer(&e);
    assert_eq!((o.rarity, o.candidates.len()), (Rarity::Common, 3));
    assert!(e.snapshot().stars.is_empty(), "the star was taken");
    let hash = e.state_hash();
    for _ in 0..10 {
        e.step(idle());
        assert_eq!(
            (e.tick(), e.state_hash()),
            (tick, hash),
            "the world is stopped"
        );
        assert!(e.events().is_empty());
    }
}

#[test]
fn an_offer_lists_its_options_in_order() {
    use OfferOption::{Banish, Reroll, Skip, Take};
    let mut e = with(
        &starry(&[1000], stars(json!({})), vec![targets(1, 1, NEAR_Y)]),
        commons(4),
    );
    until_offer(&mut e, idle());
    let o = offer(&e);
    assert_eq!((o.rerolls_left, o.banishes_left), (1, 1));
    let take = (0..3).map(|candidate| Take { candidate });
    let reroll = (0..3).map(|candidate| Reroll { candidate });
    let banish = (0..3).map(|candidate| Banish { candidate });
    let all: Vec<OfferOption> = take
        .clone()
        .chain(reroll)
        .chain([Skip])
        .chain(banish)
        .collect();
    assert_eq!(o.options, all);

    let mut e = with(
        &starry(
            &[1000],
            stars(json!({ "rerolls": 0, "banishes": 0 })),
            vec![targets(1, 1, NEAR_Y)],
        ),
        commons(4),
    );
    until_offer(&mut e, idle());
    let few: Vec<OfferOption> = take.chain([Skip]).collect();
    assert_eq!(offer(&e).options, few);
}

#[test]
fn taking_a_candidate_applies_its_reward_and_the_run_goes_on() {
    let five_hp = rewards(&[
        ("a", "common", "max_hp", 5, 5),
        ("b", "common", "max_hp", 5, 5),
        ("c", "common", "max_hp", 5, 5),
    ]);
    let mut e = with(
        &starry(&[1000], stars(json!({})), vec![targets(1, 1, NEAR_Y)]),
        five_hp,
    );
    until_offer(&mut e, idle());
    let (tick, hp) = (e.tick(), e.snapshot().player.hp);
    let first = candidates(&e)[0].clone();
    assert_eq!(e.choose(0), Ok(()));
    assert_eq!(
        e.events(),
        [DomainEvent::RewardTaken {
            reward: ContentId(first),
            taken: 1
        }]
    );
    assert_eq!(e.snapshot().player.hp, hp + 5);
    assert!(!e.offer_open());
    assert_eq!(e.tick(), tick, "a choice is between ticks");
    e.step(idle());
    assert_eq!(e.tick(), tick + 1);
}

#[test]
fn skip_closes_the_offer_and_takes_nothing() {
    let mut e = with(
        &starry(&[1000], stars(json!({})), vec![targets(1, 1, NEAR_Y)]),
        commons(4),
    );
    until_offer(&mut e, idle());
    let hp = e.snapshot().player.hp;
    e.choose(index_of(&e, OfferOption::Skip)).unwrap();
    assert_eq!(e.events(), [DomainEvent::OfferSkipped]);
    assert!(!e.offer_open());
    assert_eq!(e.snapshot().player.hp, hp);
}

#[test]
fn a_reroll_draws_that_candidate_again_and_spends_a_reroll() {
    let mut e = with(
        &starry(&[1000], stars(json!({})), vec![targets(1, 1, NEAR_Y)]),
        commons(4),
    );
    until_offer(&mut e, idle());
    let before = candidates(&e);
    // Four rewards, three offered: the only one that can come is the one left out.
    let left_out = (0..4)
        .map(|i| format!("c{i}"))
        .find(|id| !before.contains(id))
        .unwrap();
    e.choose(index_of(&e, OfferOption::Reroll { candidate: 1 }))
        .unwrap();
    assert_eq!(
        e.events(),
        [DomainEvent::CandidateReplaced {
            slot: 1,
            reward: ContentId(left_out.clone())
        }]
    );
    let after = candidates(&e);
    assert_eq!(after, [before[0].clone(), left_out, before[2].clone()]);
    let o = offer(&e);
    assert_eq!(o.rerolls_left, 0);
    assert!(
        !o.options
            .iter()
            .any(|op| matches!(op, OfferOption::Reroll { .. })),
        "no reroll is left: {:?}",
        o.options
    );
}

#[test]
fn no_reroll_is_offered_when_no_other_reward_could_come() {
    let mut e = with(
        &starry(&[1000], stars(json!({})), vec![targets(1, 1, NEAR_Y)]),
        commons(3),
    );
    until_offer(&mut e, idle());
    assert!(
        !offer(&e)
            .options
            .iter()
            .any(|op| matches!(op, OfferOption::Reroll { .. }))
    );
}

#[test]
fn a_banished_reward_leaves_the_pool_for_the_rest_of_the_run() {
    let st = stars(json!({ "rerolls": 0, "banishes": 1 }));
    let mut e = with(
        &starry(&[1000], st, vec![targets(1, 12, NEAR_Y)]),
        commons(5),
    );
    until_offer(&mut e, idle());
    let banished = candidates(&e)[0].clone();
    e.choose(index_of(&e, OfferOption::Banish { candidate: 0 }))
        .unwrap();
    assert!(
        matches!(
            e.events(),
            [DomainEvent::RewardBanished { reward }, DomainEvent::CandidateReplaced { slot: 0, .. }]
                if reward.0 == banished
        ),
        "{:?}",
        e.events()
    );
    assert_eq!(offer(&e).banishes_left, 0);
    let mut seen = Vec::new();
    for _ in 0..8 {
        seen.extend(candidates(&e));
        e.choose(0).unwrap();
        if run_until(&mut e, idle(), 600, Engine::offer_open).is_empty() || !e.offer_open() {
            break;
        }
    }
    assert!(seen.len() > 10, "{seen:?}");
    assert!(!seen.contains(&banished), "{seen:?}");
}

#[test]
fn banishing_the_last_reward_closes_the_offer_and_an_empty_pool_opens_none() {
    let st = stars(json!({ "rerolls": 0, "banishes": 2 }));
    let mut e = with(
        &starry(&[1000], st, vec![targets(1, 2, NEAR_Y)]),
        commons(1),
    );
    until_offer(&mut e, idle());
    assert_eq!(candidates(&e), ["c0"]);
    e.choose(index_of(&e, OfferOption::Banish { candidate: 0 }))
        .unwrap();
    assert_eq!(
        e.events(),
        [DomainEvent::RewardBanished {
            reward: ContentId("c0".into())
        }]
    );
    assert!(!e.offer_open());
    // The second star is taken, but nothing is left to offer.
    let events = run_until(&mut e, idle(), 600, |_| false);
    assert!(
        events
            .iter()
            .any(|(_, ev)| matches!(ev, DomainEvent::StarTaken { .. }))
    );
    assert!(!e.offer_open());
}

#[test]
fn a_reward_at_its_stack_limit_is_offered_no_more() {
    let list = rewards(&[
        ("once", "common", "max_hp", 1, 1),
        ("b", "common", "max_hp", 1, 9),
        ("c", "common", "max_hp", 1, 9),
        ("d", "common", "max_hp", 1, 9),
    ]);
    let st = stars(json!({ "rerolls": 0, "banishes": 0 }));
    let mut e = with(&starry(&[1000], st, vec![targets(1, 12, NEAR_Y)]), list);
    let mut taken = false;
    let mut after = Vec::new();
    for _ in 0..12 {
        if run_until(&mut e, idle(), 600, Engine::offer_open).is_empty() && !e.offer_open() {
            break;
        }
        let c = candidates(&e);
        if taken {
            after.extend(c.clone());
        }
        let pick = c.iter().position(|id| id == "once").unwrap_or(0);
        taken |= c[pick] == "once";
        e.choose(u32::try_from(pick).unwrap()).unwrap();
    }
    assert!(taken && after.len() >= 9, "{after:?}");
    assert!(!after.iter().any(|id| id == "once"), "{after:?}");
}

#[test]
fn a_pool_short_of_the_star_s_rarity_fills_from_the_rarities_below() {
    let list = rewards(&[
        ("common_a", "common", "max_hp", 1, 5),
        ("rare_a", "rare", "max_hp", 1, 5),
        ("common_b", "common", "max_hp", 1, 5),
        ("legend", "legendary", "max_hp", 1, 5),
    ]);
    let st = stars(json!({ "tiers": [{ "from_level": 1, "weights": { "legendary": 1 } }] }));
    let mut e = with(&starry(&[1000], st, vec![targets(1, 1, NEAR_Y)]), list);
    until_offer(&mut e, idle());
    let o = offer(&e);
    assert_eq!(o.rarity, Rarity::Legendary);
    let rarities: Vec<Rarity> = o.candidates.iter().map(|c| c.rarity).collect();
    assert_eq!(rarities, [Rarity::Legendary, Rarity::Rare, Rarity::Common]);
}

#[test]
fn two_stars_taken_on_one_tick_open_two_offers_one_after_another() {
    // Two enemies 9 pixels either side of the line of fire leave two stars as far from it;
    // walking straight up reaches both on one tick.
    let side = |x: i32| wave(1, 1, 0, enemy(10, hold(x, FAR_Y), json!([])), x, FAR_Y);
    let mut e = with(
        &starry(
            &[1000],
            stars(json!({})),
            vec![side(START_X - 2304), side(START_X + 2304)],
        ),
        commons(4),
    );
    run_until(&mut e, idle(), 600, |e| e.snapshot().stars.len() == 2);
    let events = until_offer(&mut e, moving(0, -127));
    let tick = e.tick();
    let taken = events
        .iter()
        .filter(|(t, ev)| *t == tick && matches!(ev, DomainEvent::StarTaken { .. }))
        .count();
    assert_eq!(taken, 2);
    e.choose(index_of(&e, OfferOption::Skip)).unwrap();
    assert!(
        matches!(
            e.events(),
            [DomainEvent::OfferSkipped, DomainEvent::OfferOpened { .. }]
        ),
        "{:?}",
        e.events()
    );
    assert!(e.offer_open(), "the second offer opens at once");
    e.choose(index_of(&e, OfferOption::Skip)).unwrap();
    assert!(!e.offer_open());
    assert_eq!(e.tick(), tick);
}

#[test]
fn experience_rewards_raise_the_experience_of_every_kill() {
    // The first kill drops a star at once; with +50% experience, each later kill of a 2-xp
    // enemy gives 3.
    let list = rewards(&[
        ("insight", "common", "xp_pct", 50, 1),
        ("x", "common", "max_hp", 1, 1),
    ]);
    let mut first = targets(1, 1, NEAR_Y);
    first["xp"] = json!(2);
    let mut later = targets(300, 1, FAR_Y);
    later["xp"] = json!(2);
    let st = stars(json!({ "rerolls": 0, "banishes": 0, "cap": 1 }));
    let mut e = with(&starry(&[1000], st, vec![first, later]), list);
    until_offer(&mut e, idle());
    let pick = candidates(&e)
        .iter()
        .position(|id| id == "insight")
        .unwrap();
    e.choose(u32::try_from(pick).unwrap()).unwrap();
    let xp = e.snapshot().progress.unwrap().xp;
    assert_eq!(xp, 2);
    run_until(&mut e, idle(), 900, |e| {
        e.snapshot().progress.unwrap().xp > 2
    });
    assert_eq!(e.snapshot().progress.unwrap().xp, 2 + 3);
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

#[test]
fn a_boss_kill_opens_the_boss_offer_at_the_tier_s_highest_rarity_and_without_a_skip() {
    let mut e = with(&boss_stage(10), mixed());
    let events = until_offer(&mut e, idle());
    let tick = e.tick();
    let this_tick: Vec<&DomainEvent> = events
        .iter()
        .filter(|(t, _)| *t == tick)
        .map(|(_, ev)| ev)
        .collect();
    assert!(
        matches!(
            this_tick[..],
            [
                ..,
                DomainEvent::StageCleared,
                DomainEvent::OfferOpened {
                    rarity: Rarity::Rare,
                    kind: OfferKind::Boss,
                    ..
                }
            ]
        ),
        "{this_tick:?}"
    );
    assert_eq!(
        e.outcome(),
        Outcome::Cleared,
        "the boss kill clears the stage"
    );
    let o = offer(&e);
    assert_eq!((o.kind, o.rarity), (OfferKind::Boss, Rarity::Rare));
    let rarities: Vec<Rarity> = o.candidates.iter().map(|c| c.rarity).collect();
    assert_eq!(rarities, [Rarity::Rare; 3]);
    assert!(!o.options.contains(&OfferOption::Skip), "{:?}", o.options);
    assert_eq!(
        e.choose(u32::try_from(o.options.len()).unwrap()),
        Err(ChoiceError::OutOfRange {
            index: u32::try_from(o.options.len()).unwrap(),
            options: u32::try_from(o.options.len()).unwrap()
        })
    );
}

#[test]
fn the_boss_offer_is_answered_after_the_clear_and_the_run_stays_over() {
    let mut e = with(&boss_stage(10), mixed());
    until_offer(&mut e, idle());
    let (tick, hp) = (e.tick(), e.snapshot().player.hp);
    assert_eq!(e.choose(0), Ok(()));
    assert!(matches!(e.events(), [DomainEvent::RewardTaken { .. }]));
    assert!(!e.offer_open());
    assert_eq!(e.snapshot().player.hp, hp + 1);
    e.step(idle());
    assert_eq!((e.tick(), e.outcome()), (tick, Outcome::Cleared));
}

#[test]
fn a_star_offer_still_due_when_the_stage_ends_does_not_open() {
    // The kill of the last enemy clears a stage without a boss, and its star is taken on that
    // same tick: the run is over, so its offer never opens.
    let mut stage = starry(&[1000], stars(json!({})), vec![targets(1, 1, NEAR_Y)]);
    stage.boss = None;
    let mut e = with(&stage, commons(4));
    let events = run_until(&mut e, idle(), 600, |e| e.outcome() != Outcome::Running);
    assert_eq!(e.outcome(), Outcome::Cleared);
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
    run_until(&mut e, idle(), 600, |e| e.outcome() != Outcome::Running);
    assert_eq!(e.outcome(), Outcome::Cleared);
    assert!(!e.offer_open());
}

// --- Refusals ----------------------------------------------------------------------------------

#[test]
fn a_choice_outside_the_offer_is_refused_and_changes_nothing() {
    let mut e = with(
        &starry(&[1000], stars(json!({})), vec![targets(1, 1, NEAR_Y)]),
        commons(4),
    );
    until_offer(&mut e, idle());
    let (hash, options) = (e.state_hash(), offer(&e).options.len());
    let options = u32::try_from(options).unwrap();
    for index in [options, u32::MAX] {
        assert_eq!(
            e.choose(index),
            Err(ChoiceError::OutOfRange { index, options })
        );
    }
    assert_eq!(e.state_hash(), hash);
    assert!(e.offer_open());
}

#[test]
fn a_spend_while_an_offer_is_open_is_refused() {
    // Level 2 at 1 experience: the kill gives a point and the star at once.
    let mut e = with(
        &starry(&[1], stars(json!({})), vec![targets(1, 1, NEAR_Y)]),
        commons(4),
    );
    until_offer(&mut e, idle());
    assert_eq!(e.snapshot().progress.unwrap().points, 1);
    let hash = e.state_hash();
    assert_eq!(e.spend(0), Err(SpendError::OfferOpen));
    assert_eq!(e.state_hash(), hash);
    e.choose(index_of(&e, OfferOption::Skip)).unwrap();
    assert_eq!(e.spend(0), Ok(()));
}

#[test]
fn the_stars_and_the_offer_are_in_the_state_hash() {
    let stage = starry(&[1000], stars(json!({})), vec![targets(1, 1, NEAR_Y)]);
    let mut a = with(&stage, commons(4));
    until_offer(&mut a, idle());
    let mut b = a.clone();
    a.choose(0).unwrap();
    b.choose(1).unwrap();
    assert_ne!(a.state_hash(), b.state_hash(), "a different reward taken");
}

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
