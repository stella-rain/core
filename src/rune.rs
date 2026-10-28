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
    /// On-hit: an explosion around the point a combo step hits.
    Explosion,
    /// On-hit: a strike called down on the point a combo step hits, after a wind-up.
    Thunder,
    /// On-hit: thrown from the main character to the point a combo step hits, and back.
    Boomerang,
    /// On-hit: missiles launched from the main character that steer to aim assist's targets,
    /// each hitting once.
    HomingMissiles,
    /// Damage over time: a combo step's hit puts the `bleed` status on the enemy hit, or a stack
    /// more of it.
    Bleed,
    /// Damage over time: a combo step's hit puts the `poison` status, Dendro, on the enemy hit,
    /// or a stack more of it.
    Poison,
    /// Damage over time: a combo step's hit puts the `burn` status, Pyro, on the enemy hit, or
    /// a stack more of it.
    Burn,
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
    pub const ALL: [Rune; 7] = [
        Rune::Explosion,
        Rune::Thunder,
        Rune::Boomerang,
        Rune::HomingMissiles,
        Rune::Bleed,
        Rune::Poison,
        Rune::Burn,
    ];

    /// The rune's place in `ALL`, from 0.
    pub fn index(self) -> u8 {
        self as u8
    }

    /// The rune's content ID: its name in snake case, as a setup's rune pool and starting rune
    /// name it (ADR-107).
    pub fn id(self) -> &'static str {
        match self {
            Rune::Explosion => "explosion",
            Rune::Thunder => "thunder",
            Rune::Boomerang => "boomerang",
            Rune::HomingMissiles => "homing_missiles",
            Rune::Bleed => "bleed",
            Rune::Poison => "poison",
            Rune::Burn => "burn",
        }
    }

    /// The rune whose content ID is `id`.
    pub fn from_id(id: &str) -> Option<Rune> {
        Rune::ALL.into_iter().find(|r| r.id() == id)
    }

    /// The damage-over-time rune whose status is `kind`; `None` for every other status.
    pub(crate) fn of_dot(kind: crate::engine::StatusKind) -> Option<Rune> {
        use crate::engine::StatusKind;
        match kind {
            StatusKind::Bleed => Some(Rune::Bleed),
            StatusKind::Poison => Some(Rune::Poison),
            StatusKind::Burn => Some(Rune::Burn),
            StatusKind::AtkUp | StatusKind::Vulnerable | StatusKind::Slow | StatusKind::Stun => {
                None
            }
        }
    }

    pub fn kind(self) -> RuneKind {
        match self {
            Rune::Explosion | Rune::Thunder | Rune::Boomerang | Rune::HomingMissiles => {
                RuneKind::OnHit
            }
            Rune::Bleed | Rune::Poison | Rune::Burn => RuneKind::DamageOverTime,
        }
    }
}
