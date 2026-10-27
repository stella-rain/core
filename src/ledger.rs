//! Run ledger (ADR-BRDG-6): counts kills, bestiary bits, picks and skill uses
//! inside the simulation from domain events and state observations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::id::ContentId;
use crate::rewards::Rarity;

/// A star in the run's build (core#NS3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StarPick {
    pub rarity: Rarity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A run's ledger of counts and bestiary encounters, verified with the replay (ADR-BRDG-6).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunLedger {
    /// Kill statistics: `kills.<weapon>.<class>` and `slain.<kind_id>`.
    pub kills: Vec<(String, u32)>,
    /// Bestiary bits per enemy kind ID: bit 0 encountered, bits 1..=10 for `DamageType::ALL`.
    pub bestiary: Vec<(ContentId, u16)>,
    /// The stars the main character took, in pick order: the run's build.
    pub picks: Vec<StarPick>,
    /// Stage skill uses per stage skill ID.
    pub skill_uses: Vec<(ContentId, u32)>,
}

impl RunLedger {
    pub fn stat(&self, id: &str) -> u32 {
        if let Some((_, count)) = self.kills.iter().find(|(k, _)| k == id) {
            return *count;
        }
        if let Some(skill) = id.strip_prefix("skill_uses.") {
            if let Some((_, count)) = self.skill_uses.iter().find(|(k, _)| k.0 == skill) {
                return *count;
            }
        }
        if let Some(rarity) = id.strip_prefix("stars_picked.") {
            return self
                .picks
                .iter()
                .filter(|p| rarity_str(p.rarity) == rarity)
                .count() as u32;
        }
        0
    }

    pub fn bits(&self, kind: &str) -> Option<u16> {
        self.bestiary
            .iter()
            .find(|(k, _)| k.0 == kind)
            .map(|(_, b)| *b)
    }
}

pub(crate) fn rarity_str(rarity: Rarity) -> &'static str {
    match rarity {
        Rarity::Common => "common",
        Rarity::Uncommon => "uncommon",
        Rarity::Rare => "rare",
        Rarity::Epic => "epic",
        Rarity::Legendary => "legendary",
        Rarity::Supernova => "supernova",
    }
}
