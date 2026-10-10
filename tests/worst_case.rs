//! The worst-case tick (ADR-020, ADR-021): the stage of `benches/worst_case/` holds every dynamic
//! cap at once, on the tick the benchmark counts and on the ticks after it. A benchmark of a
//! tick that does not would pass without measuring the worst case.

#[path = "../benches/worst_case/mod.rs"]
mod worst_case;

use stella_rain_core::engine::{Load, Outcome};

#[test]
fn the_measured_tick_reaches_every_dynamic_cap() {
    let mut engine = worst_case::warmed_up();
    engine.step(worst_case::input(worst_case::WARMUP_TICKS + 1));
    assert_eq!(engine.outcome(), Outcome::Running, "the run ended");
    assert_eq!(engine.load(), Load::CAPS);
}

#[test]
fn the_caps_are_held_tick_after_tick() {
    let mut engine = worst_case::warmed_up();
    for tick in worst_case::WARMUP_TICKS + 1..=worst_case::WARMUP_TICKS + 60 {
        engine.step(worst_case::input(tick));
        assert_eq!(
            engine.outcome(),
            Outcome::Running,
            "the run ended on tick {tick}"
        );
        assert_eq!(engine.load(), Load::CAPS, "on tick {tick}");
    }
}
