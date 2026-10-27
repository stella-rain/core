//! The shrine (ADR-090 points 4 and 5, app#259): an elite object that cannot be damaged and does
//! nothing until the main character touches it. Touched, it activates once, rings itself with
//! guardians and starts a timer; the guardians dead in time give the elite's usual reward, the
//! timer out leaves the shrine (unkilled, no reward) and the guardians as ordinary enemies. The
//! boss's call does not touch it: its `boss_call` is `stay`.

use serde_json::{Value, json};
use stella_rain_core::behaviour::{BossCall, Elite, Movement};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::Stage;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::trig;
use stella_rain_core::validate::{self, Error, limits};
use stella_rain_core::version::Versions;

use crate::common;

use common::*;

/// Far above the main character, in the line of its shots.
const FAR_Y: i32 = 120_000;
/// 100 pixels to the side of the main character: out of its line of fire.
const SIDE: (i32, i32) = (START_X + 25_600, START_Y);
/// 40 pixels: the guardians' ring.
const RING: i32 = 10_240;

/// A guardian: it stands where it is put and has no attack.
fn guardian(hp: u32) -> Value {
    enemy(
        hp,
        json!({ "type": "straight", "vx": 0, "vy": 0 }),
        json!([]),
    )
}

fn guardians(enemy: Value, count: u8, radius: i32) -> Value {
    json!({ "enemy": enemy, "count": count, "radius": radius })
}

fn shrine_movement(fight_ticks: u32, guardians: Value) -> Value {
    json!({ "type": "shrine", "fight_ticks": fight_ticks, "guardians": guardians })
}

/// A shrine with `guardians` guardians of `guardian_hp` on a ring of 40 pixels, `fight_ticks` to
/// clear them.
fn shrine(fight_ticks: u32, count: u8, guardian_hp: u32) -> Value {
    let mut e = enemy(
        10,
        shrine_movement(fight_ticks, guardians(guardian(guardian_hp), count, RING)),
        json!([]),
    );
    e["radius"] = json!(3072);
    e["elite"] = json!({ "range": 256, "boss_call": "stay" });
    e
}

/// An ordinary elite, which the boss calls.
fn joiner() -> Value {
    let mut e = enemy(
        10,
        json!({ "type": "follow", "distance": 0 }),
        json!([rule(always(), json!("player"), telegraph(), 0)]),
    );
    e["elite"] = json!({ "range": 256 });
    e
}

fn rewards() -> Rewards {
    let file = json!({ "rarities": common::rarities(), "rewards": [
        { "id": "c", "rarity": "common", "effect": "max_hp", "amount": 1, "limit": 9,
          "title": "Reward", "description": "" } ] });
    Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT)
        .expect("the test rewards validate")
}

fn stage(waves: Vec<Value>) -> Stage {
    let mut stage = stage_full(18_000, Some(far_boss()), waves, vec![]);
    stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).expect("levels parse"));
    stage.stars = Some(
        serde_json::from_value(json!({ "drop_per_mille": 0, "pity": 2, "cap": 16,
                    "pickup_radius": 256,
                    "tiers": [{ "from_level": 1, "weights": { "common": 1 } }] }))
        .expect("stars parse"),
    );
    stage
}

fn with(stage: &Stage) -> Engine {
    let content = RunContent::new(RunStats::bundled().clone(), rewards());
    Engine::new_with(stage, &content).expect("the test stage is valid")
}

/// The shrine alone at `at`, with the other waves.
fn shrine_stage(at: (i32, i32), shrine: Value, mut others: Vec<Value>) -> Stage {
    others.insert(0, wave(1, 1, 0, shrine, at.0, at.1));
    stage(others)
}

fn positions(e: &Engine, elite: bool) -> Vec<(i32, i32)> {
    e.snapshot()
        .entities
        .iter()
        .filter(|v| {
            matches!(v.kind, EntityKind::Enemy | EntityKind::Shrine)
                && (v.elite || v.kind == EntityKind::Shrine) == elite
        })
        .map(|v| (v.at.x.0, v.at.y.0))
        .collect()
}

fn activated(events: &[(u32, DomainEvent)]) -> Vec<u32> {
    ticks_of(events, |ev| {
        matches!(ev, DomainEvent::ShrineActivated { .. })
    })
}

