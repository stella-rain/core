//! The stage file (ADR-006): data only, parsed strictly (ADR-018, ADR-020). Unknown fields are
//! rejected everywhere. Field names follow ADR-036's shape and are placeholders until the
//! freeze (ADR-035); the validator (budgets, registry, text) is separate.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::AttackRef;
use crate::behaviour::{Agent, Condition, Rule, Selector, Skill};
use crate::fixed::{Fx, Point};
use crate::id::ContentId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    /// `crate::version::SCHEMA_VERSION` when the game wrote the file.
    pub schema_version: u32,
    /// `crate::version::SIM_VERSION` when the game wrote the file.
    pub sim_version: u32,
    /// The stage ID: the file name and the first half of the version tag.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Seeds the run's SplitMix64 (ADR-033). 32 bits so that every tool reading the file as
    /// JSON, GDScript included, keeps it exact; the simulation widens it.
    pub seed: u32,
    /// Ticks until the stage ends if the boss is still alive.
    pub length_ticks: u32,
    pub player: Player,
    #[serde(default)]
    pub companions: Vec<Agent>,
    #[serde(default)]
    pub waves: Vec<Wave>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boss: Option<Boss>,
}

/// The main character, as the creator configured it; challengers get exactly this (ADR-012).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Player {
    pub base: ContentId,
    pub hp: u32,
    pub atk: u32,
    /// The main shot, fired on ticks whose input has `fire` (ADR-038).
    pub shot: AttackRef,
    /// Up to 3 active skills (ADR-032).
    #[serde(default)]
    pub skills: Vec<SkillSlot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillSlot {
    /// 1 to 3; the input's `held` field names it.
    pub slot: u8,
    pub target: Selector,
    pub skill: Skill,
    #[serde(default)]
    pub cooldown_ticks: u32,
    /// Uses per run; absent means unlimited, limited by the cooldown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charges: Option<u16>,
}

/// `count` copies of `enemy`, the first at `at_tick`, then one every `every_ticks`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Wave {
    pub at_tick: u32,
    pub enemy: Agent,
    pub spawn: Point,
    pub count: u16,
    #[serde(default)]
    pub every_ticks: u32,
}

/// Tier 2 (ADR-036): body parts and an ordered list of phases that only move forward.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Boss {
    pub base: ContentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<ContentId>,
    pub hp: u32,
    pub radius: Fx,
    pub spawn: Point,
    #[serde(default)]
    pub parts: Vec<BossPart>,
    pub phases: Vec<Phase>,
}

/// A breakable part with its own HP and hurt circle (ADR-007, ADR-036).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BossPart {
    /// Unique within the boss; `part_broken` and the `part` selector use it.
    pub id: ContentId,
    /// Asset ID of the part's sprite.
    pub asset: ContentId,
    pub hp: u32,
    /// Offset from the boss's centre.
    pub offset: Point,
    pub radius: Fx,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Phase {
    /// Ends the phase; absent on the last phase, which lasts until the boss dies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<Condition>,
    /// Runs when the phase begins.
    #[serde(default)]
    pub transition: Vec<Transition>,
    /// The choreography, one step at a time.
    #[serde(default)]
    pub timeline: Vec<Step>,
    /// Reactions, evaluated like an agent's rules.
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transition {
    ClearBullets,
    InvulnerableTicks { ticks: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    MoveTo {
        to: Point,
        ticks: u32,
    },
    Wait {
        ticks: u32,
    },
    Telegraph {
        ticks: u32,
    },
    Attack {
        attack: AttackRef,
    },
    /// Starts the timeline again from its first step.
    Loop,
}
