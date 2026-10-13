//! Weapons (ADR-050, core#83): the main character carries a Main and a Sub weapon, and each
//! attacks on its own, on its own cooldown, at what its targeting picks. The Main weapon brings
//! the charged skill (skill 2), the Sub weapon the Sub skill (skill 3). A weapon is a preset by
//! ID or an inline definition, as an attack is. A preset is a whole weapon: one ID with a Main
//! role and a Sub role, which differ (the game design's weapon table); in `player.main` a
//! preset runs its Main role, in `player.sub` its Sub role. The run's setup will take IDs only
//! (ADR-051).
//!
//! Field names are placeholders until the freeze (ADR-035).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::AttackRef;
use crate::behaviour::Selector;
use crate::id::ContentId;

/// A weapon preset by ID, or an inline weapon in one role (`MainWeapon` or `SubWeapon`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum WeaponRef<W> {
    Preset { id: ContentId },
    Inline(Box<W>),
}

/// A weapon as Main: its automatic attack and its charged skill (skill 2, ADR-050).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MainWeapon {
    pub auto: WeaponAttack,
    pub charge: Charge,
}

/// A weapon as Sub: its automatic attack and its skill (skill 3, ADR-050), which happens on a
/// ready press, with no charge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubWeapon {
    pub auto: WeaponAttack,
    pub skill: WeaponAttack,
}

/// An attack a weapon makes at what its targeting picks: on its own every `cooldown_ticks`
/// (an automatic attack), or on a ready press (the Sub skill).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeaponAttack {
    /// An automatic attack never slows the main character: every phase keeps `move_pct` at 100
    /// (the validator). A skill may.
    pub attack: AttackRef,
    /// The nearest enemy when absent (ADR-050).
    #[serde(default)]
    pub targeting: Targeting,
    /// Ticks from one attack's start to the next one's. If the attack lasts longer, the next
    /// one waits for it to end.
    pub cooldown_ticks: u32,
}

/// The charged Main skill (ADR-050): a ready press starts the charge, the ticks held raise
/// its level, and the release attacks along the facing at the level reached. Releasing inside
/// the level's perfect window adds its perfect bonus; at `max_ticks` it releases by itself,
/// without the bonus. The frontend slows time while charging; the rules do not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Charge {
    /// In order of `at_ticks`; the first is at 0.
    pub levels: Vec<ChargeLevel>,
    /// The longest hold, in ticks.
    pub max_ticks: u32,
    /// Ticks from the release to the next ready press. An evade or a hit that breaks the charge
    /// spends half of it, rounded up.
    pub cooldown_ticks: u32,
    /// The main character's movement speed while charging, in percent; 100 when absent.
    #[serde(default = "full_speed", skip_serializing_if = "is_full_speed")]
    pub move_pct: u8,
}

/// One level of a charge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChargeLevel {
    /// Ticks held at which the level is reached.
    pub at_ticks: u32,
    /// The release at this level.
    pub attack: AttackRef,
    /// The perfect window: the first `perfect_ticks` ticks of the level. 0 for none, which the
    /// first level must be, or a tap would be perfect.
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub perfect_ticks: u32,
    /// The perfect bonus: this much more damage, in percent, for every hit of the release.
    #[serde(default, skip_serializing_if = "is_zero_u16")]
    pub perfect_pct: u16,
    /// A perfect release makes this attack instead, with `perfect_pct` too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perfect_attack: Option<AttackRef>,
}

fn full_speed() -> u8 {
    100
}

fn is_full_speed(v: &u8) -> bool {
    *v == 100
}

fn is_zero_u16(v: &u16) -> bool {
    *v == 0
}

/// What a weapon attacks: along the main character's facing, or at an enemy one of ADR-036's
/// selectors picks. Only the selectors that pick an enemy are here, so a weapon cannot aim at
/// an ally. With a selector and nothing to pick, the weapon does not attack and spends no
/// cooldown.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Targeting {
    /// `aimed` directions point along the facing at the attack's start.
    Facing,
    #[default]
    NearestEnemy,
    /// The boss while it is alive, else the nearest enemy.
    BossFirst,
    /// A boss part, by the part ID the stage gives it, until it breaks.
    Part(ContentId),
}

impl Targeting {
    /// The selector this targeting picks with; `None` for `facing`.
    pub fn selector(&self) -> Option<Selector> {
        match self {
            Targeting::Facing => None,
            Targeting::NearestEnemy => Some(Selector::NearestEnemy),
            Targeting::BossFirst => Some(Selector::BossFirst),
            Targeting::Part(id) => Some(Selector::Part(id.clone())),
        }
    }
}

/// Which of the main character's two weapons a reference is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Main,
    Sub,
}

/// A weapon preset's definition: both of its roles.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeaponPreset {
    pub main: MainWeapon,
    pub sub: SubWeapon,
}
