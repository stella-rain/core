//! Stage skills (ADR-075): what the `skill` button uses. A run's one slot starts empty and is
//! filled from falling stars: a reward with the `stage_skill` effect names one of the rewards
//! file's skills. Taking a skill other than the one held replaces it, and it arrives ready;
//! taking the same skill again raises its level, up to its highest.
//!
//! Data picks, code acts (ADR-067): each level's effect is one of a closed list of kinds built
//! into `core`, with its numbers here. A new kind is a code change and a new `sim_version`.
//! Every number is bounded (ADR-020); the bounds are tuning values.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::AttackRef;
use crate::fixed::Fx;
use crate::id::ContentId;

/// The most stage skills a rewards file lists.
pub const MAX_STAGE_SKILLS: usize = 16;
/// The most levels a skill has.
pub const MAX_SKILL_LEVELS: usize = 5;
/// The longest cooldown: one minute.
pub const MAX_SKILL_COOLDOWN_TICKS: u32 = 3_600;
/// The longest a black hole, a buff, a debuff or a shield lasts: 10 seconds.
pub const MAX_SKILL_DURATION_TICKS: u32 = 600;
/// The largest area: a radius of 256 pixels.
pub const MAX_SKILL_RADIUS: i32 = 65_536;
/// The farthest ahead a skill acts: 512 pixels.
pub const MAX_SKILL_AHEAD: i32 = 131_072;
/// The strongest pull of a black hole: 8 pixels a tick.
pub const MAX_PULL: i32 = 2_048;
/// The largest buff or debuff: +1000%.
pub const MAX_SKILL_PCT: u16 = 1_000;
/// The most damage a shield absorbs.
pub const MAX_SHIELD: u32 = 10_000;

/// One stage skill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StageSkill {
    /// The skill's own ID, unique within the file; a reward names it.
    pub id: ContentId,
    /// How far ahead of the main character its point is, along the facing, or toward the aim
    /// assist's target while `assist` is on (ADR-072); 0, the default, is at the main
    /// character. A damage area's attack places its own hit boxes instead.
    #[serde(default, skip_serializing_if = "is_zero_fx")]
    pub ahead: Fx,
    /// Level 1 first; every level's effect is of the same kind.
    pub levels: Vec<SkillLevel>,
}

impl StageSkill {
    /// The kind of its effects: the first level's.
    pub fn kind(&self) -> SkillKind {
        self.levels
            .first()
            .map_or(SkillKind::Buff, |l| l.effect.kind())
    }
}

fn is_zero_fx(v: &Fx) -> bool {
    v.0 == 0
}

/// One level of a stage skill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillLevel {
    /// Ticks after a use before it is ready again, from 1 to `MAX_SKILL_COOLDOWN_TICKS`.
    pub cooldown_ticks: u32,
    pub effect: SkillEffect,
}

/// What a use does (ADR-075 §4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillEffect {
    /// For `duration_ticks`, every enemy whose circle reaches within `radius` of the point is
    /// pulled `pull` toward it each tick, after the enemies have moved, and so held there. The
    /// boss and its parts are not pulled. A new use replaces a black hole still pulling.
    BlackHole {
        radius: Fx,
        pull: Fx,
        duration_ticks: u32,
    },
    /// The main character's attack `atk_pct` percent up for `duration_ticks` (the `atk_up`
    /// status, ADR-036).
    Buff { atk_pct: u16, duration_ticks: u32 },
    /// Every enemy, the boss and its parts included, whose circle reaches within `radius` of
    /// the point takes `vulnerable_pct` percent more damage for `duration_ticks` (the
    /// `vulnerable` status, ADR-036).
    DebuffArea {
        radius: Fx,
        vulnerable_pct: u16,
        duration_ticks: u32,
    },
    /// `attack` runs in the stage skill's own attack slot (ADR-069), along the aim, at the main
    /// character's attack stat: it neither waits for nor cancels a combo step. A new use ends
    /// the one still running.
    DamageArea { attack: AttackRef },
    /// Absorbs up to `amount` damage to the main character for `duration_ticks`. A new use
    /// replaces the shield held.
    Shield { amount: u32, duration_ticks: u32 },
}

impl SkillEffect {
    pub fn kind(&self) -> SkillKind {
        match self {
            SkillEffect::BlackHole { .. } => SkillKind::BlackHole,
            SkillEffect::Buff { .. } => SkillKind::Buff,
            SkillEffect::DebuffArea { .. } => SkillKind::DebuffArea,
            SkillEffect::DamageArea { .. } => SkillKind::DamageArea,
            SkillEffect::Shield { .. } => SkillKind::Shield,
        }
    }
}

/// The kinds of stage skill, for the frontend's icon and the validator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillKind {
    BlackHole,
    Buff,
    DebuffArea,
    DamageArea,
    Shield,
}

impl SkillKind {
    pub const ALL: [SkillKind; 5] = [
        SkillKind::BlackHole,
        SkillKind::Buff,
        SkillKind::DebuffArea,
        SkillKind::DamageArea,
        SkillKind::Shield,
    ];
}
