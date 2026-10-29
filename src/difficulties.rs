//! Difficulties (ADR-110, ADR-067: data picks, code acts): `easy`, `normal`, `hard` and `rainy`,
//! each a content ID with the percent its enemies' hit points and damage are scaled by, the
//! range of rarities its drops may have and, optionally, its own length, boss times, level table and
//! Stardust time factor tiers (ADR-109, counted by `sim::stardust`) for the mode it is
//! played in. A setup names one (`RunSetup::difficulty`).
//!
//! The file is untrusted (ADR-020), as every content file is: a size limit before parsing,
//! strict parsing, the current versions only, and every field bounded: 1 to `MAX_DIFFICULTIES`
//! difficulties with unique, well-formed IDs, percents from 1 to `MAX_PCT`, a rarity range that
//! does not run backwards, a length with its boss times (both or neither) that obey the mode
//! file's rules, and a level table the stage's own rules accept. IDs are the file's own: the pack
//! records the `sim_version` that introduced each (ADR-080), and a released ID is never renamed
//! or removed (ADR-026).
//! Each difficulty also names the Rune Shards a clear pays (`rune_shards`, 0 to
//! `MAX_RUNE_SHARDS`) and, optionally, the earlier difficulty whose clear opens it
//! (`opens_after`). The simulation never reads either; the app and the settlement do.
//! The bundled file, `content/difficulties.json`, is the default; the SHA-256 of whatever bytes
//! a run used is part of its content hash, so a run replay verifies only with the same file.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::id::ContentId;
use crate::pack::Pack;
use crate::registry;
use crate::rewards::{Rarity, Rewards};
use crate::stage::{LevelTable, StardustTier, stardust_tiers_problem};
use crate::time_limit::{MAX_BOSS_TIMES, MAX_LENGTH_TICKS};
use crate::validate::{Error, Issue};
use crate::version::{self, Versions};

/// The largest difficulties file read (a tuning value; ADR-020 decides that one exists).
pub const MAX_DIFFICULTIES_BYTES: usize = 8 * 1024;
/// The most difficulties a file lists (4 today).
pub const MAX_DIFFICULTIES: usize = 8;
/// The largest percent of any kind: hit points and damage (ADR-103's 32-bit values saturate
/// rather than wrap, `scale_pct`).
pub const MAX_PCT: u32 = 1_000;
/// The most Rune Shards a clear of one difficulty may pay (30 today; a tuning value, ADR-SVC-1
/// decides that a bound exists).
pub const MAX_RUNE_SHARDS: u32 = 1_000;
/// The difficulty of a setup that names none: a setup written before difficulties existed.
/// New setups always name theirs.
pub const DEFAULT_DIFFICULTY: &str = "normal";

/// `value` times `pct` percent, truncated toward zero, saturating at the largest `u32`: how a
/// difficulty scales an enemy's hit points and an attack's damage (ADR-110 point 2).
pub fn scale_pct(value: u32, pct: u32) -> u32 {
    u32::try_from(u64::from(value) * u64::from(pct) / 100).unwrap_or(u32::MAX)
}

/// One difficulty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Difficulty {
    /// The difficulty's own ID, unique within the file.
    pub id: ContentId,
    /// Percent of the content's hit points every enemy, elite and boss (and each boss part)
    /// starts with, from 1 to `MAX_PCT` (100 for normal).
    pub hp_pct: u32,
    /// Percent of the content's damage every attack of an enemy, elite or boss does, from 1 to
    /// `MAX_PCT`.
    pub damage_pct: u32,
    /// The lowest rarity its drops and offers may have, over all six.
    pub rarity_min: Rarity,
    /// The highest rarity its drops and offers may have; not below `rarity_min`.
    pub rarity_max: Rarity,
    /// The Rune Shards a clear of this difficulty pays (game design, Rewards), 0 to
    /// `MAX_RUNE_SHARDS`. The simulation never reads it; the app credits it (ADR-SAVE-4).
    pub rune_shards: u32,
    /// Absent: open with no clear. Otherwise the ID of an earlier difficulty of this file: this
    /// one opens, in a mode, once that one is cleared there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opens_after: Option<ContentId>,
    /// The mode's run length in ticks, in place of its file's: with `boss_ticks`, or neither.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_ticks: Option<u32>,
    /// The mode's boss times in ticks, in place of its file's: rising, the last at
    /// `length_ticks`, at most `time_limit::MAX_BOSS_TIMES`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boss_ticks: Option<Vec<u32>>,
    /// The expertise levels (ADR-106 point 4) in place of the mode's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_table: Option<LevelTable>,
    /// The Stardust time factor's tiers (ADR-109 decision 2) in place of the mode's: as the
    /// mode file's `stardust_tiers`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stardust_tiers: Option<Vec<StardustTier>>,
}

