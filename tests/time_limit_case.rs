//! The heaviest tick of a `time_limit` run (core#212 step d): the stage of
//! `bench_fixtures::time_limit_case` holds the horde at its cap on the tick the benchmark counts. A
//! benchmark of a tick that does not would pass without measuring the heavy case.

use stella_rain_core::bench_fixtures::{enemy_cap, time_limit_case};

use std::collections::BTreeSet;

use stella_rain_core::engine::Outcome;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::Wave;
use stella_rain_core::time_limit::TimeLimit;

#[test]
fn the_bats_spawn_a_hundred_enemies_a_minute() {
    // The regular roles of R2 come on top of the bats, which have no ID; the bench takes the bats
    // (see `stage`).
    let a_minute: u32 = TimeLimit::bundled()
        .waves()
        .iter()
        .filter(|w| w.id.is_none())
        .map(|w| 3600 / w.every_ticks)
        .sum();
    assert_eq!(a_minute, 100);
}

/// A bench wave is a wave of the mode's schedule but for its timing, hit points, rewards and the
/// exploder's self-destruct, which the bench leaves off as its rule (the exploders burst and
/// stay, so the measured tick holds the cap with the burst running).
fn is_kind_of(bench: &Wave, mode: &Wave) -> bool {
    let mut rules = mode.enemy.rules.clone();
    for rule in &mut rules {
        rule.self_destruct = false;
    }
    bench.id == mode.id
        && bench.around == mode.around
        && bench.enemy.base == mode.enemy.base
        && bench.enemy.movement == mode.enemy.movement
        && bench.enemy.rules == rules
}

#[test]
fn the_stage_has_every_enemy_kind_of_the_schedule_in_its_ratio_and_the_cap_of_them() {
    let stage = time_limit_case::stage();
    let mode = TimeLimit::bundled();
    assert_eq!(
        stage.waves.iter().map(|w| u32::from(w.count)).sum::<u32>(),
        u32::from(enemy_cap())
    );
    let spawns: u32 = mode.waves().iter().map(|w| u32::from(w.count)).sum();
    for kind in mode.waves() {
        let count: u32 = stage
            .waves
            .iter()
            .filter(|w| is_kind_of(w, kind))
            .map(|w| u32::from(w.count))
            .sum();
        assert!(
            count >= 1,
            "no enemy of {:?} ({:?})",
            kind.id,
            kind.enemy.base
        );
        // In proportion to the schedule's spawns, to the nearest enemy.
        let exact = u32::from(enemy_cap()) * u32::from(kind.count);
        assert!(
            count * spawns <= exact + spawns && exact <= count * spawns + spawns,
            "{count} of {} for {:?}: {} of {spawns} spawns",
            enemy_cap(),
            kind.id,
            kind.count
        );
    }
    for w in &stage.waves {
        assert!(
            mode.waves().iter().any(|m| is_kind_of(w, m)),
            "a wave that is not the mode's: {:?}",
            w.enemy.base
        );
        assert!(w.count <= 100, "a wave of at most MAX_WAVE_COUNT");
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
    assert_eq!(
        load.enemies,
        u32::from(enemy_cap()),
        "the cap on enemies alive"
    );
    // BR1a: no orb is fired any more; the horde attacks with zones and melee.
    assert_eq!(load.orbs, 0, "no orbs alive");
    assert!(load.tasks >= 1, "enemies are attacking");
    assert!(load.zones >= 1, "zones are placed");
    let snapshot = engine.snapshot();
    assert!(snapshot.player.hp > 0, "the main character is alive");
    let alive = |kind| snapshot.entities.iter().filter(|e| e.kind == kind).count();
    assert_eq!(alive(EntityKind::Enemy), usize::from(enemy_cap()));
    assert_eq!(alive(EntityKind::Boss), 1);
    // Every sprite the schedule's kinds use is alive on the tick (the kinds that share a sprite
    // are told apart by `the_stage_has_every_enemy_kind_of_the_schedule_in_its_ratio`).
    let sprites: BTreeSet<_> = TimeLimit::bundled()
        .waves()
        .iter()
        .map(|w| w.enemy.base.clone())
        .collect();
    let on_the_tick: BTreeSet<_> = snapshot
        .entities
        .iter()
        .filter(|e| e.kind == EntityKind::Enemy)
        .map(|e| e.asset.clone())
        .collect();
    assert_eq!(on_the_tick, sprites, "the sprites of the schedule's kinds");
    let hits = engine
        .events()
        .iter()
        .filter(|ev| matches!(ev, DomainEvent::Hit { source, .. } if *source == EntityId(0)))
        .count();
    assert!(hits >= 1, "the Main weapon hits on the measured tick");
}
