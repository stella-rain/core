//! Domain events, backend to frontend over the event bus (ADR-016). They drive the frontend's
//! effects and sounds, the debug overlay and golden tests; the simulation never reads them.
//! In a recording each is `{"type": ..., "data": {...}}`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::id::{ContentId, EntityId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", deny_unknown_fields)]
pub enum DomainEvent {
    /// `source` damaged `target`.
    Hit {
        source: EntityId,
        target: EntityId,
        damage: u32,
    },
    /// The main character was hit.
    PlayerHit {
        hp_left: u32,
    },
    /// `target` regained `amount` hit points.
    Healed {
        target: EntityId,
        amount: u32,
    },
    StatusApplied {
        target: EntityId,
        status: ContentId,
        stacks: u16,
        duration_ticks: u32,
    },
    /// The main character cast a skill from a slot (ADR-032).
    SkillCast {
        slot: u8,
        target: Option<EntityId>,
    },
    /// An agent's rule fired: `rule` is its index in the agent's (or boss phase's) list (ADR-036).
    RuleFired {
        agent: EntityId,
        rule: u8,
        target: Option<EntityId>,
    },
    /// The boss entered phase `phase` (0-based).
    PhaseChanged {
        phase: u8,
    },
    /// An attack is announced `ticks` ahead.
    Telegraph {
        source: EntityId,
        ticks: u32,
    },
    PartBroken {
        part: ContentId,
    },
    Died {
        entity: EntityId,
    },
    StageCleared,
    StageFailed,
}
