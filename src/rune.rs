//! Runes (ADR-099): unique effects on the Main weapon for the rest of a run. Each is one of a
//! closed list built into `core`, of two kinds; the Main holds each at most once, never
//! upgraded or stacked, and never carries one to another run. A run gets them from the random
//! rune, a star reward that draws one the Main does not hold when it is picked (`rewards`).
//! A list for the prototype; adding to it is a schema change in version 0.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The most runes the Main weapon holds: each rune once (ADR-099 point 7).
pub const MAX_RUNES: usize = Rune::ALL.len();

/// A rune, one of the closed list.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Rune {
    /// On-hit: an explosion where a combo step hits. It does nothing yet.
    Explosion,
}

/// What sets a rune off (ADR-099 point 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuneKind {
    /// A combo step's hit fires the rune's own attack, then the rune waits out its cooldown.
    OnHit,
    /// A hit puts a status on the enemy that deals damage every tick interval.
    DamageOverTime,
}

impl Rune {
    /// Every rune, in the order the random rune draws among them.
    pub const ALL: [Rune; 1] = [Rune::Explosion];

    /// The rune's place in `ALL`, from 0: what the state hash writes.
    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn kind(self) -> RuneKind {
        match self {
            Rune::Explosion => RuneKind::OnHit,
        }
    }
}
