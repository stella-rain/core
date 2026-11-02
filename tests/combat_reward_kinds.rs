//! The named stars' combat reward kinds (core#NS2b, app#336): `crit_chance_pct`,
//! `crit_damage_pct`, `echo_hit_pct` and `kill_burst`. Bounded like every kind (ADR-020), and
//! each acts in the rules: a crit is a roll that spends none of the run's random draws, an echo
//! is a second, weaker hit one tick later, a kill burst damages the enemies around a kill.
//! Events are read as their JSON, the form a recording writes.

use serde_json::{Value, json};
use stella_rain_core::stage::Stage;

use crate::common::*;
use crate::reward_kinds::{
    AMOUNT, NEAR_Y, entry, file_of, invalid_at, kind, near, next_offer, parse, plain, starry, take,
    weapon, with,
};

fn ok(effects: Vec<Value>) {
    parse(&file_of(effects.clone())).unwrap_or_else(|e| panic!("{effects:?}: {e:?}"));
}

#[test]
fn crit_and_echo_kinds_parse_and_are_bounded() {
    for (name, most) in [
        ("crit_chance_pct", 50),
        ("crit_damage_pct", 200),
        ("echo_hit_pct", 100),
    ] {
        ok(vec![entry(name, 1)]);
        ok(vec![entry(name, most)]);
        invalid_at(&file_of(vec![entry(name, 0)]), AMOUNT);
        invalid_at(&file_of(vec![entry(name, most + 1)]), AMOUNT);
    }
    // One reward's entries add up within the kind's largest pick.
    invalid_at(
        &file_of(vec![
            entry("crit_chance_pct", 30),
            entry("crit_chance_pct", 21),
        ]),
        "$.rewards[0].effects",
    );
}

fn burst(pct: u32, radius: Option<u32>, depth: Option<u32>) -> Value {
    let mut e = entry("kill_burst", pct);
    if let Some(r) = radius {
        e["radius"] = json!(r);
    }
    if let Some(d) = depth {
        e["depth"] = json!(d);
    }
    e
}

#[test]
fn kill_burst_holds_a_percent_a_radius_and_a_depth() {
    ok(vec![burst(1, Some(1), Some(1))]);
    ok(vec![burst(100, Some(240), Some(5))]);
    invalid_at(&file_of(vec![burst(0, Some(120), Some(3))]), AMOUNT);
    invalid_at(&file_of(vec![burst(101, Some(120), Some(3))]), AMOUNT);
    let radius = "$.rewards[0].effects[0].radius";
    let depth = "$.rewards[0].effects[0].depth";
    invalid_at(&file_of(vec![burst(40, None, Some(3))]), radius);
    invalid_at(&file_of(vec![burst(40, Some(0), Some(3))]), radius);
    invalid_at(&file_of(vec![burst(40, Some(241), Some(3))]), radius);
    invalid_at(&file_of(vec![burst(40, Some(120), None)]), depth);
    invalid_at(&file_of(vec![burst(40, Some(120), Some(0))]), depth);
    invalid_at(&file_of(vec![burst(40, Some(120), Some(6))]), depth);
    // No other kind takes a radius or a depth.
    let mut stray = entry("rerolls", 1);
    stray["radius"] = json!(5);
    invalid_at(&file_of(vec![stray]), radius);
    let mut stray = entry("rerolls", 1);
    stray["depth"] = json!(2);
    invalid_at(&file_of(vec![stray]), depth);
}

// The effects in a run: a first enemy dies to the weapon and drops the star whose offer holds
// the reward; a second wave, 400 ticks in, is what the reward acts on.

const SECOND_WAVE: u32 = 400;
const PX: i32 = 256;

fn two_waves(second: Vec<Value>) -> Stage {
    let mut waves = vec![wave(
        1,
        1,
        0,
        enemy(10, hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    )];
    waves.extend(second);
    starry(armed(
        stage_full(18_000, Some(far_boss()), waves, vec![]),
        weapon(),
    ))
}

fn holder(hp: u32, dy_px: i32) -> Value {
    let y = NEAR_Y - dy_px * PX;
    wave(
        SECOND_WAVE,
        1,
        0,
        enemy(hp, hold(START_X, y), json!([])),
        START_X,
        y,
    )
}

/// A run that takes `effects` from the first offer (a plain reward when none), then plays
/// `ticks` more ticks: the events of those ticks as `(tick, JSON)`.
fn play(stage: &Stage, effects: Vec<Value>, ticks: u32) -> Vec<(u32, Value)> {
    let mut list = vec![plain("a", 1), plain("b", 1), plain("c", 1)];
    let taken = if effects.is_empty() {
        "a"
    } else {
        list.push(kind("k", effects, 1_000, 2));
        "k"
    };
    let mut e = with(stage, list);
    next_offer(&mut e);
    take(&mut e, taken);
    let mut seen = Vec::new();
    for _ in 0..ticks {
        e.step(firing(idle(), e.tick()));
        seen.extend(
            e.events()
                .iter()
                .map(|ev| (e.tick(), serde_json::to_value(ev).unwrap())),
        );
    }
    seen
}

fn of_type<'a>(events: &'a [(u32, Value)], ty: &str) -> Vec<&'a (u32, Value)> {
    events.iter().filter(|(_, v)| v["type"] == ty).collect()
}

