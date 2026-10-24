//! The instruction count of the worst melee tick within the caps of ADR-020
//! (`bench_fixtures::hit_case`): 500 enemies under the main character's hit box, four companions'
//! arcs and 128 zones, 112 of them friendly, with no projectile in the air. `hit_tick_spread` is the
//! same tick with the crowd spread over the largest map (`bench_fixtures::hit_case::spread_stage`).
//! Run it with `cargo bench --bench hit_tick` (needs what `worst_tick` needs).

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::bench_fixtures::{self, Case};
use stella_rain_core::engine::Engine;

#[library_benchmark]
#[bench::hit_tick(bench_fixtures::hit_tick())]
// The same crowd over the largest map (core#146): the grid spans the map, and its build reads it.
#[bench::hit_tick_spread(bench_fixtures::hit_tick_spread())]
fn hit_tick(case: Case) -> Engine {
    let Case { mut engine, input } = case;
    engine.step(input);
    engine
}

library_benchmark_group!(
    name = hit_group;
    benchmarks = hit_tick
);

main!(library_benchmark_groups = hit_group);
