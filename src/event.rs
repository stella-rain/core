//! Domain events, backend to frontend over the event bus (ADR-016). They drive the frontend's
//! effects and sounds, the debug overlay and golden tests; the simulation never reads them.
//! In a recording each is `{"type": ..., "data": {...}}`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::id::{ContentId, EntityId};
use crate::rewards::Rarity;

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
    /// The main character started an evade (ADR-050).
    Evaded,
    /// The charged Main skill was released at `level` (from 1), `perfect` inside the level's
    /// perfect window (ADR-050, core#83).
    ChargeReleased {
        level: u8,
        perfect: bool,
    },
    /// A charge ended without an attack: an evade, or a hit from an attack that breaks charges.
    ChargeBroken,
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
    /// The main character reached `level`, and gained a stat point (ADR-066 §2).
    LeveledUp {
        level: u32,
    },
    /// A stat point was spent on the run stat `stat`, between ticks: it now has `spent`
    /// points (ADR-066 §4).
    StatRaised {
        stat: ContentId,
        spent: u8,
    },
    /// A kill dropped the falling star `star` (ADR-066 §2).
    StarDropped {
        star: EntityId,
        rarity: Rarity,
    },
    /// The main character took the star `star`; its offer opens at the end of the tick.
    StarTaken {
        star: EntityId,
        rarity: Rarity,
    },
    /// An offer opened, and the world stopped until a choice closes it (ADR-066 §3).
    OfferOpened {
        rarity: Rarity,
        candidates: Vec<ContentId>,
    },
    /// The reward was taken: the run now has it `taken` times.
    RewardTaken {
        reward: ContentId,
        taken: u8,
    },
    /// The offer was closed with nothing taken.
    OfferSkipped,
    /// The reward left the run's pool for good.
    RewardBanished {
        reward: ContentId,
    },
    /// The candidate at `slot` was drawn again, after a reroll or a banish.
    CandidateReplaced {
        slot: u8,
        reward: ContentId,
    },
}
