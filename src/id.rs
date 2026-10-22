//! Identifiers shared by the stage, event and snapshot types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A content ID from the registry (ADR-026): asset, palette, part, preset, status or pattern.
/// Lowercase `snake_case`, at most 32 characters; the validator checks the format and that the
/// ID exists.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ContentId(pub String);

impl From<&str> for ContentId {
    fn from(s: &str) -> Self {
        ContentId(s.to_owned())
    }
}

/// An entity in a run, numbered in spawn order. Agents are evaluated in this order (ADR-036).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct EntityId(pub u32);
