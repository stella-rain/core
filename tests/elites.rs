//! Elites (ADR-068, app#214): enemies that hold where they spawned and act only while the main
//! character is within their range, with their own cap on the map; a killed elite always drops
//! a star, drawn from the drop table's next tier up.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::rewards::{Rarity, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{self, Error, limits};
use stella_rain_core::version::Versions;

use crate::common;

use common::*;

/// Far above the main character: out of its pickup circle, in the line of its shots.
const FAR_Y: i32 = 100_000;
/// 64 pixels.
const RANGE: i32 = 16_384;

/// An elite of `hp` that would follow the main character, with `range` and a rule that
/// telegraphs at it whenever it can.
fn elite(hp: u32, range: i32) -> Value {
    let mut e = enemy(
        hp,
        json!({ "type": "follow", "distance": 0 }),
        json!([rule(always(), json!("player"), telegraph(), 0)]),
    );
    e["elite"] = json!({ "range": range });
    e
}

/// A stage with levels (1000 experience a level) and `waves`; `stars` if given.
fn stage(waves: Vec<Value>, stars: Option<Value>) -> Stage {
    let mut stage = stage_full(18_000, Some(far_boss()), waves, vec![]);
    stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).expect("levels parse"));
    stage.stars = stars.map(|v| serde_json::from_value(v).expect("stars parse"));
    stage
}

fn rewards() -> Rewards {
    let file = json!({ "rarities": common::rarities(), "rewards": [
        { "id": "c", "rarity": "common", "effect": "max_hp", "amount": 1, "limit": 9,
          "title": "Reward", "description": "" },
        { "id": "l", "rarity": "legendary", "effect": "max_hp", "amount": 1, "limit": 9,
          "title": "Reward", "description": "" } ] });
    Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT)
        .expect("the test rewards validate")
}

fn with(stage: &Stage) -> Engine {
    let content = RunContent::new(RunStats::bundled().clone(), rewards());
    Engine::new_with(stage, &content).expect("the test stage is valid")
}

fn fired(events: &[(u32, DomainEvent)]) -> usize {
    events
        .iter()
        .filter(|(_, ev)| matches!(ev, DomainEvent::RuleFired { .. }))
        .count()
}

fn elites(e: &Engine) -> Vec<(i32, i32)> {
    e.snapshot()
        .entities
        .iter()
        .filter(|v| v.elite)
        .map(|v| (v.at.x.0, v.at.y.0))
        .collect()
}

// --- Holding and range -------------------------------------------------------------------------

#[test]
fn an_elite_holds_where_it_spawned_whatever_its_movement() {
    // 100 pixels to the side of the main character, out of its line of fire and its range.
    let at = (START_X + 25_600, START_Y);
    let mut e = with(&stage(
        vec![wave(1, 1, 0, elite(10, RANGE), at.0, at.1)],
        None,
    ));
    run_until(&mut e, idle(), 300, |_| false);
    assert_eq!(elites(&e), [at]);
}

#[test]
fn an_elite_acts_only_while_the_main_character_is_within_its_range() {
    let side = |dx: i32| {
        let mut e = with(&stage(
            vec![wave(1, 1, 0, elite(10, RANGE), START_X + dx, START_Y)],
            None,
        ));
        fired(&run_until(&mut e, idle(), 120, |_| false))
    };
    assert!(side(RANGE) > 0, "at its range it acts");
    assert_eq!(side(RANGE + 256), 0, "a pixel beyond, it waits");
}

#[test]
fn an_ordinary_enemy_acts_at_any_distance() {
    let mut far = elite(10, RANGE);
    far.as_object_mut().unwrap().remove("elite");
    let mut e = with(&stage(
        vec![wave(1, 1, 0, far, START_X + 25_600, FAR_Y)],
        None,
    ));
    assert!(fired(&run_until(&mut e, idle(), 120, |_| false)) > 0);
}

