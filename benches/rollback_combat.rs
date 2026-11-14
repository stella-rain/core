//! The instruction count of a rollback over the worst combat tick's state (ADR-SIM-6,
//! ADR-SIM-7): the `warmed_up` engine of `worst_tick_combat` (64 queued step repeats, staggered
//! and controlled foes) is cloned and 8 ticks are played on the clone
//! (`bench_fixtures::combat_case`). Run it with `cargo bench --bench rollback_combat`. It is
//! budgeted like the other targets: the `[rollback_combat]` key of `benches/budgets.toml`
//! (ADR-SIM-6), which `tests/bench_teardown.rs` requires of every bench.

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::bench_fixtures::combat_case;
use stella_rain_core::engine::Engine;

mod support;

#[library_benchmark]
#[bench::rollback_combat(args = (combat_case::warmed_up()), teardown = support::forget_measured)]
fn rollback_combat(saved: Engine) -> (Engine, Engine) {
    combat_case::rollback(saved)
}

library_benchmark_group!(
    name = rollback_combat_group;
    benchmarks = rollback_combat
);

main!(library_benchmark_groups = rollback_combat_group);
