//! The run setup (ADR-051 §1): what a run of the launch game starts from, and all it starts
//! from. The mode, the Main and the Sub weapon (content IDs, ADR-026), each weapon's expertise
//! level, and the seed of the run's SplitMix64 (ADR-033). The mode's content is bundled in
//! `core` (`modes/`), so replaying a run never needs the player's save data or a stage file.
//!
//! A setup is untrusted (ADR-020): a size limit before parsing, strict parsing, the current
//! versions only, IDs that exist with the right kind and no newer than the setup's
//! `sim_version`, and expertise levels within the maximum. Anything else fails closed, with one
//! message per problem and its path, never text from the file.
//!
//! Expertise levels are validated but grant nothing yet: what a level grants is content of a
//! `sim_version`, and version 0 has none so far.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::id::ContentId;
use crate::registry::{self, Entry, Kind};
use crate::stage::Stage;
use crate::validate::{self, Error, Issue};
use crate::version::{SCHEMA_VERSION, SIM_VERSION};
use crate::weapon::WeaponRef;

/// The largest setup file read; a setup is a few hundred bytes.
pub const MAX_SETUP_BYTES: usize = 4096;
/// The highest expertise level of a weapon (a tuning value; its existence is the decision).
pub const MAX_EXPERTISE: u8 = 10;
/// The most rewards a star pool names: as many as a rewards file holds.
pub const MAX_STAR_POOL: usize = crate::rewards::MAX_REWARDS;
/// The most extra rerolls a setup brings.
pub const MAX_EXTRA_REROLLS: u8 = 20;

/// What a run starts from: built by the app from local storage when a run starts, and stored
/// in the run's replay (ADR-051).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunSetup {
    pub schema_version: u32,
    pub sim_version: u32,
    /// A mode ID: the content the run plays.
    pub mode: ContentId,
    /// A weapon preset, used in its Main role.
    pub main: ContentId,
    /// A weapon preset, used in its Sub role.
    pub sub: ContentId,
    /// The Main weapon's expertise level, at most `MAX_EXPERTISE` (10); grants nothing yet.
    pub main_expertise: u8,
    /// The Sub weapon's expertise level, at most `MAX_EXPERTISE` (10); grants nothing yet.
    pub sub_expertise: u8,
    /// Seeds the run's SplitMix64 (ADR-033), all 64 bits.
    pub seed: u64,
    /// The rewards an offer may hold, by the IDs of the run's rewards file: the player's choice
    /// in the Star Chart (ADR-066 §1), each at most once. Absent: every reward in the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub star_pool: Option<Vec<ContentId>>,
    /// Rerolls the player's progress adds to the stage's, at most `MAX_EXTRA_REROLLS`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub extra_rerolls: u8,
}

fn is_zero(n: &u8) -> bool {
    *n == 0
}

/// Checks the size, parses strictly and validates: the one call for a setup file's bytes.
pub fn parse_and_validate(bytes: &[u8]) -> Result<RunSetup, Error> {
    if bytes.len() > MAX_SETUP_BYTES {
        return Err(Error::TooLarge {
            size: bytes.len(),
            limit: MAX_SETUP_BYTES,
        });
    }
    // Versions first: a setup from a newer game may have fields this one rejects.
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
    check_versions(versions.schema_version, versions.sim_version)?;
    let setup: RunSetup = serde_json::from_slice(bytes).map_err(parse_error)?;
    validate(&setup)?;
    Ok(setup)
}

/// Validates a parsed setup against the registry.
pub fn validate(setup: &RunSetup) -> Result<(), Error> {
    validate_with(setup, registry::ENTRIES)
}

/// As `validate`, against a given registry (tests use one with newer IDs).
pub fn validate_with(setup: &RunSetup, registry: &[Entry]) -> Result<(), Error> {
    check_versions(setup.schema_version, setup.sim_version)?;
    let mut issues = Vec::new();
    let mut id = |key: &str, id: &ContentId, kind: Kind| {
        let problem = if !registry::is_well_formed(&id.0) {
            Some("not a valid content ID (lowercase snake_case, at most 32 characters)".to_owned())
        } else {
            match registry::find(registry, kind, &id.0) {
                None => Some(format!("no such {kind:?} ID in the registry")),
                Some(e) if e.introduced > setup.sim_version => Some(format!(
                    "introduced in sim_version {}, newer than this setup's {}",
                    e.introduced, setup.sim_version
                )),
                Some(_) => None,
            }
        };
        if let Some(message) = problem {
            issues.push(Issue {
                path: format!("$.{key}"),
                message,
            });
        }
    };
    id("mode", &setup.mode, Kind::Mode);
    id("main", &setup.main, Kind::Weapon);
    id("sub", &setup.sub, Kind::Weapon);
    for (key, level) in [
        ("main_expertise", setup.main_expertise),
        ("sub_expertise", setup.sub_expertise),
    ] {
        if level > MAX_EXPERTISE {
            issues.push(Issue {
                path: format!("$.{key}"),
                message: format!("at most {MAX_EXPERTISE} (found {level})"),
            });
        }
    }
    if let Some(pool) = &setup.star_pool {
        if pool.len() > MAX_STAR_POOL {
            issues.push(Issue {
                path: "$.star_pool".into(),
                message: format!("at most {MAX_STAR_POOL} rewards (found {})", pool.len()),
            });
        }
        for (i, id) in pool.iter().enumerate().take(MAX_STAR_POOL) {
            let message = if !registry::is_well_formed(&id.0) {
                "not a valid ID (lowercase snake_case, at most 32 characters)"
            } else if pool[..i].contains(id) {
                "the same reward as an earlier entry"
            } else {
                continue;
            };
            issues.push(Issue {
                path: format!("$.star_pool[{i}]"),
                message: message.into(),
            });
        }
    }
    if setup.extra_rerolls > MAX_EXTRA_REROLLS {
        issues.push(Issue {
            path: "$.extra_rerolls".into(),
            message: format!(
                "at most {MAX_EXTRA_REROLLS} (found {})",
                setup.extra_rerolls
            ),
        });
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}

/// The bundled content of mode `id`: a stage file. Every mode in the registry has one
/// (`tests/run_setup.rs`).
pub fn mode_stage(id: &str) -> Option<&'static [u8]> {
    match id {
        "trial" => Some(include_bytes!("../modes/trial.json")),
        _ => None,
    }
}

/// The stage a setup plays: its mode's stage, validated, with the setup's weapon presets in
/// the Main and Sub roles, validated again. The stage's own `seed` is left as it is:
/// `Engine::start` seeds the run with the setup's.
pub fn stage(setup: &RunSetup) -> Result<Stage, Error> {
    validate(setup)?;
    let content = mode_stage(&setup.mode.0).ok_or_else(|| {
        Error::Invalid(vec![Issue {
            path: "$.mode".into(),
            message: "the mode has no content".into(),
        }])
    })?;
    let mut stage = validate::parse_and_validate(content)?;
    stage.player.main = WeaponRef::Preset {
        id: setup.main.clone(),
    };
    stage.player.sub = WeaponRef::Preset {
        id: setup.sub.clone(),
    };
    validate::validate(&stage)?;
    Ok(stage)
}

fn check_versions(schema_version: u32, sim_version: u32) -> Result<(), Error> {
    if schema_version == SCHEMA_VERSION && sim_version == SIM_VERSION {
        Ok(())
    } else {
        Err(Error::UnsupportedVersion {
            schema_version,
            sim_version,
        })
    }
}
