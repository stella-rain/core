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

use crate::engine::Engine;
use crate::input::Input;

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
/// the tests' helpers are not reachable from `src/`. The tests below pin what they build.
mod common;

#[cfg(test)]
mod tests {
    use super::*;

    /// The state hash before and after the measured tick, as the benches' own case modules
    /// (`benches/*_case/mod.rs`, built on `tests/common/`) produced them when the fixtures were
    /// written: the fixtures must build the same engine, or the instruction counts move.
    fn pinned(case: Case, before: u64, after: u64) {
        let Case { mut engine, input } = case;
        assert_eq!(engine.state_hash(), before, "the warmed-up engine");
        engine.step(input);
        assert_eq!(engine.state_hash(), after, "after the measured tick");
    }

    #[test]
    fn the_worst_tick_is_the_benches_case() {
        pinned(worst_tick(), 0x8da23ca6e42b539c, 0x4951b14566391b5d);
    }

    #[test]
    fn the_hit_tick_is_the_benches_case() {
        pinned(hit_tick(), 0xe85bd3364b22c9e5, 0x95527ebc68a22c23);
    }

    #[test]
    fn the_spread_hit_tick_is_the_benches_case() {
        pinned(hit_tick_spread(), 0x3d264a19f06cff25, 0xf143b972e536a94b);
    }

    #[test]
    fn the_time_limit_tick_is_the_benches_case() {
        pinned(time_limit_tick(), 0xb8352ca01a24a44c, 0x3b62752b8bc228ce);
    }
}
