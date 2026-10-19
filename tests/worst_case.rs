//! The worst-case tick (ADR-020, ADR-021): the stage of `benches/worst_case/` holds every dynamic
//! cap at once, on the tick the benchmark counts and on the ticks after it. A benchmark of a
//! tick that does not would pass without measuring the worst case. Every attacker also holds an
//! active hit box (ADR-055). The main character holds every rune at full Rune Haste, its combo
//! steps hit all 500 enemies each tick, and each enemy carries all three damage-over-time
//! statuses at their stack cap (ADR-099).

// The benchmark's own module, which brings its own copy of `tests/common`; in the one test
// binary that copy sits beside `crate::common`, as it did when this file was a binary of its own.
#[path = "../benches/worst_case/mod.rs"]
#[allow(clippy::duplicate_mod, clippy::module_inception)]
mod worst_case;

use stella_rain_core::attack::PhaseKind;
use stella_rain_core::engine::{Engine, Load, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::rune::{Rune, RuneKind};
use stella_rain_core::runes::MAX_DOT_STACKS;
use stella_rain_core::snapshot::EntityKind;

/// The most Rune Haste there is (`MAX_RUNE_HASTE_PCT`, ADR-099 point 5).
const FULL_HASTE: u32 = 300;

/// Every enemy and every companion is in an active phase with a hit box, every companion's
/// cleave lands on all 500 enemies, and nothing hurts a companion.
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
    assert_eq!(attackers.len(), 504);
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
    assert_eq!(hits, 4 * 500);
}

/// The main character holds all seven runes at full Rune Haste, and every enemy carries bleed,
/// poison and burn at the stack cap, with time left to run.
fn assert_runes_and_statuses(engine: &Engine) {
    let snapshot = engine.snapshot();
    let mut held: Vec<Rune> = snapshot.player.runes.iter().map(|r| r.rune).collect();
    held.sort();
    assert_eq!(held, Rune::ALL, "every rune held");
    assert_eq!(snapshot.player.rune_haste_pct, FULL_HASTE);
    let enemies: Vec<_> = snapshot
        .entities
        .iter()
        .filter(|e| e.kind == EntityKind::Enemy)
        .collect();
    assert_eq!(enemies.len(), 500);
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
    let on_enemies = hit.values().filter(|n| **n >= 4).count();
    assert_eq!(
        on_enemies, 500,
        "enemies hit by the step and three statuses"
    );
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
        assert_eq!(engine.load(), Load::CAPS, "on tick {tick}");
        assert_hit_boxes(&engine);
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