fn left(events: &[(u32, DomainEvent)]) -> Vec<u32> {
    ticks_of(events, |ev| matches!(ev, DomainEvent::EliteLeft { .. }))
}

fn is_activated(events: &[(u32, DomainEvent)]) -> bool {
    !activated(events).is_empty()
}

/// Walks the main character to the right, onto the shrine at `SIDE`, until it activates; the
/// events so far.
fn touch(e: &mut Engine) -> Vec<(u32, DomainEvent)> {
    let mut seen = Vec::new();
    for _ in 0..200 {
        seen.extend(run_until(e, moving(127, 0), 1, |_| false));
        if is_activated(&seen) {
            return seen;
        }
    }
    panic!("the shrine never activated: {seen:?}");
}

/// Where the guardians of a shrine at `at` stand: evenly spaced from the east, `count` of them
/// on a ring of `radius`.
fn ring(at: (i32, i32), count: i32, radius: i32) -> Vec<(i32, i32)> {
    (0..count)
        .map(|i| {
            let angle = i * trig::PER_TURN / count;
            let (sin, cos) = trig::sin_cos(angle);
            let off = |unit: i32| {
                i32::try_from(i64::from(radius) * i64::from(unit) / i64::from(trig::SCALE)).unwrap()
            };
            (at.0 + off(cos), at.1 + off(sin))
        })
        .collect()
}

fn sorted(mut v: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    v.sort_unstable();
    v
}

// --- Untouched ---------------------------------------------------------------------------------

#[test]
fn an_untouched_shrine_takes_no_damage_and_the_main_character_s_shots_pass_through() {
    // The shrine is in the line of fire, with an ordinary enemy of 10 hp behind it.
    let behind = wave(
        1,
        1,
        0,
        enemy(10, hold(START_X, 100_000), json!([])),
        START_X,
        100_000,
    );
    let mut e = with(&shrine_stage(
        (START_X, FAR_Y),
        shrine(600, 4, 5),
        vec![behind],
    ));
    let events = fire_until(&mut e, idle(), 300, |_| false);
    let hits: Vec<_> = events
        .iter()
        .filter_map(|(_, ev)| match ev {
            DomainEvent::Hit { target, .. } => Some(*target),
            _ => None,
        })
        .collect();
    assert!(!hits.is_empty(), "the enemy behind it was hit: {events:?}");
    let shrine_id = e
        .snapshot()
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Shrine)
        .map(|v| (v.id, v.hp))
        .expect("the shrine is still there");
    assert_eq!(shrine_id.1, 10, "undamaged");
    assert!(
        hits.iter().all(|t| *t != shrine_id.0),
        "no hit names the shrine: {events:?}"
    );
    assert_eq!(positions(&e, true), [(START_X, FAR_Y)]);
    assert!(!is_activated(&events));
}

#[test]
fn an_untouched_shrine_does_not_move_or_attack() {
    let mut e = with(&shrine_stage(SIDE, shrine(600, 4, 5), vec![]));
    let events = run_until(&mut e, idle(), 900, |_| false);
    assert_eq!(positions(&e, true), [SIDE]);
    assert_eq!(positions(&e, false), Vec::<(i32, i32)>::new());
    assert!(
        events.iter().all(|(_, ev)| !matches!(
            ev,
            DomainEvent::RuleFired { .. } | DomainEvent::Hit { .. } | DomainEvent::Died { .. }
        )),
        "{events:?}"
    );
}

#[test]
fn an_untouched_shrine_counts_toward_the_elite_cap() {
    let mut st = shrine_stage(SIDE, shrine(600, 4, 5), vec![]);
    st.elite_cap = Some(1);
    st.waves
        .push(serde_json::from_value(wave(1, 1, 0, joiner(), SIDE.0, SIDE.1 - 50_000)).unwrap());
    let mut e = with(&st);
    run_until(&mut e, idle(), 5, |_| false);
    assert_eq!(
        positions(&e, true).len(),
        1,
        "the cap of 1 is spent on the shrine"
    );
}

// --- Touched -----------------------------------------------------------------------------------

