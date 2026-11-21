//! Expertise (ADR-CONT-1): one level inside one run, for the one weapon, with no highest
//! level. This is its content: the points levels 1 to 10 each need, the growth factor for every
//! level after and the most a level needs, and for each weapon its three styles (a path: the
//! style, then two upgrades and a capstone, at the levels of the mode's level table,
//! `stage::LevelTable`) and its common pool, drawn at the other levels. No option has a stack
//! limit and none leaves its pool by being taken (the attack speed total is held at
//! `run_stats::MAX_ATTACK_SPEED_PCT`); cooldown reduction is no expertise option but Rune
//! Haste, a star reward. Each enemy's points per hit and per kill are in the stage
//! (`stage::ExpertisePoints`).
//!
//! Data picks, code acts: an option's effect is one of the run stats' effect kinds
//! (`StatEffect`), under the same names, or `range_pct`, the range reward's widening.
//! The gauge reads the thresholds, the growth and the maximum, the common offers
//! draw from the common pool, and the style offers show the styles and the chosen style's
//! upgrades and capstone (`sim`). A style level may also name mechanic kinds (`StyleKind`): a
//! step's charge or hold, a charge whose levels come sooner, a delayed repeat or a proc factor of
//! a step's hits, the weapon's smash
//! effect (`reap` among them) or a weapon gauge that fills from hits and damage taken and spends
//! on a full-gauge attack, or a spirit level that the full-gauge smash raises and a countdown
//! lowers, each level with its own effects, or a counter bonus that a near miss of the evade opens
//! and the first dash-branch step inside its window takes, or a combo tempo whose stacks, gained
//! from the swings that hit, speed the attacks the combo starts, or a giant trigger that the
//! full-gauge smash starts and that runs a time with a closed list of effects, or a breaker whose
//! closed list of effects scales the stagger and the poise damage the player deals, the damage of
//! a smash on a boss part and the burst round a target the player staggers; a path may name it at
//! several levels, the effects stacking in path order (ADR-SIM-9). A repeat and a factor are runtime-only fields of the
//! step on the style's copy of the tree.
//!
//! The file is untrusted (ADR-SVC-1), as the rewards are: a size limit before parsing, strict
//! parsing, the current versions only, and every field bounded. The thresholds rise strictly,
//! the factor is above 1, no pool is empty, every weapon preset has three complete styles, and
//! every number is capped. Option and style IDs are the file's own, unique within a weapon. The
//! bundled file, `content/expertise.json`, is the default; its styles are placeholders built
//! from the effect kinds `core` has.

use sha2::{Digest, Sha256};

use crate::id::ContentId;
use crate::pack::Pack;
use crate::validate::Error;
use crate::version::{self, Versions};

