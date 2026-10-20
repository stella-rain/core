//! Tier 1 of the behaviour model (ADR-036): agents with ordered rule lists, and the conditions,
//! selectors, actions and skills they are built from.
//!
//! Every variant here is a content ID (its serialised name) and will be recorded in the
//! registry with the `sim_version` that introduced it (ADR-026). Names and fields are
//! placeholders until the freeze (ADR-035).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::{AttackRef, Profile};
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
    /// Reduces every hit it takes to `K / (K + defence)` of it (ADR-056); 0 when absent.
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub defence: u32,
    /// What it is weak and strong against (ADR-100); without one, every type is neutral against
    /// it. An enemy's or an elite's only: a companion has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
    pub movement: Movement,
    /// Evaluated in order each tick; the first rule that can fire, fires (ADR-036).
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// An enemy that holds where it spawned and acts only while the main character is within
    /// its range, until a boss calls it; its star is a tier higher (ADR-068). Not a companion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elite: Option<Elite>,
}

/// What makes an enemy an elite.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Elite {
    /// How near the main character's centre must come to its centre for its rules to fire.
    pub range: Fx,
    /// What the boss's call does to it (ADR-090); `join` when absent.
    #[serde(default, skip_serializing_if = "BossCall::is_join")]
    pub boss_call: BossCall,
}

/// What a boss's call does to an elite.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BossCall {
    /// It is called: it leaves its place and fights beside the boss.
    #[default]
    Join,
    /// It is not called: it leaves the map instead, unkilled and with no reward.
    Leave,
    /// It is not called and does not leave: it stays where it is (the shrine's, ADR-090).
    Stay,
}

impl BossCall {
    fn is_join(&self) -> bool {
        *self == Self::Join
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Movement {
    /// Circles the main character. A companion is set on its circle; an enemy moves to its
    /// place on it, at most its orbit step plus 4 pixels a tick (ADR-058).
    Orbit { radius: Fx },
    /// Trails the main character at a distance.
    Follow { distance: Fx },
    /// Keeps to a point. A companion is set on it; an enemy walks to it from where it is, at
    /// most 4 pixels a tick, and walks back when it is pushed off it (ADR-058).
    Hold { at: Point },
    /// Moves at a constant velocity per tick.
    Straight { vx: Fx, vy: Fx },
    /// An elite's (it needs the `elite` field): holds its place until it is disturbed, by
    /// damage or by the main character coming within `disturb_radius` of its centre, then runs
    /// directly away from the main character at `speed` a tick, kept on the map, and leaves it
    /// unkilled `leave_after_ticks` ticks after the disturbance (ADR-090).
    FleeWhenDisturbed {
        disturb_radius: Fx,
        speed: Fx,
        leave_after_ticks: u32,
    },
    /// An elite's (it needs the `elite` field, with `boss_call` `stay`): an object that cannot be
    /// damaged and does nothing until the main character's body touches it. It then activates
    /// once: its guardians appear on a ring around it and it gives its kill reward when they are
    /// all dead within `fight_ticks` ticks of the touch, else it leaves unkilled (ADR-090).
    Shrine {
        fight_ticks: u32,
        guardians: Guardians,
    },
}

/// The guardians of a shrine: `count` ordinary enemies placed on a ring of `radius` around it,
/// evenly spaced from the east, kept on the map. The enemy cap still applies to them: one it
/// refuses is not spawned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Guardians {
    /// Not an elite and not a shrine; it does not summon.
    pub enemy: Box<Agent>,
    /// 1 to `MAX_SHRINE_GUARDIANS`.
    pub count: u8,
    /// How far from the shrine's centre they stand.
    pub radius: Fx,
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

/// A skill block (ADR-008, ADR-036). Percentages are whole percent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Skill {
    Shot { pattern: AttackRef, damage: u32 },
    AtkUp { value_pct: u16, duration_ticks: u32 },
    Vulnerable { value_pct: u16, duration_ticks: u32 },
    Slow { value_pct: u16, duration_ticks: u32 },
    Heal { amount: u32 },
}

/// For `skip_serializing_if`: a 0 that is the default is left out of the file.
pub(crate) fn is_zero(v: &u32) -> bool {
    *v == 0
}
