//! Rewards (ADR-066 §3, ADR-067, ADR-068): what an offer from a falling star holds. Each has a
//! rarity, a stack limit (how many times one run takes it) and an effect. They are content, not
//! code: a JSON list of any length up to `MAX_REWARDS`, read in its order. Data picks, code
//! acts: a reward's effect is one of the run stats' effect kinds (`StatEffect`), adding
//! `amount` to that effect's total, as a stat point adds its step.
//!
//! The file is untrusted (ADR-020), as the run stats are: a size limit before parsing, strict
//! parsing, the current versions only, and every field bounded. Reward IDs are the file's own,
//! unique within it; a run setup's star pool names them. The bundled file,
//! `content/rewards.json`, is the default; the SHA-256 of whatever bytes a run used is part of
//! its content hash.

use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::id::ContentId;
use crate::registry;
use crate::run_stats::{self, StatEffect};
use crate::validate::{Error, Issue};
use crate::version::{SCHEMA_VERSION, SIM_VERSION};

/// The largest rewards file read (a tuning value; ADR-020 decides that one exists).
pub const MAX_REWARDS_BYTES: usize = 32 * 1024;
/// The most rewards a file lists.
pub const MAX_REWARDS: usize = 64;
/// The largest amount: hit points, or a percentage.
pub const MAX_AMOUNT: u32 = 100;
/// The most times one run takes one reward.
pub const MAX_STACK: u8 = 20;

/// A reward's rarity, from the most common up (ADR-066 §2).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Epic,
    Legendary,
    Supernova,
}

impl Rarity {
    /// Every rarity, from the most common up.
    pub const ALL: [Rarity; 6] = [
        Rarity::Common,
        Rarity::Uncommon,
        Rarity::Rare,
        Rarity::Epic,
        Rarity::Legendary,
        Rarity::Supernova,
    ];

    /// The rarity's place in `ALL`, from 0: what the state hash writes.
    pub fn index(self) -> u8 {
        self as u8
    }
}

/// One reward an offer can hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Reward {
    /// The reward's own ID, unique within the file.
    pub id: ContentId,
    pub rarity: Rarity,
    pub effect: StatEffect,
    /// What taking it adds to the effect's total, from 1 to `MAX_AMOUNT`.
    pub amount: u32,
    /// The most times one run takes it, from 1 to `MAX_STACK`.
    pub limit: u8,
    /// The name the choice screen shows, 1 to `run_stats::MAX_TITLE_CHARS` characters.
    pub title: String,
    /// What it does, in words, at most `run_stats::MAX_DESCRIPTION_CHARS` characters.
    pub description: String,
}

/// The rewards file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RewardsFile {
    pub schema_version: u32,
    pub sim_version: u32,
    /// The rewards, in the order offers draw them from.
    pub rewards: Vec<Reward>,
}

/// Rewards that passed validation, and the SHA-256 of the bytes they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rewards {
    rewards: Vec<Reward>,
    sha256: [u8; 32],
}

impl Rewards {
    /// The bundled file's bytes.
    pub const BUNDLED: &'static [u8] = include_bytes!("../content/rewards.json");

    /// The bundled rewards, the default of every run.
    pub fn bundled() -> &'static Rewards {
        static BUNDLED: OnceLock<Rewards> = OnceLock::new();
        BUNDLED.get_or_init(|| Rewards::parse(Rewards::BUNDLED).expect("the bundled rewards"))
    }

    /// Checks the size, parses strictly and validates: the one call for a file's bytes.
    pub fn parse(bytes: &[u8]) -> Result<Rewards, Error> {
        if bytes.len() > MAX_REWARDS_BYTES {
            return Err(Error::TooLarge {
                size: bytes.len(),
                limit: MAX_REWARDS_BYTES,
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
        let file: RewardsFile = serde_json::from_slice(bytes).map_err(parse_error)?;
        validate(&file)?;
        Ok(Rewards {
            rewards: file.rewards,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The rewards, in the file's order.
    pub fn rewards(&self) -> &[Reward] {
        &self.rewards
    }

    /// The place of the reward `id` in the file, if it has one.
    pub fn index_of(&self, id: &ContentId) -> Option<usize> {
        self.rewards.iter().position(|r| &r.id == id)
    }

    /// The SHA-256 of the bytes they were parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &RewardsFile) -> Result<(), Error> {
    let mut issues = Vec::new();
    let mut issue = |path: String, message: String| issues.push(Issue { path, message });
    if file.rewards.is_empty() || file.rewards.len() > MAX_REWARDS {
        issue(
            "$.rewards".into(),
            format!("1 to {MAX_REWARDS} rewards (found {})", file.rewards.len()),
        );
    }
    for (i, reward) in file.rewards.iter().enumerate() {
        let at = |field: &str| format!("$.rewards[{i}].{field}");
        if !registry::is_well_formed(&reward.id.0) {
            issue(
                at("id"),
                "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
            );
        } else if file.rewards[..i].iter().any(|r| r.id == reward.id) {
            issue(at("id"), "the same ID as an earlier reward".into());
        }
        if !(1..=MAX_AMOUNT).contains(&reward.amount) {
            issue(
                at("amount"),
                format!("1 to {MAX_AMOUNT} (found {})", reward.amount),
            );
        }
        if !(1..=MAX_STACK).contains(&reward.limit) {
            issue(
                at("limit"),
                format!("1 to {MAX_STACK} (found {})", reward.limit),
            );
        }
        if let Some(problem) = run_stats::text_problem(&reward.title, 1, run_stats::MAX_TITLE_CHARS)
        {
            issue(at("title"), problem);
        }
        if let Some(problem) =
            run_stats::text_problem(&reward.description, 0, run_stats::MAX_DESCRIPTION_CHARS)
        {
            issue(at("description"), problem);
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}
