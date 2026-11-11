//! The live (version 2) counterparts of `horde_tests.rs`: the same rules read through the live
//! accessors, started from the bundled files. The frozen file stays the guard of version 1.

use super::live_support::{
    live_bundled_run, live_engine_edited, live_run_of, live_time_limit_json,
};
use super::*;

/// The bundled mode's number `key` of every boss that calls its elites, which must agree.
fn bundled_call(key: &str) -> u64 {
    let mode = live_time_limit_json();
    let mut values: Vec<u64> = mode["bosses"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["call_elites"][key].as_u64())
        .collect();
    values.dedup();
    assert_eq!(
        values.len(),
        1,
        "every calling boss has one {key}: {values:?}"
    );
    values[0]
}

/// The bundled mode's overtime, 5 minutes of ticks.
fn overtime() -> u32 {
    let ticks = u32::try_from(live_time_limit_json()["overtime_ticks"].as_u64().unwrap()).unwrap();
    assert_eq!(ticks, 5 * 60 * 60);
    ticks
}

fn engine_edited(
    boss_ticks: &[u32],
    no_call_ticks: &[u32],
    edit: impl FnOnce(&mut serde_json::Value),
) -> Engine {
    live_engine_edited(boss_ticks, no_call_ticks, edit)
}

fn run_of(seed: u64, edit: impl FnOnce(&mut serde_json::Value)) -> Engine {
    live_run_of(seed, edit)
}

fn bundled_run(difficulty: &str, seed: u64) -> Engine {
    live_bundled_run(difficulty, seed)
}

fn engine(boss_ticks: &[u32]) -> Engine {
    engine_with(boss_ticks, &[])
}

fn engine_with(boss_ticks: &[u32], no_call_ticks: &[u32]) -> Engine {
    engine_edited(boss_ticks, no_call_ticks, |_| {})
}

fn run_to(e: &mut Engine, tick: u32) {
    while e.tick() < tick {
        e.step(&[Input::default(), Default::default(), Default::default()]);
    }
}

/// The tick each boss came on and the tick it called its elites on, the main character
/// kept alive.
fn arrivals_and_calls(e: &mut Engine, until: u32) -> (Vec<u32>, Vec<u32>) {
    let (mut arrived, mut called) = (Vec::new(), Vec::new());
    let mut last = None;
    while e.tick() < until {
        e.run.solo_mut().hp = e.run.solo().max_hp;
        e.step(&[Input::default(), Default::default(), Default::default()]);
        let id = e.run.live().boss.as_ref().map(|b| b.id);
        if id.is_some() && id != last {
            arrived.push(e.tick());
        }
        last = id;
        if e.events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::ElitesCalled { .. }))
        {
            called.push(e.tick());
        }
    }
    (arrived, called)
}

#[test]
fn a_boss_calls_its_elites_after_the_same_time_whenever_it_arrives() {
    // Mid-bosses at 60 and 3000, the final boss at 6000; the mode's call is 1800 ticks.
    let mut e = engine(&[60, 3000, 6000]);
    let (arrived, called) = arrivals_and_calls(&mut e, 6000);
    assert_eq!(arrived, [60, 3000, 6000]);
    let after = u32::try_from(bundled_call("after_ticks")).unwrap();
    assert_eq!(after, 1800);
    let delays: Vec<u32> = arrived.iter().zip(&called).map(|(a, c)| c - a).collect();
    assert_eq!(delays, [after, after], "{arrived:?} {called:?}");
}

/// Drops the boss on the field below 70% of its hit points and steps once: whether it
/// called its elites on that tick.
fn calls_below_70_pct(e: &mut Engine) -> bool {
    calls_at(e, 69)
}

/// Sets the boss on the field to `pct` percent of its hit points and steps once: whether it
/// called its elites on that tick. The mode's `below_pct` is 70.
fn calls_at(e: &mut Engine, pct: u32) -> bool {
    assert_eq!(bundled_call("below_pct"), 70);
    let boss = e.run.live_mut().boss.as_mut().unwrap();
    boss.hp = boss.max_hp * pct / 100;
    e.step(&[Input::default(), Default::default(), Default::default()]);
    e.events()
        .iter()
        .any(|ev| matches!(ev, DomainEvent::ElitesCalled { .. }))
}

