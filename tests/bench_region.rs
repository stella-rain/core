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

fn measured() -> [(&'static str, Allocs); 6] {
    [
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
    ]
}

/// The real bench helpers call the allocator as `BENCH_TICK_ALLOCS` pins. An allocation in a
/// helper moves a zero case off zero; a simulation change that drops the allocations of
/// `worst_tick` or `worst_minute` fails until the pin is lowered.
#[test]
fn the_bench_helpers_allocate_as_pinned() {
    let pins = crate::alloc::BENCH_TICK_ALLOCS;
    let measured = measured();
    assert_eq!(measured.len(), pins.len());
    for ((case, counts), (pinned_case, pin)) in measured.into_iter().zip(pins) {
        assert_eq!(case, pinned_case, "the cases keep the order of the pins");
        assert_eq!(counts, pin, "{case}: (allocs, reallocs, frees)");
    }
}

/// The four cases whose region the simulation leaves allocation-free stay at nothing.
#[test]
fn the_allocation_free_regions_stay_free() {
    let zero = none();
    for (case, counts) in measured() {
        if !matches!(case, "worst_tick" | "worst_minute") {
            assert_eq!(counts, zero, "{case}: (allocs, reallocs, frees)");
        }
    }
}

/// The count sees a harness allocation: a region that makes one is not zero.
#[test]
fn a_harness_allocation_is_seen() {
    let counts = counted3(|| Vec::<u8>::with_capacity(1));
    assert_ne!(counts, none());
}