#[test]
fn touching_it_activates_it_once_and_rings_it_with_guardians() {
    let mut e = with(&shrine_stage(SIDE, shrine(6000, 4, 100_000), vec![]));
    let mut events = touch(&mut e);
    let at = activated(&events)[0];
    assert_eq!(e.tick(), at);
    // Exactly at the ring, on the tick it activates; none placed by chance.
    assert_eq!(
        sorted(positions(&e, false)),
        sorted(ring(SIDE, 4, RING)),
        "four guardians on a ring of 40 pixels, evenly spaced from the east"
    );
    // The main character stays on it: it does not activate again.
    events.extend(run_until(&mut e, idle(), 120, |_| false));
    assert_eq!(activated(&events), [at]);
    assert_eq!(positions(&e, true), [SIDE], "the shrine stays");
}

#[test]
fn the_event_names_the_shrine() {
    let mut e = with(&shrine_stage(SIDE, shrine(6000, 3, 100_000), vec![]));
    let events = touch(&mut e);
    let id = e
        .snapshot()
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Shrine)
        .map(|v| v.id)
        .unwrap();
    assert!(
        events
            .iter()
            .any(|(_, ev)| *ev == DomainEvent::ShrineActivated { elite: id }),
        "{events:?}"
    );
    assert_eq!(sorted(positions(&e, false)), sorted(ring(SIDE, 3, RING)));
}

#[test]
fn a_ring_beyond_the_map_is_clamped_to_it() {
    // 20 pixels from the left edge, with a ring of 40: the western guardians stand on the edge.
    let at = (5_120, START_Y);
    let mut e = with(&shrine_stage(at, shrine(6000, 4, 100_000), vec![]));
    // The main character walks left onto it.
    let mut events = Vec::new();
    for _ in 0..300 {
        events.extend(run_until(&mut e, moving(-127, 0), 1, |_| false));
        if is_activated(&events) {
            break;
        }
    }
    assert!(is_activated(&events), "{events:?}");
    assert!(
        positions(&e, false).iter().all(|p| p.0 >= 0),
        "{:?}",
        positions(&e, false)
    );
}

// --- Cleared and failed ------------------------------------------------------------------------

fn stars(events: &[(u32, DomainEvent)]) -> Vec<u32> {
    ticks_of(events, |ev| matches!(ev, DomainEvent::StarDropped { .. }))
}

#[test]
fn clearing_the_guardians_in_time_gives_the_elite_reward_and_the_shrine_goes() {
    // Two guardians, east and west of the shrine. The main character, which faces the way it
    // walked (east, onto the shrine), shoots the east one, then walks west and shoots the other.
    let mut e = with(&shrine_stage(SIDE, shrine(600, 2, 5), vec![]));
    let mut events = touch(&mut e);
    let shrine_id = e
        .snapshot()
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Shrine)
        .unwrap()
        .id;
    events.extend(fire_until(&mut e, idle(), 60, |e| {
        positions(e, false).len() == 1
    }));
    assert_eq!(
        positions(&e, false),
        [ring(SIDE, 2, RING)[1]],
        "the east one died"
    );
    assert_eq!(
        stars(&events),
        Vec::<u32>::new(),
        "the shrine is not done with a guardian left: {events:?}"
    );
    assert_eq!(positions(&e, true), [SIDE]);
    events.extend(fire_until(&mut e, moving(-127, 0), 120, |e| {
        positions(e, true).is_empty()
    }));
    let died: Vec<_> = events
        .iter()
        .filter_map(|(_, ev)| match ev {
            DomainEvent::Died { entity, .. } => Some(*entity),
            _ => None,
        })
        .collect();
    assert_eq!(died.len(), 3, "two guardians and the shrine: {events:?}");
    assert_eq!(
        died.iter().filter(|d| **d == shrine_id).count(),
        1,
        "the shrine died once: {events:?}"
    );
    assert_eq!(
        stars(&events).len(),
        1,
        "an elite always drops a star: {events:?}"
    );
    assert_eq!(left(&events), Vec::<u32>::new());
    assert!(positions(&e, true).is_empty());
}