#[test]
fn a_boss_calls_below_70_pct_unless_its_time_is_a_no_call_time() {
    // The first time stands for the 5-minute boss, the second for the 10-minute one.
    let mut e = engine_with(&[50, 200, 400], &[50]);
    run_to(&mut e, 60);
    assert!(!calls_below_70_pct(&mut e), "a no-call boss never calls");
    run_to(&mut e, 210);
    assert!(!calls_at(&mut e, 70), "exactly 70% is not below it");
    assert!(calls_below_70_pct(&mut e), "well before its 30 seconds");
}

/// The first summon rule's agent of a mid-boss gets this many hit points, plus the boss's
/// place in the mode file's list, so each boss's summon tells which boss made it.
const SUMMON_HP: u32 = 50_000;

fn summoning_mode(v: &mut serde_json::Value) {
    let agent = serde_json::json!({
        "base": "fairy_a",
        "hp": SUMMON_HP,
        "radius": 2048,
        "movement": {"type": "follow", "distance": 12288, "speed": 1},
        "rules": []
    });
    for (i, boss) in v["bosses"].as_array_mut().unwrap().iter_mut().enumerate() {
        let mut agent = agent.clone();
        agent["hp"] = serde_json::json!(SUMMON_HP + u32::try_from(i).unwrap());
        boss["phases"][0]["rules"] = serde_json::json!([{
            "when": {"type": "always"},
            "target": "self",
            "do": {"type": "summon", "count": 1, "agent": agent},
            "once": true
        }]);
    }
}

#[test]
fn a_boss_summon_outlives_its_boss_and_keeps_its_definition() {
    let mut e = engine_edited(&[50, 100, 150], &[], summoning_mode);
    run_to(&mut e, 60);
    let first = e.run.live().boss.as_ref().unwrap().id;
    let summon_hp = |e: &Engine| -> Vec<u32> {
        e.run
            .live()
            .enemies
            .iter()
            .map(|en| e.run.live_world().defs.agent(en.def).hp)
            .collect()
    };
    let made = summon_hp(&e);
    assert_eq!(made.len(), 1, "the first boss has summoned");
    e.run.live_mut().boss.as_mut().unwrap().hp = 0;
    for _ in 0..60 {
        e.run.solo_mut().hp = e.run.solo().max_hp;
        e.step(&[Input::default(), Default::default(), Default::default()]);
    }
    assert_ne!(
        e.run.live().boss.as_ref().unwrap().id,
        first,
        "the next boss has come"
    );
    let all = summon_hp(&e);
    assert_eq!(
        all[0], made[0],
        "the first summon still resolves to the first boss's summon agent"
    );
    assert_eq!(all.len(), 2, "the second boss summoned its own");
    assert_ne!(all[1], all[0], "from its own definition");
}

/// The definition of the boss on the field.
fn boss_def(e: &Engine) -> &crate::stage::Boss {
    &e.run
        .live_world()
        .boss(e.run.live().boss.as_ref().unwrap().boss)
        .boss
}

/// The slot of the boss on the field in the run's list of boss definitions.
fn boss_slot(e: &Engine) -> u32 {
    u32::from(e.run.live().boss.as_ref().unwrap().boss.0)
}

#[test]
fn the_world_compiles_each_horde_boss_once() {
    // Two sprites with no definition of their own are made from the template, and
    // each is its own boss; the same sprite coming twice is one.
    let mut e = engine_edited(&[50, 100, 150], &[], |v| {
        let pool = ["horn_charger", "weaver"];
        v["mid_boss_pool"] = serde_json::json!(pool);
        v["final_boss"] = serde_json::json!("horn_charger");
        v["bosses"]
            .as_array_mut()
            .unwrap()
            .retain(|b| !pool.contains(&b["base"].as_str().unwrap()));
    });
    // Three definitions of the file, and the template once for each of the two sprites.
    let bases: Vec<&str> = e
        .run
        .live_world()
        .bosses
        .iter()
        .map(|b| b.boss.base.as_str())
        .collect();
    assert_eq!(bases.len(), 5, "{bases:?}");
    let mut distinct = bases.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 5, "one definition per sprite: {bases:?}");
    let mut slots = Vec::new();
    for until in [60, 110, 160] {
        run_to(&mut e, until);
        slots.push(boss_slot(&e));
    }
    assert_ne!(
        slots[0], slots[1],
        "two sprites, two definitions: {slots:?}"
    );
    assert!(
        slots[2] == slots[0] || slots[2] == slots[1],
        "the final boss shares the slot of its sprite's first arrival: {slots:?}"
    );
}

