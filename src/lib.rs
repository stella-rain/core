//! Stella Rain's deterministic simulation (ADR-004).
//!
//! The same stage, seed and inputs give the same state hash every tick on every platform
//! (ADR-019). Simulation state uses fixed-width integers only: no floats, no `usize`.

pub mod attack;
pub mod behaviour;
pub mod content_hash;
pub mod event;
pub mod fixed;
pub mod hash;
pub mod id;
pub mod input;
pub mod replay;
pub mod rng;
pub mod schema;
pub mod snapshot;
pub mod spike;
pub mod stage;
pub mod version;
