//! The instruction count of a `time_limit` run's heaviest tick (core#212 step d,
//! `benches/time_limit_case`): the mode's horde at the cap of 500 enemies alive, 4,000 bullets
//! in the air, a boss alive and the main character attacking. Run it with
//! `cargo bench --bench time_limit_tick` (needs what `worst_tick` needs).

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::engine::Engine;

mod time_limit_case;

#[library_benchmark]
#[bench::time_limit_tick(time_limit_case::warmed_up())]
fn time_limit_tick(mut engine: Engine) -> Engine {
    engine.step(time_limit_case::input(time_limit_case::WARMUP_TICKS + 1));
    engine
}

library_benchmark_group!(
    name = time_limit_group;
    benchmarks = time_limit_tick
);

main!(library_benchmark_groups = time_limit_group);