#[test]
fn a_shrine_is_its_own_kind_in_the_snapshot_and_a_guardian_is_an_enemy() {
    let mut e = with(&shrine_stage(SIDE, shrine(6000, 2, 100_000), vec![]));
    run_until(&mut e, idle(), 3, |_| false);
    let kinds = |e: &Engine| -> Vec<(EntityKind, bool)> {
        let mut v: Vec<_> = e
            .snapshot()
            .entities
            .iter()
            .filter(|v| v.at.x.0 == SIDE.0 && v.at.y.0 == SIDE.1)
            .map(|v| (v.kind, v.elite))
            .collect();
        v.sort_by_key(|k| k.1);
        v
    };
    assert_eq!(kinds(&e), [(EntityKind::Shrine, false)]);
    touch(&mut e);
    assert!(
        e.snapshot()
            .entities
            .iter()
            .filter(|v| v.kind == EntityKind::Enemy)
            .count()
            == 2,
        "the guardians are ordinary enemies"
    );
}

#[test]
fn a_shrine_is_not_an_elite_in_the_view_but_an_ordinary_elite_is() {
    // DS44 item 1: `elite` means elite enemies only; a shrine's kind already says what it is.
    let mut st = shrine_stage(SIDE, shrine(6000, 2, 100_000), vec![]);
    st.waves
        .push(serde_json::from_value(wave(1, 1, 0, joiner(), SIDE.0, SIDE.1 - 50_000)).unwrap());
    let mut e = with(&st);
    run_until(&mut e, idle(), 3, |_| false);
    let snap = e.snapshot();
    let shrine = snap.entities.iter().find(|v| v.kind == EntityKind::Shrine);
    assert_eq!(shrine.map(|v| v.elite), Some(false));
    let joiner = snap
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Enemy && v.at.y.0 == SIDE.1 - 50_000);
    assert_eq!(joiner.map(|v| v.elite), Some(true), "an ordinary elite");
}

#[test]
fn a_shrine_death_reports_its_class() {
    let mut e = with(&shrine_stage(SIDE, shrine(600, 2, 5), vec![]));
    let mut events = touch(&mut e);
    let shrine_id = e
        .snapshot()
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Shrine)
        .unwrap()
        .id;
    events.extend(fire_until(&mut e, idle(), 60, |e| {
        positions(e, false).len() == 1
    }));
    events.extend(fire_until(&mut e, moving(-127, 0), 120, |e| {
        positions(e, true).is_empty()
    }));
    let died: Vec<_> = crate::died_class::deaths(&events);
    let shrine_died: Vec<_> = died.iter().filter(|d| d.0 == shrine_id).collect();
    assert_eq!(
        shrine_died,
        [&(shrine_id, EntityKind::Shrine, false)],
        "{events:?}"
    );
    // The guardians are ordinary enemies.
    assert_eq!(
        died.iter()
            .filter(|d| *d == &(d.0, EntityKind::Enemy, false))
            .count(),
        2,
        "{died:?}"
    );
}

#[test]
fn running_out_of_time_with_guardians_alive_leaves_the_shrine_without_a_reward() {
    let mut e = with(&shrine_stage(SIDE, shrine(120, 3, 100_000), vec![]));
    let mut events = touch(&mut e);
    let at = e.tick();
    events.extend(run_until(&mut e, idle(), 300, |_| false));
    assert_eq!(left(&events), [at + 120], "120 ticks from the activation");
    assert_eq!(
        positions(&e, true),
        Vec::<(i32, i32)>::new(),
        "the shrine is gone"
    );
    assert_eq!(
        positions(&e, false).len(),
        3,
        "the guardians stay, ordinary enemies"
    );
    assert!(
        events
            .iter()
            .all(|(_, ev)| !matches!(ev, DomainEvent::Died { .. })),
        "{events:?}"
    );
    assert_eq!(stars(&events), Vec::<u32>::new());
}

#[test]
fn a_cleared_fight_in_the_last_tick_still_counts() {
    // The guardians die on the same tick as the timer ends or before: cleared wins.
    let mut e = with(&shrine_stage(SIDE, shrine(40, 1, 5), vec![]));
    let mut events = touch(&mut e);
    events.extend(fire_until(&mut e, idle(), 100, |_| false));
    assert_eq!(left(&events), Vec::<u32>::new(), "{events:?}");
    assert_eq!(stars(&events).len(), 1, "{events:?}");
}

// --- The snapshot's count-down (app#345) ----------------------------------------------------------

/// The shrine's fight count-down in the snapshot, as `(remaining, total)`; `None` when the
/// snapshot shows no shrine fight (no shrine, or none running).
fn fight_of(e: &Engine) -> Option<(u32, u32)> {
    let snap = e.snapshot();
    let shrine = snap
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Shrine)?;
    shrine.fight.as_ref().map(|c| (c.remaining, c.total))
}

