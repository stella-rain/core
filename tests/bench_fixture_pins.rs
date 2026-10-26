//! The four bench cases (`bench_fixtures`, ADR-SIM-6) build the engine the benches count: the
//! state hash before and after the measured tick is pinned in `tests/bench_pins.toml`. A pin asserts an output, so it
//! lives in `tests/`, not in the bench input: a change that moves a hash and touches `src/` is
//! then not refused by the bench guard (core#360).
//!
//! The values are those of the benches' own case modules (built on `tests/common/`) when the
//! fixtures were written: the fixtures must build the same engine, or the instruction counts move.

use stella_rain_core::bench_fixtures::worst_minute_case::{MINUTE_TICKS, Run};
use stella_rain_core::bench_fixtures::{self, Case};

/// The pinned hash `key` of case `id` in `tests/bench_pins.toml`.
fn pin(id: &str, key: &str) -> u64 {
    let file = include_str!("bench_pins.toml")
        .parse::<toml::Table>()
        .expect("tests/bench_pins.toml is TOML");
    let hex = file
        .get(id)
        .and_then(|case| case.get(key))
        .and_then(toml::Value::as_str)
        .unwrap_or_else(|| panic!("{id}.{key} is missing from tests/bench_pins.toml"));
    u64::from_str_radix(hex, 16).unwrap_or_else(|_| panic!("{id}.{key} is not 16 hex: {hex}"))
}

fn pinned(id: &str, case: Case) {
    let Case { mut engine, input } = case;
    assert_eq!(
        engine.state_hash(),
        pin(id, "before"),
        "{id}: the warmed-up engine"
    );
    engine.step(input);
    assert_eq!(
        engine.state_hash(),
        pin(id, "after"),
        "{id}: after the measured tick"
    );
}

#[test]
fn the_worst_tick_is_the_benches_case() {
    pinned("worst_tick", bench_fixtures::worst_tick());
}

#[test]
fn the_hit_tick_is_the_benches_case() {
    pinned("hit_tick", bench_fixtures::hit_tick());
}

#[test]
fn the_spread_hit_tick_is_the_benches_case() {
    pinned("hit_tick_spread", bench_fixtures::hit_tick_spread());
}

#[test]
fn the_time_limit_tick_is_the_benches_case() {
    pinned("time_limit_tick", bench_fixtures::time_limit_tick());
}

fn pinned_run(id: &str, mut run: Run, ticks: u32) {
    assert_eq!(
        run.engine.state_hash(),
        pin(id, "before"),
        "{id}: the fast-forwarded engine"
    );
    run.play(ticks);
    assert_eq!(
        run.engine.state_hash(),
        pin(id, "after"),
        "{id}: after the measured ticks"
    );
}

// The two constants of the densest minute are re-pinned together with these hashes after any
// outcome change (`src/bench_fixtures/worst_minute_case.rs`).
#[test]
fn the_worst_minute_is_the_benches_case() {
    pinned_run("worst_minute", bench_fixtures::worst_minute(), MINUTE_TICKS);
}

#[test]
fn the_worst_minute_peak_tick_is_the_benches_case() {
    pinned_run("worst_minute_peak", bench_fixtures::worst_minute_peak(), 1);
}
