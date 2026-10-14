//! The upgrades a level-up offers (ADR-051 §2). Placeholders until game design names the skills
//! and items of a run: each is a step on a stat the main character already has. What a stack
//! does is a rule of the `sim_version` (`rules_v0`'s `levels`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Upgrade {
    /// One more hit point at most, and one healed.
    MaxHp,
    /// More damage from every attack of the main character.
    Attack,
    /// Shorter cooldowns for both weapons' automatic attacks.
    Haste,
    /// A shorter evade cooldown.
    Evade,
    /// A higher movement speed.
    Speed,
}

impl Upgrade {
    /// Every upgrade, in the order an offer draws from.
    pub const ALL: [Upgrade; 5] = [
        Upgrade::MaxHp,
        Upgrade::Attack,
        Upgrade::Haste,
        Upgrade::Evade,
        Upgrade::Speed,
    ];
}

/// Options in an offer, or fewer when fewer upgrades are left (a tuning value).
pub const OFFER_SIZE: u32 = 3;
/// Stacks an upgrade can reach; at this many it is no longer offered (a tuning value).
pub const MAX_STACKS: u8 = 5;
