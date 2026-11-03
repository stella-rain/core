//! The events of the first rules as released (ADR-REPL-13 point 3, ADR-REPL-13 points 1 to 3): the shape
//! version 1 emitted before the live event gained its player fields. Never changed, never
//! serialised; `engine/v1_events.rs` turns each into the live `crate::event::DomainEvent`.

use std::sync::Arc;

use crate::attack::{DamageType, MatchUp};
use crate::fixed::Point;
use crate::id::{ContentId, EntityId};
use crate::kinds::KindIndex;
use crate::rewards::Rarity;
use crate::rune::Rune;
use crate::snapshot::{EntityKind, ExpertiseOfferKind, OfferKind, Press};

/// What kind of thing hurt the main character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitKind {
    /// A hit box of an attack, or an orb that an attack fired.
    Attack,
    /// A zone an attack placed.
    Zone,
    /// The beam between two linked orbs.
    Link,
}

/// What hurt the main character (PHIT).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HitSource {
    pub kind: HitKind,
    /// The attacker that owns it (`Hit::source` is the same kind of ID).
    pub owner: EntityId,
    /// The attack preset it came from; absent for an inline attack.
    pub attack: Option<Arc<str>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DomainEvent {
    /// `source` damaged `target`.
    Hit {
        source: EntityId,
        target: EntityId,
        /// The kind of `target` (`EntityView::kind_index`), an index into the run's `KindTable`: left out for the main character and
        /// for a mimic still in disguise.
        target_kind: Option<KindIndex>,
        damage: u32,
        /// The damage type the hit was dealt in, whatever its source: a weapon, a rune,
        /// the stage skill, a burst or a damage-over-time tick; left out when neutral.
        damage_type: Option<DamageType>,
        /// The match-up the hit got; left out when none.
        match_up: MatchUp,
        /// The rune that dealt the hit: the on-hit rune whose attack landed, or the
        /// damage-over-time rune whose status ticked; left out for every other hit.
        rune: Option<Rune>,
        /// The main character's hit was a critical hit; left out when not.
        crit: bool,
        /// The hit is an echo of an earlier one, a tick later; left out when not.
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
        /// What hurt it.
        source: HitSource,
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
    /// `target`'s stun ran out; it is immune to a new one for the immunity ticks.
    StunEnded {
        target: EntityId,
    },
    /// The main character started an evade.
    Evaded,
    /// The main character used its stage skill, at `level`.
    StageSkillUsed {
        skill: ContentId,
        level: u8,
    },
    /// A press took a combo step: the step `depth` presses from the root.
    ComboStep {
        press: Press,
        depth: u8,
    },
    /// The combo went back to the root: its window passed, an evade, a broken charge, or a press
    /// with no next step.
    ComboReset,
    /// A charged step was released at `level` (from 1; a release before level 1 is a plain step
    /// and says nothing), `perfect` inside the level's perfect window.
    ChargeReleased {
        level: u8,
        perfect: bool,
    },
    /// A charge ended without an attack: an evade, or a hit from an attack that breaks charges.
    ChargeBroken,
    /// A held step's hold was let go, or reached its `max_ticks`, after `ticks` ticks held; its
    /// release attack follows.
    HoldReleased {
        ticks: u32,
    },
    /// A hold ended without a release: an evade, or a hit from an attack that breaks charges.
    HoldBroken,
    /// An agent's rule fired: `rule` is its index in the agent's (or boss phase's) list (ADR-SIM-18).
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
        /// The kind of the victim (`EntityView::kind_index`), an index into the run's `KindTable`; left out for a mimic still in
        /// disguise.
        kind: Option<KindIndex>,
        /// What died, as the snapshot classifies it (`EntityView::kind`), so a reader never
        /// guesses it from earlier snapshots: not even for an elite that dies on its first tick.
        class: EntityKind,
        /// An elite (`EntityView::elite`). Absent when false.
        elite: bool,
    },
    StageCleared,
    StageFailed,
    /// The main character reached `level` (ADR-REPL-16): a stat is raised by the run's priority,
    /// or a stat offer is due (DS18).
    LeveledUp {
        level: u32,
    },
    /// A level-up raised the run stat `stat`, by the priority or by a choice: it now has
    /// `spent` points (ADR-REPL-16, DS18).
    StatRaised {
        stat: ContentId,
        spent: u8,
    },
    /// A stat offer opened for a level reached with `pick_stats`, and the world stopped until a
    /// choice closes it: `options` are the stats below their limit, in the run stats' order
    /// (DS18).
    StatOfferOpened {
        options: Vec<ContentId>,
    },
    /// A kill dropped the falling star `star` (ADR-REPL-16).
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
    /// and every elite that comes later is called at once.
    ElitesCalled {
        elites: u32,
    },
    /// An elite minute of a `time_limit` run brought the elite `elite`, of the sprite `asset`
    ///; for a mimic, the sprite it is disguised as. Not sent when the cap on elites
    /// alive held it back.
    EliteSpawned {
        elite: EntityId,
        asset: ContentId,
        /// The elite's kind (`EntityView::kind_index`), an index into the run's `KindTable`; left out for a mimic, which is disguised.
        kind: Option<KindIndex>,
    },
    /// The leash moved the enemy `entity` just outside the view, on the side opposite the view's centre, or on the
    /// enemy's own side when that one is off the map, so the frontend snaps its body instead of interpolating across
    /// the screen (EC2, DS80).
    EnemyRelocated {
        entity: EntityId,
    },
    /// The elite `elite` left the map without dying: no kill, no reward.
    EliteLeft {
        elite: EntityId,
    },
    /// The main character touched the shrine `elite`: its guardians appeared and its timer began
    ///.
    ShrineActivated {
        elite: EntityId,
    },
    /// The main character came within the mimic `elite`'s reveal radius: it is no longer
    /// disguised, takes damage and acts as a grade elite from now on. Sent once.
    MimicRevealed {
        elite: EntityId,
    },
    /// The main character's hit box broke the breakable orb of `owner` at `at`, and its burst did
    /// not run.
    OrbBroken {
        owner: EntityId,
        at: Point,
    },
    /// An offer opened, and the world stopped until a choice closes it (ADR-REPL-16).
    OfferOpened {
        kind: OfferKind,
        rarity: Rarity,
        candidates: Vec<ContentId>,
    },
    /// The reward was taken: the run now has it `taken` times. For a random rune, `rune` is
    /// the rune it drew, now on the Main weapon.
    RewardTaken {
        reward: ContentId,
        taken: u8,
        rune: Option<Rune>,
        /// The star's name when the reward is a named star.
        star: Option<Arc<str>>,
    },
    /// The reward taken just before completed enough of the constellation `set` to reach its
    /// tier `tier`, counted from 1: the tier's effects replace the lower tier's.
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
    /// choice closes it (ADR-REPL-16).
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