impl Difficulty {
    /// The range of rarities this difficulty draws from, given the rarities `rewards` has:
    /// `rarity_min` and `rarity_max` clamped to the lowest and highest rarity of a reward inside
    /// them. A range with no reward in it stays as it is (nothing could be offered either way).
    /// Only three rarities have rewards today; the rest come with the six-rarity pool.
    pub fn rarity_range(&self, rewards: &Rewards) -> (Rarity, Rarity) {
        let inside = |r: &Rarity| (self.rarity_min..=self.rarity_max).contains(r);
        let has = |r: Rarity| rewards.rewards().iter().any(|x| x.rarity == r);
        let mut present = Rarity::ALL.into_iter().filter(inside).filter(|r| has(*r));
        match present.next() {
            Some(low) => (low, present.next_back().unwrap_or(low)),
            None => (self.rarity_min, self.rarity_max),
        }
    }
}

/// The difficulties file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DifficultiesFile {
    /// The difficulties, 1 to `MAX_DIFFICULTIES`.
    pub difficulties: Vec<Difficulty>,
}

/// Difficulties that passed validation, and the SHA-256 of the bytes they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Difficulties {
    list: Vec<Difficulty>,
    sha256: [u8; 32],
}

impl Difficulties {
    /// The bundled difficulties, the default of every run.
    pub fn bundled() -> &'static Difficulties {
        Pack::bundled().content().difficulties()
    }

    /// Checks the size and the versions given (the pack's), parses strictly and validates: the one
    /// call for a file's bytes.
    pub fn parse(bytes: &[u8], versions: Versions) -> Result<Difficulties, Error> {
        let file: DifficultiesFile = version::read_member(bytes, MAX_DIFFICULTIES_BYTES, versions)?;
        validate(&file)?;
        Ok(Difficulties {
            list: file.difficulties,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The difficulties, in the file's order.
    pub fn list(&self) -> &[Difficulty] {
        &self.list
    }

    /// The difficulty `id`.
    pub fn get(&self, id: &ContentId) -> Option<&Difficulty> {
        self.list.iter().find(|d| d.id == *id)
    }

    /// The SHA-256 of the bytes they were parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &DifficultiesFile) -> Result<(), Error> {
    let mut issues = Vec::new();
    let mut issue = |path: String, message: String| issues.push(Issue { path, message });
    if file.difficulties.is_empty() || file.difficulties.len() > MAX_DIFFICULTIES {
        issue(
            "$.difficulties".into(),
            format!(
                "1 to {MAX_DIFFICULTIES} difficulties (found {})",
                file.difficulties.len()
            ),
        );
    }
    for (i, d) in file.difficulties.iter().enumerate().take(MAX_DIFFICULTIES) {
        let at = |field: &str| format!("$.difficulties[{i}].{field}");
        if !registry::is_well_formed(d.id.as_str()) {
            issue(
                at("id"),
                "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
            );
        } else if file.difficulties[..i].iter().any(|x| x.id == d.id) {
            issue(at("id"), "the same ID as an earlier difficulty".into());
        }
        for (field, pct) in [("hp_pct", d.hp_pct), ("damage_pct", d.damage_pct)] {
            if !(1..=MAX_PCT).contains(&pct) {
                issue(at(field), format!("1 to {MAX_PCT} (found {pct})"));
            }
        }
        if d.rarity_max < d.rarity_min {
            issue(at("rarity_max"), "not below rarity_min".into());
        }
        if d.rune_shards > MAX_RUNE_SHARDS {
            issue(
                at("rune_shards"),
                format!("0 to {MAX_RUNE_SHARDS} (found {})", d.rune_shards),
            );
        }
        if let Some(after) = &d.opens_after {
            if !registry::is_well_formed(after.as_str()) {
                issue(
                    at("opens_after"),
                    "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
                );
            } else if !file.difficulties[..i].iter().any(|x| x.id == *after) {
                issue(
                    at("opens_after"),
                    "not an earlier difficulty of this file".into(),
                );
            }
        }
        match (d.length_ticks, &d.boss_ticks) {
            (None, None) => {}
            (Some(_), None) => issue(at("boss_ticks"), "needed with length_ticks".into()),
            (None, Some(_)) => issue(at("length_ticks"), "needed with boss_ticks".into()),
            (Some(length), Some(times)) => {
                if !(1..=MAX_LENGTH_TICKS).contains(&length) {
                    issue(
                        at("length_ticks"),
                        format!("1 to {MAX_LENGTH_TICKS} (found {length})"),
                    );
                }
                if times.is_empty() || times.len() > MAX_BOSS_TIMES {
                    issue(
                        at("boss_ticks"),
                        format!("1 to {MAX_BOSS_TIMES} times (found {})", times.len()),
                    );
                } else {
                    let last = times.len() - 1;
                    for (t, &tick) in times.iter().enumerate() {
                        let message = if tick == 0 || tick > length {
                            "inside the run: after the start, no later than the length"
                        } else if t > 0 && tick <= times[t - 1] {
                            "later than the time before it"
                        } else if t == last && tick != length {
                            "the last boss comes at the length"
                        } else {
                            continue;
                        };
                        issue(
                            format!("$.difficulties[{i}].boss_ticks[{t}]"),
                            message.into(),
                        );
                    }
                }
            }
        }
        if let Some(problem) = d.level_table.as_ref().and_then(LevelTable::problem) {
            issue(at("level_table"), problem);
        }
        if let Some(problem) = d.stardust_tiers.as_deref().and_then(stardust_tiers_problem) {
            issue(at("stardust_tiers"), problem);
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}
