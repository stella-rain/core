//! Attacks as phases (ADR-055): each phase lasts its ticks and spawns its nodes on its first
//! tick; an attacker runs one attack at a time; the patterns a phase spawns run to their own
//! end; a wind-up announces itself; `move_pct` scales the attacker's movement; a boss timeline
//! step waits for its attack; the snapshot shows the phase.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::attack::PhaseKind;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::Stage;

const OX: i32 = 46_080;
const OY: i32 = 50_000;

fn down() -> Value {
    json!({ "type": "orb", "speed": 256, "ticks": 100,
            "direction": { "type": "absolute", "angle": 0 } })
}

fn phase(kind: &str, ticks: u32, spawn: Vec<Value>) -> Value {
    json!({ "kind": kind, "ticks": ticks, "spawn": spawn })
}

/// An enemy holding still that attacks with `phases` whenever its rule lets it.
fn attacker(phases: Vec<Value>, once: bool) -> Value {
    enemy(
        50,
        hold(OX, OY),
        json!([{ "when": always(), "target": "player", "once": once, "cooldown_ticks": 0,
                 "do": { "type": "attack", "attack": { "inline": phases } } }]),
    )
}

/// The weapons are quiet, so that every bullet is the attacker's.
fn stage_of(enemy: Value) -> Stage {
    quiet(stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, enemy, OX, OY)],
        vec![],
    ))
}

/// The ticks on which new bullets appeared, over `ticks` ticks.
fn bullet_ticks(stage: &Stage, ticks: u32) -> Vec<u32> {
    let mut e = engine(stage);
    let mut out = Vec::new();
    let mut before = 0;
    for _ in 0..ticks {
        e.step(idle());
        let now = e.snapshot().bullets.len();
        out.extend(std::iter::repeat_n(e.tick(), now.saturating_sub(before)));
        before = now;
    }
    out
}

#[test]
fn each_phase_lasts_its_ticks_and_spawns_on_its_first() {
    // Started on tick 1: wind-up 1..=5, active 6, recovery 7..=9, active 10.
    let stage = stage_of(attacker(
        vec![
            phase("wind_up", 5, vec![]),
            phase("active", 1, vec![down()]),
            phase("recovery", 3, vec![]),
            phase("active", 1, vec![down()]),
        ],
        true,
    ));
    assert_eq!(bullet_ticks(&stage, 20), [6, 10]);
}

#[test]
fn a_wind_up_announces_itself() {
    let stage = stage_of(attacker(
        vec![
            phase("wind_up", 5, vec![]),
            phase("active", 1, vec![down()]),
        ],
        true,
    ));
    let mut e = engine(&stage);
    let events = run_until(&mut e, idle(), 10, |_| false);
    let telegraphs: Vec<(u32, u32)> = events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::Telegraph { source, ticks } => {
                assert_ne!(*source, EntityId(0), "the enemy announces it");
                Some((*t, *ticks))
            }
            _ => None,
        })
        .collect();
    assert_eq!(telegraphs, [(1, 5)]);
}

#[test]
fn an_attacker_runs_one_attack_at_a_time() {
    // The rule could fire every tick; the attack lasts 10 ticks, so it starts on 1, 11 and 21.
    let stage = stage_of(attacker(
        vec![
            phase("active", 1, vec![down()]),
            phase("recovery", 9, vec![]),
        ],
        false,
    ));
    assert_eq!(bullet_ticks(&stage, 25), [1, 11, 21]);
}

#[test]
fn a_spawned_pattern_runs_past_the_end_of_its_phase() {
    let repeat = json!({ "type": "repeat", "count": 3, "interval_ticks": 4, "body": [down()] });
    let stage = stage_of(attacker(vec![phase("active", 1, vec![repeat])], true));
    let mut e = engine(&stage);
    e.step(idle());
    e.step(idle());
    let enemy = e
        .snapshot()
        .entities
        .into_iter()
        .find(|v| v.kind == EntityKind::Enemy)
        .expect("the enemy");
    assert!(enemy.attack.is_none(), "the one-tick attack is over");
    assert_eq!(bullet_ticks(&stage, 20).len(), 3);
}

