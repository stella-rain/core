//! The `time_limit` mode file (core#212 step a): how long a run lasts, when its bosses come,
//! which bosses they may be, the overtime after the last one, the Star-Hunter's extra time and
//! the map. Read from `modes/time_limit.json` as untrusted bytes (ADR-020), like the other
//! content files: a size limit before parsing, strict parsing, the current versions only, every
//! content ID in the registry, and each number under a named cap. Only the file is read here;
//! its end and the elites come in later steps.
//!
//! Step b (core#212) adds the schedule: `waves`, the enemies that spawn over the run, as a stage's
//! waves do (`stage::Wave`), checked as a stage's are with larger caps and without elites.
//!
//! Every time is in ticks (60 a second), as `length_ticks` of a stage is. The SHA-256 of the
//! bytes read is part of the run's content hash with the other mode files (`run_setup::mode_file`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::fixed::{Fx, Point};
use crate::id::ContentId;
use crate::pack::Pack;
use crate::registry::{self, Kind};
use crate::stage::{Map, Wave};
use crate::validate::limits::{MAX_MAP_SIDE, MIN_MAP_SIDE};
use crate::validate::{Error, Issue};
use crate::version::{SCHEMA_VERSION, SIM_VERSION};

/// The largest mode file read; the bundled one is about 3 KB.
pub const MAX_TIME_LIMIT_BYTES: usize = 16 * 1024;
/// The longest run: 30 minutes.
pub const MAX_LENGTH_TICKS: u32 = 30 * 60 * 60;
/// The most boss times: the mid-bosses and the final boss.
pub const MAX_BOSS_TIMES: usize = 8;
/// The most bosses in the mid-boss pool.
pub const MAX_POOL: usize = 16;
/// The longest overtime: 10 minutes.
pub const MAX_OVERTIME_TICKS: u32 = 10 * 60 * 60;
/// The longest extra time for the Star-Hunter: 3 minutes.
pub const MAX_EXTRA_TICKS: u32 = 3 * 60 * 60;
/// The most waves in the schedule.
pub const MAX_WAVES: usize = 8;
/// The most enemies one wave spawns.
pub const MAX_SPAWN_COUNT: u16 = 2000;
/// The most enemies the whole schedule spawns: 100 a minute for the longest run is 3000.
pub const MAX_TOTAL_SPAWNS: u32 = 6000;

/// The `time_limit` mode file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimeLimitFile {
    pub schema_version: u32,
    pub sim_version: u32,
    /// The mode's ID, `time_limit`.
    pub id: ContentId,
    /// The run's length in ticks, 1 to `MAX_LENGTH_TICKS`.
    pub length_ticks: u32,
    /// When a boss comes, in ticks from the start: 1 to `MAX_BOSS_TIMES` times that rise
    /// strictly, inside the length, the last at the length. All but the last are mid-bosses;
    /// the last is the final boss.
    pub boss_ticks: Vec<u32>,
    /// The bosses a mid-boss is drawn from (sprite IDs): at least one for each mid-boss time,
    /// at most `MAX_POOL`.
    pub mid_boss_pool: Vec<ContentId>,
    /// The final boss (a sprite ID).
    pub final_boss: ContentId,
    /// How long the run goes on past the length while the final boss lives, in ticks, 1 to
    /// `MAX_OVERTIME_TICKS`.
    pub overtime_ticks: u32,
    /// The time the Star-Hunter adds, in ticks, 1 to `MAX_EXTRA_TICKS`. Read and checked now;
    /// a later step uses it.
    pub star_hunter_extra_ticks: u32,
    pub map: TimeLimitMap,
    /// The enemies that spawn over the run, 0 to `MAX_WAVES` waves of up to `MAX_SPAWN_COUNT`
    /// enemies, `MAX_TOTAL_SPAWNS` in all, none an elite, each starting before the length. A
    /// wave with `around` spawns that far from the main character; the placeholder content is
    /// the trial's enemies.
    pub waves: Vec<Wave>,
}

/// An open field: its sides in 1/256 pixels and where the player starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimeLimitMap {
    /// `MIN_MAP_SIDE` to `MAX_MAP_SIDE`.
    pub width: Fx,
    /// `MIN_MAP_SIDE` to `MAX_MAP_SIDE`.
    pub height: Fx,
    /// On the map.
    pub start: Point,
}

/// A mode file that passed validation, and the SHA-256 of the bytes it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeLimit {
    file: TimeLimitFile,
    bytes: Vec<u8>,
    sha256: [u8; 32],
}

impl TimeLimit {
    /// The bundled file's bytes.
    pub const BUNDLED: &'static [u8] = include_bytes!("../modes/time_limit.json");

