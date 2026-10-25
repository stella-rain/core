//! The dense minute of the `time_limit` mode (BR1e-2, core#368): `bench_fixtures::worst_minute_case`
//! scripts a minute with the crowd at its caps, elites alive and a boss arriving, and finds its
//! heaviest tick; this checks the minute is that and pins where. The same minute's peak heap stays
//! at or below 16 MiB.
use stella_rain_core::bench_fixtures::worst_minute_case::{MINUTE_TICKS, Run, worst_minute_window};
use stella_rain_core::engine::{Load, Outcome};
use stella_rain_core::snapshot::EntityKind;

use crate::alloc::{live_bytes, peak_bytes, reset_peak};

/// Where the densest minute starts and its heaviest tick, as ticks played before them. The bench
/// finds them itself; these pin what it finds. After any change of the run's outcome that moves
/// them, this test names the new values: set them here and the hashes in
/// `tests/bench_fixture_pins.rs` (a `src/` PR then edits only `tests/`, which the bench guard
/// allows).
const WORST_MINUTE_START: u32 = 1_684;
const WORST_TICK_IN_MINUTE: u32 = 2_699;

/// The most heap the engine may hold at once in the worst minute (16 MiB).
const PEAK_HEAP_LIMIT: i64 = 16 * 1024 * 1024;

#[test]
fn the_densest_minute_of_the_run_is_where_it_is_pinned() {
    assert_eq!(
        worst_minute_window(),
        (WORST_MINUTE_START, WORST_TICK_IN_MINUTE),
        "the densest minute (by the unweighted sum of Load) and its heaviest tick moved: re-pin          the constants here and the hashes in tests/bench_fixture_pins.rs"
    );
}

#[test]
fn the_densest_minute_holds_at_most_16_mib() {
    let held = live_bytes();
    let mut run = Run::fast_forwarded(WORST_MINUTE_START);
    reset_peak();
    run.play(MINUTE_TICKS);
    let peak = peak_bytes() - held;
    eprintln!("peak heap of the densest minute: {peak} bytes");
    assert!(
        peak <= PEAK_HEAP_LIMIT,
        "peak heap {peak} bytes, limit {PEAK_HEAP_LIMIT}"
    );
}

#[test]
fn the_minute_starts_with_the_crowd_at_its_cap_and_an_elite_alive() {
    let run = Run::fast_forwarded(WORST_MINUTE_START);
    let entities = run.engine.snapshot().entities;
    let enemies = entities
        .iter()
        .filter(|e| e.kind == EntityKind::Enemy)
        .count();
    assert_eq!(enemies as u32, Load::CAPS.enemies);
    assert!(entities.iter().any(|e| e.elite), "no elite is alive");
}

#[test]
fn a_boss_arrives_inside_the_minute_and_the_player_survives_it() {
    let mut run = Run::fast_forwarded(WORST_MINUTE_START);
    let has_boss = |run: &Run| {
        run.engine
            .snapshot()
            .entities
            .iter()
            .any(|e| e.kind == EntityKind::Boss)
    };
    assert!(!has_boss(&run), "a boss is already there at the start");
    let mut arrived = None;
    for _ in 0..MINUTE_TICKS {
        run.play(1);
        if arrived.is_none() && has_boss(&run) {
            arrived = Some(run.engine.tick());
        }
    }
    let at = arrived.expect("no boss arrived in the minute");
    assert!((WORST_MINUTE_START..WORST_MINUTE_START + MINUTE_TICKS).contains(&at));
    assert_eq!(
        run.engine.outcome(),
        Outcome::Running,
        "the player did not survive"
    );
    assert!(run.engine.snapshot().player.hp > 0);
}

#[test]
fn the_heaviest_tick_lies_inside_the_minute() {
    let (start, peak) = worst_minute_window();
    assert!((start..start + MINUTE_TICKS).contains(&peak));
}
