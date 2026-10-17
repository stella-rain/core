//! Weapons (ADR-070): the main character carries a Main and a Sub weapon and attacks with them
//! by hand. The Main weapon owns the combos: a tree of steps reached by presses of `main` (A)
//! and `sub` (B), each step a phased attack (ADR-069) that may charge. The Sub weapon has no
//! attack of its own; it adds its bonus to every step reached by `sub`. A weapon is a preset by
//! ID or an inline definition, as an attack is. A preset is a whole weapon: one ID with a Main
//! role and a Sub role; in `player.main` a preset runs its Main role, in `player.sub` its Sub
//! role. The run's setup takes IDs only (ADR-066).
//!
//! Field names are placeholders until the freeze (ADR-035).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::AttackRef;
use crate::fixed::Fx;
use crate::id::ContentId;

/// A weapon preset by ID, or an inline weapon in one role (`MainWeapon` or `SubWeapon`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum WeaponRef<W> {
    Preset { id: ContentId },
    Inline(Box<W>),
}

/// A weapon as Main (ADR-070): its combo tree and the timings that join its steps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MainWeapon {
    /// The first step for each press, from the root.
    pub combo: Branches,
    /// The first step for each press during an evade's dash or within `dash_strike_ticks` after
    /// it: the dash-strike. A press with no step here starts from `combo`.
    #[serde(default, skip_serializing_if = "Branches::is_empty")]
    pub dash: Branches,
    /// Ticks the combo keeps its place after a step's recovery has run out with no press
    /// buffered; a press within them takes the next step, after them it starts from the root.
    pub window_ticks: u32,
    /// Ticks after an evade's dash ends in which a press still takes the dash branch.
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub dash_strike_ticks: u32,
}

/// The steps that follow: at most one for a press of each button.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Branches {
    /// The step a press of `main` (A) takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main: Option<Box<Step>>,
    /// The step a press of `sub` (B) takes: a combo part, which the Sub weapon's bonus joins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<Box<Step>>,
}

impl Branches {
    pub fn is_empty(&self) -> bool {
        self.main.is_none() && self.sub.is_none()
    }

    /// The step for a press of `main` (`sub` false) or of `sub`.
    pub fn get(&self, sub: bool) -> Option<&Step> {
        if sub { &self.sub } else { &self.main }.as_deref()
    }
}

/// One step of a combo (ADR-070).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// The step's own attack: what a tap makes, and level 0 of a charge.
    pub attack: AttackRef,
    /// While the button that reached the step stays held as it starts, it charges instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charge: Option<Charge>,
    #[serde(default, skip_serializing_if = "Branches::is_empty")]
    pub next: Branches,
}

/// A charged step (ADR-070): the ticks held raise its level, and letting go attacks at the
/// level reached; inside the level's perfect window it adds its perfect bonus. At `max_ticks`
/// it releases by itself, without the bonus. Level 0, before the first of `levels`, is the
/// step's own attack. The frontend slows time from level 1; the rules do not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Charge {
    /// Level 1 and up, in order of `at_ticks`, the first after 0.
    pub levels: Vec<ChargeLevel>,
    /// The longest hold, in ticks.
    pub max_ticks: u32,
    /// The main character's movement speed while charging, in percent; 100 when absent.
    #[serde(default = "full_speed", skip_serializing_if = "is_full_speed")]
    pub move_pct: u8,
}

/// One level of a charge, from level 1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChargeLevel {
    /// Ticks held at which the level is reached.
    pub at_ticks: u32,
    /// The release at this level.
    pub attack: AttackRef,
    /// The perfect window: the first `perfect_ticks` ticks of the level; 0 for none.
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

/// A weapon as Sub (ADR-070): no attack of its own, only its combo-part bonus.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubWeapon {
    pub bonus: SubBonus,
}

/// What the Sub weapon adds to every hit of a step reached by `sub` (ADR-070): one of a closed
/// list built into `core`, its numbers in JSON (ADR-067).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SubBonus {
    /// The enemy hit is drawn toward the main character, at most `distance`, over `ticks`, and
    /// stops where their circles touch. A boss, its parts and elites resist it.
    Pull { distance: Fx, ticks: u32 },
}

/// A weapon preset's definition: both of its roles.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeaponPreset {
    pub main: MainWeapon,
    pub sub: SubWeapon,
}