#[test]
fn an_untouched_shrine_has_no_fight_count_down() {
    let mut e = with(&shrine_stage(SIDE, shrine(120, 3, 100_000), vec![]));
    run_until(&mut e, idle(), 1, |_| false);
    assert_eq!(positions(&e, true), [SIDE], "the shrine is there");
    assert_eq!(fight_of(&e), None);
    run_until(&mut e, idle(), 30, |_| false);
    assert_eq!(fight_of(&e), None, "still untouched");
}

#[test]
fn the_fight_count_down_starts_full_and_falls_one_a_tick() {
    let mut e = with(&shrine_stage(SIDE, shrine(120, 3, 100_000), vec![]));
    touch(&mut e);
    assert_eq!(
        fight_of(&e),
        Some((120, 120)),
        "full on the tick it activates"
    );
    for left in (110..120).rev() {
        run_until(&mut e, idle(), 1, |_| false);
        assert_eq!(fight_of(&e), Some((left, 120)));
    }
}

#[test]
fn the_fight_count_down_is_gone_when_the_timer_runs_out() {
    let mut e = with(&shrine_stage(SIDE, shrine(40, 3, 100_000), vec![]));
    touch(&mut e);
    run_until(&mut e, idle(), 39, |_| false);
    assert_eq!(fight_of(&e), Some((1, 40)), "one tick left");
    run_until(&mut e, idle(), 1, |_| false);
    assert_eq!(
        positions(&e, true),
        Vec::<(i32, i32)>::new(),
        "the shrine left"
    );
    assert_eq!(fight_of(&e), None);
}

#[test]
fn the_fight_count_down_is_gone_when_the_guardians_are_cleared() {
    let mut e = with(&shrine_stage(SIDE, shrine(600, 1, 5), vec![]));
    touch(&mut e);
    assert!(fight_of(&e).is_some());
    fire_until(&mut e, idle(), 200, |_| false);
    assert_eq!(fight_of(&e), None, "cleared");
}

// --- The enemy cap -----------------------------------------------------------------------------

#[test]
fn the_enemy_cap_limits_the_guardians() {
    // 257 enemies and the shrine make 258 of the 260 alive at most: two guardians fit.
    let filler = |count: u16, x: i32| {
        wave(
            1,
            count,
            1,
            enemy(10, hold(x, 40_000), json!([])),
            x,
            40_000,
        )
    };
    let others = vec![filler(100, 1_000), filler(100, 2_000), filler(57, 3_000)];
    let mut e = with(&shrine_stage(SIDE, shrine(6000, 4, 100_000), others));
    // The last of them comes at tick 100.
    run_until(&mut e, idle(), 110, |_| false);
    assert_eq!(e.snapshot().entities.len() - 1, 258, "257 and the shrine");
    let events = touch(&mut e);
    assert!(is_activated(&events));
    let ring = ring(SIDE, 4, RING);
    let near: Vec<_> = positions(&e, false)
        .into_iter()
        .filter(|p| ring.contains(p))
        .collect();
    assert_eq!(
        near,
        ring[..2],
        "the first two on the ring spawned, the others were refused"
    );
    assert_eq!(
        e.snapshot().entities.len() - 1,
        260,
        "260 enemies alive, the shrine one of them"
    );
}

// --- The boss ----------------------------------------------------------------------------------

/// A stage whose boss calls its elites at `call_after`, with a shrine and an ordinary elite.
fn calling(call_after: u32) -> Stage {
    let mut boss = boss_at(1_000_000, FAR_Y);
    boss["call_elites"] = json!({ "after_ticks": call_after });
    let mut st = shrine_stage(
        SIDE,
        shrine(6000, 3, 100_000),
        vec![wave(1, 1, 0, joiner(), START_X - 25_600, START_Y)],
    );
    st.boss = Some(serde_json::from_value(boss).expect("boss parses"));
    st
}

#[test]
fn the_boss_s_call_skips_an_untouched_shrine() {
    let mut e = with(&calling(60));
    let events = run_until(&mut e, idle(), 120, |_| false);
    let calls: Vec<_> = events
        .iter()
        .filter(|(_, ev)| matches!(ev, DomainEvent::ElitesCalled { .. }))
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].1,
        DomainEvent::ElitesCalled { elites: 1 },
        "only the joiner"
    );
    assert_eq!(left(&events), Vec::<u32>::new());
    assert!(
        positions(&e, true).contains(&SIDE),
        "the shrine stays where it is"
    );
}

