//! Run stats (ADR-066 §4, ADR-067): the stats a stat point can raise during a run, each with
//! its step and its limit. They are content, not code: a JSON list of any length up to
//! `MAX_STATS`, read in its order, so a stat is added, removed or retuned by editing the file.
//! Data picks, code acts: each stat names one of the effect kinds built in here
//! (`StatEffect`), and a new kind of effect is a code change. Stats that share an effect add
//! up.
//!
//! The file is untrusted (ADR-020), because it may later come from outside the app: a size
//! limit before parsing, strict parsing, the current versions only, and every field bounded.
//! Stat IDs must be well-formed and unique within the file; they are the file's own, not
//! registry entries, so a file can bring a stat this build has never seen as long as its
//! effect kind is one this build knows. The bundled file, `content/run_stats.json`, is the
//! default; the SHA-256 of whatever bytes a run used is part of its content hash, so a run
//! replay verifies only with the same file.

use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::id::ContentId;
use crate::registry;
use crate::validate::{Error, Issue};
use crate::version::{SCHEMA_VERSION, SIM_VERSION};

/// The largest run stats file read (a tuning value; ADR-020 decides that one exists).
pub const MAX_RUN_STATS_BYTES: usize = 16 * 1024;
/// The most stats a file lists.
pub const MAX_STATS: usize = 32;
/// The largest step: a point of hit points, or a percentage.
pub const MAX_STEP: u32 = 100;
/// The most points one stat takes.
pub const MAX_LIMIT: u8 = 20;
/// The longest title, in characters.
pub const MAX_TITLE_CHARS: usize = 32;
/// The longest description, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 160;

/// What a point in a stat does, `step` times over. The closed list of version 0's effects;
/// percentages add up over every stat with the same effect and round down.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum StatEffect {
    /// `step` more hit points at most, and as many healed.
    MaxHp,
    /// `step` percent more damage from every attack of the main character.
    AttackPct,
    /// `step` percent shorter cooldowns for both weapons' automatic attacks (at least 1 tick).
    HastePct,
    /// A `step` percent shorter evade cooldown (at least 1 tick).
    EvadePct,
    /// `step` percent more movement speed.
    SpeedPct,
}

impl StatEffect {
    /// Every effect, in the order the state keeps their totals.
    pub const ALL: [StatEffect; 5] = [
        StatEffect::MaxHp,
        StatEffect::AttackPct,
        StatEffect::HastePct,
        StatEffect::EvadePct,
        StatEffect::SpeedPct,
    ];
}

/// One stat a point can raise.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunStat {
    /// The stat's own ID, unique within the file.
    pub id: ContentId,
    pub effect: StatEffect,
    /// What one point adds, from 1 to `MAX_STEP`.
    pub step: u32,
    /// The most points the stat takes in a run, from 1 to `MAX_LIMIT`.
    pub limit: u8,
    /// The name the pause screen shows, 1 to `MAX_TITLE_CHARS` characters.
    pub title: String,
    /// What a point does, in words, at most `MAX_DESCRIPTION_CHARS` characters.
    pub description: String,
}

/// The run stats file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunStatsFile {
    pub schema_version: u32,
    pub sim_version: u32,
    /// The stats, in the order the pause screen lists them and `spend` numbers them.
    pub stats: Vec<RunStat>,
}

/// Run stats that passed validation, and the SHA-256 of the bytes they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunStats {
    stats: Vec<RunStat>,
    sha256: [u8; 32],
}

impl RunStats {
    /// The bundled file's bytes.
    pub const BUNDLED: &'static [u8] = include_bytes!("../content/run_stats.json");

    /// The bundled run stats, the default of every run.
    pub fn bundled() -> &'static RunStats {
        static BUNDLED: OnceLock<RunStats> = OnceLock::new();
        BUNDLED.get_or_init(|| RunStats::parse(RunStats::BUNDLED).expect("the bundled run stats"))
    }

    /// Checks the size, parses strictly and validates: the one call for a file's bytes.
    pub fn parse(bytes: &[u8]) -> Result<RunStats, Error> {
        if bytes.len() > MAX_RUN_STATS_BYTES {
            return Err(Error::TooLarge {
                size: bytes.len(),
                limit: MAX_RUN_STATS_BYTES,
            });
        }
        // Versions first: a file for a newer game may have fields this one rejects.
        #[derive(Deserialize)]
        struct Versions {
            schema_version: u32,
            sim_version: u32,
        }
        let parse_error = |e: serde_json::Error| Error::Parse {
            line: e.line(),
            column: e.column(),
        };
        let versions: Versions = serde_json::from_slice(bytes).map_err(parse_error)?;
        if versions.schema_version != SCHEMA_VERSION || versions.sim_version != SIM_VERSION {
            return Err(Error::UnsupportedVersion {
                schema_version: versions.schema_version,
                sim_version: versions.sim_version,
            });
        }
        let file: RunStatsFile = serde_json::from_slice(bytes).map_err(parse_error)?;
        validate(&file)?;
        Ok(RunStats {
            stats: file.stats,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The stats, in the file's order.
    pub fn stats(&self) -> &[RunStat] {
        &self.stats
    }

    /// The SHA-256 of the bytes they were parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &RunStatsFile) -> Result<(), Error> {
    let mut issues = Vec::new();
    let mut issue = |path: String, message: String| issues.push(Issue { path, message });
    if file.stats.is_empty() || file.stats.len() > MAX_STATS {
        issue(
            "$.stats".into(),
            format!("1 to {MAX_STATS} stats (found {})", file.stats.len()),
        );
    }
    for (i, stat) in file.stats.iter().enumerate() {
        let at = |field: &str| format!("$.stats[{i}].{field}");
        if !registry::is_well_formed(&stat.id.0) {
            issue(
                at("id"),
                "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
            );
        } else if file.stats[..i].iter().any(|s| s.id == stat.id) {
            issue(at("id"), "the same ID as an earlier stat".into());
        }
        if !(1..=MAX_STEP).contains(&stat.step) {
            issue(at("step"), format!("1 to {MAX_STEP} (found {})", stat.step));
        }
        if !(1..=MAX_LIMIT).contains(&stat.limit) {
            issue(
                at("limit"),
                format!("1 to {MAX_LIMIT} (found {})", stat.limit),
            );
        }
        if let Some(problem) = text_problem(&stat.title, 1, MAX_TITLE_CHARS) {
            issue(at("title"), problem);
        }
        if let Some(problem) = text_problem(&stat.description, 0, MAX_DESCRIPTION_CHARS) {
            issue(at("description"), problem);
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}

/// Why `text` is not `min` to `max` characters of printable text, if it is not.
fn text_problem(text: &str, min: usize, max: usize) -> Option<String> {
    let chars = text.chars().count();
    if !(min..=max).contains(&chars) {
        Some(format!("{min} to {max} characters (found {chars})"))
    } else if text.chars().any(char::is_control) {
        Some("control characters are not allowed".into())
    } else {
        None
    }
}