#[test]
fn move_pct_scales_the_attackers_movement() {
    // Walks down 100 a tick; the attack's one phase lasts 10 ticks (1..=10).
    let walker = |move_pct: u8| {
        enemy(
            50,
            json!({ "type": "straight", "vx": 0, "vy": 100 }),
            json!([{ "when": always(), "target": "player", "once": true,
                     "do": { "type": "attack", "attack": { "inline": [
                         { "kind": "active", "ticks": 10, "move_pct": move_pct }] } } }]),
        )
    };
    let ys = |move_pct: u8| {
        let mut e = engine(&stage_of(walker(move_pct)));
        (0..12)
            .map(|_| {
                e.step(idle());
                e.snapshot()
                    .entities
                    .into_iter()
                    .find(|v| v.kind == EntityKind::Enemy)
                    .map(|v| v.at.y.0 - OY)
                    .expect("the enemy")
            })
            .collect::<Vec<_>>()
    };
    // It moves on tick 1 before its rule fires; ticks 2..=10 run the phase; 11 is free again.
    assert_eq!(
        ys(0),
        [100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 200, 300]
    );
    assert_eq!(
        ys(50),
        [100, 150, 200, 250, 300, 350, 400, 450, 500, 550, 650, 750]
    );
    assert_eq!(
        ys(100),
        [
            100, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 1100, 1200
        ]
    );
}

#[test]
fn the_snapshot_shows_the_phase() {
    let stage = stage_of(attacker(
        vec![
            phase("wind_up", 3, vec![]),
            json!({ "kind": "active", "ticks": 2,
                    "spawn": [down()] }),
        ],
        true,
    ));
    let mut e = engine(&stage);
    let mut seen = Vec::new();
    for _ in 0..7 {
        e.step(idle());
        let enemy = e
            .snapshot()
            .entities
            .into_iter()
            .find(|v| v.kind == EntityKind::Enemy)
            .expect("the enemy");
        seen.push(enemy.attack.map(|a| (a.phase, a.kind, a.tick, a.ticks)));
    }
    assert_eq!(
        seen,
        [
            Some((0, PhaseKind::WindUp, 0, 3)),
            Some((0, PhaseKind::WindUp, 1, 3)),
            Some((0, PhaseKind::WindUp, 2, 3)),
            Some((1, PhaseKind::Active, 0, 2)),
            Some((1, PhaseKind::Active, 1, 2)),
            None,
            None,
        ]
    );
    // An inline attack has no preset ID; the facing points at the main character, below.
    let mut e = engine(&stage);
    e.step(idle());
    let view = e
        .snapshot()
        .entities
        .into_iter()
        .find(|v| v.kind == EntityKind::Enemy)
        .and_then(|v| v.attack)
        .expect("an attack");
    assert_eq!(view.preset, None);
    assert!(view.facing.y > 0 && view.facing.x.abs() < view.facing.y);
}

#[test]
fn a_boss_timeline_attack_step_waits_for_its_attack() {
    let attack = json!({ "type": "attack", "attack": { "inline": [
        phase("active", 1, vec![down()]), phase("recovery", 19, vec![]) ] } });
    let boss = json!({ "base": "golem", "hp": 1_000_000, "radius": 8192,
                       "spawn": { "x": OX, "y": OY },
                       "phases": [{ "timeline": [attack.clone(), attack] }] });
    let stage = quiet(stage_full(18_000, Some(boss), vec![], vec![]));
    assert_eq!(bullet_ticks(&stage, 45), [1, 21]);
}

#[test]
fn leaving_a_boss_phase_ends_its_attack() {
    // The first phase starts a 30-tick attack and ends after 5 ticks; its second active phase
    // (tick 30) never comes.
    let boss = json!({ "base": "golem", "hp": 1_000_000, "radius": 8192,
                       "spawn": { "x": OX, "y": OY },
                       "phases": [
                           { "until": { "type": "time_above", "ticks": 5 },
                             "timeline": [{ "type": "attack", "attack": { "inline": [
                                 phase("active", 1, vec![down()]),
                                 phase("recovery", 28, vec![]),
                                 phase("active", 1, vec![down()]) ] } }] },
                           {} ] });
    let stage = quiet(stage_full(18_000, Some(boss), vec![], vec![]));
    assert_eq!(bullet_ticks(&stage, 40), [1]);
}
