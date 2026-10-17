//! The worst-case tick (ADR-020, ADR-021): the stage of `benches/worst_case/` holds every dynamic
//! cap at once, on the tick the benchmark counts and on the ticks after it. A benchmark of a
//! tick that does not would pass without measuring the worst case. Every attacker also holds an
//! active hit box (ADR-055).

#[path = "../benches/worst_case/mod.rs"]
mod worst_case;

use stella_rain_core::attack::PhaseKind;
use stella_rain_core::engine::{Engine, Load, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::snapshot::EntityKind;

/// Every enemy and every companion is in an active phase with a hit box, every companion's
/// cleave lands on all 500 enemies, and nothing hurts a companion.
fn assert_hit_boxes(engine: &Engine) {
    let snapshot = engine.snapshot();
    let attackers: Vec<_> = snapshot
        .entities
        .iter()
        .filter(|e| matches!(e.kind, EntityKind::Enemy | EntityKind::Companion))
        .collect();
    assert_eq!(attackers.len(), 504);
    for e in attackers {
        let a = e.attack.as_ref().expect("an attack runs");
        assert_eq!(a.kind, PhaseKind::Active, "entity {:?}", e.id);
        assert!(a.hit_box.is_some(), "entity {:?}", e.id);
    }
    let hits = engine
        .events()
        .iter()
        .filter(|ev| matches!(ev, DomainEvent::Hit { .. }))
        .count();
    assert_eq!(hits, 4 * 500);
}

/// The push-apart has a fresh stack to part (ADR-058): the pass moves an enemy at most 4 pixels
/// (1,024 units) from the point the stack stood on, so after the tick at least 480 enemies are
/// that near it. The others, the 15 or 20 that started their attack the tick before and so were
/// not ordered back, are within 8 pixels more.
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
    assert!(stacked >= 480, "{stacked} enemies were on the point");
    assert!(
        far.iter().all(|d| *d <= 3 * 1_024),
        "{:?}",
        far.iter().max()
    );
}

#[test]
fn the_measured_tick_reaches_every_dynamic_cap() {
    let mut engine = worst_case::warmed_up();
    engine.step(worst_case::input(worst_case::WARMUP_TICKS + 1));
    assert_eq!(engine.outcome(), Outcome::Running, "the run ended");
    assert_eq!(engine.load(), Load::CAPS);
    assert_hit_boxes(&engine);
    assert_fresh_stack(&engine);
    // Aim assist looked through every enemy and found none in range, so no step faces one.
    assert_eq!(engine.snapshot().player.target, None);
}

#[test]
fn the_caps_are_held_tick_after_tick() {
    let mut engine = worst_case::warmed_up();
    for tick in worst_case::WARMUP_TICKS + 1..=worst_case::WARMUP_TICKS + 60 {
        engine.step(worst_case::input(tick));
        assert_eq!(
            engine.outcome(),
            Outcome::Running,
            "the run ended on tick {tick}"
        );
        assert_eq!(engine.load(), Load::CAPS, "on tick {tick}");
        assert_hit_boxes(&engine);
        assert_fresh_stack(&engine);
    }
}
