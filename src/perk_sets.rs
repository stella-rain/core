//! Perk sets (ADR-CONT-1: data picks, code acts): a small bundle of
//! effects, like a class, that a run applies from tick 0 for its whole length. Each set is a
//! content ID with a title and a short list of effects, each effect one kind of the closed list
//! built in here (`PerkEffect`) and an amount. Which set a player has open is the save's
//! business, never `core`'s.
//!
//! The file is untrusted (ADR-SVC-1), as every content file is: a size limit before parsing,
//! strict parsing, the current versions only, and every field bounded: 1 to `MAX_PERK_SETS`
//! sets with unique, well-formed IDs, at most `MAX_PERK_EFFECTS` effects in a set, each kind
//! once, and each amount from 1 to its kind's cap. Set IDs are the file's own: the pack records
//! the `sim_version` that introduced each (ADR-CONT-5), and a released ID is never renamed or
//! removed (ADR-CONT-2).
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

/// The largest perk sets file read (a tuning value; ADR-SVC-1 decides that one exists).
pub const MAX_PERK_SETS_BYTES: usize = 8 * 1024;
/// The most sets a file lists (4 today).
pub const MAX_PERK_SETS: usize = 16;
/// The most effects in a set: one of each kind.
pub const MAX_PERK_EFFECTS: usize = PerkEffect::ALL.len();
/// The largest amount of a percentage effect.
pub const MAX_PERK_PCT: u32 = 100;
/// The most hit points a set adds: 500 shown hit points, scaled as the rewards'
/// amounts are.
pub const MAX_PERK_MAX_HP: u32 = 500 * DAMAGE_SCALE;
/// The most hit points a set regenerates each second: 10 shown hit points, scaled.
pub const MAX_PERK_REGEN: u32 = 10 * DAMAGE_SCALE;
/// The most rerolls or banishes a set adds: as many as a setup may add (`MAX_EXTRA_REROLLS`).
pub const MAX_PERK_OFFER_EXTRAS: u32 = MAX_EXTRA_REROLLS as u32;

/// What a perk does, `amount` times over. The closed list of perk effects; the
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
    /// `amount` percent more weight for the named stars a member's drops roll (equipment mode,
    /// `sim_version` 2; a no-op in the frozen rules).
    DropWeight,
}

impl PerkEffect {
    /// Every effect.
    pub const ALL: [PerkEffect; 9] = [
        PerkEffect::MaxHp,
        PerkEffect::Regen,
        PerkEffect::AttackPct,
        PerkEffect::AttackSpeedPct,
        PerkEffect::EvadePct,
        PerkEffect::SpeedPct,
        PerkEffect::ExtraRerolls,
        PerkEffect::ExtraBanishes,
        PerkEffect::DropWeight,
    ];

    /// The run stats' effect kind it adds to, if it is one.
    pub fn stat(self) -> Option<StatEffect> {
        Some(match self {
            PerkEffect::MaxHp => StatEffect::MaxHp,
            PerkEffect::AttackPct => StatEffect::AttackPct,
            PerkEffect::AttackSpeedPct => StatEffect::AttackSpeedPct,
            PerkEffect::EvadePct => StatEffect::EvadePct,
            PerkEffect::SpeedPct => StatEffect::SpeedPct,
            PerkEffect::Regen
            | PerkEffect::ExtraRerolls
            | PerkEffect::ExtraBanishes
            | PerkEffect::DropWeight => {
                return None;
            }
        })
    }

    /// The stat a Growth effect becomes in equipment mode, if it is one.
    pub fn growth_stat(self) -> Option<StatEffect> {
        match self {
            PerkEffect::ExtraRerolls => Some(StatEffect::Rerolls),
            PerkEffect::ExtraBanishes => Some(StatEffect::Banishes),
            _ => None,
        }
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
            | PerkEffect::SpeedPct
            | PerkEffect::DropWeight => MAX_PERK_PCT,
        }
    }
}

/// The longest content ID, in bytes (`registry::is_well_formed`).
const MAX_ID_BYTES: usize = 32;

/// An equipment category's ID held inline, so that a perk entry stays `Copy`. Written and read
/// as the ID's string.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PerkCategory {
    len: u8,
    bytes: [u8; MAX_ID_BYTES],
}

impl PerkCategory {
    /// The category's ID.
    pub fn as_str(&self) -> &str {
        // Built only from a `&str` of at most `MAX_ID_BYTES` bytes.
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }

    /// The category named `id`, if it fits an ID.
    pub fn new(id: &str) -> Option<PerkCategory> {
        let b = id.as_bytes();
        if b.len() > MAX_ID_BYTES {
            return None;
        }
        let mut bytes = [0; MAX_ID_BYTES];
        bytes[..b.len()].copy_from_slice(b);
        Some(PerkCategory {
            len: u8::try_from(b.len()).ok()?,
            bytes,
        })
    }
}

impl std::fmt::Debug for PerkCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl Serialize for PerkCategory {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PerkCategory {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<PerkCategory, D::Error> {
        let id = String::deserialize(d)?;
        PerkCategory::new(&id).ok_or_else(|| serde::de::Error::custom("not a valid ID"))
    }
}

impl JsonSchema for PerkCategory {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        ContentId::schema_name()
    }

    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        ContentId::json_schema(g)
    }
}

/// One effect of a set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PerkEntry {
    pub effect: PerkEffect,
    /// From 1 to the effect's cap (`PerkEffect::most`).
    pub amount: u32,
    /// The equipment category a `drop_weight` entry weighs, one of the rewards file's: with
    /// that effect only, and required by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<PerkCategory>,
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
        if !registry::is_well_formed(set.id.as_str()) {
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
            let path = format!("$.perk_sets[{i}].effects[{e}].category");
            match (entry.effect, entry.category) {
                (PerkEffect::DropWeight, None) => {
                    issue(path, "a drop weight names its category".into())
                }
                (PerkEffect::DropWeight, Some(c)) => {
                    if !registry::is_well_formed(c.as_str()) {
                        issue(path, "not a valid ID".into());
                    }
                }
                (_, Some(_)) => issue(path, "only a drop weight names a category".into()),
                (_, None) => {}
            }
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}
