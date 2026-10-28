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
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_AGENT_HP))]
    pub hp: u32,
    /// Radius of the hurt circle.
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_RADIUS))]
    pub radius: Fx,
    /// Reduces every hit it takes to `K / (K + defence)` of it (ADR-056); 0 when absent.
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    #[schemars(range(min = 0, max = crate::validate::limits::MAX_DEFENCE))]
    pub defence: u32,
    /// What it is weak and strong against (ADR-100); without one, every type is neutral against
    /// it. An enemy's or an elite's only: a companion has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
    pub movement: Movement,
    /// Evaluated in order each tick; the first rule that can fire, fires (ADR-036).
    #[serde(default)]
    #[schemars(length(max = crate::validate::limits::MAX_RULES))]
    pub rules: Vec<Rule>,
    /// An enemy that holds where it spawned and acts only while the main character is within
    /// its range, until a boss calls it; its star is a tier higher (ADR-068). Not a companion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elite: Option<Elite>,
}

/// What makes an enemy an elite.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Elite {
    /// How near the main character's centre must come to its centre for its rules to fire.
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_LEN))]
    pub range: Fx,
    /// What the boss's call does to it (ADR-090); `join` when absent.
    #[serde(default, skip_serializing_if = "BossCall::is_join")]
    pub boss_call: BossCall,
    /// A pack leader's aura (ADR-090): while it is alive, the non-elite enemies near it deal more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aura: Option<Aura>,
    /// A pack leader's escort (ADR-090): ordinary enemies that spawn with it on a ring around it.
    /// Kept here, with the aura, because both are about what makes this enemy an elite of its
    /// kind, as `boss_call` is; the ring is the shrine's guardians' (`Guardians`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escort: Option<Guardians>,
    /// A mimic's disguise (ADR-090): until the main character comes near, it is drawn as another
    /// sprite and nothing can touch it. Kept here, with the aura and the escort, because it is
    /// about what makes this enemy an elite of its kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mimic: Option<Mimic>,
}

/// What a mimic is (ADR-090). It starts disguised: it does not move or attack, nothing damages
/// it, aim assist and the selectors pass it over, and it is drawn as `disguise`. The first tick
/// the main character's centre is within `reveal_radius` of its centre (the edge counts) it
/// reveals, for good: from that tick's collisions on it takes damage, and from the next tick it
/// acts by its movement and rules like a grade elite. The boss's call skips it while it is
/// disguised and calls it once it is revealed. Needs `boss_call` `join`, and no aura, escort,
/// shrine or flee movement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Mimic {
    /// Asset ID of the sprite it is drawn as while disguised (ADR-007).
    pub disguise: ContentId,
    /// 1 to `MAX_LEN`.
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_LEN))]
    pub reveal_radius: Fx,
}

/// What a pack leader does for the enemies near it (ADR-090). Every non-elite enemy whose centre
/// is within `radius` of the leader's centre (the edge counts) deals its damage times
/// `100 + damage_percent` over 100, rounded down; when several leaders reach it, the highest
/// `damage_percent` counts. It ends with the leader's death.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Aura {
    /// 1 to `MAX_LEN`.
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_LEN))]
    pub radius: Fx,
    /// 1 to `MAX_AURA_PERCENT`.
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_AURA_PERCENT))]
    pub damage_percent: u16,
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
    Orbit {
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_LEN))]
        radius: Fx,
    },
    /// Trails the main character at a distance, at most `speed` a tick (a tank's is slower);
    /// absent, 4 pixels a tick as it always was (ADR-058).
    Follow {
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_LEN))]
        distance: Fx,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_LEN))]
        speed: Option<Fx>,
    },
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

/// The guardians of a shrine, or the escort of a pack leader: `count` ordinary enemies placed on
/// a ring of `radius` around it, evenly spaced from the east, kept on the map. The enemy cap
/// still applies to them: one it refuses is not spawned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Guardians {
    /// Not an elite and not a shrine; it does not summon.
    pub enemy: Box<Agent>,
    /// 1 to `MAX_SHRINE_GUARDIANS`.
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_SHRINE_GUARDIANS))]
    pub count: u8,
    /// How far from the shrine's centre they stand.
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_LEN))]
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
    /// An enemy that fires this rule removes itself with it, as an exploder does. It is gone
    /// at the end of the tick, but it was not defeated: it gives no experience, expertise or
    /// Stardust and drops no star or heal pickup, and the burst's attack still fires. An
    /// exploder the main character kills before the rule fires gives its full credit. Ignored
    /// for a companion and the boss.
    #[serde(default, skip_serializing_if = "is_false")]
    pub self_destruct: bool,
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
        #[schemars(range(min = 0, max = crate::validate::limits::MAX_AGGRO))]
        value: u32,
    },
    PartBroken {
        part: ContentId,
    },
    /// Ticks since the agent spawned, or since the boss phase began.
    TimeAbove {
        ticks: u32,
    },
    /// The main character's centre is within `distance` of the agent's centre (the edge counts):
    /// an exploder's burst (ADR-036).
    NearTarget {
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_LEN))]
        distance: Fx,
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
    Cast {
        skill: Skill,
    },
    Attack {
        attack: AttackRef,
        /// A regular enemy's attack at range (core Z2): the run holds one of the
        /// `MAX_RANGED_ATTACKERS` tokens while it winds up and is active, and a start that would
        /// exceed the cap waits for the next tick with its cooldown still ready. Not valid on a
        /// companion, an elite or the boss.
        #[serde(default, skip_serializing_if = "is_false")]
        ranged: bool,
    },
    Summon {
        agent: Box<Agent>,
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_SUMMON_COUNT))]
        count: u8,
    },
    MoveTo {
        to: Point,
        ticks: u32,
    },
    Telegraph {
        ticks: u32,
    },
}

/// A skill block (ADR-008, ADR-036). Percentages are whole percent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Skill {
    Shot {
        pattern: AttackRef,
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_DAMAGE))]
        damage: u32,
    },
    AtkUp {
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_BUFF_PCT))]
        value_pct: u16,
        duration_ticks: u32,
    },
    Vulnerable {
        value_pct: u16,
        duration_ticks: u32,
    },
    Slow {
        value_pct: u16,
        duration_ticks: u32,
    },
    Heal {
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_DAMAGE))]
        amount: u32,
    },
}

/// For `skip_serializing_if`: a `false` that is the default is left out of the file.
pub(crate) fn is_false(v: &bool) -> bool {
    !*v
}

/// For `skip_serializing_if`: a 0 that is the default is left out of the file.
pub(crate) fn is_zero(v: &u32) -> bool {
    *v == 0
}
