//! The match-up table (ADR-100, ADR-067): how much a hit's match-up changes its damage, one
//! table for every target. A weak hit deals `weak_pct` percent more, a strong or resisted hit
//! `strong_pct` or `resisted_pct` percent less; a hit with no match-up is unchanged. Which
//! match-up a hit gets is code (`attack::Profile::match_up`); the percents are tuning values.
//! The same rules hold the most the type bonuses add to one attack, `max_type_bonus_pct`
//! (ADR-100 point 7).
//!
//! The file is untrusted (ADR-020), as every content file is: a size limit before parsing,
//! strict parsing, the current versions only, each percent at most `MAX_MATCH_UP_PCT` and the
//! type bonus limit 1 to `MAX_TYPE_BONUS_PCT`.
//! The bundled file, `content/match_ups.json`, is the default; the SHA-256 of whatever bytes a
//! run used is part of its content hash, so a run replay verifies only with the same file.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::attack::MatchUp;
use crate::pack::Pack;
use crate::rewards::MAX_TYPE_BONUS_PCT;
use crate::validate::{Error, Issue};
use crate::version::{SCHEMA_VERSION, SIM_VERSION};

/// The largest match-up file read (a tuning value; ADR-020 decides that one exists).
pub const MAX_MATCH_UPS_BYTES: usize = 1024;
/// The most any match-up changes a hit, in percent: a strong or resisted hit at the cap deals
/// nothing before the floor of 1, a weak one twice its damage.
pub const MAX_MATCH_UP_PCT: u8 = 100;

/// The match-up file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MatchUpsFile {
    pub schema_version: u32,
    pub sim_version: u32,
    /// Percent more damage from a weak hit, 0 to `MAX_MATCH_UP_PCT`.
    pub weak_pct: u8,
    /// Percent less damage from a strong hit, 0 to `MAX_MATCH_UP_PCT`.
    pub strong_pct: u8,
    /// Percent less damage from a resisted hit, 0 to `MAX_MATCH_UP_PCT`.
    pub resisted_pct: u8,
    /// The most the type bonuses that apply to an attack add to its damage, in percent, 1 to
    /// `MAX_TYPE_BONUS_PCT`: their sum saturates here.
    pub max_type_bonus_pct: u32,
}

/// A match-up table that passed validation, and the SHA-256 of the bytes it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchUps {
    weak_pct: u8,
    strong_pct: u8,
    resisted_pct: u8,
    max_type_bonus_pct: u32,
    sha256: [u8; 32],
}

impl MatchUps {
    /// The bundled table, the default of every run.
    pub fn bundled() -> &'static MatchUps {
        Pack::bundled().content().match_ups()
    }

    /// Checks the size, parses strictly and validates: the one call for a file's bytes.
    pub fn parse(bytes: &[u8]) -> Result<MatchUps, Error> {
        if bytes.len() > MAX_MATCH_UPS_BYTES {
            return Err(Error::TooLarge {
                size: bytes.len(),
                limit: MAX_MATCH_UPS_BYTES,
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
        let file: MatchUpsFile = serde_json::from_slice(bytes).map_err(parse_error)?;
        validate(&file)?;
        Ok(MatchUps {
            weak_pct: file.weak_pct,
            strong_pct: file.strong_pct,
            resisted_pct: file.resisted_pct,
            max_type_bonus_pct: file.max_type_bonus_pct,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// What `match_up` adds to a hit, in percent: `damage × (100 + pct) / 100`.
    pub fn pct(&self, match_up: MatchUp) -> i64 {
        match match_up {
            MatchUp::Weak => i64::from(self.weak_pct),
            MatchUp::Strong => -i64::from(self.strong_pct),
            MatchUp::Resisted => -i64::from(self.resisted_pct),
            MatchUp::None => 0,
        }
    }

    /// The most the type bonuses add to one attack, in percent.
    pub fn max_type_bonus_pct(&self) -> u32 {
        self.max_type_bonus_pct
    }

    /// The SHA-256 of the bytes it was parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &MatchUpsFile) -> Result<(), Error> {
    let mut issues: Vec<Issue> = [
        ("weak_pct", file.weak_pct),
        ("strong_pct", file.strong_pct),
        ("resisted_pct", file.resisted_pct),
    ]
    .into_iter()
    .filter(|(_, pct)| *pct > MAX_MATCH_UP_PCT)
    .map(|(field, pct)| Issue {
        path: format!("$.{field}"),
        message: format!("0 to {MAX_MATCH_UP_PCT} (found {pct})"),
    })
    .collect();
    if !(1..=MAX_TYPE_BONUS_PCT).contains(&file.max_type_bonus_pct) {
        issues.push(Issue {
            path: "$.max_type_bonus_pct".into(),
            message: format!(
                "1 to {MAX_TYPE_BONUS_PCT} (found {})",
                file.max_type_bonus_pct
            ),
        });
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}