#[test]
fn a_boss_coming_leaves_the_stage_as_it_was() {
    let mut e = engine(&[50, 100]);
    let before = e.run.live_world().stage.clone();
    run_to(&mut e, 100);
    assert!(e.run.live().boss.is_some(), "a boss has come");
    assert_eq!(
        e.run.live_world().stage,
        before,
        "the stage is read-only for a run (core#271)"
    );
}

#[test]
fn the_final_boss_is_the_core_pair_and_its_death_clears_the_run() {
    let mut e = engine(&[50, 100]);
    run_to(&mut e, 100);
    assert_eq!(boss_def(&e).base.as_str(), "core_pair");
    assert_eq!(e.run.live().boss.as_ref().unwrap().parts.len(), 2);
    e.run.live_mut().boss.as_mut().unwrap().hp = 0;
    e.step(&[Input::default(), Default::default(), Default::default()]);
    assert_eq!(e.outcome(), Outcome::Cleared);
}

#[test]
fn a_mid_boss_s_death_does_not_clear_the_run_and_the_final_boss_s_does() {
    let mut e = engine(&[50, 100]);
    run_to(&mut e, 50);
    assert!(e.run.live().boss.as_ref().is_some_and(|b| b.hp > 0));
    // The mid-boss falls; the stage's own rule would clear the run here.
    e.run.live_mut().boss.as_mut().unwrap().hp = 0;
    run_to(&mut e, 99);
    assert_eq!(e.outcome(), Outcome::Running);
    run_to(&mut e, 100);
    let final_boss = e.run.live().boss.as_ref().unwrap();
    assert!(final_boss.hp > 0, "a new boss at the last time");
    e.run.live_mut().boss.as_mut().unwrap().hp = 0;
    e.step(&[Input::default(), Default::default(), Default::default()]);
    assert_eq!(e.outcome(), Outcome::Cleared);
    assert!(e.events().contains(&DomainEvent::StageCleared));
}

#[test]
fn the_main_character_s_death_fails_the_run_and_wins_over_the_final_boss_s() {
    let mut e = engine(&[50, 100]);
    run_to(&mut e, 100);
    e.run.live_mut().boss.as_mut().unwrap().hp = 0;
    e.run.solo_mut().hp = 0;
    e.step(&[Input::default(), Default::default(), Default::default()]);
    assert_eq!(e.outcome(), Outcome::Failed);
    assert!(e.events().contains(&DomainEvent::StageFailed));
    assert!(!e.events().contains(&DomainEvent::StageCleared));
}

#[test]
fn the_run_lasts_its_length_and_its_overtime_and_an_empty_field_does_not_clear_it() {
    let mut e = engine(&[50, 100]);
    run_to(&mut e, 99);
    assert_eq!(e.outcome(), Outcome::Running);
    assert_eq!(
        e.run.live_world().length_ticks,
        100 + overtime(),
        "the run's overtime"
    );
}

#[test]
fn the_run_fails_when_the_overtime_runs_out_with_the_final_boss_alive() {
    let mut e = engine(&[50, 100]);
    e.run.solo_mut().hp = u32::MAX;
    e.run.solo_mut().max_hp = u32::MAX;
    run_to(&mut e, 100 + overtime() - 1);
    assert_eq!(e.outcome(), Outcome::Running);
    e.step(&[Input::default(), Default::default(), Default::default()]);
    assert_eq!(e.outcome(), Outcome::Failed);
    assert!(e.events().contains(&DomainEvent::StageFailed));
}

/// Steps to `until` and lists each elite spawned: the tick, the sprite.
fn elites_spawned(e: &mut Engine, until: u32) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    while e.tick() < until {
        e.step(&[Input::default(), Default::default(), Default::default()]);
        for ev in e.events() {
            if let DomainEvent::EliteSpawned { asset, .. } = ev {
                out.push((e.tick(), asset.to_string()));
            }
        }
    }
    out
}

