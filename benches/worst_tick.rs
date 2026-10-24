//! The instruction count of a worst-case tick (ADR-021): every dynamic cap of ADR-020 reached
//! at once (`tests/worst_case.rs` checks that). Counted under Valgrind, so the number is the
//! same on every run of a build and does not depend on the CI runner's load; CI compares it with
//! the one of the base commit and fails on a rise (`.github/workflows/rust.yml`).
//!
//! Run it with `cargo bench --bench worst_tick`; it needs `valgrind` and `iai-callgrind-runner`
//! 0.16.1 (`cargo install iai-callgrind-runner --version 0.16.1 --locked`).

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::bench_fixtures::{self, Case};
use stella_rain_core::engine::Engine;

#[library_benchmark]
// The warm-up is the setup: it runs before the count starts, so only the one tick is counted.
#[bench::worst_tick(bench_fixtures::worst_tick())]
fn worst_tick(case: Case) -> Engine {
    let Case { mut engine, input } = case;
    engine.step(input);
    engine
}

library_benchmark_group!(
    name = worst_case_group;
    benchmarks = worst_tick
);

main!(library_benchmark_groups = worst_case_group);
