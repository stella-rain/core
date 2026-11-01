//! A rollback measurement harness (throwaway branch, never merged). Stub: the first commit of
//! this branch is the failing test, `tests/spike_rollback.rs`.

use stella_rain_core::engine::Engine;

/// The state hashes of one rollback cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cycle {
    pub at_clone: u64,
    pub straight: u64,
    pub restored: u64,
    pub resimulated: u64,
}

/// Clones `engine`, steps it `w` ticks with `step`, restores the clone and steps the same `w`
/// ticks again.
pub fn rollback_cycle(engine: &mut Engine, _step: &mut dyn FnMut(&mut Engine), _w: u32) -> Cycle {
    let at_clone = engine.state_hash();
    Cycle {
        at_clone,
        straight: at_clone,
        restored: at_clone,
        resimulated: at_clone,
    }
}

/// A run of the bundled `time_limit` mode `ticks` in, saved as a savepoint and restored: the
/// state hash before saving and of the restored engine, and the savepoint's size in bytes.
pub fn savepoint_round_trip(_ticks: u32) -> (u64, u64, usize) {
    (0, 1, 0)
}
