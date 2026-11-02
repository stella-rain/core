//! Stella Rain's deterministic simulation (ADR-004).
//!
//! The same stage, seed and inputs give the same state hash every tick on every platform
//! (ADR-019). Simulation state uses fixed-width integers only: no floats, no `usize`.

#![forbid(unsafe_code)]

pub mod attack;
pub mod attack_presets;
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
pub mod ledger;
pub mod limits;
pub mod match_ups;
pub mod pack;
pub mod packs;
pub mod parsed_stage_hash;
pub mod perk_sets;
pub mod presets;
pub mod progression;
#[cfg(feature = "record")]
pub mod recording;
pub mod registry;
pub mod replay;
pub mod rewards;
pub mod rng;
pub mod rules;
pub mod run_content;
pub mod run_replay;
pub mod run_setup;
pub mod run_stats;
pub mod rune;
pub mod runes;
#[cfg_attr(not(test), allow(dead_code))]
pub mod savepoint;
pub mod schema;
pub mod share;
pub(crate) mod sim;
pub(crate) mod sim_v1;
pub mod snapshot;
pub mod stage;
pub mod stage_skill;
mod step_log;
pub mod stardust;
pub mod starting_unlocks;
pub mod text;
pub mod time_limit;
pub mod trig;
pub mod validate;
pub mod version;
pub mod weapon;
pub mod weapons;

/// The core minor this build is, as a packs.json channel key: "major.minor" of the package
/// version, read at compile time. An app picks `packs::Packs::channel(CORE_MINOR)`, so it
/// ignores a channel made for another minor.
pub const CORE_MINOR: &str = concat!(
    env!("CARGO_PKG_VERSION_MAJOR"),
    ".",
    env!("CARGO_PKG_VERSION_MINOR")
);
