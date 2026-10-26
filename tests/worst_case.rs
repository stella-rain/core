//! The worst-case tick (ADR-020, ADR-021): the stage of `bench_fixtures::worst_case` holds every dynamic
//! cap at once, on the tick the benchmark counts and on the ticks after it. A benchmark of a
//! tick that does not would pass without measuring the worst case. Every attacker also holds an
//! active hit box (ADR-055). The main character holds every rune at full Rune Haste, its combo
//! steps hit every enemy of the cap (260) each tick, and each enemy carries all three damage-over-time
//! statuses at their stack cap (ADR-099). 128 zones are alive and active (core Z1).

use stella_rain_core::bench_fixtures::worst_case;

use stella_rain_core::attack::PhaseKind;
use stella_rain_core::engine::{Engine, Load, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::rune::{Rune, RuneKind};
use stella_rain_core::runes::MAX_DOT_STACKS;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::validate::limits::MAX_ELITES_ALIVE;

/// The enemies the run's cap allows (DIR1): the crowd of the case.
fn crowd(engine: &Engine) -> usize {
    engine.caps().enemies as usize
}

/// The most Rune Haste there is (`DEFAULT_MAX_RUNE_HASTE_PCT`, ADR-099 point 5).
const FULL_HASTE: u32 = 300;

/// Every enemy and every companion is in an active phase with a hit box, every companion's
/// cleave lands on every enemy, and nothing hurts a companion.
fn assert_hit_boxes(engine: &Engine) {
    let snapshot = engine.snapshot();
    let companions: Vec<_> = snapshot
        .entities
        .iter()
        .filter(|e| e.kind == EntityKind::Companion)
        .map(|e| e.id)
        .collect();
    let attackers: Vec<_> = snapshot
        .entities
        .iter()
        .filter(|e| matches!(e.kind, EntityKind::Enemy | EntityKind::Companion))
        .collect();
    assert_eq!(attackers.len(), crowd(engine) + 4);
    for e in attackers {
        let a = e.attack.as_ref().expect("an attack runs");
        assert_eq!(a.kind, PhaseKind::Active, "entity {:?}", e.id);
        assert!(a.hit_box.is_some(), "entity {:?}", e.id);
    }
    let hits = engine
        .events()
        .iter()
        .filter(|ev| matches!(ev, DomainEvent::Hit { source, .. } if companions.contains(source)))
        .count();
    assert_eq!(hits, 4 * crowd(engine));
}

/// Every zone cap is held (core Z1): 128 zones alive, 127 of them active and the fourth shooter's
/// warning line (RA1a) not, 16 of them the companions', which cover the crowd.
fn assert_zones(engine: &Engine) {
    let zones = engine.snapshot().zones;
    assert_eq!(zones.len(), 128);
    let warning = zones
        .iter()
        .filter(|z| z.phase != PhaseKind::Active)
        .count();
    assert_eq!(warning, 1, "the shooter's warn-only line");
    assert_eq!(zones.iter().filter(|z| z.friendly).count(), 16);
}

/// The main character holds all seven runes at full Rune Haste, and every enemy carries bleed,
/// poison and burn at the stack cap, with time left to run.
fn assert_runes_and_statuses(engine: &Engine) {
    let snapshot = engine.snapshot();
    let mut held: Vec<Rune> = snapshot.player.runes.iter().map(|r| r.rune).collect();
    held.sort();
    assert_eq!(held, Rune::ALL, "every rune held");
    assert_eq!(snapshot.player.rune_haste_pct, FULL_HASTE);
    assert!(
        !engine
            .events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::PlayerHit { .. })),
        "nothing hurts the main character"
    );
    let enemies: Vec<_> = snapshot
        .entities
        .iter()
        .filter(|e| e.kind == EntityKind::Enemy)
        .collect();
    assert_eq!(enemies.len(), crowd(engine));
    for e in enemies {
        let mut names: Vec<&str> = e
            .damage_over_time
            .iter()
            .map(|d| d.status.0.as_str())
            .collect();
        names.sort();
        assert_eq!(names, ["bleed", "burn", "poison"], "enemy {:?}", e.id);
        for d in &e.damage_over_time {
            assert_eq!(
                u32::from(d.stacks),
                MAX_DOT_STACKS,
                "{} on {:?}",
                d.status.0,
                e.id
            );
            assert!(d.ticks_left > 0, "{} on {:?}", d.status.0, e.id);
        }
    }
}

