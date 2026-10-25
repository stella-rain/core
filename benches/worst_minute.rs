//! The instruction count of the dense minute of the `time_limit` mode (BR1e-2,
//! `bench_fixtures::worst_minute_case`): 3,600 ticks from `MINUTE_START`, the crowd at its cap,
//! elites alive and a boss arriving, with the wave spawns and death bursts that a one-tick bench
//! misses. The setup (the run up to the minute) is not counted. Run it with
//! `cargo bench --bench worst_minute` (needs what `worst_tick` needs).

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::bench_fixtures;
use stella_rain_core::bench_fixtures::worst_minute_case::{MINUTE_TICKS, Run};

mod support;

#[library_benchmark]
#[bench::worst_minute(args = (bench_fixtures::worst_minute()), teardown = support::forget_measured)]
fn worst_minute(mut run: Run) -> Run {
    run.play(MINUTE_TICKS);
    run
}

library_benchmark_group!(
    name = worst_minute_group;
    benchmarks = worst_minute
);

main!(library_benchmark_groups = worst_minute_group);
