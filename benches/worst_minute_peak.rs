//! The instruction count of the heaviest tick of that minute (BR1e,
//! `bench_fixtures::worst_minute_case::worst_minute_window`), by the same load. Its own target so
//! that it has a budget of its own: a key of `benches/budgets.toml` is a bench target. Run it
//! with `cargo bench --bench worst_minute_peak`.

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::bench_fixtures;
use stella_rain_core::bench_fixtures::worst_minute_case::Run;

#[library_benchmark]
#[bench::worst_minute_peak(args = (bench_fixtures::worst_minute_peak()), teardown = drop_measured)]
fn worst_minute_peak(mut run: Run) -> Run {
    run.play(1);
    run
}

// The returned value is dropped here, outside the counted region: the free path's cost
// depends on the heap layout and is not the operation this bench names.
fn drop_measured(value: Run) {
    drop(value);
}

library_benchmark_group!(
    name = worst_minute_peak_group;
    benchmarks = worst_minute_peak
);

main!(library_benchmark_groups = worst_minute_peak_group);
