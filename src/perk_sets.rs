//! Perk sets (ADR-105 point 3, ADR-107, ADR-067: data picks, code acts): a small bundle of
//! effects, like a class, that a run applies from tick 0 for its whole length. Each set is a
//! content ID with a title and a short list of effects, each effect one kind of the closed list
//! built in here (`PerkEffect`) and an amount. Which set a player has open is the save's
//! business, never `core`'s.
//!
//! The file is untrusted (ADR-020), as every content file is: a size limit before parsing,
//! strict parsing, the current versions only, and every field bounded: 1 to `MAX_PERK_SETS`
//! sets with unique, well-formed IDs, at most `MAX_PERK_EFFECTS` effects in a set, each kind
//! once, and each amount from 1 to its kind's cap. Set IDs are the file's own: the pack records
//! the `sim_version` that introduced each (ADR-080), and a released ID is never renamed or
//! removed (ADR-026).
//! The bundled file, `content/perk_sets.json`, is the default; the SHA-256 of whatever bytes a
//! run used is part of its content hash, so a run replay verifies only with the same file.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::id::ContentId;
use crate::pack::Pack;
use crate::registry;
use crate::run_setup::MAX_EXTRA_REROLLS;
use crate::run_stats::StatEffect;
use crate::validate::limits::DAMAGE_SCALE;
use crate::validate::{Error, Issue};
use crate::version::{self, Versions};

/// The largest perk sets file read (a tuning value; ADR-020 decides that one exists).
pub const MAX_PERK_SETS_BYTES: usize = 8 * 1024;
/// The most sets a file lists (4 today).
pub const MAX_PERK_SETS: usize = 16;
/// The most effects in a set: one of each kind.
pub const MAX_PERK_EFFECTS: usize = PerkEffect::ALL.len();
/// The largest amount of a percentage effect.
pub const MAX_PERK_PCT: u32 = 100;
/// The most hit points a set adds: 500 shown hit points, scaled (ADR-103) as the rewards'
/// amounts are.
pub const MAX_PERK_MAX_HP: u32 = 500 * DAMAGE_SCALE;
/// The most hit points a set regenerates each second: 10 shown hit points, scaled (ADR-103).
pub const MAX_PERK_REGEN: u32 = 10 * DAMAGE_SCALE;
/// The most rerolls or banishes a set adds: as many as a setup may add (`MAX_EXTRA_REROLLS`).
pub const MAX_PERK_OFFER_EXTRAS: u32 = MAX_EXTRA_REROLLS as u32;

/// What a perk does, `amount` times over. The closed list of version 0's perk effects; the
/// stat kinds are the run stats' (`StatEffect`) and add to the same totals.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PerkEffect {
    /// `amount` more hit points at most, from tick 0 (the run starts at the most).
    MaxHp,
    /// `amount` hit points back every second (60 ticks), never beyond the most.
    Regen,
    /// `amount` percent more damage from every attack of the main character.
    AttackPct,
    /// `amount` percent faster attacks.
    AttackSpeedPct,
    /// An `amount` percent shorter evade cooldown.
    EvadePct,
    /// `amount` percent more movement speed.
    SpeedPct,
    /// `amount` more rerolls for offers, on top of the stage's and the setup's.
    ExtraRerolls,
    /// `amount` more banishes for offers, on top of the stage's.
    ExtraBanishes,
}

impl PerkEffect {
    /// Every effect.
    pub const ALL: [PerkEffect; 8] = [
        PerkEffect::MaxHp,
        PerkEffect::Regen,
        PerkEffect::AttackPct,
        PerkEffect::AttackSpeedPct,
        PerkEffect::EvadePct,
        PerkEffect::SpeedPct,
        PerkEffect::ExtraRerolls,
        PerkEffect::ExtraBanishes,
    ];

    /// The run stats' effect kind it adds to, if it is one.
    pub fn stat(self) -> Option<StatEffect> {
        Some(match self {
            PerkEffect::MaxHp => StatEffect::MaxHp,
            PerkEffect::AttackPct => StatEffect::AttackPct,
            PerkEffect::AttackSpeedPct => StatEffect::AttackSpeedPct,
            PerkEffect::EvadePct => StatEffect::EvadePct,
            PerkEffect::SpeedPct => StatEffect::SpeedPct,
            PerkEffect::Regen | PerkEffect::ExtraRerolls | PerkEffect::ExtraBanishes => {
                return None;
            }
        })
    }

