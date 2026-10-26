//! The four bench cases (`bench_fixtures`, ADR-SIM-6) build the engine the benches count: the
//! state hash before and after the measured tick is pinned here. A pin asserts an output, so it
//! lives in `tests/`, not in the bench input: a change that moves a hash and touches `src/` is
//! then not refused by the bench guard (core#360).
//!
//! The values are those of the benches' own case modules (built on `tests/common/`) when the
//! fixtures were written: the fixtures must build the same engine, or the instruction counts move.

use stella_rain_core::bench_fixtures::worst_minute_case::{MINUTE_TICKS, Run};
use stella_rain_core::bench_fixtures::{self, Case};

fn pinned(case: Case, before: u64, after: u64) {
    let Case { mut engine, input } = case;
    assert_eq!(engine.state_hash(), before, "the warmed-up engine");
    engine.step(input);
    assert_eq!(engine.state_hash(), after, "after the measured tick");
}

#[test]
fn the_worst_tick_is_the_benches_case() {
    pinned(
        bench_fixtures::worst_tick(),
        0x5c1e1548f693fc2c,
        0x6181713d8f5da927,
    );
}

#[test]
fn the_hit_tick_is_the_benches_case() {
    pinned(
        bench_fixtures::hit_tick(),
        0xe85bd3364b22c9e5,
        0x95527ebc68a22c23,
    );
}

#[test]
fn the_spread_hit_tick_is_the_benches_case() {
    pinned(
        bench_fixtures::hit_tick_spread(),
        0x4b485ac6af094984,
        0xab1fe8f19a88d366,
    );
}

#[test]
fn the_time_limit_tick_is_the_benches_case() {
    pinned(
        bench_fixtures::time_limit_tick(),
        0xda515bd754b15f82,
        0x6ae2317b0139581f,
    );
}

fn pinned_run(mut run: Run, ticks: u32, before: u64, after: u64) {
    assert_eq!(run.engine.state_hash(), before, "the fast-forwarded engine");
    run.play(ticks);
    assert_eq!(run.engine.state_hash(), after, "after the measured ticks");
}

// The two constants of the densest minute are re-pinned together with these hashes after any
// outcome change (`src/bench_fixtures/worst_minute_case.rs`).
#[test]
fn the_worst_minute_is_the_benches_case() {
    pinned_run(
        bench_fixtures::worst_minute(),
        MINUTE_TICKS,
        0x69668d2fa05e60ea,
        0x3912386003add26a,
    );
}

#[test]
fn the_worst_minute_peak_tick_is_the_benches_case() {
    pinned_run(
        bench_fixtures::worst_minute_peak(),
        1,
        0x80713e02c97d88ae,
        0xea386f48b5c6bdbd,
    );
}
