//! Domain events, backend to frontend over the event bus (ADR-BRDG-3). They drive the frontend's
//! effects and sounds, the debug overlay and golden tests; the simulation never reads them.
//! In a recording each is `{"type": ..., "data": {...}}`.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::{DamageType, MatchUp};
use crate::fixed::Point;
use crate::id::{ContentId, EntityId, PlayerIdx};
use crate::kinds::KindIndex;
use crate::rewards::Rarity;
use crate::rune::Rune;
use crate::snapshot::EntityKind;

/// Which button's press took a combo step: A or B.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Press {
    Main,
    Smash,
}

/// What kind of thing hurt the main character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HitKind {
    /// A hit box of an attack, or an orb that an attack fired.
    Attack,
    /// A zone an attack placed.
    Zone,
    /// The beam between two linked orbs.
    Link,
}

/// What hurt the main character (PHIT).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HitSource {
    pub kind: HitKind,
    /// The attacker that owns it (`Hit::source` is the same kind of ID).
    pub owner: EntityId,
    /// The attack preset it came from; absent for an inline attack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attack: Option<Arc<str>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", deny_unknown_fields)]
pub enum DomainEvent {
    /// `source` damaged `target`.
    Hit {
        source: EntityId,
        target: EntityId,
        /// The kind of `target` (`EntityView::kind_index`), an index into the run's `KindTable`: left out for the main character and
        /// for a mimic still in disguise.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_kind: Option<KindIndex>,
        damage: u32,
        /// The damage type the hit was dealt in, whatever its source: a weapon, a rune,
        /// the stage skill, a burst or a damage-over-time tick; left out when neutral.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        damage_type: Option<DamageType>,
        /// The match-up the hit got; left out when none.
        #[serde(default, skip_serializing_if = "MatchUp::is_none")]
        match_up: MatchUp,
        /// The rune that dealt the hit: the on-hit rune whose attack landed, or the
        /// damage-over-time rune whose status ticked; left out for every other hit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rune: Option<Rune>,
        /// The main character's hit was a critical hit (core#NS2b); left out when not.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        crit: bool,
        /// The hit is an echo of an earlier one, a tick later; left out when not.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        echo: bool,
        /// The player credited with the hit, by its entity number (0 to 2, as `Healed::target`
        /// names one): the owner of the shot, or slot 0 for a companion's. Left out for a hit
        /// on an ally and for the frozen rules of `sim_version` 1 (ADR-SIM-7).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        player: Option<EntityId>,
        /// The other player whose attack-up was on `player` when the hit landed. Left out when
        /// there is none, when it is `player` itself, and at `sim_version` 1.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        buffed_by: Option<EntityId>,
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
    /// A party player reached 0 hit points and went down (ADR-SIM-7).
    PlayerDowned {
        player: PlayerIdx,
    },
    /// A downed player was revived by `reviver` (ADR-SIM-7).
    PlayerRevived {
        player: PlayerIdx,
        reviver: PlayerIdx,
    },
    /// A downed player's out timer expired (ADR-SIM-7).
    PlayerOut {
        player: PlayerIdx,
    },
    /// `target` regained `amount` hit points.
    Healed {
        target: EntityId,
        amount: u32,
        /// The player that gave the heal (its entity number); left out when a companion or an
        /// enemy gave it, and at `sim_version` 1.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<EntityId>,
    },
    StatusApplied {
        target: EntityId,
        status: ContentId,
        stacks: u16,
        duration_ticks: u32,
        /// The player that applied it (its entity number); left out when a companion or an
        /// enemy did, and at `sim_version` 1.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<EntityId>,
    },
    /// `target`'s stun ran out; it is immune to a new one for the immunity ticks.
    StunEnded {
        target: EntityId,
    },
    /// `target`'s flinch ran out, or a stun that landed on it ended it; it is immune to a new
    /// flinch for the rules' `flinch_immune_ticks`.
    FlinchEnded {
        target: EntityId,
    },
    /// `target`'s stagger ran out: its gauge fills again from empty, each later bar at its own
    /// refill rate.
    StaggerEnded {
        target: EntityId,
    },
    /// `target`'s knockdown ran out: the enemy acts again unless a stun still holds it.
    KnockdownEnded {
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
        #[schemars(range(min = 1, max = i64::from(crate::validate::limits::MAX_COMBO_DEPTH)))]
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
    /// An agent's rule fired: `rule` is its index in the agent's (or boss phase's) list (ADR-SIM-4).
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<KindIndex>,
        /// What died, as the snapshot classifies it (`EntityView::kind`), so a reader never
        /// guesses it from earlier snapshots: not even for an elite that dies on its first tick.
        class: EntityKind,
        /// An elite (`EntityView::elite`). Absent when false.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        elite: bool,
        /// The player whose hit took it to 0 hit points (its entity number, the first hit of
        /// the tick's fixed order). Left out when no hit did (a shrine cleared), for an ally,
        /// and at `sim_version` 1.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        killer: Option<EntityId>,
    },
    StageCleared,
    StageFailed,
    /// The main character reached `level` (ADR-REPL-5): a stat is raised by the run's priority,
    /// or a stat offer is due (DS18).
    LeveledUp {
        level: u32,
    },
    /// A level-up raised the run stat `stat`, by the priority or by a choice: it now has
    /// `spent` points (ADR-REPL-5, DS18).
    StatRaised {
        stat: ContentId,
        spent: u8,
    },
    /// A kill dropped the falling star `star` (ADR-REPL-5).
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
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
    /// not run (core Z2).
    OrbBroken {
        owner: EntityId,
        at: Point,
        /// The player whose swing or full-gauge clear broke it; absent in a version 1 run.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        player: Option<PlayerIdx>,
    },
    /// The reward was taken: the run now has it `taken` times. For a random rune, `rune` is
    /// the rune it drew, now on the Main weapon.
    RewardTaken {
        reward: ContentId,
        taken: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rune: Option<Rune>,
        /// The star's name when the reward is a named star (core#NS3).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        star: Option<Arc<str>>,
    },
    /// The reward taken just before completed enough of the constellation `set` to reach its
    /// tier `tier`, counted from 1: the tier's effects replace the lower tier's (core#NS3).
    Constellation {
        set: ContentId,
        tier: u8,
    },
    /// The weapon gauge of `player` gained `amount` (more than 0, after the step cap and the
    /// max); `value` is the gauge after. A friendly combo swing or a hit taken fills it
    /// (ADR-SIM-9).
    GaugeFilled {
        player: PlayerIdx,
        amount: u32,
        value: u32,
        /// `value` reached the style's max.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        full: bool,
    },
    /// The full-gauge smash of `player` spent `amount`, the whole gauge (ADR-SIM-9).
    GaugeSpent {
        player: PlayerIdx,
        amount: u32,
    },
    /// A near miss of `player` was a perfect evade: its slow and its cooldown have just started
    /// (ADR-SIM-8).
    PerfectEvade {
        player: PlayerIdx,
    },
}