fn damage(ev: &(u32, Value)) -> u64 {
    ev.1["data"]["damage"].as_u64().unwrap()
}

fn flag(ev: &(u32, Value), name: &str) -> bool {
    ev.1["data"][name].as_bool().unwrap_or(false)
}

#[test]
fn without_a_crit_reward_no_hit_is_a_crit() {
    let stage = two_waves(vec![holder(50_000, 0)]);
    let hits = play(&stage, vec![], 600);
    let hits = of_type(&hits, "Hit");
    assert!(hits.len() > 20);
    assert!(hits.iter().all(|h| !flag(h, "crit")));
    assert!(hits.iter().all(|h| damage(h) == damage(hits[0])));
}

#[test]
fn crit_chance_and_damage_raise_the_hits_that_roll_a_crit() {
    let stage = two_waves(vec![holder(50_000, 0)]);
    let base = {
        let hits = play(&stage, vec![], 600);
        damage(of_type(&hits, "Hit")[0])
    };
    let run = |effects| play(&stage, effects, 600);
    // 50 percent chance, the base crit of twice the damage.
    let events = run(vec![entry("crit_chance_pct", 50)]);
    let hits = of_type(&events, "Hit");
    let crits: Vec<_> = hits.iter().filter(|h| flag(h, "crit")).collect();
    assert!(
        !crits.is_empty() && crits.len() < hits.len(),
        "some, not all"
    );
    assert!(crits.iter().all(|h| damage(h) == base * 2));
    assert!(
        hits.iter()
            .filter(|h| !flag(h, "crit"))
            .all(|h| damage(h) == base)
    );
    // With 100 more percent of crit damage a crit is three times the damage.
    let events = run(vec![
        entry("crit_chance_pct", 50),
        entry("crit_damage_pct", 100),
    ]);
    let hits = of_type(&events, "Hit");
    assert!(
        hits.iter()
            .any(|h| flag(h, "crit") && damage(h) == base * 3)
    );
    // Crit damage alone crits nothing; the same inputs give the same run.
    let events = run(vec![entry("crit_damage_pct", 200)]);
    assert!(of_type(&events, "Hit").iter().all(|h| !flag(h, "crit")));
    assert_eq!(
        run(vec![entry("crit_chance_pct", 50)]),
        run(vec![entry("crit_chance_pct", 50)])
    );
}

#[test]
fn an_echo_repeats_each_hit_once_a_tick_later_at_its_percent() {
    let stage = two_waves(vec![holder(50_000, 0)]);
    let base = damage(of_type(&play(&stage, vec![], 600), "Hit")[0]);
    let events = play(&stage, vec![entry("echo_hit_pct", 50)], 600);
    let hits = of_type(&events, "Hit");
    let echoes: Vec<_> = hits.iter().filter(|h| flag(h, "echo")).collect();
    let firsts: Vec<_> = hits.iter().filter(|h| !flag(h, "echo")).collect();
    assert!(!firsts.is_empty());
    // One echo for each hit, but for the last hit's, which lands after the run's last tick.
    assert_eq!(echoes.len() + 1, firsts.len(), "one echo for each hit");
    for (hit, echo) in firsts.iter().zip(&echoes) {
        assert_eq!(echo.0, hit.0 + 1, "one tick later");
        assert_eq!(echo.1["data"]["target"], hit.1["data"]["target"]);
        assert_eq!(damage(echo), base / 2);
        assert!(!flag(echo, "crit"));
    }
}

#[test]
fn an_echo_never_echoes() {
    let stage = two_waves(vec![holder(50_000, 0)]);
    let events = play(&stage, vec![entry("echo_hit_pct", 100)], 600);
    let hits = of_type(&events, "Hit");
    let echoes = hits.iter().filter(|h| flag(h, "echo")).count();
    // As many echoes as hits (less the last, still to come), never an echo of an echo.
    assert_eq!(echoes * 2 + 1, hits.len());
}

fn pack(dy_px: &[i32]) -> Vec<Value> {
    dy_px.iter().map(|d| holder(10, *d)).collect()
}

fn died(events: &[(u32, Value)]) -> usize {
    of_type(events, "Died").len()
}

#[test]
fn a_kill_burst_kills_the_enemies_in_its_radius_and_not_beyond() {
    // The weapon reaches the first only (32 px); the others are 100, 200 and 300 px further.
    let stage = two_waves(pack(&[0, 100, 200, 300]));
    let base = died(&play(&stage, vec![], 600));
    assert_eq!(base, 1, "only the enemy in reach dies");
    let events = play(&stage, vec![burst(100, Some(120), Some(1))], 600);
    // Depth 1: the kill in reach bursts once, and its neighbour dies of it.
    assert_eq!(died(&events), base + 1);
    let bursts = of_type(&events, "KillBurst");
    assert_eq!(bursts.len(), 1);
    assert_eq!(bursts[0].1["data"]["radius"], 120 * PX);
}

