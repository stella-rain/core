//! Stella Rain's deterministic simulation (ADR-004).
//!
//! The same stage, seed and inputs give the same state hash every tick on every platform
//! (ADR-019). Simulation state uses fixed-width integers only: no floats, no `usize`.

pub mod attack;
pub mod base64;
pub mod behaviour;
pub mod content_hash;
pub mod engine;
pub mod event;
pub mod fixed;
pub mod hash;
pub mod id;
pub mod input;
pub mod parsed_stage_hash;
pub mod presets;
#[cfg(feature = "record")]
pub mod recording;
pub mod registry;
pub mod replay;
pub mod rng;
pub(crate) mod rules_v0;
pub mod schema;
pub mod share;
pub mod snapshot;
pub mod spike;
pub mod stage;
pub mod trig;
pub mod validate;
pub mod version;
