//! The render snapshot: what the frontend draws and interpolates between ticks (ADR-005). It is
//! a view of the state, not the state; the state hash does not read it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::{HitBox, PhaseKind};
use crate::event::Press;
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId};
use crate::rewards::Rarity;

/// An attack in progress (ADR-055), for the frontend to draw its telegraph and its swing on the
/// same tick that `core` judges them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttackView {
    /// 0 for an agent's or the boss's attacks and the main character's combo (ADR-069).
    pub slot: u8,
    /// The preset it came from; absent for an inline attack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<ContentId>,
    /// Which phase, from 0.
    pub phase: u8,
    pub kind: PhaseKind,
    /// Ticks done in the phase: 0 on its first tick.
    pub tick: u32,
    /// How long the phase lasts.
    pub ticks: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_box: Option<HitBox>,
    /// The facing locked when the attack started.
    pub facing: Facing,
}

/// A direction as a unit vector times `trig::SCALE`; `y` grows down the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Facing {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub tick: u32,
    /// The map's size, so the frontend can bound its camera (core#87).
    pub map: MapView,
    pub player: PlayerView,
    /// Companions, enemies, the boss and its parts, in entity-ID order.
    pub entities: Vec<EntityView>,
    pub bullets: Vec<BulletView>,
    /// Falling stars on the map, in the order they dropped (ADR-066 §2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stars: Vec<StarView>,
    /// Experience, level, stat points and the open offer; absent in a stage without levels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<ProgressView>,
}

/// Where the run stands in experience, levels and stat points (ADR-066 §2, §4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgressView {
    /// From 1.
    pub level: u32,
    /// Experience gained toward the next level.
    pub xp: u32,
    /// Experience the next level needs; 0 at the highest level.
    pub xp_next: u32,
    /// Stat points not spent yet.
    pub points: u32,
    /// Points spent on each run stat, in the run stats' order (ADR-067).
    pub spent: Vec<u8>,
    /// The open offer: while it is open the world is stopped, and `Engine::choose` takes one
    /// of its options (ADR-066 §3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offer: Option<OfferView>,
}

/// A falling star waiting on the map to be taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StarView {
    pub id: EntityId,
    pub at: Point,
    pub rarity: Rarity,
}

/// An offer from a falling star (ADR-066 §3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OfferView {
    pub kind: OfferKind,
    /// The star's rarity, or for a boss offer the highest of the player's tier. A candidate is
    /// of a lower one when the pool has too few left of it.
    pub rarity: Rarity,
    pub candidates: Vec<CandidateView>,
    /// What `Engine::choose` takes, by index: the candidates, then a reroll of each while
    /// rerolls are left and another reward could come, then the skip (not in a boss offer),
    /// then a banish of each while banishes are left.
    pub options: Vec<OfferOption>,
    /// Rerolls and banishes left in the run.
    pub rerolls_left: u32,
    pub banishes_left: u32,
}

/// What opened an offer (ADR-066 §2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OfferKind {
    /// A falling star the main character took; it can be skipped.
    Star,
    /// A boss kill: at the highest rarity of the player's tier, with no skip.
    Boss,
}

/// A reward an offer holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateView {
    /// Its ID in the run's rewards file.
    pub reward: ContentId,
    pub rarity: Rarity,
}

/// One option of an open offer; `candidate` is a place in its candidates, from 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OfferOption {
    /// Takes the candidate's reward and closes the offer.
    Take { candidate: u8 },
    /// Draws that candidate again, spending a reroll; the offer stays open.
    Reroll { candidate: u8 },
    /// Closes the offer, taking nothing.
    Skip,
    /// Takes the candidate's reward out of the run's pool for good, spending a banish, and
    /// draws its place again; the offer stays open, or closes when nothing is left to offer.
    Banish { candidate: u8 },
}

/// The map's width and height, from its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MapView {
    pub width: Fx,
    pub height: Fx,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlayerView {
    pub at: Point,
    pub hp: u32,
    /// The main character's hurt circle.
    pub radius: Fx,
    /// The last non-zero movement direction (ADR-055).
    pub facing: Facing,
    /// Ticks of invulnerability left, after a hit or from an evade; the frontend can flash it.
    pub invulnerable: u32,
    pub evade: EvadeView,
    /// Where it is in its Main weapon's combos (ADR-070).
    pub combo: ComboView,
    /// The current step charging, if it is (ADR-070).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charge: Option<ChargeView>,
    /// The enemy aim assist and auto mode would face, while either is on (ADR-072), so the
    /// frontend can mark it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<EntityId>,
    /// Its attacks in progress, by slot.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attacks: Vec<AttackView>,
}

/// The evade (ADR-074).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvadeView {
    /// A press on the next tick would dash.
    pub ready: bool,
    /// Ticks until it is ready again, counted from the press.
    pub cooldown_left: u32,
    /// Ticks of the dash still to move: 0 when not dashing.
    pub dash_left: u32,
}

/// Where the main character is in its combos (ADR-070): what the frontend's combo trail draws,
/// with the next possible presses read from the weapon's tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComboView {
    /// The presses from the root to the current step; empty at the root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presses: Vec<Press>,
    /// The presses began in the dash branch (a dash-strike).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dash: bool,
    /// The press waiting for the next step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffered: Option<Press>,
    /// Ticks left in which a press continues the combo, once a step has run out.
    pub window_left: u32,
}

/// A charge in progress (ADR-070). `core` judges the perfect window on this tick, so what the
/// frontend shows is what a release on the next tick gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChargeView {
    /// The level reached: 0 before level 1, when letting go is a plain step and the frontend
    /// keeps its clock at full speed (ADR-071).
    pub level: u8,
    /// Ticks held since the step began to charge, 0 on its first tick.
    pub held: u32,
    /// Inside the level's perfect window: letting go on the next tick is a perfect release.
    pub perfect: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntityView {
    pub id: EntityId,
    pub kind: EntityKind,
    pub asset: ContentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<ContentId>,
    pub at: Point,
    /// The hurt circle that hits use.
    pub radius: Fx,
    pub hp: u32,
    /// The attack it is making, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attack: Option<AttackView>,
    /// An elite enemy (ADR-068), which the minimap shows wherever it is.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub elite: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Companion,
    Enemy,
    Boss,
    BossPart,
}

/// Bullets carry no ID or asset name: thousands are drawn per tick, batched by `style`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BulletView {
    pub at: Point,
    /// How far the bullet moves each tick, in 1/256 pixel: it moved by this to reach `at`, and
    /// moves by it again on the next tick. The frontend draws it at `at` plus this times the
    /// fraction of the tick that has passed.
    pub vx: Fx,
    pub vy: Fx,
    pub radius: Fx,
    /// The bullet asset, as an index into the bullet asset IDs the stage uses, in order of
    /// first appearance.
    pub style: u16,
    /// Fired by the main character or a companion, rather than an enemy.
    pub friendly: bool,
}
