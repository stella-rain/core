//! The `time_limit` mode file (core#212 step a): how long a run lasts, when its bosses come,
//! which bosses they may be, the overtime after the last one, the Star-Hunter's extra time and
//! the map. Read from `modes/time_limit.json` as untrusted bytes (ADR-020), like the other
//! content files: a size limit before parsing, strict parsing, the current versions only, every
//! content ID in the registry, and each number under a named cap. Only the file is read here;
//! its end and the elites come in later steps. The elite table and the elite cap (ADR-090) are
//! read by the run's elite minutes (`rules_v0::horde`, ADR-090).
//!
//! Step b (core#212) adds the schedule: `waves`, the enemies that spawn over the run, as a stage's
//! waves do (`stage::Wave`), checked as a stage's are with larger caps and without elites.
//!
//! Every time is in ticks (60 a second), as `length_ticks` of a stage is. The SHA-256 of the
//! bytes read is part of the run's content hash with the other mode files (`run_setup::mode_file`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::behaviour::Agent;
use crate::difficulties::MAX_PCT;
use crate::fixed::{Fx, Point};
use crate::id::ContentId;
use crate::pack::Pack;
use crate::registry::{self, Kind};
use crate::stage::{
    Boss, ExpertisePoints, LevelTable, Levels, Map, StardustTier, Stars, Wave,
    stardust_tiers_problem,
};
use crate::validate::limits::{
    self, MAX_CROWD_TARGET, MAX_ELITES_ALIVE, MAX_MAP_SIDE, MIN_MAP_SIDE,
};
use crate::validate::{Error, Issue};
use crate::version::{SCHEMA_VERSION, SIM_VERSION};

/// The largest mode file read; the bundled one is about 60 KB: the bosses and elites are most of it
/// (zones and orbs take more words than a preset call did, BR1a), the enemy roles of R2 and
/// the boss definitions (core#336) the rest.
pub const MAX_TIME_LIMIT_BYTES: usize = 128 * 1024;
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
/// The most waves in the schedule: the two bats and the roles, with room for more (BR1a).
pub const MAX_WAVES: usize = 16;
/// The most enemies one wave spawns.
pub const MAX_SPAWN_COUNT: u16 = 2000;
/// The most enemies the whole schedule spawns: 100 a minute for the longest run is 3000.
pub const MAX_TOTAL_SPAWNS: u32 = 6000;
/// The most time tiers in the elite table: one for each phase of a run, 16 is generous.
pub const MAX_ELITE_TIERS: usize = 16;
/// The most elite types one tier names.
pub const MAX_ELITE_TYPES: usize = 8;
/// The most elite types the file defines.
pub const MAX_ELITE_DEFINITIONS: usize = 16;
/// The most boss definitions the file gives (core#336): as many as the pool, which is its bound.
pub const MAX_BOSS_DEFINITIONS: usize = MAX_POOL;
/// The largest weight of an elite type, as `MAX_RARITY_WEIGHT` is for a star's rarity.
pub const MAX_ELITE_WEIGHT: u32 = 10_000;

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
    /// The boss times at which no elite is called (core#336), each once, at most
    /// `MAX_BOSS_TIMES`. A time that is no boss time of the run has no effect; every other boss calls its elites as its definition says.
    #[serde(default)]
    pub no_call_ticks: Vec<u32>,
    /// The mid-bosses' hit points by slot, in percent (core#336): entry k scales the k-th
    /// mid-boss's hit points and its parts', before the difficulty does; a slot past the list
    /// is at 100. At most `MAX_BOSS_TIMES` entries of 1 to `difficulties::MAX_PCT`. The final
    /// boss is never scaled by it.
    #[serde(default)]
    pub mid_boss_hp_pct: Vec<u32>,
    /// The boss definitions (core#336), 0 to `MAX_BOSS_DEFINITIONS`, each for one sprite: a
    /// pool entry or the final boss with a definition here is made from it, one without is made
    /// from the trial's placeholder boss with its sprite. Each is checked as a stage's boss is,
    /// when a run starts (`run_setup`), and its sprite is an asset.
    #[serde(default)]
    pub bosses: Vec<Boss>,
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
    /// The elites each whole minute that starts no boss brings (ADR-090): 1 to
    /// `MAX_ELITE_TIERS` time tiers, the first at tick 0, the rest rising strictly and starting
    /// before the length. An elite minute draws from it (`rules_v0::horde`).
    pub elite_table: Vec<EliteTier>,
    /// The elite types: 1 to `MAX_ELITE_DEFINITIONS`, each with a unique ID (ADR-090). The
    /// table names them; one no tier names is allowed.
    pub elite_types: Vec<EliteType>,
    /// The most elites alive at once, 1 to `MAX_ELITES_ALIVE` (the stage's `elite_cap` has the
    /// same bound). The run's stage takes it as its own.
    pub elite_cap: u16,
    /// The crowd target of the mode's runs (DIR1, DS103): the pack's `rules.json` gives the
    /// default (200), this overrides it, 1 to `MAX_CROWD_TARGET`. The run's stage takes it as its
    /// own, and the director holds the mode's wave spawns at it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crowd_target: Option<u16>,
    /// The expertise levels of the style path (ADR-106 §4): the style first, the levels rising
    /// strictly, two upgrades and a capstone, each from 1 to `stage::MAX_STYLE_LEVEL`. The
    /// run's stage takes it as its own.
    pub level_table: LevelTable,
    /// The experience curve of the run (XP2): the experience each next level needs, as a
    /// stage's `levels` (1 to `limits::MAX_LEVELS` entries, each 1 to `limits::MAX_LEVEL_XP`),
    /// the last repeating. Not `level_table`, which is the style path's expertise. The run's
    /// stage takes it as its own.
    pub levels: Levels,
    /// The falling stars' drop table of the run (XP2), as a stage's `stars`, checked the same
    /// way; its tiers follow `levels`. The run's stage takes it as its own.
    pub stars: Stars,
    /// The Stardust time factor (ADR-109 decision 2): 1 to `limits::MAX_STARDUST_TIERS` tiers,
    /// the first at tick 0, the ticks rising strictly, each percent from 1 to
    /// `limits::MAX_STARDUST_PCT`. A run takes it unless its difficulty gives its own (ADR-110).
    pub stardust_tiers: Vec<StardustTier>,
    /// The base Stardust a kill of a summoned enemy earns before the factor, times 10, at most
    /// `limits::MAX_STARDUST_BASE`; the run's stage takes it as its `stardust.summon_base`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stardust_summon_base: Option<u32>,
}

