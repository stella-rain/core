//! Domain events, backend to frontend over the event bus (ADR-016). They drive the frontend's
//! effects and sounds, the debug overlay and golden tests; the simulation never reads them.
//! In a recording each is `{"type": ..., "data": {...}}`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::{DamageType, MatchUp};
use crate::fixed::Point;
use crate::id::{ContentId, EntityId, KindId};
use crate::rewards::Rarity;
use crate::rune::Rune;
use crate::snapshot::{ExpertiseOfferKind, OfferKind};

/// Which button's press took a combo step (ADR-070): A or B.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Press {
    Main,
    Smash,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", deny_unknown_fields)]
pub enum DomainEvent {
    /// `source` damaged `target`.
    Hit {
        source: EntityId,
        target: EntityId,
        /// The kind id of `target` (`EntityView::kind_id`): left out for the main character and
        /// for a mimic still in disguise.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_kind: Option<KindId>,
        damage: u32,
        /// The damage type the hit was dealt in (ADR-100), whatever its source: a weapon, a rune,
        /// the stage skill, a burst or a damage-over-time tick; left out when neutral.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        damage_type: Option<DamageType>,
        /// The match-up the hit got (ADR-100); left out when none.
        #[serde(default, skip_serializing_if = "MatchUp::is_none")]
        match_up: MatchUp,
        /// The rune that dealt the hit (ADR-099): the on-hit rune whose attack landed, or the
        /// damage-over-time rune whose status ticked; left out for every other hit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rune: Option<Rune>,
        /// The main character's hit was a critical hit (core#NS2b); left out when not.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        crit: bool,
        /// The hit is an echo of an earlier one, a tick later; left out when not.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        echo: bool,
    },
    /// A kill burst went off where an enemy fell: it damaged the enemies within `radius`
    /// (1/256 pixel) of `at`. `depth` counts the links of the chain, from 1 for the burst of
    /// a kill by the main character.
    KillBurst {
        at: Point,
        radius: i32,
        depth: u8,
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
    /// `target`'s stun ran out; it is immune to a new one for the immunity ticks (ADR-085).
    StunEnded {
        target: EntityId,
    },
    /// The main character started an evade (ADR-074).
    Evaded,
    /// The main character used its stage skill, at `level` (ADR-075).
    StageSkillUsed {
        skill: ContentId,
        level: u8,
    },
    /// A press took a combo step (ADR-070): the step `depth` presses from the root.
    ComboStep {
        press: Press,
        depth: u8,
    },
    /// The combo went back to the root: its window passed, an evade, a broken charge, or a press
    /// with no next step.
    ComboReset,
    /// A charged step was released at `level` (from 1; a release before level 1 is a plain step
    /// and says nothing), `perfect` inside the level's perfect window (ADR-070).
    ChargeReleased {
        level: u8,
        perfect: bool,
    },
    /// A charge ended without an attack: an evade, or a hit from an attack that breaks charges.
    ChargeBroken,
    /// A held step's hold was let go, or reached its `max_ticks`, after `ticks` ticks held; its
    /// release attack follows (ADR-084 point 6).
    HoldReleased {
        ticks: u32,
    },
    /// A hold ended without a release: an evade, or a hit from an attack that breaks charges.
    HoldBroken,
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
        /// The kind id of the victim (`EntityView::kind_id`); left out for a mimic still in
        /// disguise.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<KindId>,
    },
    StageCleared,
    StageFailed,
    /// The main character reached `level`, and gained a stat point (ADR-079 §2).
    LeveledUp {
        level: u32,
    },
    /// A stat point was spent on the run stat `stat`, between ticks: it now has `spent`
    /// points (ADR-079 §4).
    StatRaised {
        stat: ContentId,
        spent: u8,
    },
    /// A kill dropped the falling star `star` (ADR-079 §2).
    StarDropped {
        star: EntityId,
        rarity: Rarity,
    },
    /// The main character took the star `star`; its offer opens at the end of the tick.
    StarTaken {
        star: EntityId,
        rarity: Rarity,
    },
    /// A regular kill dropped the heal pickup `pickup`.
    HealDropped {
        pickup: EntityId,
    },
    /// The boss called its elites: the `elites` alive leave their places and fight beside it,
    /// and every elite that comes later is called at once (ADR-068).
    ElitesCalled {
        elites: u32,
    },
    /// An elite minute of a `time_limit` run brought the elite `elite`, of the sprite `asset`
    /// (ADR-090); for a mimic, the sprite it is disguised as. Not sent when the cap on elites
    /// alive held it back.
    EliteSpawned {
        elite: EntityId,
        asset: ContentId,
        /// The elite's kind id (`EntityView::kind_id`); left out for a mimic, which is disguised.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<KindId>,
    },
    /// The elite `elite` left the map without dying: no kill, no reward (ADR-090).
    EliteLeft {
        elite: EntityId,
    },
    /// The main character touched the shrine `elite`: its guardians appeared and its timer began
    /// (ADR-090).
    ShrineActivated {
        elite: EntityId,
    },
    /// The main character came within the mimic `elite`'s reveal radius: it is no longer
    /// disguised, takes damage and acts as a grade elite from now on. Sent once (ADR-090).
    MimicRevealed {
        elite: EntityId,
    },
    /// An offer opened, and the world stopped until a choice closes it (ADR-079 §3).
    OfferOpened {
        kind: OfferKind,
        rarity: Rarity,
        candidates: Vec<ContentId>,
    },
    /// The reward was taken: the run now has it `taken` times. For a random rune, `rune` is
    /// the rune it drew, now on the Main weapon (ADR-099).
    RewardTaken {
        reward: ContentId,
        taken: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rune: Option<Rune>,
        /// The star's name when the reward is a named star (core#NS3).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        star: Option<String>,
    },
    /// The reward taken just before completed enough of the constellation `set` to reach its
    /// tier `tier`, counted from 1: the tier's effects replace the lower tier's (core#NS3).
    Constellation {
        set: ContentId,
        tier: u8,
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
    /// An expertise offer of `kind` opened for the level `level`, and the world stopped until a
    /// choice closes it (ADR-106 §4, §5; ADR-079 §2, §3).
    ExpertiseOfferOpened {
        level: u32,
        kind: ExpertiseOfferKind,
        candidates: Vec<ContentId>,
    },
    /// The expertise option was taken: the run now has it `taken` times. A style, an upgrade or
    /// a capstone is taken once.
    ExpertiseOptionTaken {
        option: ContentId,
        taken: u8,
    },
    /// The expertise candidate at `slot` was drawn again, after a reroll.
    ExpertiseCandidateReplaced {
        slot: u8,
        option: ContentId,
    },
}