/// The largest expertise file read (a tuning value; ADR-SVC-1 decides that one exists).
pub const MAX_EXPERTISE_BYTES: usize = 64 * 1024;
/// The levels whose thresholds the file lists: 1 to 10, so far as the style path's last level
/// (`stage::MAX_STYLE_LEVEL`).
pub const TABLE_LEVELS: usize = 10;
/// The most points one level needs, a threshold or the growth's maximum.
pub const MAX_LEVEL_POINTS: u32 = 1_000_000;
/// The largest growth factor, in percent: four times the level before.
pub const MAX_GROWTH_PCT: u16 = 400;
/// The styles of each weapon: exactly three. The plain base is no style (ADR-SIM-9 point 2): it
/// is the weapon's base tree, with no entry here.
pub const STYLES: usize = 3;
/// The fixed choices a style offers at each of its two upgrades and its capstone: exactly one
/// (ADR-SIM-9 point 3).
pub const MAX_STYLE_CHOICES: usize = 1;
/// The most mechanic kinds one level names (a tuning placeholder): the style pick's, an
/// upgrade's choice's or the capstone's.
pub const MAX_LEVEL_KINDS: usize = 4;
/// The most effects one giant trigger kind names (a bound: one per `TriggerEffect` variant).
pub const MAX_TRIGGER_EFFECTS: usize = 6;
/// The most effects one combo tempo kind names (a bound: one per `TempoEffect` variant).
pub const MAX_TEMPO_EFFECTS: usize = 5;
/// The largest `pct` of a giant trigger's `hold_move_pct`: full speed, the `Hold.move_pct` ceiling.
pub const MAX_HOLD_MOVE_PCT: u32 = 100;
/// The longest delay of a `repeat` kind, in ticks (a tuning placeholder: 2 seconds).
pub const MAX_REPEAT_AFTER_TICKS: u32 = 120;
/// The largest `proc_pct` of a `repeat` or `proc_factor` kind: a factor is a chance (a
/// placeholder).
pub const MAX_PROC_PCT: u32 = 100;
/// The largest `chance_pct` of a `repeat_breaks_orbs` kind: a chance (a placeholder).
pub const MAX_ORB_BREAK_PCT: u32 = 100;
/// The most stacks one repeat of a `repeat_bleed` kind adds (a tuning placeholder: the spirit's cap).
pub const MAX_REPEAT_BLEED_STACKS: u32 = 4;
/// Ticks between two damage ticks of the repeat bleed: the runes bleed's own interval, not
/// content and not scaled by rune haste (it is no rune).
pub const REPEAT_BLEED_INTERVAL_TICKS: u32 = 20;
/// The largest `pct` of a `charge_sooner` kind: a share of the charge's ticks taken off (a bound,
/// not a design number).
pub const MAX_CHARGE_SOONER_PCT: u32 = 90;
/// The most repeats on one step, summed over one style's path (a tuning placeholder). Counted per
/// member: a `repeat` on a group adds one to each member the tree has, and a kind that would put
/// any member over the cap is refused whole.
pub const MAX_STEP_REPEATS: usize = 4;
/// The most extra bleed stacks a spirit level's `smash_bleed_stacks` adds to a smash hit (a
/// tuning placeholder).
pub const MAX_SPIRIT_BLEED_STACKS: u32 = 4;
/// The most options in a weapon's common pool.
pub const MAX_COMMON_OPTIONS: usize = 32;
pub use crate::limits::MAX_SET_STATS;

mod check;
mod kind_checks;
mod style;
mod types;

pub use check::validate;
pub(crate) use style::{
    grant_step, sooner_step, step_mut, style_levels, weapon_attacks, weapon_with_style,
};
#[cfg(test)]
pub(crate) use style::{sooner_at, step_ref};
pub use types::{
    Breaker, BreakerEffect, BurstEffect, ComboTempo, CounterBonus, CounterEffect, ExpertiseEffect,
    ExpertiseFile, ExpertiseOption, Gauge, GaugeSpend, GiantTrigger, SetStat, Spirit, SpiritEffect,
    StepGrant, StepGroup, StepRef, Style, StyleKind, TempoEffect, TriggerEffect, WeaponExpertise,
};

/// Expertise content that passed validation, and the SHA-256 of the bytes it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expertise {
    thresholds: Vec<u32>,
    growth_pct: u16,
    max_points: u32,
    weapons: Vec<WeaponExpertise>,
    sha256: [u8; 32],
}

impl Expertise {
    /// The bundled expertise content.
    pub fn bundled() -> &'static Expertise {
        Pack::bundled().content().expertise()
    }

    /// Checks the size and the versions given (the pack's), parses strictly and validates: the one
    /// call for a file's bytes.
    pub fn parse(bytes: &[u8], versions: Versions) -> Result<Expertise, Error> {
        let file: ExpertiseFile = version::read_member(bytes, MAX_EXPERTISE_BYTES, versions)?;
        validate(&file, versions.sim_version)?;
        Ok(Expertise {
            thresholds: file.thresholds,
            growth_pct: file.growth_pct,
            max_points: file.max_points,
            weapons: file.weapons,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The points each of levels 1 to 10 needs from the level before.
    pub fn thresholds(&self) -> &[u32] {
        &self.thresholds
    }

    /// The growth factor after level 10, in percent.
    pub fn growth_pct(&self) -> u16 {
        self.growth_pct
    }

    /// The most points any level needs.
    pub fn max_points(&self) -> u32 {
        self.max_points
    }

    /// Every weapon's expertise, in the file's order.
    pub fn weapons(&self) -> &[WeaponExpertise] {
        &self.weapons
    }

    /// The expertise of the weapon preset `id`.
    pub fn weapon(&self, id: &ContentId) -> Option<&WeaponExpertise> {
        self.weapons.iter().find(|w| &w.weapon == id)
    }

    /// The SHA-256 of the bytes it was parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}