/// From `start_tick` until the next tier starts, an elite is drawn by these weights.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EliteTier {
    /// When the tier starts, in ticks from the start of the run.
    pub start_tick: u32,
    /// 1 to `MAX_ELITE_TYPES` elite types, each named once.
    pub weights: Vec<EliteWeight>,
}

/// How likely an elite type is to be drawn, against the other types of its tier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EliteWeight {
    /// The ID of an elite type of the file's `elite_types`.
    pub elite: ContentId,
    /// 1 to `MAX_ELITE_WEIGHT`.
    pub weight: u32,
}

/// An elite type (ADR-090): a content ID (ADR-026) and the enemy it spawns, with what a kill of
/// it gives, as the fields of a wave of the schedule do. The sprite is the enemy's `base`; the
/// enemy must be an elite (ADR-068). A grade is a type; later kinds of elite add entries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EliteType {
    /// Lowercase snake_case, unique in the file; never renamed or removed once released.
    pub id: ContentId,
    /// Checked as a wave's enemy is, and an elite.
    pub enemy: Agent,
    /// The experience a kill gives; as a wave's `xp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xp: Option<u32>,
    /// The expertise points it gives; as a wave's `expertise`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expertise: Option<ExpertisePoints>,
    /// The Stardust a kill of it earns before the time factor, times 10; as a wave's
    /// `stardust`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stardust: Option<u32>,
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

    /// The boss times at which no elite is called.
    pub fn no_call_ticks(&self) -> &[u32] {
        &self.file.no_call_ticks
    }

    /// The mid-bosses' hit points by slot, in percent.
    pub fn mid_boss_hp_pct(&self) -> &[u32] {
        &self.file.mid_boss_hp_pct
    }

    /// The boss definitions, by sprite.
    pub fn bosses(&self) -> &[Boss] {
        &self.file.bosses
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

    /// The elite table's tiers, by start.
    pub fn elite_table(&self) -> &[EliteTier] {
        &self.file.elite_table
    }

    /// The most elites alive at once.
    pub fn elite_cap(&self) -> u16 {
        self.file.elite_cap
    }

    /// The mode's crowd target, if it overrides the pack's.
    pub fn crowd_target(&self) -> Option<u16> {
        self.file.crowd_target
    }

    /// The expertise levels of the style path.
    pub fn level_table(&self) -> &LevelTable {
        &self.file.level_table
    }

    /// The run's experience curve (not the expertise `level_table`).
    pub fn levels(&self) -> &Levels {
        &self.file.levels
    }

    /// The run's falling-star drop table.
    pub fn stars(&self) -> &Stars {
        &self.file.stars
    }

    /// The Stardust time factor's tiers a run takes unless its difficulty gives its own.
    pub fn stardust_tiers(&self) -> &[StardustTier] {
        &self.file.stardust_tiers
    }

    /// The base Stardust of a summoned enemy's kill, times 10, if the mode pays one.
    pub fn stardust_summon_base(&self) -> Option<u32> {
        self.file.stardust_summon_base
    }

    /// The elite types the file defines.
    pub fn elite_types(&self) -> &[EliteType] {
        &self.file.elite_types
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
    if file.no_call_ticks.len() > MAX_BOSS_TIMES {
        issue(
            "$.no_call_ticks".into(),
            format!(
                "at most {MAX_BOSS_TIMES} times (found {})",
                file.no_call_ticks.len()
            ),
        );
    }
    for (i, t) in file.no_call_ticks.iter().enumerate().take(MAX_BOSS_TIMES) {
        if file.no_call_ticks[..i].contains(t) {
            issue(
                format!("$.no_call_ticks[{i}]"),
                "the same time as an earlier entry".into(),
            );
        }
    }
    let hp_pct = &file.mid_boss_hp_pct;
    if hp_pct.len() > MAX_BOSS_TIMES {
        issue(
            "$.mid_boss_hp_pct".into(),
            format!("at most {MAX_BOSS_TIMES} percents (found {})", hp_pct.len()),
        );
    }
    for (i, &pct) in hp_pct.iter().enumerate().take(MAX_BOSS_TIMES) {
        if !(1..=MAX_PCT).contains(&pct) {
            issue(
                format!("$.mid_boss_hp_pct[{i}]"),
                format!("1 to {MAX_PCT} (found {pct})"),
            );
        }
    }
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
    let defs = &file.bosses;
    if defs.len() > MAX_BOSS_DEFINITIONS {
        issue(
            "$.bosses".into(),
            format!(
                "at most {MAX_BOSS_DEFINITIONS} definitions (found {})",
                defs.len()
            ),
        );
    }
    for (i, def) in defs.iter().enumerate().take(MAX_BOSS_DEFINITIONS) {
        if defs[..i].iter().any(|before| before.base == def.base) {
            issue(
                format!("$.bosses[{i}].base"),
                "the same sprite as an earlier definition".into(),
            );
        }
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
    for (i, def) in defs.iter().enumerate().take(MAX_BOSS_DEFINITIONS) {
        boss(format!("$.bosses[{i}].base"), &def.base);
    }

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
            facing: None,
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
            let Some(id) = &w.id else { continue };
            if !registry::is_well_formed(&id.0) {
                issue(
                    format!("$.waves[{i}].id"),
                    "not a valid content ID (lowercase snake_case, at most 32 characters)".into(),
                );
            } else if waves[..i]
                .iter()
                .any(|before| before.id.as_ref() == Some(id))
            {
                issue(
                    format!("$.waves[{i}].id"),
                    "the same ID as an earlier wave".into(),
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

    if let Some(problem) = file.level_table.problem() {
        issue("$.level_table".into(), problem);
    }
    let field = Map {
        width: file.map.width,
        height: file.map.height,
        start: Some(file.map.start),
        facing: None,
    };
    if let Err(found) = crate::validate::validate_levels_and_stars(&file.levels, &file.stars, field)
    {
        for i in found {
            issue(i.path, i.message);
        }
    }
    if let Some(problem) = stardust_tiers_problem(&file.stardust_tiers) {
        issue("$.stardust_tiers".into(), problem);
    }
    if file
        .stardust_summon_base
        .is_some_and(|b| b > limits::MAX_STARDUST_BASE)
    {
        issue(
            "$.stardust_summon_base".into(),
            format!("at most {}", limits::MAX_STARDUST_BASE),
        );
    }
    if !(1..=MAX_ELITES_ALIVE).contains(&file.elite_cap) {
        issue(
            "$.elite_cap".into(),
            format!("1 to {MAX_ELITES_ALIVE} (found {})", file.elite_cap),
        );
    }

    if let Some(target) = file
        .crowd_target
        .filter(|t| !(1..=MAX_CROWD_TARGET).contains(t))
    {
        issue(
            "$.crowd_target".into(),
            format!("1 to {MAX_CROWD_TARGET} (found {target})"),
        );
    }

    let types = &file.elite_types;
    if types.is_empty() || types.len() > MAX_ELITE_DEFINITIONS {
        issue(
            "$.elite_types".into(),
            format!("1 to {MAX_ELITE_DEFINITIONS} types (found {})", types.len()),
        );
    } else {
        let field = Map {
            width: map.width,
            height: map.height,
            start: Some(map.start),
            facing: None,
        };
        for (i, t) in types.iter().enumerate() {
            if !registry::is_well_formed(&t.id.0) {
                issue(
                    format!("$.elite_types[{i}].id"),
                    "not a valid content ID (lowercase snake_case, at most 32 characters)".into(),
                );
            } else if types[..i].iter().any(|before| before.id == t.id) {
                issue(
                    format!("$.elite_types[{i}].id"),
                    "the same ID as an earlier type".into(),
                );
            }
            // Checked as a wave of one enemy at the start, which holds the enemy's own bounds
            // and the kill's experience and expertise.
            let wave = Wave {
                id: None,
                at_tick: 0,
                enemy: t.enemy.clone(),
                spawn: map.start,
                around: None,
                count: 1,
                every_ticks: 0,
                xp: t.xp,
                expertise: t.expertise,
                stardust: t.stardust,
            };
            if let Err(found) = crate::validate::validate_waves(
                std::slice::from_ref(&wave),
                field,
                file.length_ticks.max(1),
                1,
            ) {
                for f in found {
                    let rest = f.path.trim_start_matches("$[0]");
                    issue(format!("$.elite_types[{i}]{rest}"), f.message);
                }
            }
            if t.enemy.elite.is_none() {
                issue(
                    format!("$.elite_types[{i}].enemy.elite"),
                    "an elite type is an elite".into(),
                );
            }
        }
    }

    let tiers = &file.elite_table;
    if tiers.is_empty() || tiers.len() > MAX_ELITE_TIERS {
        issue(
            "$.elite_table".into(),
            format!("1 to {MAX_ELITE_TIERS} tiers (found {})", tiers.len()),
        );
    } else {
        for (i, tier) in tiers.iter().enumerate() {
            let message = if i == 0 && tier.start_tick != 0 {
                Some("the first tier starts with the run, at tick 0")
            } else if i > 0 && tier.start_tick <= tiers[i - 1].start_tick {
                Some("later than the tier before it")
            } else if tier.start_tick >= file.length_ticks {
                Some("before the run ends")
            } else {
                None
            };
            if let Some(message) = message {
                issue(format!("$.elite_table[{i}].start_tick"), message.into());
            }
            let weights = &tier.weights;
            if weights.is_empty() || weights.len() > MAX_ELITE_TYPES {
                issue(
                    format!("$.elite_table[{i}].weights"),
                    format!("1 to {MAX_ELITE_TYPES} types (found {})", weights.len()),
                );
                continue;
            }
            for (j, w) in weights.iter().enumerate() {
                if !file.elite_types.iter().any(|t| t.id == w.elite) {
                    issue(
                        format!("$.elite_table[{i}].weights[{j}].elite"),
                        "no such elite type in $.elite_types".into(),
                    );
                } else if weights[..j].iter().any(|before| before.elite == w.elite) {
                    issue(
                        format!("$.elite_table[{i}].weights[{j}].elite"),
                        "named once in a tier".into(),
                    );
                }
                if !(1..=MAX_ELITE_WEIGHT).contains(&w.weight) {
                    issue(
                        format!("$.elite_table[{i}].weights[{j}].weight"),
                        format!("1 to {MAX_ELITE_WEIGHT} (found {})", w.weight),
                    );
                }
            }
        }
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}
