//! The instruction count of a `time_limit` run's heaviest tick (core#212 step d,
//! `bench_fixtures::time_limit_case`): the mode's horde at the cap of 500 enemies alive, of every kind the
//! schedule spawns (the two bats and the six roles) in the ratio of its spawns, no projectile in the
//! air (since BR1a the mode's content fires none), a boss alive and the main character attacking. Run it with
//! `cargo bench --bench time_limit_tick` (needs what `worst_tick` needs).

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::bench_fixtures::{self, Case};
use stella_rain_core::engine::Engine;

mod support;

#[library_benchmark]
#[bench::time_limit_tick(args = (bench_fixtures::time_limit_tick()), teardown = support::forget_measured)]
fn time_limit_tick(case: Case) -> Engine {
    let Case { mut engine, input } = case;
    engine.step(input);
    engine
}

library_benchmark_group!(
    name = time_limit_group;
    benchmarks = time_limit_tick
);

main!(library_benchmark_groups = time_limit_group);