#[test]
fn elite_minutes_land_on_each_whole_minute_but_boss_ticks_and_overtime() {
    // Room for all sixteen: the cap is not what is tested here.
    let mut e = run_of(1, |v| v["elite_cap"] = serde_json::json!(32));
    let ticks: Vec<u32> = elites_spawned(&mut e, 72_000 + 3_600 * 2)
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    let expected: Vec<u32> = (1..=19)
        .map(|k| k * 3_600)
        .filter(|t| ![18_000, 36_000, 54_000].contains(t))
        .collect();
    assert_eq!(expected.len(), 16);
    assert_eq!(ticks, expected);
}

#[test]
fn a_tier_boundary_on_an_elite_minute_picks_the_later_tier() {
    let mut e = run_of(1, |v| {
        v["length_ticks"] = serde_json::json!(18_000);
        v["boss_ticks"] = serde_json::json!([18_000]);
        v["mid_boss_pool"] = serde_json::json!([]);
        v["elite_cap"] = serde_json::json!(32);
        v["elite_table"] = serde_json::json!([
            { "start_tick": 0, "weights": [{ "elite": "elite_low", "weight": 1 }] },
            { "start_tick": 10_800, "weights": [{ "elite": "elite_mid", "weight": 1 }] }
        ]);
    });
    let spawned = elites_spawned(&mut e, 14_400);
    let sprites: Vec<(u32, &str)> = spawned.iter().map(|(t, a)| (*t, a.as_str())).collect();
    assert_eq!(
        sprites,
        [
            (3_600, "bat"),
            (7_200, "bat"),
            (10_800, "fairy_a"),
            (14_400, "fairy_a")
        ]
    );
}

#[test]
fn the_draws_are_those_of_a_table_of_the_same_shape_before_types_were_content() {
    // Recorded before elite types became content: same weights, same seed. The positions are
    // those after the boss's call counts from its arrival. Re-recorded on the horde
    // stream when the generator split per purpose (R1): the weights and the seed are the same.
    // The positions are those on the spawn rectangle (VS1); the sprites and the stream's state
    // are as before, one horde draw per elite.
    // The two left-side places moved one point down when each corner went to one side (SPAWN-CORNER).
    // They moved again with the 840 px view (VIEW-RECT): the rectangle is ±484 by ±244 px around
    // (524,288; 524,288), every place on its top or bottom side.
    let table = serde_json::json!([{ "start_tick": 0, "weights": [
        { "elite": "elite_low", "weight": 1 }, { "elite": "elite_mid", "weight": 2 } ] },
        { "start_tick": 10_800, "weights": [
        { "elite": "elite_low", "weight": 1 }, { "elite": "elite_high", "weight": 1 }] }]);
    let mut e = run_of(7, |v| {
        v["elite_cap"] = serde_json::json!(32);
        v["elite_table"] = table.clone();
    });
    let spawned = elites_spawned(&mut e, 5 * 3_600 + 10);
    let sprites: Vec<(u32, &str)> = spawned.iter().map(|(t, a)| (*t, a.as_str())).collect();
    assert_eq!(
        sprites,
        [
            (3_600, "fairy_a"),
            (7_200, "fairy_a"),
            (10_800, "bat"),
            (14_400, "bat")
        ]
    );
    // The generator is derived by replaying the horde stream: per elite one draw for its type
    // over the tier's total weight (3 in the first tier, 2 in the second), then one for its
    // place over the spawn rectangle's perimeter, `4 * (ex + ey)` points.
    let (ex, ey) = (123_904_i64, 62_464_i64);
    let perimeter = u32::try_from(4 * (ex + ey)).unwrap();
    assert_eq!(perimeter, 745_472, "the rectangle's perimeter points");
    let mut replay = crate::rng::derive(7, crate::rng::TAG_HORDE);
    let mut draws = Vec::new();
    for total in [3, 3, 2, 2] {
        replay.below(total);
        draws.push(replay.below(perimeter));
    }
    // The first mid-boss comes at 18_000 and is drawn from the pool of four (`horde.rs`, the
    // pick over the entries not yet used).
    replay.below(4);
    assert_eq!(e.run.live().rng.horde.state(), replay.state());
    // The player stands still at the map's start, whose view centre is the map's middle
    // (`view_centre`: 1_048_576 units a side, so 524_288 on each axis), and every place is on
    // the rectangle's top or bottom side. The recorded places equal version 1's.
    let at: Vec<(i32, i32)> = e
        .run
        .live()
        .enemies
        .iter()
        .map(|a| (a.at.x.0, a.at.y.0))
        .collect();
    assert_eq!(at.len(), draws.len());
    let centre = 524_288_i64;
    for (x, y) in &at {
        let (dx, dy) = (
            (i64::from(*x) - centre).abs(),
            (i64::from(*y) - centre).abs(),
        );
        assert!(
            (dy == ey && dx <= ex) || (dx == ex && dy <= ey),
            "({x}, {y}) is on the perimeter"
        );
    }
    assert_eq!(
        at,
        [
            (544_293, 461_824),
            (465_996, 461_824),
            (455_762, 586_752),
            (529_460, 586_752)
        ]
    );
}

