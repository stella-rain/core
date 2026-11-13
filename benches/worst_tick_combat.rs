//! The instruction count of the worst combat tick (ADR-SIM-6): the worst-case tick with the step
//! repeats, the stagger and the hard controls at their caps (`bench_fixtures::combat_case`,
//! `src/bench_fixtures/combat_case.rs` checks that). Counted under Valgrind, so the number is the
//! same on every run of a build; its budget is the key `worst_tick_combat` of `budgets.toml`.
//!
//! Run it with `cargo bench --bench worst_tick_combat`; it needs `valgrind` and
//! `iai-callgrind-runner` 0.16.1.

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::bench_fixtures::{self, Case};
use stella_rain_core::engine::Engine;

mod support;

#[library_benchmark]
// The warm-up and the written caps are the setup: they run before the count starts.
#[bench::worst_tick_combat(args = (bench_fixtures::worst_tick_combat()), teardown = support::forget_measured)]
fn worst_tick_combat(case: Case) -> Engine {
    support::tick(case)
}

library_benchmark_group!(
    name = worst_combat_group;
    benchmarks = worst_tick_combat
);

main!(library_benchmark_groups = worst_combat_group);
