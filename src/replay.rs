//! The replay header (ADR-016): the small JSON part of a `.replay` file. The packed per-tick
//! inputs that follow it come with the replay loader.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplayHeader {
    /// Must equal the stage's (ADR-020).
    pub schema_version: u32,
    /// Must equal the stage's (ADR-020).
    pub sim_version: u32,
    /// Must equal the stage's.
    pub seed: u32,
    /// SHA-256 of the exact stage file bytes, lowercase hex: a changed stage invalidates the
    /// replay.
    pub stage_sha256: String,
    /// Ticks recorded; at most the stage's length.
    pub ticks: u32,
    /// The FNV-1a 64 state hash after the last tick (ADR-019), 16 lowercase hex digits. Hex,
    /// because JSON readers that use doubles cannot hold every 64-bit integer.
    pub final_hash: String,
}
