//! The instruction count of the worst friendly-bullet collision tick within the caps of ADR-020
//! (`benches/collide_case`): 300 enemies and about 3,900 friendly bullets that hit none of them.
//! Run it with `cargo bench --bench collide_tick` (needs what `worst_tick` needs).

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::engine::Engine;

mod collide_case;

#[library_benchmark]
#[bench::collide_tick(collide_case::warmed_up())]
fn collide_tick(mut engine: Engine) -> Engine {
    engine.step(collide_case::input(collide_case::WARMUP_TICKS + 1));
    engine
}

library_benchmark_group!(
    name = collide_group;
    benchmarks = collide_tick
);

main!(library_benchmark_groups = collide_group);
