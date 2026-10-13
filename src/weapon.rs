//! Weapons (ADR-050, core#83): the main character carries a Main and a Sub weapon, and each
//! attacks on its own, on its own cooldown, at what its targeting picks. A weapon is a preset by
//! ID or an inline definition, as an attack is. A preset is a whole weapon: one ID with a Main
//! role and a Sub role, which differ (the game design's weapon table); in `player.main` a
//! preset runs its Main role, in `player.sub` its Sub role. The run's setup will take IDs only
//! (ADR-051).
//!
//! Field names are placeholders until the freeze (ADR-035).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::AttackRef;
use crate::behaviour::Selector;
use crate::id::ContentId;

/// A weapon preset by ID, or an inline weapon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum WeaponRef {
    Preset { id: ContentId },
    Inline(Box<Weapon>),
}

/// One role of a weapon: the attack it makes on its own, at what, and how often.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Weapon {
    /// Never slows the main character: every phase keeps `move_pct` at 100 (the validator).
    pub attack: AttackRef,
    /// The nearest enemy when absent (ADR-050).
    #[serde(default)]
    pub targeting: Targeting,
    /// Ticks from one attack's start to the next one's. If the attack lasts longer, the next
    /// one waits for it to end.
    pub cooldown_ticks: u32,
}

/// What a weapon attacks: along the main character's facing, or at an enemy one of ADR-036's
/// selectors picks. Only the selectors that pick an enemy are here, so a weapon cannot aim at
/// an ally. With a selector and nothing to pick, the weapon does not attack and spends no
/// cooldown.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Targeting {
    /// `aimed` directions point along the facing at the attack's start.
    Facing,
    #[default]
    NearestEnemy,
    /// The boss while it is alive, else the nearest enemy.
    BossFirst,
    /// A boss part, by the part ID the stage gives it, until it breaks.
    Part(ContentId),
}

impl Targeting {
    /// The selector this targeting picks with; `None` for `facing`.
    pub fn selector(&self) -> Option<Selector> {
        match self {
            Targeting::Facing => None,
            Targeting::NearestEnemy => Some(Selector::NearestEnemy),
            Targeting::BossFirst => Some(Selector::BossFirst),
            Targeting::Part(id) => Some(Selector::Part(id.clone())),
        }
    }
}

/// Which of the main character's two weapons a reference is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Main,
    Sub,
}

/// A weapon preset's definition: both of its roles.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeaponPreset {
    pub main: Weapon,
    pub sub: Weapon,
}

impl WeaponPreset {
    pub fn role(&self, role: Role) -> &Weapon {
        match role {
            Role::Main => &self.main,
            Role::Sub => &self.sub,
        }
    }
}
