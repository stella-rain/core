//! Stella Rain's deterministic simulation (ADR-004).
//!
//! The same stage, seed and inputs give the same state hash every tick on every platform
//! (ADR-019). Simulation state uses fixed-width integers only: no floats, no `usize`.

pub mod fixed;
pub mod hash;
pub mod rng;
pub mod spike;
