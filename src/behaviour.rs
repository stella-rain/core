//! Tier 1 of the behaviour model (ADR-036): agents with ordered rule lists, and the conditions,
//! selectors, actions and skills they are built from. The main character's skills (ADR-032)
//! reuse the same selectors and skills.
//!
//! Every variant here is a content ID (its serialised name) and will be recorded in the
//! registry with the `sim_version` that introduced it (ADR-026). Names and fields are
//! placeholders until the freeze (ADR-035).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::AttackRef;
use crate::fixed::{Fx, Point};
use crate::id::ContentId;

/// A companion, a regular enemy or (inside a boss phase) the boss: a movement and a rule list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    /// Asset ID of the sprite (ADR-007).
    pub base: ContentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<ContentId>,
    pub hp: u32,
    /// Radius of the hurt circle.
    pub radius: Fx,
    pub movement: Movement,
    /// Evaluated in order each tick; the first rule that can fire, fires (ADR-036).
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Movement {
    /// Circles the main character.
    Orbit { radius: Fx },
    /// Trails the main character at a distance.
    Follow { distance: Fx },
    /// Stays at a point.
    Hold { at: Point },
    /// Moves at a constant velocity per tick.
    Straight { vx: Fx, vy: Fx },
}

/// `when <condition>` → `target <selector>` → `do <action>` (ADR-036).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub when: Condition,
    pub target: Selector,
    #[serde(rename = "do")]
    pub action: Action,
    /// Ticks after firing before the rule is ready again.
    #[serde(default)]
    pub cooldown_ticks: u32,
    /// Times the rule may fire; absent means unlimited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charges: Option<u16>,
    /// Fires at most once (the same as one charge, kept for editor clarity).
    #[serde(default)]
    pub once: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    Always,
    HpBelow {
        who: Selector,
        pct: u8,
    },
    HasStatus {
        who: Selector,
        status: ContentId,
    },
    AggroAbove {
        who: Selector,
        value: u32,
    },
    PartBroken {
        part: ContentId,
    },
    /// Ticks since the agent spawned, or since the boss phase began.
    TimeAbove {
        ticks: u32,
    },
    /// The main character fires on this tick (ADR-038).
    PlayerFiring,
}

/// Who a rule, condition or skill acts on. Ties break by squared fixed-point distance, then
/// entity ID (ADR-036).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Selector {
    #[serde(rename = "self")]
    Myself,
    Player,
    NearestEnemy,
    BossFirst,
    NearestAlly,
    LowestHpAlly,
    HighestAggroAlly,
    /// A boss part, by the part ID the stage gives it.
    Part(ContentId),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Cast { skill: Skill },
    Attack { attack: AttackRef },
    Summon { agent: Box<Agent>, count: u8 },
    MoveTo { to: Point, ticks: u32 },
    Telegraph { ticks: u32 },
}

/// A skill block (ADR-008, ADR-032). Percentages are whole percent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Skill {
    Shot { pattern: AttackRef, damage: u32 },
    AtkUp { value_pct: u16, duration_ticks: u32 },
    Vulnerable { value_pct: u16, duration_ticks: u32 },
    Slow { value_pct: u16, duration_ticks: u32 },
    Heal { amount: u32 },
}
