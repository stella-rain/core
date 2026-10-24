//! The four bench cases (`bench_fixtures`, ADR-SIM-6) build the engine the benches count: the
//! state hash before and after the measured tick is pinned here. A pin asserts an output, so it
//! lives in `tests/`, not in the bench input: a change that moves a hash and touches `src/` is
//! then not refused by the bench guard (core#360).
//!
//! The values are those of the benches' own case modules (built on `tests/common/`) when the
//! fixtures were written: the fixtures must build the same engine, or the instruction counts move.

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
        0x8da23ca6e42b539c,
        0x4951b14566391b5d,
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
        0x3d264a19f06cff25,
        0xf143b972e536a94b,
    );
}

#[test]
fn the_time_limit_tick_is_the_benches_case() {
    pinned(
        bench_fixtures::time_limit_tick(),
        0xb8352ca01a24a44c,
        0x3b62752b8bc228ce,
    );
}
