//! The instruction count of a rolling-back `Session` (ADR-SIM-5, ADR-SIM-6, ADR-SIM-7): the
//! rollback `Session` of `stella-rain-net` has played the three-player equipment run of the
//! `rollback_equip` case up to tick 10,800, predicted a window of ticks with both remotes silent,
//! and now gets their real inputs; the counted `advance` restores the first mispredicted tick,
//! re-simulates the window (a clone saved per tick) and steps one new tick. It is counted at the
//! default window (4) and the largest (16), and one save of a clone ring is counted alone. The
//! rollback runs on the in-place ring: a save copies into a slot already held and a restore
//! copies into the session's engine. `clone_ring_save` counts a first fill (one clone into an
//! empty ring); `clone_ring_resave` counts a reused slot (a copy into the oldest slot of a ring
//! that has filled).
//! Run it with `cargo bench --bench rollback_session`; each case's setup plays about 10,800 ticks
//! through a `Session`, so it is longer than `rollback_equip`'s (the CI bench job has a 30 minute
//! timeout). It is budgeted like the other targets: the `[rollback_session]` key of
//! `benches/budgets.toml` (ADR-SIM-6), which `tests/bench_teardown.rs` requires of every bench.
//! The eviction drop of a steady-state ring is outside the count, by the teardown rule.

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use stella_rain_core::bench_fixtures::equip_case;
use stella_rain_core::engine::Engine;
use stella_rain_net::save::CloneRing;

mod session_case;
mod support;

#[library_benchmark]
#[bench::rollback_session_w4(args = (session_case::driven(4)), teardown = support::forget_measured)]
#[bench::rollback_session_w16(args = (session_case::driven(16)), teardown = support::forget_measured)]
fn rollback_session(prepared: session_case::Prepared) -> session_case::Prepared {
    session_case::rollback(prepared)
}

#[library_benchmark]
#[bench::clone_ring_save(
    args = ((CloneRing::new(session_case::RING_CAPACITY), equip_case::saved())),
    teardown = support::forget_measured
)]
fn clone_ring_save(case: (CloneRing, Engine)) -> (CloneRing, Engine) {
    session_case::save_once(case)
}

#[library_benchmark]
#[bench::clone_ring_resave(args = (session_case::full_ring()), teardown = support::forget_measured)]
fn clone_ring_resave(case: (CloneRing, Engine)) -> (CloneRing, Engine) {
    session_case::save_once(case)
}

library_benchmark_group!(
    name = rollback_session_group;
    benchmarks = rollback_session, clone_ring_save, clone_ring_resave
);

main!(library_benchmark_groups = rollback_session_group);
