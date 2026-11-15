//! The instruction count of a rollback over the combat state of a party of 3 (ADR-SIM-6,
//! ADR-SIM-7): the `warmed_up_party(3)` engine (64 queued step repeats shared by the three
//! owners, staggered and controlled foes, every slot equipped and scripted like slot 0) is cloned
//! and 8 ticks are played on the clone, the members tapping Main
//! (`bench_fixtures::combat_case`). Run it with `cargo bench --bench rollback_combat_3p`. It is
//! budgeted like the other targets: the `[rollback_combat_3p]` key of `benches/budgets.toml`
//! (ADR-SIM-6), which `tests/bench_teardown.rs` requires of every bench.

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::bench_fixtures::combat_case;
use stella_rain_core::engine::Engine;

mod support;

#[library_benchmark]
#[bench::rollback_combat_3p(args = (combat_case::warmed_up_party(combat_case::PARTY)), teardown = support::forget_measured)]
fn rollback_combat_3p(saved: Engine) -> (Engine, Engine) {
    combat_case::rollback_party(saved, combat_case::PARTY)
}

library_benchmark_group!(
    name = rollback_combat_3p_group;
    benchmarks = rollback_combat_3p
);

main!(library_benchmark_groups = rollback_combat_3p_group);