#[test]
fn each_grade_is_spawned_from_its_own_definition() {
    let mode = RunContent::bundled().time_limit();
    for (id, sprite) in [
        ("elite_low", "bat"),
        ("elite_mid", "fairy_a"),
        ("elite_high", "horns_2"),
    ] {
        let def = mode
            .elite_types()
            .iter()
            .find(|t| t.id.as_str() == id)
            .unwrap();
        let mut e = run_of(1, |v| {
            v["elite_table"] =
                serde_json::json!([{"start_tick": 0, "weights": [{"elite": id, "weight": 1}]}]);
        });
        let spawned = elites_spawned(&mut e, 3_600);
        assert_eq!(spawned, [(3_600, sprite.to_string())], "{id}");
        let elites: Vec<_> = e
            .run
            .live()
            .enemies
            .iter()
            .filter(|a| a.elite.is_some())
            .collect();
        assert_eq!(elites.len(), 1);
        assert_eq!(
            elites[0].hp, def.enemy.hp,
            "{id}'s health is its definition's"
        );
    }
    let hp = |id: &str| {
        mode.elite_types()
            .iter()
            .find(|t| t.id.as_str() == id)
            .unwrap()
            .enemy
            .hp
    };
    assert!(hp("elite_low") < hp("elite_mid") && hp("elite_mid") < hp("elite_high"));
}

#[test]
fn the_cap_holds_an_elite_back_but_the_generator_is_drawn_the_same() {
    let table = serde_json::json!([{ "start_tick": 0, "weights": [
        { "elite": "elite_low", "weight": 1 }, { "elite": "elite_mid", "weight": 1 } ] }]);
    let run = |cap: u32| {
        run_of(7, |v| {
            v["elite_cap"] = serde_json::json!(cap);
            v["elite_table"] = table.clone();
        })
    };
    let (mut tight, mut roomy) = (run(1), run(32));
    let (mut few, mut many) = (0, 0);
    for minute in 1..=4 {
        let at = minute * 3_600;
        few += elites_spawned(&mut tight, at).len();
        many += elites_spawned(&mut roomy, at).len();
        assert_eq!(
            tight.run.live().rng.horde.state(),
            roomy.run.live().rng.horde.state(),
            "the generator after minute {minute}"
        );
    }
    assert_eq!((few, many), (1, 4), "the cap held three back");
    let alive = |e: &Engine| {
        e.run
            .live()
            .enemies
            .iter()
            .filter(|a| a.elite.is_some())
            .count()
    };
    assert_eq!((alive(&tight), alive(&roomy)), (1, 4));
}

#[test]
fn the_boss_calls_the_elites_of_the_minutes_before_and_after_it_comes() {
    let mut e = run_of(3, |v| {
        v["length_ticks"] = serde_json::json!(20_000);
        v["boss_ticks"] = serde_json::json!([4_000, 20_000]);
        v["mid_boss_pool"] = serde_json::json!(["golem"]);
        v["elite_cap"] = serde_json::json!(32);
        v["elite_table"] = serde_json::json!([{"start_tick": 0, "weights": [{"elite": "elite_low", "weight": 1}]}]);
    });
    let mut called = Vec::new();
    while e.tick() < 7_200 {
        e.step(&[Input::default(), Default::default(), Default::default()]);
        for ev in e.events() {
            if let DomainEvent::ElitesCalled { elites } = ev {
                called.push((e.tick(), *elites));
            }
        }
    }
    // The elite of minute 1 is alive when the boss comes at 4000, and is called when the
    // boss's 1800 ticks from its arrival are up.
    assert_eq!(called, [(5_800, 1)]);
    // The elite of minute 2 spawned after the call: called at once.
    let elites: Vec<bool> = e
        .run
        .live()
        .enemies
        .iter()
        .filter_map(|a| a.elite.map(|el| el.called))
        .collect();
    assert_eq!(elites, [true, true]);
}

