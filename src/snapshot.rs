//! The render snapshot: what the frontend draws and interpolates between ticks (ADR-005). It is
//! a view of the state, not the state; the state hash does not read it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub tick: u32,
    pub player: PlayerView,
    /// Companions, enemies, the boss and its parts, in entity-ID order.
    pub entities: Vec<EntityView>,
    pub bullets: Vec<BulletView>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlayerView {
    pub at: Point,
    pub hp: u32,
    pub focus: bool,
    pub firing: bool,
    /// The slot held this tick, 0 for none; the frontend slows its tick clock while a ready
    /// skill is held (ADR-032).
    pub held: u8,
    pub skills: Vec<SkillView>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillView {
    pub slot: u8,
    pub ready: bool,
    pub cooldown_left: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charges_left: Option<u16>,
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
    pub hp: u32,
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
    pub radius: Fx,
    /// The bullet asset, as an index into the bullet asset IDs the stage uses, in order of
    /// first appearance.
    pub style: u16,
    /// Fired by the main character or a companion, rather than an enemy.
    pub friendly: bool,
}
