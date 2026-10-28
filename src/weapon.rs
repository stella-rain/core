//! Weapons (ADR-084): the main character carries one weapon and attacks with it by hand. The
//! weapon owns the combo tree, reached by presses of `main` (A) and `smash` (B), each step a
//! phased attack (ADR-069) that may charge. A weapon is a preset by ID or an inline definition,
//! as an attack is, and a preset is the whole weapon: `player.weapon` names one. A smash (a step
//! reached by B) may carry one smash effect. The run's setup takes one ID (ADR-107).
//!
//! Field names are placeholders until the freeze (ADR-035).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::AttackRef;
use crate::fixed::Fx;
use crate::id::ContentId;

/// A weapon preset by ID, or an inline weapon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum WeaponRef<W> {
    /// A preset, in one of its trees (ADR-117): none is the weapon's first tree.
    Preset {
        id: ContentId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tree: Option<ContentId>,
    },
    Inline(Box<W>),
}

/// A weapon (ADR-084): its combo tree, the timings that join its steps and its smash effect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Weapon {
    /// The first step for each press, from the root.
    pub combo: Branches,
    /// The first step for each press during an evade's dash or within `dash_strike_ticks` after
    /// it: the dash-strike. A press with no step here starts from `combo`.
    #[serde(default, skip_serializing_if = "Branches::is_empty")]
    pub dash: Branches,
    /// Ticks the combo keeps its place after a step's recovery has run out with no press
    /// buffered; a press within them takes the next step, after them it starts from the root.
    #[schemars(range(min = 0, max = crate::validate::limits::MAX_COMBO_WINDOW_TICKS))]
    pub window_ticks: u32,
    /// Ticks after an evade's dash ends in which a press still takes the dash branch.
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    #[schemars(range(min = 0, max = crate::validate::limits::MAX_DASH_STRIKE_TICKS))]
    pub dash_strike_ticks: u32,
    /// How far aim assist and auto mode look for a target (ADR-072): an enemy whose circle
    /// reaches within this distance of the main character.
    #[schemars(range(min = 0, max = crate::validate::limits::MAX_ASSIST_RANGE))]
    pub assist_range: Fx,
    /// What every hit of a smash (a step reached by `smash`) adds, in the dash branch too; none
    /// when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smash: Option<SmashEffect>,
}

/// The steps that follow: at most one for a press of each button.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Branches {
    /// The step a press of `main` (A) takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main: Option<Box<Step>>,
    /// The step a press of `smash` (B) takes: a smash, which the weapon's smash effect joins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smash: Option<Box<Step>>,
}

impl Branches {
    pub fn is_empty(&self) -> bool {
        self.main.is_none() && self.smash.is_none()
    }

    /// The step for a press of `main` (`smash` false) or of `smash`.
    pub fn get(&self, smash: bool) -> Option<&Step> {
        if smash { &self.smash } else { &self.main }.as_deref()
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
    /// Held past its last `active` phase, the button that reached the step starts a hold
    /// instead of the attack's recovery (ADR-084 point 6). A step has a charge or a hold, never
    /// both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<Hold>,
    #[serde(default, skip_serializing_if = "Branches::is_empty")]
    pub next: Branches,
}

/// A held step (ADR-084 point 6): an attack that goes on while the button stays held, then a
/// release attack when it is let go, or at `max_ticks`. The hold begins where the step's
/// recovery after its last `active` phase would, so that phase is the hold's place.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Hold {
    /// The attack that runs while held: its phases are `active`, and it starts again from its
    /// first phase each time it ends, each loop a new attack with its facing locked at its start.
    #[serde(rename = "loop")]
    pub loop_attack: AttackRef,
    /// The attack released when the button is let go, or at `max_ticks`, with its facing locked
    /// when it starts.
    pub release: AttackRef,
    /// The longest hold, in ticks.
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_HOLD_TICKS))]
    pub max_ticks: u32,
    /// The main character's movement speed while held, in percent; 100 when absent.
    #[serde(default = "full_speed", skip_serializing_if = "is_full_speed")]
    #[schemars(range(min = 0, max = 100))]
    pub move_pct: u8,
}

/// A charged step (ADR-070): the ticks held raise its level, and letting go attacks at the
/// level reached; inside the level's perfect window it adds its perfect bonus. At `max_ticks`
/// it releases by itself, without the bonus. Level 0, before the first of `levels`, is the
/// step's own attack. The frontend slows time from level 1; the rules do not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Charge {
    /// Level 1 and up, in order of `at_ticks`, the first after 0.
    #[schemars(length(min = 1, max = crate::validate::limits::MAX_CHARGE_LEVELS))]
    pub levels: Vec<ChargeLevel>,
    /// The longest hold, in ticks.
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_CHARGE_TICKS))]
    pub max_ticks: u32,
    /// The main character's movement speed while charging, in percent; 100 when absent.
    #[serde(default = "full_speed", skip_serializing_if = "is_full_speed")]
    #[schemars(range(min = 0, max = 100))]
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
    #[schemars(range(min = 0, max = crate::validate::limits::MAX_PERFECT_PCT))]
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

/// What a smash (a step reached by `smash`) does to every enemy it hits (ADR-084 point 8): one
/// of a closed list built into `core`, its numbers in JSON (ADR-067).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SmashEffect {
    /// The enemy hit is drawn toward the main character, at most `distance`, over `ticks`, and
    /// stops where their circles touch. A boss, its parts and elites resist it.
    Pull {
        #[schemars(range(min = 0, max = crate::validate::limits::MAX_PULL_DISTANCE))]
        distance: Fx,
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_PULL_TICKS))]
        ticks: u32,
    },
    /// The enemy hit is pushed away from the main character, at most `distance`, over `ticks`,
    /// and stays inside the map. A boss, its parts and elites resist it, as they resist the pull.
    Knockback {
        #[schemars(range(min = 0, max = crate::validate::limits::MAX_PULL_DISTANCE))]
        distance: Fx,
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_PULL_TICKS))]
        ticks: u32,
    },
    /// The enemy hit gets the `bleed` status, the one bleed of ADR-099 with the bleed rune's
    /// numbers from the run's runes file, none of its own (ADR-084 point 8). A run whose file has
    /// no bleed gives none.
    Bleed {},
    /// The enemy hit is stunned for `ticks` (ADR-085): it stops what it was doing and cannot act
    /// until the stun runs out. Bosses resist it, and an elite takes half the ticks.
    Stun {
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_STUN_TICKS))]
        ticks: u32,
    },
}
