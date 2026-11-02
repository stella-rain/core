//! The region a bench counts calls the allocator exactly as the bench's own code does (ADR-SIM-8).
//! The bodies of the benches live in `benches/support/mod.rs`; this file counts those same
//! helpers with the global counting allocator, so an allocation added to a bench body fails here.

#[path = "../benches/support/mod.rs"]
#[allow(dead_code)]
mod support;

use crate::alloc::{Allocs, counted3};
use stella_rain_core::bench_fixtures::worst_minute_case::{MINUTE_TICKS, Run};
use stella_rain_core::bench_fixtures::{self, Case};

/// What an empty region counts: nothing (the fields of `Allocs` are private to `alloc`).
fn none() -> Allocs {
    counted3(|| ())
}

fn tick_case(build: fn() -> Case) -> Allocs {
    let case = build();
    let mut out = None;
    let counts = counted3(|| out = Some(support::tick(case)));
    std::mem::forget(out);
    counts
}

fn minute_case(build: fn() -> Run, ticks: u32) -> Allocs {
    let run = build();
    let mut out = None;
    let counts = counted3(|| out = Some(support::minute(run, ticks)));
    std::mem::forget(out);
    counts
}

#[test]
fn every_bench_region_is_allocation_free() {
    let measured = [
        ("worst_tick", tick_case(bench_fixtures::worst_tick)),
        ("hit_tick", tick_case(bench_fixtures::hit_tick)),
        (
            "hit_tick_spread",
            tick_case(bench_fixtures::hit_tick_spread),
        ),
        (
            "time_limit_tick",
            tick_case(bench_fixtures::time_limit_tick),
        ),
        (
            "worst_minute",
            minute_case(bench_fixtures::worst_minute, MINUTE_TICKS),
        ),
        (
            "worst_minute_peak",
            minute_case(bench_fixtures::worst_minute_peak, 1),
        ),
    ];
    let zero = none();
    let failures: Vec<String> = measured
        .iter()
        .filter(|(_, counts)| *counts != zero)
        .map(|(case, counts)| format!("{case}: (allocs, reallocs, frees) = {counts:?}"))
        .collect();
    assert!(
        failures.is_empty(),
        "expected none:\n{}",
        failures.join("\n")
    );
}
