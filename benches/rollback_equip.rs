//! The instruction count of an equipment rollback (ADR-SIM-6, ADR-SIM-7): the saved engine of a
//! three-player equipment run is cloned and 8 ticks are played on the clone, each player
//! equipping a star and extracting another (`bench_fixtures::equip_case`). Run it with
//! `cargo bench --bench rollback_equip`; its setup plays about 10,400 ticks, so it takes a
//! minute under valgrind. It has no budget yet (`UNBUDGETED` in `tests/bench_teardown.rs`).

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::bench_fixtures::equip_case;
use stella_rain_core::engine::Engine;

mod support;

#[library_benchmark]
#[bench::rollback_equip(args = (equip_case::saved()), teardown = support::forget_measured)]
fn rollback_equip(saved: Engine) -> (Engine, Engine) {
    equip_case::rollback_equip(saved)
}

library_benchmark_group!(
    name = rollback_equip_group;
    benchmarks = rollback_equip
);

main!(library_benchmark_groups = rollback_equip_group);
