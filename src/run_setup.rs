//! The run setup (ADR-051 §1): what a run of the launch game starts from, and all it starts
//! from. The mode, the Main and the Sub weapon (content IDs, ADR-026), and the seed of the
//! run's SplitMix64 (ADR-033). No expertise level: every run starts at level 0 (ADR-079 §1).
//! The mode's content is bundled in `core` (`modes/`), so replaying a run never needs the
//! player's save data or a stage file.
//!
//! A setup is untrusted (ADR-020): a size limit before parsing, strict parsing, the current
//! versions only, and IDs that exist with the right kind and no newer than the setup's
//! `sim_version`. Anything else fails closed, with one message per problem and its path, never
//! text from the file.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::id::ContentId;
use crate::registry::{self, Entry, Kind};
use std::sync::OnceLock;

use crate::engine::Horde;
use crate::stage::{Map, Stage};
use crate::time_limit::TimeLimit;
use crate::validate::{self, Error, Issue};
use crate::version::{SCHEMA_VERSION, SIM_VERSION};
use crate::weapon::WeaponRef;

/// The largest setup file read; a setup is a few hundred bytes.
pub const MAX_SETUP_BYTES: usize = 4096;
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
    /// Seeds the run's SplitMix64 (ADR-033), all 64 bits.
    pub seed: u64,
    /// The rewards an offer may hold, by the IDs of the run's rewards file: the player's choice
    /// in the Star Chart (ADR-079 §1), each at most once. Absent: every reward in the file.
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

/// The bundled file of mode `id`: what the run's content hash covers. Every mode in the
/// registry has one (`tests/run_setup.rs`). For `time_limit` it is the mode file followed by the
/// stage it takes its placeholder content from (`mode_stage("trial")`).
pub fn mode_file(id: &str) -> Option<&'static [u8]> {
    static TIME_LIMIT: OnceLock<Vec<u8>> = OnceLock::new();
    match id {
        "time_limit" => Some(TIME_LIMIT.get_or_init(|| time_limit_file(TimeLimit::bundled()))),
        _ => mode_stage(id),
    }
}

/// The bytes the content hash covers for a `time_limit` mode file `mode`: the file, then the
/// stage its placeholder content comes from.
pub fn time_limit_file(mode: &TimeLimit) -> Vec<u8> {
    [mode.bytes(), TIME_LIMIT_BASE].concat()
}

/// The stage the `time_limit` run takes its main character, companions, levels, stars and boss
/// from: placeholders until the mode has its own.
const TIME_LIMIT_BASE: &[u8] = include_bytes!("../modes/trial.json");

/// The bundled content of mode `id` when it is a stage file; `time_limit` is not one (its run
/// is built from its mode file, `time_limit_run`).
pub fn mode_stage(id: &str) -> Option<&'static [u8]> {
    match id {
        "trial" => Some(TIME_LIMIT_BASE),
        _ => None,
    }
}

/// The stage a setup plays: its mode's stage, validated, with the setup's weapon presets in
/// the Main and Sub roles, validated again. The stage's own `seed` is left as it is:
/// `Engine::start` seeds the run with the setup's. For `time_limit`, the stage of its run
/// (`time_limit_run`).
pub fn stage(setup: &RunSetup) -> Result<Stage, Error> {
    stage_with(setup, TimeLimit::bundled())
}

/// As `stage`, with `time_limit` in place of the bundled `time_limit` mode file.
pub fn stage_with(setup: &RunSetup, time_limit: &TimeLimit) -> Result<Stage, Error> {
    validate(setup)?;
    if setup.mode.0 == "time_limit" {
        return Ok(time_limit_run(setup, time_limit)?.0);
    }
    let content = mode_stage(&setup.mode.0).ok_or_else(|| {
        Error::Invalid(vec![Issue {
            path: "$.mode".into(),
            message: "the mode has no stage".into(),
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

/// The stage and the bosses of a `time_limit` run of `mode` (core#212): the placeholder stage
/// with the setup's weapons, its map, schedule of waves and length from the mode file, the
/// length being the run and its overtime (a clear is the final boss's death; running out of
/// ticks fails), and no boss until their times (`Horde`). Only the base is validated as a
/// stage: the mode file validated its own waves, which a stage's caps would refuse.
pub(crate) fn time_limit_run(setup: &RunSetup, mode: &TimeLimit) -> Result<(Stage, Horde), Error> {
    if setup.mode.0 != "time_limit" {
        return Err(Error::Invalid(vec![Issue {
            path: "$.mode".into(),
            message: "not the time_limit mode".into(),
        }]));
    }
    let mut stage = validate::parse_and_validate(TIME_LIMIT_BASE)?;
    stage.player.main = WeaponRef::Preset {
        id: setup.main.clone(),
    };
    stage.player.sub = WeaponRef::Preset {
        id: setup.sub.clone(),
    };
    validate::validate(&stage)?;
    let template = stage.boss.take().ok_or_else(|| {
        Error::Invalid(vec![Issue {
            path: "$.boss".into(),
            message: "the placeholder stage has no boss".into(),
        }])
    })?;
    stage.id = "time-limit".into();
    stage.title = "Time Limit".into();
    stage.description = String::new();
    stage.length_ticks = mode.length_ticks().saturating_add(mode.overtime_ticks());
    stage.map = Some(Map {
        width: mode.map().width,
        height: mode.map().height,
        start: Some(mode.map().start),
    });
    stage.waves = mode.waves().to_vec();
    let horde = Horde {
        boss_ticks: mode.boss_ticks().to_vec(),
        pool: mode.mid_boss_pool().to_vec(),
        final_boss: mode.final_boss().clone(),
        template,
        spawned: 0,
        used: vec![false; mode.mid_boss_pool().len()],
    };
    Ok((stage, horde))
}

/// The most ticks a replay of `stage` may hold. A `time_limit` run's stage ends at the length
/// and the overtime; its replay may also hold the Star-Hunter's extra time (ADR-079).
pub fn max_replay_ticks(setup: &RunSetup, stage: &Stage, time_limit: &TimeLimit) -> u32 {
    if setup.mode.0 == "time_limit" {
        stage
            .length_ticks
            .saturating_add(time_limit.star_hunter_extra_ticks())
    } else {
        stage.length_ticks
    }
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