#[test]
fn a_shrine_that_comes_after_the_call_is_not_called_either() {
    let mut boss = boss_at(1_000_000, FAR_Y);
    boss["call_elites"] = json!({ "after_ticks": 0 });
    let mut st = stage(vec![wave(
        100,
        1,
        0,
        shrine(6000, 3, 100_000),
        SIDE.0,
        SIDE.1,
    )]);
    st.boss = Some(serde_json::from_value(boss).expect("boss parses"));
    let mut e = with(&st);
    let events = run_until(&mut e, idle(), 300, |_| false);
    assert_eq!(left(&events), Vec::<u32>::new());
    assert_eq!(positions(&e, true), [SIDE]);
}

#[test]
fn an_active_fight_keeps_running_when_the_boss_calls() {
    let mut e = with(&calling(100));
    let mut events = touch(&mut e);
    let at = e.tick();
    assert!(at < 100, "the fight starts before the call: {at}");
    events.extend(run_until(&mut e, idle(), 200, |_| false));
    assert!(
        events
            .iter()
            .any(|(_, ev)| matches!(ev, DomainEvent::ElitesCalled { .. })),
        "{events:?}"
    );
    assert_eq!(
        left(&events),
        Vec::<u32>::new(),
        "its 6,000 ticks have not run out"
    );
    assert!(positions(&e, true).contains(&SIDE));
    assert!(positions(&e, false).len() >= 3, "the guardians fight on");
}

fn setup() -> RunSetup {
    RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        weapon_tree: None,
        seed: 3,
        star_pool: None,
        extra_rerolls: 0,
        upgrades: Vec::new(),
        perk_set: "duelist".into(),
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
        stat_priority: Vec::new(),
        pick_stats: false,
        difficulty: "normal".into(),
    }
}

