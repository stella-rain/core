//! The densest real minute of the `time_limit` corpus run (BR1e, core#317, D-BUDGET A in DS49):
//! `bench_fixtures::worst_minute_case` finds where the benches start counting; this pins where.
//! The same minute's peak heap stays at or below 16 MiB.
use stella_rain_core::bench_fixtures::worst_minute_case::{MINUTE_TICKS, Run, worst_minute_window};

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