    /// The bundled mode, the default of every run of the mode.
    pub fn bundled() -> &'static TimeLimit {
        Pack::bundled().content().time_limit()
    }

    /// Checks the size, parses strictly and validates: the one call for a file's bytes.
    pub fn parse(bytes: &[u8]) -> Result<TimeLimit, Error> {
        if bytes.len() > MAX_TIME_LIMIT_BYTES {
            return Err(Error::TooLarge {
                size: bytes.len(),
                limit: MAX_TIME_LIMIT_BYTES,
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
        let file: TimeLimitFile = serde_json::from_slice(bytes).map_err(parse_error)?;
        validate(&file)?;
        Ok(TimeLimit {
            file,
            bytes: bytes.to_vec(),
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The run's length in ticks.
    pub fn length_ticks(&self) -> u32 {
        self.file.length_ticks
    }

    /// When each boss comes, in ticks; the last is the final boss, at the length.
    pub fn boss_ticks(&self) -> &[u32] {
        &self.file.boss_ticks
    }

    /// The bosses a mid-boss is drawn from.
    pub fn mid_boss_pool(&self) -> &[ContentId] {
        &self.file.mid_boss_pool
    }

    /// The final boss.
    pub fn final_boss(&self) -> &ContentId {
        &self.file.final_boss
    }

    /// The overtime in ticks.
    pub fn overtime_ticks(&self) -> u32 {
        self.file.overtime_ticks
    }

    /// The Star-Hunter's extra time in ticks.
    pub fn star_hunter_extra_ticks(&self) -> u32 {
        self.file.star_hunter_extra_ticks
    }

    /// The enemy schedule.
    pub fn waves(&self) -> &[Wave] {
        &self.file.waves
    }

    /// The map and the player's start.
    pub fn map(&self) -> &TimeLimitMap {
        &self.file.map
    }

    /// The bytes it was parsed from.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The SHA-256 of the bytes it was parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &TimeLimitFile) -> Result<(), Error> {
    let mut issues = Vec::new();
    let mut issue = |path: String, message: String| issues.push(Issue { path, message });

    if file.id.0 != "time_limit" {
        issue("$.id".into(), "must be time_limit".into());
    }
    if !(1..=MAX_LENGTH_TICKS).contains(&file.length_ticks) {
        issue(
            "$.length_ticks".into(),
            format!("1 to {MAX_LENGTH_TICKS} (found {})", file.length_ticks),
        );
    }

    let times = &file.boss_ticks;
    if times.is_empty() || times.len() > MAX_BOSS_TIMES {
        issue(
            "$.boss_ticks".into(),
            format!("1 to {MAX_BOSS_TIMES} times (found {})", times.len()),
        );
    } else {
        let last = times.len() - 1;
        for (i, &t) in times.iter().enumerate() {
            let message = if t == 0 || t > file.length_ticks {
                "inside the run: after the start, no later than the length"
            } else if i > 0 && t <= times[i - 1] {
                "later than the time before it"
            } else if i == last && t != file.length_ticks {
                "the last boss comes at the length"
            } else {
                continue;
            };
            issue(format!("$.boss_ticks[{i}]"), message.into());
        }
    }

    let mid_bosses = times.len().saturating_sub(1);
    let pool = &file.mid_boss_pool;
    if pool.len() > MAX_POOL || pool.len() < mid_bosses {
        issue(
            "$.mid_boss_pool".into(),
            format!(
                "{mid_bosses} to {MAX_POOL} bosses, one for each mid-boss time (found {})",
                pool.len()
            ),
        );
    }
    let mut boss = |path: String, id: &ContentId| {
        let message = if !registry::is_well_formed(&id.0) {
            "not a valid content ID (lowercase snake_case, at most 32 characters)"
        } else if registry::find(registry::ENTRIES, Kind::Asset, &id.0)
            .is_none_or(|e| e.introduced > file.sim_version)
        {
            "no such sprite ID in the registry"
        } else {
            return;
        };
        issue(path, message.into());
    };
    for (i, id) in pool.iter().enumerate().take(MAX_POOL) {
        boss(format!("$.mid_boss_pool[{i}]"), id);
    }
    boss("$.final_boss".into(), &file.final_boss);

    if !(1..=MAX_OVERTIME_TICKS).contains(&file.overtime_ticks) {
        issue(
            "$.overtime_ticks".into(),
            format!("1 to {MAX_OVERTIME_TICKS} (found {})", file.overtime_ticks),
        );
    }
    if !(1..=MAX_EXTRA_TICKS).contains(&file.star_hunter_extra_ticks) {
        issue(
            "$.star_hunter_extra_ticks".into(),
            format!(
                "1 to {MAX_EXTRA_TICKS} (found {})",
                file.star_hunter_extra_ticks
            ),
        );
    }

    let map = &file.map;
    let sides = i64::from(MIN_MAP_SIDE)..=i64::from(MAX_MAP_SIDE);
    for (key, side) in [("width", map.width), ("height", map.height)] {
        if !sides.contains(&i64::from(side.0)) {
            issue(
                format!("$.map.{key}"),
                format!("{MIN_MAP_SIDE} to {MAX_MAP_SIDE} (found {})", side.0),
            );
        }
    }
    for (key, at, side) in [
        ("x", map.start.x, map.width),
        ("y", map.start.y, map.height),
    ] {
        if at.0 < 0 || at.0 > side.0 {
            issue(
                format!("$.map.start.{key}"),
                format!("on the map, 0 to {} (found {})", side.0, at.0),
            );
        }
    }

    let waves = &file.waves;
    if waves.len() > MAX_WAVES {
        issue(
            "$.waves".into(),
            format!("at most {MAX_WAVES} waves (found {})", waves.len()),
        );
    } else {
        let field = Map {
            width: map.width,
            height: map.height,
            start: Some(map.start),
        };
        if let Err(found) =
            crate::validate::validate_waves(waves, field, file.length_ticks, MAX_SPAWN_COUNT)
        {
            for i in found {
                issue(
                    format!("$.waves{}", i.path.trim_start_matches('$')),
                    i.message,
                );
            }
        }
        for (i, w) in waves.iter().enumerate() {
            if w.enemy.elite.is_some() {
                issue(
                    format!("$.waves[{i}].enemy.elite"),
                    "no elites in the schedule".into(),
                );
            }
        }
        let total: u32 = waves.iter().map(|w| u32::from(w.count)).sum();
        if total > MAX_TOTAL_SPAWNS {
            issue(
                "$.waves".into(),
                format!("at most {MAX_TOTAL_SPAWNS} enemies in all (found {total})"),
            );
        }
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}