/// A short run whose only elite is a shrine, which comes at tick 3,600, and whose boss comes at
/// tick `boss`.
fn shrine_run(boss: u32) -> TimeLimit {
    let mut v: Value = serde_json::from_slice(common::bundled_file("time_limit")).unwrap();
    v["length_ticks"] = json!(boss + 600);
    v["boss_ticks"] = json!([boss, boss + 600]);
    v["waves"] = json!([]);
    v["elite_types"] = json!([{ "id": "elite_test_shrine", "enemy": {
        "base": "bat", "hp": 240, "radius": 3072,
        "movement": shrine_movement(600, guardians(guardian(10), 4, RING)),
        "elite": { "range": 256, "boss_call": "stay" } } }]);
    v["elite_table"] =
        json!([{ "start_tick": 0, "weights": [{ "elite": "elite_test_shrine", "weight": 1 }] }]);
    TimeLimit::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn an_untouched_shrine_stays_when_a_boss_spawns() {
    let mode = shrine_run(4_500);
    let mut e = Engine::start_time_limit(&setup(), RunContent::bundled(), &mode).unwrap();
    let events = run_until(&mut e, idle(), 4_600, |_| false);
    assert_eq!(
        ticks_of(&events, |ev| matches!(ev, DomainEvent::EliteSpawned { .. })),
        [3_600]
    );
    assert_eq!(left(&events), Vec::<u32>::new());
    assert_eq!(
        positions(&e, true).len(),
        1,
        "the boss came, the shrine stays"
    );
}

// --- Data and schema ---------------------------------------------------------------------------

#[test]
fn boss_call_accepts_stay() {
    let stay: Elite = serde_json::from_value(json!({ "range": 256, "boss_call": "stay" })).unwrap();
    assert_eq!(stay.boss_call, BossCall::Stay);
    assert_eq!(
        serde_json::to_value(stay).unwrap(),
        json!({ "range": 256, "boss_call": "stay" })
    );
}

#[test]
fn the_bundled_mode_has_the_shrine_in_every_tier() {
    let mode = TimeLimit::bundled();
    let t = mode
        .elite_types()
        .iter()
        .find(|t| t.id.0 == "elite_shrine")
        .expect("elite_shrine is defined");
    assert_eq!(t.enemy.rules.len(), 1, "it pulses (Z3b)");
    assert_eq!(
        t.enemy.elite.as_ref().expect("an elite").boss_call,
        BossCall::Stay
    );
    assert!(matches!(t.enemy.movement, Movement::Shrine { .. }));
    for tier in mode.elite_table() {
        assert!(
            tier.weights.iter().any(|w| w.elite.0 == "elite_shrine"),
            "{tier:?}"
        );
    }
}

fn issues(stage: &Stage) -> Vec<String> {
    match validate::validate(stage) {
        Ok(()) => Vec::new(),
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        Err(e) => panic!("{e:?}"),
    }
}

fn shrine_stage_with(movement: Value) -> Stage {
    let mut s = shrine(600, 4, 5);
    s["movement"] = movement;
    stage(vec![wave(1, 1, 0, s, 1000, 1000)])
}

#[test]
fn the_fight_is_bounded() {
    let (max, ticks, most) = (
        limits::MAX_LEN,
        limits::MAX_TICKS,
        limits::MAX_SHRINE_GUARDIANS,
    );
    let m = "$.waves[0].enemy.movement";
    let g =
        |t: u32, count: u8, radius: i32| shrine_movement(t, guardians(guardian(5), count, radius));
    let cases: [(Value, Vec<String>); 11] = [
        (g(1, 1, 1), vec![]),
        (g(ticks, most, max), vec![]),
        (g(0, 1, 1), vec![format!("{m}.fight_ticks")]),
        (g(ticks + 1, 1, 1), vec![format!("{m}.fight_ticks")]),
        (g(1, 0, 1), vec![format!("{m}.guardians.count")]),
        (g(1, most + 1, 1), vec![format!("{m}.guardians.count")]),
        (g(1, 1, 0), vec![format!("{m}.guardians.radius")]),
        (g(1, 1, max + 1), vec![format!("{m}.guardians.radius")]),
        (
            g(0, 0, 0),
            vec![
                format!("{m}.fight_ticks"),
                format!("{m}.guardians.count"),
                format!("{m}.guardians.radius"),
            ],
        ),
        (
            shrine_movement(
                1,
                guardians(
                    enemy(0, json!({ "type": "follow", "distance": 0 }), json!([])),
                    1,
                    1,
                ),
            ),
            vec![format!("{m}.guardians.enemy.hp")],
        ),
        (
            shrine_movement(
                1,
                guardians(
                    enemy(5, json!({ "type": "follow", "distance": -1 }), json!([])),
                    1,
                    1,
                ),
            ),
            vec![format!("{m}.guardians.enemy.movement.distance")],
        ),
    ];
    for (movement, expected) in cases {
        assert_eq!(
            issues(&shrine_stage_with(movement.clone())),
            expected,
            "{movement}"
        );
    }
}

#[test]
fn a_guardian_is_an_ordinary_enemy() {
    let m = "$.waves[0].enemy.movement.guardians.enemy";
    // Not an elite.
    let mut elite = guardian(5);
    elite["elite"] = json!({ "range": 256 });
    assert_eq!(
        issues(&shrine_stage_with(shrine_movement(
            1,
            guardians(elite, 1, 1)
        ))),
        [format!("{m}.elite")]
    );
    // Not a shrine itself, nor a flier-away: guardians do not nest.
    let nested = enemy(
        5,
        shrine_movement(1, guardians(guardian(5), 1, 1)),
        json!([]),
    );
    assert_eq!(
        issues(&shrine_stage_with(shrine_movement(
            1,
            guardians(nested, 1, 1)
        ))),
        [format!("{m}.movement")]
    );
    // Does not summon.
    let summoner = enemy(
        5,
        json!({ "type": "follow", "distance": 0 }),
        json!([rule(always(), json!("player"), summon(2), 0)]),
    );
    assert_eq!(
        issues(&shrine_stage_with(shrine_movement(
            1,
            guardians(summoner, 1, 1)
        ))),
        [format!("{m}.rules[0].do")]
    );
}

#[test]
fn a_shrine_needs_an_elite_that_stays() {
    let mut s = shrine(600, 4, 5);
    s.as_object_mut().unwrap().remove("elite");
    assert_eq!(
        issues(&stage(vec![wave(1, 1, 0, s, 1000, 1000)])),
        ["$.waves[0].enemy.movement"]
    );
    let mut s = shrine(600, 4, 5);
    s["elite"] = json!({ "range": 256 });
    assert_eq!(
        issues(&stage(vec![wave(1, 1, 0, s, 1000, 1000)])),
        ["$.waves[0].enemy.elite.boss_call"]
    );
    let c = companion(
        10,
        shrine_movement(600, guardians(guardian(5), 1, 1)),
        json!([]),
    );
    let mut st = stage_full(600, Some(far_boss()), vec![], vec![c]);
    st.levels = None;
    assert_eq!(issues(&st), ["$.companions[0].movement"]);
}

// --- The pulse: a shrine's rules run only during its fight (Z3a-1) ----------------------------

/// A shrine whose one rule lays a small zone beside it every 300 ticks, wherever the main
/// character is. Its range is wide, so the main character at `SIDE` is in range without touching.
fn pulsing_shrine(fight_ticks: u32, count: u8, guardian_hp: u32) -> Value {
    let mut s = shrine(fight_ticks, count, guardian_hp);
    let zone = json!({ "type": "zone", "shape": { "type": "circle", "radius": 1024 },
                       "anchor": "attacker", "wind_up_ticks": 1, "active_ticks": 1 });
    let phase = json!({ "kind": "wind_up", "ticks": 1, "damage": 5, "spawn": [zone] });
    s["rules"] = json!([{ "when": always(), "target": "player", "cooldown_ticks": 300,
                          "do": { "type": "attack", "attack": { "inline": [phase] } } }]);
    s["elite"]["range"] = json!(100_000);
    s
}

/// The ticks on which a zone appears (none alive before, some after) over the next `ticks` ticks.
fn zones_appear(e: &mut Engine, input: Input, ticks: u32) -> Vec<u32> {
    let alive = |e: &Engine| !e.snapshot().zones.is_empty();
    let mut seen = Vec::new();
    for _ in 0..ticks {
        let before = alive(e);
        e.step(input);
        if !before && alive(e) {
            seen.push(e.tick());
        }
    }
    seen
}

#[test]
fn an_untouched_shrine_fires_no_rule_though_the_main_character_is_in_range() {
    let mut e = with(&shrine_stage(SIDE, pulsing_shrine(600, 2, 5), vec![]));
    assert_eq!(zones_appear(&mut e, idle(), 600), Vec::<u32>::new());
    assert_eq!(fight_of(&e), None, "still untouched");
}

#[test]
fn a_shrine_pulses_from_the_tick_after_the_touch_every_cooldown() {
    let mut e = with(&shrine_stage(SIDE, pulsing_shrine(900, 2, 100_000), vec![]));
    let t = activated(&touch(&mut e))[0];
    let first = t + 1;
    assert_eq!(
        zones_appear(&mut e, idle(), 650),
        [first, first + 300, first + 600]
    );
}

#[test]
fn a_timed_out_shrine_pulses_no_more() {
    let mut e = with(&shrine_stage(SIDE, pulsing_shrine(120, 2, 100_000), vec![]));
    let t = activated(&touch(&mut e))[0];
    let seen = zones_appear(&mut e, idle(), 600);
    assert_eq!(seen, [t + 1], "one pulse, none after it left");
    assert!(positions(&e, true).is_empty(), "the shrine is gone");
}

#[test]
fn a_cleared_shrine_pulses_no_more() {
    // As in the clearing test above: shoot the east guardian, then walk west and shoot the other.
    let mut e = with(&shrine_stage(SIDE, pulsing_shrine(900, 2, 5), vec![]));
    let t = activated(&touch(&mut e))[0];
    let mut seen = Vec::new();
    let mut alive = false;
    while !positions(&e, true).is_empty() {
        assert!(e.tick() < t + 400, "the shrine should be cleared by now");
        let input = if positions(&e, false).len() == 1 {
            moving(-127, 0)
        } else {
            idle()
        };
        e.step(firing(input, e.tick()));
        let now = !e.snapshot().zones.is_empty();
        if now && !alive {
            seen.push(e.tick());
        }
        alive = now;
    }
    assert_eq!(seen, [t + 1], "the pulse before the clear");
    assert_eq!(zones_appear(&mut e, idle(), 600), Vec::<u32>::new());
}
