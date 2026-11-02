//! Shared by the bench targets; not a target itself (Cargo discovers only `benches/*.rs`).

use stella_rain_core::bench_fixtures::Case;
use stella_rain_core::bench_fixtures::worst_minute_case::Run;
use stella_rain_core::engine::Engine;

/// The teardown of every bench: the result goes through `black_box` and is forgotten.
///
/// In iai-callgrind 0.16 the teardown runs inside the counted wrapper, so a drop here would count
/// `drop_glue` and the allocator's consolidation, which depend on the heap layout and are not the
/// operation a bench names. The value is leaked instead; each bench is its own process, so the
/// leak ends with it.
pub fn forget_measured<T>(value: T) {
    std::mem::forget(std::hint::black_box(value));
}

/// The measured body of the one-tick benches: one `Engine::step` and the engine handed back, so
/// the bench function drops nothing inside the counted region. `tests/bench_region.rs` counts
/// this same function, so the benches and the allocation guard run one definition.
#[allow(dead_code)] // each bench target uses only some of the helpers
#[inline]
pub fn tick(case: Case) -> Engine {
    let Case { mut engine, input } = case;
    engine.step(&[input]);
    engine
}

/// The measured body of the minute benches: `ticks` ticks of the run, the run handed back.
#[allow(dead_code)] // each bench target uses only some of the helpers
#[inline]
pub fn minute(mut run: Run, ticks: u32) -> Run {
    run.play(ticks);
    run
}