/// The main character's combo step lands on every enemy this tick, and each of the three
/// statuses deals its damage to every enemy: four hits from the main character on each.
fn assert_steps_and_ticks_land(engine: &Engine) {
    let player = EntityId(0);
    let mut hit: std::collections::BTreeMap<_, u32> = std::collections::BTreeMap::new();
    for ev in engine.events() {
        if let DomainEvent::Hit { source, target, .. } = ev {
            if *source == player {
                *hit.entry(*target).or_default() += 1;
            }
        }
    }
    let on_enemies = engine
        .snapshot()
        .entities
        .iter()
        .filter(|e| e.kind == EntityKind::Enemy && hit.get(&e.id).is_some_and(|n| *n >= 4))
        .count();
    assert_eq!(
        on_enemies,
        crowd(engine),
        "enemies hit by the step and three statuses"
    );
}

/// The push-apart has a fresh stack to part (ADR-058): the pass moves an enemy at most 4 pixels
/// (1,024 units) from the point the stack stood on, so after the tick at least 479 enemies are
/// that near it (all but 21: the line shooter's earlier arrival shifts the entity IDs, which
/// moves the push-apart's order; the most that were not, over the 60 measured ticks, was 21 of 500 before
/// DIR1). The others, the 15 to 21 that
/// started their attack the tick before and so were not ordered back, are within 8 pixels more.
fn assert_fresh_stack(engine: &Engine) {
    let (x, y) = worst_case::ENEMIES_AT;
    let far: Vec<i64> = engine
        .snapshot()
        .entities
        .iter()
        .filter(|e| e.kind == EntityKind::Enemy)
        .map(|e| (i64::from(e.at.x.0 - x).pow(2) + i64::from(e.at.y.0 - y).pow(2)).isqrt())
        .collect();
    let stacked = far.iter().filter(|d| **d <= 1_024).count();
    assert!(
        stacked + 21 >= crowd(engine),
        "{stacked} enemies were on the point"
    );
    assert!(
        far.iter().all(|d| *d <= 3 * 1_024),
        "{:?}",
        far.iter().max()
    );
}

/// 32 of the enemies are elites (`MAX_ELITES_ALIVE`, BR1e-3).
fn assert_elites(engine: &Engine) {
    let elites = engine
        .snapshot()
        .entities
        .iter()
        .filter(|e| e.kind == EntityKind::Enemy && e.elite)
        .count();
    assert_eq!(elites, usize::from(MAX_ELITES_ALIVE));
}

/// What the worst-case tick holds (core BR1b): the caps of the rules but the orbs' and the
/// emitter's (a handful of waits start and end each tick: `emitter_steps` is read from the engine).
/// Load counts an orb as an orb, so the 32 orbs are the only "orbs" and none is
/// made in the tick; the attack tasks are the crowd's waits. `assert_orbs_and_ranged` checks that
/// all 32 are orbs.
fn worst(engine: &Engine) -> Load {
    let steps = engine.load().emitter_steps;
    assert!((1..100).contains(&steps), "{steps} emitter steps");
    Load {
        emitter_steps: steps,
        ..engine.caps()
    }
}

/// 32 orbs are in the air (4 shooters, 8 each, core Z2), and the stage has
/// exactly four shooters, whose ranged runs (`MAX_RANGED_ATTACKERS`) each hold a token.
fn assert_orbs_and_ranged(engine: &Engine) {
    let snapshot = serde_json::to_value(engine.snapshot()).unwrap();
    let orbs = snapshot["orbs"].as_array().cloned().unwrap_or_default();
    assert_eq!(orbs.len(), 32);
    let stage = serde_json::to_value(worst_case::stage()).unwrap();
    let shooters: u64 = stage["waves"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w["enemy"]["rules"].to_string().contains("\"ranged\":true"))
        .map(|w| w["count"].as_u64().unwrap())
        .sum();
    assert_eq!(shooters, 4, "four shooters hold a ranged run");
}

#[test]
fn the_measured_tick_reaches_every_dynamic_cap() {
    let mut engine = worst_case::warmed_up();
    engine.step(worst_case::input(worst_case::WARMUP_TICKS + 1));
    assert_eq!(engine.outcome(), Outcome::Running, "the run ended");
    assert_eq!(engine.load(), worst(&engine));
    assert_hit_boxes(&engine);
    assert_zones(&engine);
    assert_elites(&engine);
    assert_orbs_and_ranged(&engine);
    assert_fresh_stack(&engine);
    assert_runes_and_statuses(&engine);
    assert_steps_and_ticks_land(&engine);
    // Aim assist looked through every enemy and found one in range to face.
    assert!(engine.snapshot().player.target.is_some());
}

