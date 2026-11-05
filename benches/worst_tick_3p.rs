//! The instruction count of the worst-case tick with three players (ADR-SIM-6): the tick of
//! `worst_tick` after slots 1 and 2 joined and pressed Main (`bench_fixtures::party_case`).
//! Run it with `cargo bench --bench worst_tick_3p`; it needs `valgrind` and `iai-callgrind-runner`
//! 0.16.1.

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::bench_fixtures::party_case::{self, Case};
use stella_rain_core::engine::Engine;

mod support;

#[library_benchmark]
// The warm-up and the joins are the setup: they run before the count starts.
#[bench::worst_tick_3p(args = (party_case::worst_tick_3p()), teardown = support::forget_measured)]
fn worst_tick_3p(case: Case) -> Engine {
    party_case::tick(case)
}

library_benchmark_group!(
    name = worst_tick_3p_group;
    benchmarks = worst_tick_3p
);

main!(library_benchmark_groups = worst_tick_3p_group);