#[test]
fn elites_past_the_elite_cap_do_not_spawn_but_other_enemies_do() {
    let mut st = stage(
        vec![
            wave(1, 5, 1, elite(10, RANGE), START_X + 25_600, START_Y),
            wave(1, 3, 1, enemy(10, hold(1000, 1000), json!([])), 1000, 1000),
        ],
        None,
    );
    st.elite_cap = Some(2);
    let mut e = with(&st);
    run_until(&mut e, idle(), 30, |_| false);
    assert_eq!(elites(&e).len(), 2);
    assert_eq!(enemies(&e.snapshot()), 5);
}

// --- An elite's star ---------------------------------------------------------------------------

/// Stars that never drop by chance, with a pity of 2, and a drop table of two tiers: common
/// from level 1, legendary from level 2.
fn stingy_stars() -> Value {
    json!({ "drop_per_mille": 0, "pity": 2, "cap": 16, "pickup_radius": 256,
            "tiers": [{ "from_level": 1, "weights": { "common": 1 } },
                      { "from_level": 2, "weights": { "legendary": 1 } }] })
}

/// One enemy of 10 hp straight above the main character at `tick`, elite or not; the Main
/// weapon kills it a few ticks later, before the next comes.
fn target(tick: u32, is_elite: bool) -> Value {
    let mut e = enemy(10, hold(START_X, FAR_Y), json!([]));
    if is_elite {
        e["elite"] = json!({ "range": 256 });
    }
    wave(tick, 1, 0, e, START_X, FAR_Y)
}

/// The kills, counted from 1, that dropped a star, and the rarities dropped.
fn drops(e: &mut Engine, ticks: u32) -> (Vec<usize>, Vec<Rarity>) {
    let (mut kills, mut at, mut rarities) = (0, Vec::new(), Vec::new());
    for _ in 0..ticks {
        e.step(firing(idle(), e.tick()));
        for ev in e.events() {
            match ev {
                DomainEvent::Died { .. } => kills += 1,
                DomainEvent::StarDropped { rarity, .. } => {
                    at.push(kills);
                    rarities.push(*rarity);
                }
                _ => {}
            }
        }
    }
    (at, rarities)
}

#[test]
fn a_killed_elite_always_drops_a_star_from_the_next_tier_up_and_leaves_the_pity_alone() {
    // Kill 1 misses (one miss); the elite, kill 2, drops; kill 3 misses (two misses); kill 4
    // drops by the pity. Had the elite's star reset the pity, kill 4 would miss too.
    let waves = vec![
        target(1, false),
        target(100, true),
        target(200, false),
        target(300, false),
    ];
    let mut e = with(&stage(waves, Some(stingy_stars())));
    let (at, rarities) = drops(&mut e, 400);
    assert_eq!(at, [2, 4]);
    assert_eq!(rarities, [Rarity::Legendary, Rarity::Common]);
}

#[test]
fn an_elite_in_the_last_tier_draws_from_the_last_tier() {
    let mut st = stingy_stars();
    st["tiers"] = json!([{ "from_level": 1, "weights": { "common": 1 } }]);
    let mut e = with(&stage(vec![target(1, true)], Some(st)));
    assert_eq!(drops(&mut e, 100), (vec![1], vec![Rarity::Common]));
}

#[test]
fn an_elite_drops_no_star_on_a_full_map() {
    let mut st = stingy_stars();
    st["cap"] = json!(1);
    st["pity"] = json!(1);
    // Kill 1 misses, kill 2 drops by the pity and fills the map; the elite, kill 3, drops none.
    let waves = vec![target(1, false), target(100, false), target(200, true)];
    let mut e = with(&stage(waves, Some(st)));
    assert_eq!(drops(&mut e, 300).0, [2]);
}

#[test]
fn summoned_elites_hold_where_they_spawn_and_count_toward_the_cap() {
    // An enemy beside the main character summons 3 elites at once, with room for 2.
    let mut call = summon(3);
    call["agent"] = elite(10, RANGE);
    let at = (START_X + 25_600, START_Y);
    let summoner = enemy(
        10,
        hold(at.0, at.1),
        json!([json!({ "when": always(), "target": "self", "do": call, "once": true })]),
    );
    let mut st = stage(vec![wave(1, 1, 0, summoner, at.0, at.1)], None);
    st.elite_cap = Some(2);
    let mut e = with(&st);
    run_until(&mut e, idle(), 120, |_| false);
    let held = elites(&e);
    assert_eq!(held.len(), 2, "{held:?}");
    // Three on one point push apart, so each stays near it, not on it.
    for (x, y) in held {
        let (dx, dy) = (i64::from(x - at.0), i64::from(y - at.1));
        assert!(
            dx * dx + dy * dy < 4096 * 4096,
            "({x}, {y}) strayed from {at:?}"
        );
    }
}

