//! Stella Rain's deterministic simulation (ADR-004).
//!
//! The same stage, seed and inputs give the same state hash every tick on every platform
//! (ADR-019). Simulation state uses fixed-width integers only: no floats, no `usize`.

#![forbid(unsafe_code)]

pub mod attack;
pub mod base64;
pub mod behaviour;
#[cfg(feature = "bench")]
#[doc(hidden)]
pub mod bench_fixtures;
pub mod constellations;
pub mod content_hash;
pub mod difficulties;
pub mod engine;
pub mod event;
pub mod expertise;
pub mod fixed;
pub mod hash;
pub mod id;
pub(crate) mod inline;
pub mod input;
pub mod kinds;
pub mod match_ups;
pub mod pack;
pub mod parsed_stage_hash;
pub mod perk_sets;
pub mod presets;
#[cfg(feature = "record")]
pub mod recording;
pub mod registry;
pub mod replay;
pub mod rewards;
pub mod rng;
pub(crate) mod rules_v0;
pub mod run_content;
pub mod run_replay;
pub mod run_setup;
pub mod run_stats;
pub mod rune;
pub mod runes;
pub mod schema;
pub mod share;
pub mod snapshot;
pub mod spike;
pub mod stage;
pub mod stage_skill;
pub mod stardust;
pub mod starting_unlocks;
pub mod time_limit;
pub mod trig;
pub mod validate;
pub mod version;
pub mod weapon;
pub mod weapons;