#[test]
fn the_caps_are_held_tick_after_tick() {
    let mut engine = worst_case::warmed_up();
    let mut fired = std::collections::BTreeSet::new();
    for tick in worst_case::WARMUP_TICKS + 1..=worst_case::WARMUP_TICKS + 60 {
        engine.step(worst_case::input(tick));
        assert_eq!(
            engine.outcome(),
            Outcome::Running,
            "the run ended on tick {tick}"
        );
        // With the cap's 260 enemies, 252 attack tasks are alive on most ticks and the 256 of
        // `MAX_ATTACK_TASKS` on the measured one (500 enemies held all 256 on every tick).
        let load = engine.load();
        assert!((252..=256).contains(&load.tasks), "{load:?} on tick {tick}");
        assert_eq!(
            Load { tasks: 0, ..load },
            Load {
                tasks: 0,
                ..worst(&engine)
            },
            "on tick {tick}"
        );
        assert_hit_boxes(&engine);
        assert_zones(&engine);
        assert_elites(&engine);
        assert_orbs_and_ranged(&engine);
        assert_fresh_stack(&engine);
        assert_runes_and_statuses(&engine);
        assert_steps_and_ticks_land(&engine);
        for r in engine.snapshot().player.runes {
            if r.rune.kind() == RuneKind::OnHit && r.cooldown_left > 0 {
                fired.insert(r.rune);
            }
        }
    }
    // Haste keeps the cooldowns short enough that each on-hit rune fires within the 60 ticks.
    assert_eq!(fired.len(), 4, "on-hit runes that fired: {fired:?}");
}

/// Whether a JSON value holds a zone spawn of shape `line` anywhere inside it.
fn has_line_zone(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Object(o) => {
            (o.get("type").and_then(|t| t.as_str()) == Some("zone")
                && o.get("shape").and_then(|s| s["type"].as_str()) == Some("line"))
                || o.values().any(has_line_zone)
        }
        serde_json::Value::Array(a) => a.iter().any(has_line_zone),
        _ => false,
    }
}

/// The shooters' attacks as the stage writes them: for each wave of ranged enemies, how many
/// there are and whether their attack spawns a zone of shape `line` (RA1a, DS83).
fn shooter_waves() -> Vec<(u64, bool)> {
    let stage = serde_json::to_value(worst_case::stage()).unwrap();
    stage["waves"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w["enemy"]["rules"].to_string().contains("\"ranged\":true"))
        .map(|w| {
            (
                w["count"].as_u64().unwrap(),
                has_line_zone(&w["enemy"]["rules"]),
            )
        })
        .collect()
}

#[test]
fn one_of_the_four_shooters_lays_a_line() {
    let waves = shooter_waves();
    let line: u64 = waves.iter().filter(|(_, l)| *l).map(|(n, _)| n).sum();
    let other: u64 = waves.iter().filter(|(_, l)| !*l).map(|(n, _)| n).sum();
    assert_eq!(
        (line, other),
        (1, 3),
        "line shooters and the others: {waves:?}"
    );
}

#[test]
fn the_shooters_stand_in_the_view_of_the_main_character() {
    // The view is 640 x 360 pixels, centred on the main character; RA1 bars ranged starts
    // outside it. Every enemy stands on the point of `ENEMIES_AT`, the shooters among them.
    let mut engine = worst_case::warmed_up();
    engine.step(worst_case::input(worst_case::WARMUP_TICKS + 1));
    let snapshot = engine.snapshot();
    let me = snapshot.player.at;
    let mut seen = 0;
    for e in snapshot
        .entities
        .iter()
        .filter(|e| e.kind == EntityKind::Enemy)
    {
        seen += 1;
        let (dx, dy) = ((e.at.x.0 - me.x.0).abs(), (e.at.y.0 - me.y.0).abs());
        assert!(dx <= 320 * 256, "{:?} is {dx} units off on x", e.id);
        assert!(dy <= 180 * 256, "{:?} is {dy} units off on y", e.id);
    }
    assert_eq!(seen, crowd(&engine));
}

#[test]
fn the_line_shooter_arrives_in_view_of_the_main_character() {
    // RA1 bars a ranged start outside the 640 x 360 view, so the shooter that lays the warning
    // line must already be in view when it arrives (tick 2), or the companions take its zone
    // place first (RA1b, DS82). On tick 2 the only enemies are the first wave and that shooter.
    let mut engine = Engine::new_with(&worst_case::stage(), &worst_case::content()).unwrap();
    for tick in 1..=3 {
        engine.step(worst_case::input(tick));
    }
    let snapshot = engine.snapshot();
    let me = snapshot.player.at;
    let enemies: Vec<_> = snapshot
        .entities
        .iter()
        .filter(|e| e.kind == EntityKind::Enemy)
        .collect();
    assert!(enemies.len() >= 2, "enemies on tick 3: {}", enemies.len());
    for e in enemies {
        let (dx, dy) = ((e.at.x.0 - me.x.0).abs(), (e.at.y.0 - me.y.0).abs());
        assert!(dx <= 320 * 256, "{:?} is {dx} units off on x", e.id);
        assert!(dy <= 180 * 256, "{:?} is {dy} units off on y", e.id);
    }
}
