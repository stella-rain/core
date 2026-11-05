//! The instruction count of a rollback (ADR-SIM-6, ADR-SIM-7): the saved engine of the
//! three-player dense minute is cloned and 8 ticks are played on the clone, the first being the
//! minute's heaviest (`bench_fixtures::party_case`). Run it with `cargo bench --bench rollback`;
//! its setup plays 10,441 ticks, so it takes a minute under valgrind.

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::bench_fixtures::party_case;
use stella_rain_core::engine::Engine;

mod support;

#[library_benchmark]
#[bench::rollback(args = (party_case::saved()), teardown = support::forget_measured)]
fn rollback(saved: Engine) -> (Engine, Engine) {
    party_case::rollback(saved)
}

library_benchmark_group!(
    name = rollback_group;
    benchmarks = rollback
);

main!(library_benchmark_groups = rollback_group);
