//! The render snapshot: what the frontend draws and interpolates between ticks (ADR-005). It is
//! a view of the state, not the state; the state hash does not read it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::{HitBox, PhaseKind};
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId};
use crate::upgrade::Upgrade;

/// An attack in progress (ADR-055), for the frontend to draw its telegraph and its swing on the
/// same tick that `core` judges them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttackView {
    /// 0 for an agent's or the boss's attacks and the main character's Main weapon; 1 for its
    /// Sub weapon, 2 for its charged skill and 3 for its Sub skill (core#83).
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
    /// Experience, level, upgrades and the open offer; absent in a stage without levels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<ProgressView>,
}

/// Where the run stands in experience and levels (ADR-051).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgressView {
    /// From 1.
    pub level: u32,
    /// Experience gained toward the next level.
    pub xp: u32,
    /// Experience the next level needs; 0 at the highest level.
    pub xp_next: u32,
    /// The upgrades taken and their stacks, in `Upgrade::ALL` order.
    pub upgrades: Vec<(Upgrade, u8)>,
    /// The open offer's options: while it is open, `step` does nothing until a choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offer: Option<Vec<Upgrade>>,
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
    /// The charge in progress (skill 2), if any (ADR-050).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charge: Option<ChargeView>,
    /// Skill 2, the charged Main skill.
    pub main_skill: SkillView,
    /// Skill 3, the Sub skill.
    pub sub_skill: SkillView,
    /// Its attacks in progress, by slot.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attacks: Vec<AttackView>,
}

/// Skill 1 (ADR-050).
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

/// A charge in progress (ADR-050). `core` judges the perfect window on this tick, so what the
/// frontend shows is what a release on the next tick gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChargeView {
    /// The level reached, from 1.
    pub level: u8,
    /// Ticks held since the press, 0 on the press's tick.
    pub held: u32,
    /// Inside the level's perfect window: letting go on the next tick is a perfect release.
    pub perfect: bool,
}

/// Skill 2 or 3 (ADR-050).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillView {
    /// A press on the next tick would start it (the Sub skill only if it finds a target).
    pub ready: bool,
    /// Ticks until it is ready again.
    pub cooldown_left: u32,
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