// --- The boss calls them ----------------------------------------------------------------------

/// 100 pixels to the side of the main character: out of its line of fire and of `RANGE`.
const SIDE: (i32, i32) = (START_X + 25_600, START_Y);

/// A stage whose boss, of `hp` straight above the main character in its line of fire, calls
/// its elites by `call`; one elite at `SIDE` from `tick`.
fn calling(hp: u32, call: Option<Value>, tick: u32) -> Stage {
    let mut boss = boss_at(hp, FAR_Y);
    if let Some(call) = call {
        boss["call_elites"] = call;
    }
    let mut st = stage(
        vec![wave(tick, 1, 0, elite(10, RANGE), SIDE.0, SIDE.1)],
        None,
    );
    st.boss = Some(serde_json::from_value(boss).expect("boss parses"));
    st
}

/// The ticks on which the boss called its elites.
fn calls(events: &[(u32, DomainEvent)]) -> Vec<u32> {
    ticks_of(events, |ev| matches!(ev, DomainEvent::ElitesCalled { .. }))
}

#[test]
fn the_boss_calls_its_elites_once_its_time_is_up_and_they_leave_their_place() {
    let mut e = with(&calling(1_000_000, Some(json!({ "after_ticks": 60 })), 1));
    let events = run_until(&mut e, idle(), 60, |_| false);
    assert_eq!(calls(&events), Vec::<u32>::new());
    assert_eq!(fired(&events), 0, "out of range before the call");
    assert_eq!(elites(&e), [SIDE]);
    // Ten ticks: soon after, it walks into the main character's line of fire.
    let events = run_until(&mut e, idle(), 10, |_| false);
    assert_eq!(
        calls(&events),
        [61],
        "the boss's 60 ticks are up on tick 61"
    );
    assert!(
        events
            .iter()
            .any(|(_, ev)| *ev == DomainEvent::ElitesCalled { elites: 1 }),
        "{events:?}"
    );
    assert!(fired(&events) > 0, "called, it acts at any distance");
    let (x, _) = elites(&e)[0];
    assert!(x < SIDE.0, "called, it follows the main character: {x}");
}

#[test]
fn the_boss_calls_them_once_its_hit_points_fall_below_its_share() {
    // 100 hp and 10 a shot: the boss is below 70% after its 4th hit, and calls on the tick after.
    let mut e = with(&calling(100, Some(json!({ "below_pct": 70 })), 1));
    let events = fire_until(&mut e, idle(), 600, |e| {
        e.events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::ElitesCalled { .. }))
    });
    let hits: Vec<u32> = events
        .iter()
        .filter(|(_, ev)| matches!(ev, DomainEvent::Hit { damage: 10, .. }))
        .map(|(t, _)| *t)
        .collect();
    assert!(hits.len() >= 4, "{hits:?}");
    assert_eq!(calls(&events), [hits[3] + 1]);
}

#[test]
fn whichever_comes_first_calls_and_0_ticks_calls_at_once() {
    let call = json!({ "after_ticks": 0, "below_pct": 70 });
    let mut e = with(&calling(1_000_000, Some(call), 1));
    assert_eq!(calls(&run_until(&mut e, idle(), 30, |_| false)), [1]);
}

#[test]
fn a_boss_without_a_call_never_calls() {
    let mut e = with(&calling(1_000_000, None, 1));
    let events = run_until(&mut e, idle(), 600, |_| false);
    assert_eq!(calls(&events), Vec::<u32>::new());
    assert_eq!(elites(&e), [SIDE]);
}

