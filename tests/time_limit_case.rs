//! The heaviest tick of a `time_limit` run (core#212 step d): the stage of
//! `benches/time_limit_case/` holds the horde at its cap on the tick the benchmark counts. A
//! benchmark of a tick that does not would pass without measuring the heavy case.

// The benchmark's own module; see `tests/worst_case.rs` for why it is a module here.
#[path = "../benches/time_limit_case/mod.rs"]
#[allow(clippy::duplicate_mod, clippy::module_inception)]
mod time_limit_case;

use stella_rain_core::engine::Outcome;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::time_limit::TimeLimit;

#[test]
fn the_mode_spawns_a_hundred_enemies_a_minute() {
    let a_minute: u32 = TimeLimit::bundled()
        .waves()
        .iter()
        .map(|w| 3600 / w.every_ticks)
        .sum();
    assert_eq!(a_minute, 100);
}

#[test]
fn the_stage_has_the_modes_enemies_and_the_cap_of_them() {
    let stage = time_limit_case::stage();
    let mode = TimeLimit::bundled();
    assert_eq!(
        stage.waves.iter().map(|w| u32::from(w.count)).sum::<u32>(),
        500
    );
    for w in &stage.waves {
        let kind = mode
            .waves()
            .iter()
            .find(|m| m.enemy.base == w.enemy.base && m.around == w.around)
            .expect("an enemy of the mode");
        assert_eq!(w.enemy.movement, kind.enemy.movement);
        assert_eq!(w.enemy.rules, kind.enemy.rules);
    }
}

#[test]
fn the_measured_tick_holds_the_horde_a_boss_and_the_main_characters_hits() {
    let mut engine = time_limit_case::warmed_up();
    assert_eq!(engine.tick(), time_limit_case::WARMUP_TICKS);
    engine.step(time_limit_case::input(time_limit_case::WARMUP_TICKS + 1));
    assert_eq!(engine.outcome(), Outcome::Running);
    assert!(!engine.offer_open());
    let load = engine.load();
    assert_eq!(load.enemies, 500, "the cap on enemies alive");
    assert!(load.bullets >= 3_900, "{} bullets alive", load.bullets);
    assert!(load.tasks >= 1, "enemies are attacking");
    let snapshot = engine.snapshot();
    assert!(snapshot.player.hp > 0, "the main character is alive");
    let alive = |kind| snapshot.entities.iter().filter(|e| e.kind == kind).count();
    assert_eq!(alive(EntityKind::Enemy), 500);
    assert_eq!(alive(EntityKind::Boss), 1);
    let hits = engine
        .events()
        .iter()
        .filter(|ev| matches!(ev, DomainEvent::Hit { source, .. } if *source == EntityId(0)))
        .count();
    assert!(hits >= 1, "the Main weapon hits on the measured tick");
}