    /// The largest amount of this effect.
    pub fn most(self) -> u32 {
        match self {
            PerkEffect::MaxHp => MAX_PERK_MAX_HP,
            PerkEffect::Regen => MAX_PERK_REGEN,
            PerkEffect::ExtraRerolls | PerkEffect::ExtraBanishes => MAX_PERK_OFFER_EXTRAS,
            PerkEffect::AttackPct
            | PerkEffect::AttackSpeedPct
            | PerkEffect::EvadePct
            | PerkEffect::SpeedPct => MAX_PERK_PCT,
        }
    }
}

/// One effect of a set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PerkEntry {
    pub effect: PerkEffect,
    /// From 1 to the effect's cap (`PerkEffect::most`).
    pub amount: u32,
}

/// One perk set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PerkSet {
    /// The set's own ID, unique within the file.
    pub id: ContentId,
    /// Its effects, at most `MAX_PERK_EFFECTS`, each kind once.
    pub effects: Vec<PerkEntry>,
}

/// The perk sets file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PerkSetsFile {
    /// The sets, 1 to `MAX_PERK_SETS`.
    pub perk_sets: Vec<PerkSet>,
}

/// Perk sets that passed validation, and the SHA-256 of the bytes they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerkSets {
    sets: Vec<PerkSet>,
    sha256: [u8; 32],
}

impl PerkSets {
    /// The bundled perk sets, the default of every run.
    pub fn bundled() -> &'static PerkSets {
        Pack::bundled().content().perk_sets()
    }

    /// Checks the size and the versions given (the pack's), parses strictly and validates: the one
    /// call for a file's bytes.
    pub fn parse(bytes: &[u8], versions: Versions) -> Result<PerkSets, Error> {
        let file: PerkSetsFile = version::read_member(bytes, MAX_PERK_SETS_BYTES, versions)?;
        validate(&file)?;
        Ok(PerkSets {
            sets: file.perk_sets,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The sets, in the file's order.
    pub fn sets(&self) -> &[PerkSet] {
        &self.sets
    }

    /// The place of the set `id` in the file.
    pub fn index_of(&self, id: &ContentId) -> Option<usize> {
        self.sets.iter().position(|s| s.id == *id)
    }

    /// The SHA-256 of the bytes they were parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &PerkSetsFile) -> Result<(), Error> {
    let mut issues = Vec::new();
    let mut issue = |path: String, message: String| issues.push(Issue { path, message });
    if file.perk_sets.is_empty() || file.perk_sets.len() > MAX_PERK_SETS {
        issue(
            "$.perk_sets".into(),
            format!("1 to {MAX_PERK_SETS} sets (found {})", file.perk_sets.len()),
        );
    }
    for (i, set) in file.perk_sets.iter().enumerate().take(MAX_PERK_SETS) {
        let at = |field: &str| format!("$.perk_sets[{i}].{field}");
        if !registry::is_well_formed(&set.id.0) {
            issue(
                at("id"),
                "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
            );
        } else if file.perk_sets[..i].iter().any(|s| s.id == set.id) {
            issue(at("id"), "the same ID as an earlier set".into());
        }
        if set.effects.len() > MAX_PERK_EFFECTS {
            issue(
                at("effects"),
                format!(
                    "at most {MAX_PERK_EFFECTS} effects (found {})",
                    set.effects.len()
                ),
            );
        }
        for (e, entry) in set.effects.iter().enumerate().take(MAX_PERK_EFFECTS) {
            let most = entry.effect.most();
            if set.effects[..e].iter().any(|x| x.effect == entry.effect) {
                issue(
                    format!("$.perk_sets[{i}].effects[{e}].effect"),
                    "the same effect as an earlier entry".into(),
                );
            }
            if !(1..=most).contains(&entry.amount) {
                issue(
                    format!("$.perk_sets[{i}].effects[{e}].amount"),
                    format!("1 to {most} (found {})", entry.amount),
                );
            }
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}