#[test]
fn an_elite_that_comes_after_the_call_is_called_at_once() {
    let mut e = with(&calling(1_000_000, Some(json!({ "after_ticks": 0 })), 100));
    let events = run_until(&mut e, idle(), 110, |_| false);
    assert_eq!(calls(&events), [1], "one call, before the elite came");
    let (x, _) = elites(&e)[0];
    assert!(x < SIDE.0, "it follows the main character at once: {x}");
}

#[test]
fn the_trial_run_meets_elites_so_the_corpus_covers_them() {
    // The trial corpus run: an elite fires within its range, one is killed, and the boss
    // calls the one left. A change to elites then shows in the corpus hashes.
    use stella_rain_core::run_replay::Command;
    let mut e = Engine::start(&run::trial_setup()).unwrap();
    let (mut fired_before_call, mut killed, mut called) = (false, false, 0);
    for c in crate::run_play::play_run(run::TRIAL_LENGTH) {
        let ids: Vec<u32> = e
            .snapshot()
            .entities
            .iter()
            .filter(|v| v.elite)
            .map(|v| v.id.0)
            .collect();
        match c {
            Command::Step(input) => e.step(input),
            Command::Choose(i) => e.choose(i).unwrap(),
        }
        for ev in e.events() {
            match ev {
                DomainEvent::RuleFired { agent, .. } if ids.contains(&agent.0) && called == 0 => {
                    fired_before_call = true;
                }
                DomainEvent::Died { entity, .. } if ids.contains(&entity.0) => killed = true,
                DomainEvent::ElitesCalled { elites } => called = *elites,
                _ => {}
            }
        }
    }
    assert!(fired_before_call && killed, "{fired_before_call} {killed}");
    assert_eq!(called, 1);
}

// --- The validator -----------------------------------------------------------------------------

fn issues(stage: &Stage) -> Vec<String> {
    match validate::validate(stage) {
        Ok(()) => Vec::new(),
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        Err(e) => panic!("{e:?}"),
    }
}

#[test]
fn an_elite_s_range_is_bounded() {
    for (range, ok) in [
        (1, true),
        (limits::MAX_LEN, true),
        (0, false),
        (limits::MAX_LEN + 1, false),
    ] {
        let st = stage(vec![wave(1, 1, 0, elite(10, range), 1000, 1000)], None);
        let found = issues(&st);
        let expected: &[&str] = if ok {
            &[]
        } else {
            &["$.waves[0].enemy.elite.range"]
        };
        assert_eq!(found, expected, "range {range}");
    }
}

#[test]
fn a_companion_cannot_be_an_elite() {
    let mut c = companion(10, hold(1000, 1000), json!([]));
    c["elite"] = json!({ "range": RANGE });
    let mut st = stage_full(600, Some(far_boss()), vec![], vec![c]);
    st.levels = None;
    assert_eq!(issues(&st), ["$.companions[0].elite"]);
}

#[test]
fn the_elite_cap_is_bounded() {
    let max = limits::MAX_ELITES_ALIVE;
    for (cap, ok) in [(1, true), (max, true), (0, false), (max + 1, false)] {
        let mut st = stage(vec![], None);
        st.elite_cap = Some(cap);
        let expected: &[&str] = if ok { &[] } else { &["$.elite_cap"] };
        assert_eq!(issues(&st), expected, "cap {cap}");
    }
}

#[test]
fn a_boss_s_call_is_bounded_and_needs_a_time_or_a_share() {
    let cases: [(Value, &[&str]); 6] = [
        (json!({ "after_ticks": 0 }), &[]),
        (json!({ "after_ticks": limits::MAX_TICKS }), &[]),
        (
            json!({ "after_ticks": limits::MAX_TICKS + 1 }),
            &["$.boss.call_elites.after_ticks"],
        ),
        (json!({ "below_pct": 100 }), &[]),
        (json!({ "below_pct": 0 }), &["$.boss.call_elites.below_pct"]),
        (json!({}), &["$.boss.call_elites"]),
    ];
    for (call, expected) in cases {
        assert_eq!(
            issues(&calling(100, Some(call.clone()), 1)),
            expected,
            "{call}"
        );
    }
    let over = json!({ "below_pct": 101 });
    assert_eq!(
        issues(&calling(100, Some(over), 1)),
        ["$.boss.call_elites.below_pct"]
    );
}