/// The sprite of the boss on the field once the run has reached `tick`.
fn boss_sprite_at(e: &mut Engine, tick: u32) -> Option<String> {
    while e.tick() < tick {
        e.run.solo_mut().hp = e.run.solo().max_hp;
        e.step(&[Input::default(), Default::default(), Default::default()]);
    }
    e.snapshot()
        .entities
        .iter()
        .find(|v| v.kind == crate::snapshot::EntityKind::Boss)
        .map(|v| v.asset.to_string())
}

/// The mid-bosses drawn from the pool come without repeating whatever the minute marks are
/// (`spawn_boss` marks each pick used), so the schedule is shortened to 60-tick steps.
fn assert_three_distinct_mid_bosses(seed: u64) {
    const POOL: [&str; 4] = ["stone_warden", "lantern_choir", "horn_charger", "weaver"];
    let times = [60, 120, 180];
    let mut e = run_of(seed, |v| {
        v["length_ticks"] = serde_json::json!(240);
        v["boss_ticks"] = serde_json::json!([60, 120, 180, 240]);
        v["no_call_ticks"] = serde_json::json!([]);
        v["elite_table"] = serde_json::json!([{"start_tick": 0, "weights": [{"elite": "elite_low", "weight": 1}]}]);
    });
    let drawn: Vec<String> = times
        .iter()
        .map(|&t| boss_sprite_at(&mut e, t).expect("a boss"))
        .collect();
    let mut unique = drawn.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 3, "seed {seed}: {drawn:?}");
    assert!(
        drawn.iter().all(|d| POOL.contains(&d.as_str())),
        "{drawn:?}"
    );
}

#[test]
fn the_run_draws_three_distinct_mid_bosses_from_the_four_seed_0() {
    assert_three_distinct_mid_bosses(0);
}

#[test]
fn the_run_draws_three_distinct_mid_bosses_from_the_four_seed_1() {
    assert_three_distinct_mid_bosses(1);
}

#[test]
fn the_run_draws_three_distinct_mid_bosses_from_the_four_seed_2() {
    assert_three_distinct_mid_bosses(2);
}

#[test]
fn the_run_draws_three_distinct_mid_bosses_from_the_four_seed_3() {
    assert_three_distinct_mid_bosses(3);
}

fn assert_easy_run_draws_one_mid_boss(seed: u64) {
    const POOL: [&str; 4] = ["stone_warden", "lantern_choir", "horn_charger", "weaver"];
    let mut e = bundled_run("easy", seed);
    let horde = e.run.live().horde.as_ref().unwrap();
    assert_eq!(horde.boss_ticks, [18_000, 36_000], "seed {seed}");
    let drawn = boss_sprite_at(&mut e, 18_000).expect("a boss");
    assert!(POOL.contains(&drawn.as_str()), "seed {seed}: {drawn}");
}

#[test]
fn the_easy_run_draws_one_mid_boss_from_the_four_and_then_the_final_boss_seed_0() {
    assert_easy_run_draws_one_mid_boss(0);
}

#[test]
fn the_easy_run_draws_one_mid_boss_from_the_four_and_then_the_final_boss_seed_1() {
    assert_easy_run_draws_one_mid_boss(1);
}

#[test]
fn each_mid_boss_s_hit_points_rise_by_its_slot_and_the_final_boss_s_do_not() {
    // Normal difficulty scales nothing; the slots are 100%, 130% and 160%.
    let mut e = engine(&[50, 100, 150, 200]);
    let mode = RunContent::bundled().time_limit();
    let mut seen = Vec::new();
    // The mode's slots: the mid-bosses' percents, then the final boss's own 100.
    let slots: Vec<u64> = live_time_limit_json()["mid_boss_hp_pct"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_u64().unwrap())
        .collect();
    assert_eq!(slots, [100, 130, 160]);
    for (t, pct) in [(50, slots[0]), (100, slots[1]), (150, slots[2]), (200, 100)] {
        run_to(&mut e, t);
        let boss = e.run.live().boss.as_ref().unwrap();
        let def = mode
            .bosses()
            .iter()
            .find(|b| b.base == boss_def(&e).base)
            .unwrap();
        let pct = u32::try_from(pct).unwrap();
        assert_eq!(boss.max_hp, def.hp * pct / 100, "at {t}");
        let parts: Vec<u32> = boss.parts.iter().map(|p| p.hp).collect();
        let want: Vec<u32> = def.parts.iter().map(|p| p.hp * pct / 100).collect();
        assert_eq!(parts, want, "at {t}");
        seen.push(def.base.to_string());
    }
    assert_eq!(seen[3], "core_pair");
}

