//! The cases the tick benches count (`benches/`, ADR-SIM-6): the worst-case tick, the collision
//! tick (crowd in a band and spread over the largest map) and the heaviest tick of a `time_limit`
//! run. The benches build their engines here, through the crate, so that a change to the
//! simulation's API edits `src/` alone and the bench job can still compare it with its base
//! (`rust.yml` refuses a PR that changes both the code and the bench inputs).
//!
//! Behind the `bench` feature; not part of the library's API.

pub mod hit_case;
pub mod time_limit_case;
pub mod worst_case;
pub mod worst_minute_case;

use crate::engine::Engine;
use crate::input::Input;
use crate::run_content::RunContent;

/// The enemy cap of the bundled content (DIR1, 260): the crowd of the cases that fill it. The
/// pack's rules give it, so a change of the crowd target or the headroom moves the cases with it.
pub fn enemy_cap() -> u16 {
    let rules = RunContent::bundled().rules();
    u16::try_from(rules.enemy_cap_for(rules.crowd_target())).expect("a cap within the ceiling")
}

/// A case ready to measure: an engine warmed up to the tick before the measured one, and the
/// input of the measured tick. The bench runs `engine.step(input)` and counts it.
pub struct Case {
    pub engine: Engine,
    pub input: Input,
}

/// The worst-case tick (`benches/worst_tick.rs`).
pub fn worst_tick() -> Case {
    Case {
        engine: worst_case::warmed_up(),
        input: worst_case::input(worst_case::WARMUP_TICKS + 1),
    }
}

/// The melee hit boxes and zones against the crowd in a clump (`benches/hit_tick.rs`).
pub fn hit_tick() -> Case {
    Case {
        engine: hit_case::warmed_up(),
        input: hit_case::input(hit_case::WARMUP_TICKS + 1),
    }
}

/// The same tick with the crowd spread over the largest map (core#146).
pub fn hit_tick_spread() -> Case {
    Case {
        engine: hit_case::spread_warmed_up(),
        input: hit_case::input(hit_case::WARMUP_TICKS + 1),
    }
}

/// The heaviest tick of a `time_limit` run (`benches/time_limit_tick.rs`).
pub fn time_limit_tick() -> Case {
    Case {
        engine: time_limit_case::warmed_up(),
        input: time_limit_case::input(time_limit_case::WARMUP_TICKS + 1),
    }
}

/// The builders of the stages: the subset of `tests/common/` that the cases use, copied because
/// the tests' helpers are not reachable from `src/`. `tests/bench_fixture_pins.rs` pins what they build.
mod common;

/// The dense minute of the `time_limit` mode (`benches/worst_minute.rs`).
pub fn worst_minute() -> worst_minute_case::Run {
    worst_minute_case::worst_minute()
}

/// The heaviest tick of that minute (`benches/worst_minute_peak.rs`).
pub fn worst_minute_peak() -> worst_minute_case::Run {
    worst_minute_case::worst_minute_peak()
}
