//! The instruction count of the worst friendly-bullet collision tick within the caps of ADR-020
//! (`bench_fixtures::collide_case`): 500 enemies and about 3,900 friendly bullets that hit none of them.
//! `collide_tick_spread` is the same tick with the enemies spread over the largest map
//! (`bench_fixtures::collide_case::spread_stage`).
//! Run it with `cargo bench --bench collide_tick` (needs what `worst_tick` needs).

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::bench_fixtures::{self, Case};
use stella_rain_core::engine::Engine;

#[library_benchmark]
#[bench::collide_tick(bench_fixtures::collide_tick())]
// The same horde over the largest map (core#146): the grid spans the map, and its build reads it.
#[bench::collide_tick_spread(bench_fixtures::collide_tick_spread())]
fn collide_tick(case: Case) -> Engine {
    let Case { mut engine, input } = case;
    engine.step(input);
    engine
}

library_benchmark_group!(
    name = collide_group;
    benchmarks = collide_tick
);

main!(library_benchmark_groups = collide_group);