/// The first boss calls nothing and the later ones call 1800 ticks after they come, on a
/// schedule a quarter as long as the real one: bosses at 4000, 8000 and 12000.
fn assert_calls_after_30_seconds(seed: u64) {
    let mut e = run_of(seed, |v| {
        v["length_ticks"] = serde_json::json!(20_000);
        v["boss_ticks"] = serde_json::json!([4_000, 8_000, 12_000, 20_000]);
        v["no_call_ticks"] = serde_json::json!([4_000]);
        v["elite_cap"] = serde_json::json!(32);
        v["elite_table"] = serde_json::json!([{"start_tick": 0, "weights": [{"elite": "elite_low", "weight": 1}]}]);
    });
    let (arrived, called) = arrivals_and_calls(&mut e, 13_800);
    assert_eq!(arrived, [4_000, 8_000, 12_000], "seed {seed}");
    assert_eq!(called, [9_800, 13_800], "seed {seed}");
}

#[test]
fn the_first_boss_calls_no_elites_and_the_later_ones_call_after_30_seconds_seed_0() {
    assert_calls_after_30_seconds(0);
}

#[test]
fn the_first_boss_calls_no_elites_and_the_later_ones_call_after_30_seconds_seed_1() {
    assert_calls_after_30_seconds(1);
}

#[test]
fn the_five_minute_boss_calls_no_elites_and_the_later_ones_call_after_30_seconds() {
    // The only test of the real schedule: the 5-minute boss comes at 18000 and calls nothing;
    // the 10-minute one at 36000 calls 1800 ticks after it came, and so does the 15-minute
    // one at 54000.
    let mut e = bundled_run("normal", 0);
    let (arrived, called) = arrivals_and_calls(&mut e, 55_800);
    assert_eq!(arrived, [18_000, 36_000, 54_000]);
    assert_eq!(called, [37_800, 55_800]);
}

// --- The spawn director (DIR1, DS103) -------------------------------------------------------

/// A run of the mode with a crowd target of `target` and `waves`, each `(at_tick, count,
/// every_ticks)`, all of the bundled mode's first wave's enemy.
fn directed(target: u16, waves: &[(u32, u16, u32)]) -> Engine {
    let template = live_time_limit_json()["waves"][0].clone();
    engine_edited(&[100_000], &[], |v| {
        v["crowd_target"] = serde_json::json!(target);
        v["waves"] = waves
            .iter()
            .map(|&(at, count, every)| {
                let mut wave = template.clone();
                wave["at_tick"] = at.into();
                wave["count"] = count.into();
                wave["every_ticks"] = every.into();
                wave
            })
            .collect();
    })
}

/// One tick with the main character kept alive.
fn step_alive(e: &mut Engine) {
    e.run.solo_mut().hp = e.run.solo().max_hp;
    e.step(&[Input::default(), Default::default(), Default::default()]);
}

fn held(e: &Engine, wave: usize) -> u32 {
    e.run.live().horde.as_ref().unwrap().held[wave]
}

fn spawned(e: &Engine, wave: usize) -> u16 {
    e.run.live().spawned[wave]
}

fn removed(e: &mut Engine, n: usize) {
    let len = e.run.live().enemies.len();
    e.run.live_mut().enemies.truncate(len - n);
}

