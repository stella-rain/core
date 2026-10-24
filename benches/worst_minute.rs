//! The instruction count of the densest minute of the `time_limit` corpus run (BR1e,
//! `bench_fixtures::worst_minute_case`): 3,600 ticks from `WORST_MINUTE_START`, with the run's
//! wave spawns, elites, boss and death bursts that a one-tick bench misses. The setup (the run
//! up to the minute) is not counted. Run it with `cargo bench --bench worst_minute` (needs what
//! `worst_tick` needs).

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::bench_fixtures;
use stella_rain_core::bench_fixtures::worst_minute_case::{MINUTE_TICKS, Run};

#[library_benchmark]
#[bench::worst_minute(bench_fixtures::worst_minute())]
fn worst_minute(mut run: Run) -> Run {
    run.play(MINUTE_TICKS);
    run
}

library_benchmark_group!(
    name = worst_minute_group;
    benchmarks = worst_minute
);

main!(library_benchmark_groups = worst_minute_group);