#[test]
fn a_burst_that_kills_bursts_again_down_to_its_depth() {
    let stage = two_waves(pack(&[0, 100, 200, 300]));
    let base = died(&play(&stage, vec![], 600));
    for depth in 1..=3 {
        let events = play(&stage, vec![burst(100, Some(120), Some(depth))], 600);
        assert_eq!(died(&events), base + depth as usize, "depth {depth}");
        assert_eq!(of_type(&events, "KillBurst").len(), depth as usize);
    }
}

#[test]
fn a_chain_goes_five_links_deep_and_stops_there() {
    // Seven enemies in a row, each in reach of the next one's burst.
    let stage = two_waves(pack(&[0, 100, 200, 300, 400, 500, 600]));
    let base = died(&play(&stage, vec![], 600));
    let events = play(&stage, vec![burst(100, Some(120), Some(5))], 600);
    assert_eq!(died(&events), base + 5);
    assert_eq!(of_type(&events, "KillBurst").len(), 5);
}

#[test]
fn burst_depth_adds_links_to_the_chain_whatever_the_order() {
    let stage = two_waves(pack(&[0, 100, 200, 300, 400, 500, 600]));
    let base = died(&play(&stage, vec![], 600));
    for effects in [
        vec![burst(100, Some(120), Some(3)), entry("burst_depth", 2)],
        vec![entry("burst_depth", 2), burst(100, Some(120), Some(3))],
    ] {
        let events = play(&stage, effects, 600);
        assert_eq!(died(&events), base + 5);
    }
    // Alone it bursts nothing, and it is bounded.
    let events = play(&stage, vec![entry("burst_depth", 2)], 600);
    assert!(of_type(&events, "KillBurst").is_empty());
    ok(vec![entry("burst_depth", 5)]);
    invalid_at(&file_of(vec![entry("burst_depth", 0)]), AMOUNT);
    invalid_at(&file_of(vec![entry("burst_depth", 6)]), AMOUNT);
}

#[test]
fn a_burst_deals_its_percent_of_the_dead_enemys_max_hit_points() {
    // The enemy in reach has 10 hit points, so 40 percent is 4; the bystander, 100 px further,
    // takes that as a hit in the tick of the burst.
    let stage = two_waves(vec![holder(10, 0), holder(1_000, 100)]);
    let events = play(&stage, vec![burst(40, Some(120), Some(1))], 600);
    let at = of_type(&events, "KillBurst")[0].0;
    let hits = of_type(&events, "Hit");
    let same_tick: Vec<_> = hits.iter().filter(|h| h.0 == at).collect();
    assert!(same_tick.iter().any(|h| damage(h) == 4), "{same_tick:?}");
}

#[test]
fn the_snapshot_shows_the_totals_held_to_their_bounds() {
    let stage = near(4);
    let view = |effects: Vec<Value>, times: u32| {
        let mut list = vec![plain("a", 1), plain("b", 1), plain("c", 1)];
        list.push(kind("k", effects, 1_000, 20));
        let mut e = with(&stage, list);
        for _ in 0..times {
            next_offer(&mut e);
            take(&mut e, "k");
        }
        serde_json::to_value(e.snapshot().players[0].clone()).unwrap()
    };
    let none = view(vec![entry("attack_pct", 1)], 1);
    for field in [
        "crit_chance_pct",
        "crit_damage_pct",
        "echo_hit_pct",
        "burst_pct",
    ] {
        assert!(
            none.get(field).is_none(),
            "{field} is left out while unheld"
        );
    }
    let crit = view(
        vec![entry("crit_chance_pct", 50), entry("crit_damage_pct", 200)],
        1,
    );
    assert_eq!(
        (
            crit["crit_chance_pct"].as_u64(),
            crit["crit_damage_pct"].as_u64()
        ),
        (Some(50), Some(200))
    );
    // Taken again, the totals stop at 75 and 300.
    let more = view(
        vec![entry("crit_chance_pct", 50), entry("crit_damage_pct", 200)],
        2,
    );
    assert_eq!(
        (
            more["crit_chance_pct"].as_u64(),
            more["crit_damage_pct"].as_u64()
        ),
        (Some(75), Some(300))
    );
    let echo = view(vec![entry("echo_hit_pct", 60)], 2);
    assert_eq!(echo["echo_hit_pct"], 100);
    let b = view(vec![burst(70, Some(150), Some(2))], 2);
    assert_eq!(
        (
            b["burst_pct"].as_u64(),
            b["burst_radius"].as_u64(),
            b["burst_depth"].as_u64()
        ),
        (Some(100), Some(240), Some(2))
    );
}