#[test]
fn a_held_wave_slides_its_schedule_and_does_not_catch_up() {
    // The filler fills the crowd to the target of 20 by tick 20. Wave 1 falls due at tick 100
    // and is held, a tick more each tick.
    let mut e = directed(20, &[(1, 20, 1), (100, 5, 60)]);
    for _ in 0..1_000 {
        if held(&e, 1) >= 300 {
            break;
        }
        step_alive(&mut e);
    }
    assert_eq!(held(&e, 1), 300, "held a tick more each tick");
    assert_eq!(spawned(&e, 1), 0, "held, not spawned");
    assert_eq!(e.run.live().enemies.len(), 20);
    // Room for ten: it spawns one, not the backlog, and the next 60 ticks later.
    removed(&mut e, 10);
    let mut came = Vec::new();
    while spawned(&e, 1) < 2 && came.len() < 2 {
        let before = spawned(&e, 1);
        step_alive(&mut e);
        if spawned(&e, 1) > before {
            came.push(e.tick());
        }
    }
    assert_eq!(came[1] - came[0], 60, "{came:?}");
    assert_eq!(held(&e, 1), 300, "what was held stays held");
    assert_eq!(spawned(&e, 1), 2, "one spawn at a time");
}

#[test]
fn the_wave_held_longest_goes_first() {
    // Waves 1 (every 60 ticks) and 2 (every 600) both fall due while the crowd is at the target;
    // 2 earlier, so it has been held longer. With room for one a tick, 2 comes first although
    // 1 has the lower index.
    let mut e = directed(20, &[(1, 20, 1), (100, 5, 60), (50, 5, 600)]);
    while e.tick() < 200 {
        step_alive(&mut e);
    }
    assert!(held(&e, 2) > held(&e, 1));
    removed(&mut e, 1);
    step_alive(&mut e);
    assert_eq!((spawned(&e, 1), spawned(&e, 2)), (0, 1), "2 comes first");
    removed(&mut e, 1);
    step_alive(&mut e);
    assert_eq!((spawned(&e, 1), spawned(&e, 2)), (1, 1), "then 1");
}

#[test]
fn a_clone_taken_while_waves_are_held_follows_the_original() {
    let mut e = directed(20, &[(1, 100, 1), (1, 100, 1), (1, 100, 1)]);
    for _ in 0..300 {
        step_alive(&mut e);
    }
    assert!((0..3).any(|w| held(&e, w) > 0));
    let mut copy = e.clone();
    assert_eq!(copy.state_hash(), e.state_hash());
    for _ in 0..600 {
        step_alive(&mut e);
        step_alive(&mut copy);
        assert_eq!(copy.state_hash(), e.state_hash(), "tick {}", e.tick());
    }
}

#[test]
fn a_spawn_draws_once_on_the_spawn_stream_and_a_held_spawn_draws_nothing() {
    // Every wave of the bundled mode gives `edge` `any` (VS1): one draw over the spawn
    // rectangle, whole on this map, so `below(745_472)` each.
    // The rectangle's perimeter points, `4 * (ex + ey)` for the view and the spawn margin.
    assert_eq!(4 * (123_904 + 62_464), 745_472);
    let drawn = |n: u32| {
        let mut r = crate::rng::derive(1, crate::rng::TAG_SPAWN);
        for _ in 0..n {
            r.below(745_472);
        }
        r.state()
    };
    // The filler spawns 20 by tick 20; wave 1 falls due at 100 and is held for 300 ticks.
    let mut e = directed(20, &[(1, 20, 1), (100, 5, 60)]);
    for _ in 0..400 {
        step_alive(&mut e);
    }
    assert!(held(&e, 1) > 0, "wave 1 was held");
    assert_eq!(
        e.run.live().rng.spawn.state(),
        drawn(20),
        "20 spawns, no draw held"
    );
    removed(&mut e, 10);
    while spawned(&e, 1) < 2 {
        step_alive(&mut e);
    }
    assert_eq!(
        e.run.live().rng.spawn.state(),
        drawn(22),
        "two more spawns, two more draws"
    );
}

#[test]
fn an_elite_draws_its_place_whether_or_not_it_can_spawn() {
    // With a cap of 1 the later elites are refused after their draws; the horde stream ends
    // where a run with room for all of them ends (the place is one draw, as the ring's was).
    let at_minute_5 = |cap: u32| {
        let mut e = run_of(7, |v| v["elite_cap"] = serde_json::json!(cap));
        elites_spawned(&mut e, 5 * 3_600 + 10);
        e.run.live().rng.horde.state()
    };
    assert_eq!(at_minute_5(1), at_minute_5(32));
}
