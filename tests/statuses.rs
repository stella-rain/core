//! The statuses in the snapshot (ADR-103 decision 3): each buff and debuff on the main character
//! with its ID, its remaining ticks and the ticks it was applied for, sorted by ID; and a pack
//! leader's aura, a status of each non-elite enemy it reaches, with no timer: it lasts while
//! the enemy is in reach. The damage-over-time statuses are in `bleed.rs`.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::{EntityKind, StatusView};
use stella_rain_core::stage::Stage;

use crate::common;

use common::*;

fn view(id: &str, remaining: u32, total: u32) -> StatusView {
    StatusView {
        id: id.into(),
        remaining,
        total,
    }
}

/// The main character's statuses in the snapshot.
fn player_statuses(e: &Engine) -> Vec<StatusView> {
    e.snapshot().players[0].statuses.clone()
}

/// A rule of an enemy that applies `skill` to the main character on the first tick, once.
fn applied_once(skill: Value) -> Value {
    json!({ "when": always(), "target": "player", "do": cast(skill), "once": true })
}

#[test]
fn the_main_character_s_timed_statuses_are_listed_by_id_and_count_down_to_zero() {
    // One enemy each applies one status on the first tick, in the order of the enemies: the
    // list is sorted by ID all the same.
    let applies = |skill: Value, x: i32| {
        let rules = json!([applied_once(skill)]);
        wave(
            1,
            1,
            0,
            enemy(5, hold(x, START_Y - 40_960), rules),
            x,
            START_Y - 40_960,
        )
    };
    let stage = quiet(stage_full(
        18_000,
        Some(far_boss()),
        vec![
            applies(
                json!({ "type": "vulnerable", "value_pct": 10, "duration_ticks": 3 }),
                START_X - 20_000,
            ),
            applies(
                json!({ "type": "atk_up", "value_pct": 10, "duration_ticks": 30 }),
                START_X,
            ),
            applies(
                json!({ "type": "slow", "value_pct": 10, "duration_ticks": 5 }),
                START_X + 20_000,
            ),
        ],
        vec![],
    ));
    let mut e = engine(&stage);
    // Each entry: the skill, its duration. It is listed on the tick it is applied and on every
    // tick before its duration is up, with what is left of it.
    let expected = |tick: u32| -> Vec<StatusView> {
        [("atk_up", 30), ("slow", 5), ("vulnerable", 3)]
            .into_iter()
            .filter(|(_, total)| *total >= tick)
            .map(|(id, total)| view(id, total + 1 - tick, total))
            .collect()
    };
    for tick in 1..=6 {
        e.step(&[Input::default()]);
        assert_eq!(player_statuses(&e), expected(tick), "tick {tick}");
    }
}

/// A pack leader holding at `LEADER`, with an aura of `RADIUS` at 25%, and a plain enemy that
/// starts 6,000 units to its side, outside their two bodies, and walks away from it.
const LEADER: (i32, i32) = (START_X + 25_600, START_Y);
const RADIUS: i32 = 10_000;

fn leader_and_walker() -> Stage {
    let mut leader = enemy(100_000, hold(LEADER.0, LEADER.1), json!([]));
    leader["radius"] = json!(3072);
    leader["elite"] = json!({ "range": 256,
        "aura": { "radius": RADIUS, "damage_percent": 25 } });
    let walker = enemy(
        5,
        json!({ "type": "straight", "vx": 1024, "vy": 0 }),
        json!([]),
    );
    quiet(stage_full(
        18_000,
        Some(far_boss()),
        vec![
            wave(1, 1, 0, leader, LEADER.0, LEADER.1),
            wave(1, 1, 0, walker, LEADER.0 + 6_000, LEADER.1),
        ],
        vec![],
    ))
}

#[test]
fn a_pack_leader_s_aura_is_a_status_of_each_enemy_in_its_reach_and_leaves_with_it() {
    let mut e = engine(&leader_and_walker());
    let aura = || view("aura", 0, 0);
    let mut reached = Vec::new();
    for tick in 1..=30 {
        e.step(&[Input::default()]);
        let snap = e.snapshot();
        let leader = snap.entities.iter().find(|v| v.elite).expect("the leader");
        let walker = snap
            .entities
            .iter()
            .find(|v| v.kind == EntityKind::Enemy && !v.elite)
            .expect("the walker");
        assert!(
            leader.statuses.is_empty(),
            "an elite is not helped: tick {tick}"
        );
        let dx = i64::from(walker.at.x.0) - i64::from(leader.at.x.0);
        let dy = i64::from(walker.at.y.0) - i64::from(leader.at.y.0);
        let within = dx * dx + dy * dy <= i64::from(RADIUS) * i64::from(RADIUS);
        let expected = if within { vec![aura()] } else { Vec::new() };
        assert_eq!(walker.statuses, expected, "tick {tick}");
        reached.push(within);
    }
    assert!(reached.contains(&true), "the walker starts in reach");
    assert!(reached.contains(&false), "and walks out of it");
}
