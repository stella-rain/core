//! The content pack (ADR-080, core#148): the game's content as one JSON file, holding its
//! `schema_version`, the `sim_version` it needs and the content files (the run stats, the
//! rewards, the stage skills, the expertise, the match-up table, the runes, the perk sets and the modes). Its
//! hash is the SHA-256 of the file's bytes.
//!
//! A pack is untrusted (ADR-020), read from the network or from the cache alike: a size limit
//! before parsing, the current versions only, strict parsing, budgets on the counts of its
//! lists, then each file through its own parser, with the bounds that file already enforces
//! (ADR-067). Anything else fails closed, with one message per problem and its path, never text
//! from the pack.
//!
//! A content ID a pack introduces, the file's own ID of a run stat, a reward, a stage skill, or
//! a style or option of a weapon's expertise, or an elite type of the `time_limit` mode, records the `sim_version` that introduced it in
//! the pack's `introduced` table, each exactly once, and is refused when that is newer than the
//! pack's own (ADR-080, as the registry's IDs are for a stage, ADR-017). The registry's IDs a
//! file names are checked against the registry by the file's own parser.
//!
//! Each file sits in the pack as a JSON object, and is read from the exact bytes it has there,
//! followed by a newline: the bytes of the same file on its own. A pack exported from the
//! bundled files (`export_bundled`) so gives each file the bundled file's SHA-256, and a run of
//! its content the bundled content hash.

use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

use crate::constellations::{Constellations, ConstellationsFile};
use crate::difficulties::{Difficulties, DifficultiesFile};
use crate::expertise::{Expertise, ExpertiseFile, WeaponExpertise};
use crate::id::ContentId;
use crate::match_ups::{MatchUps, MatchUpsFile};
use crate::perk_sets::{PerkSets, PerkSetsFile};
use crate::rewards::{Rewards, RewardsFile};
use crate::run_content::RunContent;
use crate::run_stats::{RunStats, RunStatsFile};
use crate::runes::{Runes, RunesFile};
use crate::stage::Stage;
use crate::stage_skill::{StageSkills, StageSkillsFile};
use crate::starting_unlocks::{StartingUnlocks, StartingUnlocksFile};
use crate::time_limit::{TimeLimit, TimeLimitFile};
use crate::validate::{self, Error, Issue};
use crate::version::{SCHEMA_VERSION, SIM_VERSION};

// The count budgets: about four times today's counts, rounded up to a multiple of 8 and held
// to the file's own cap where that is lower (tuning values; ADR-020 decides that they exist).
// They are checked on the pack before any file is read.

/// The most run stats (5 today; the file's own cap is 32).
pub const MAX_RUN_STATS: usize = 24;
/// The most rewards (23 today; held to the file's own cap).
pub const MAX_REWARDS: usize = crate::rewards::MAX_REWARDS;
/// The most stage skills (5 today; held to the file's own cap).
pub const MAX_STAGE_SKILLS: usize = crate::stage_skill::MAX_STAGE_SKILLS;
/// The most weapons the expertise lists (2 today).
pub const MAX_EXPERTISE_WEAPONS: usize = 8;
/// The most common options of one weapon's expertise (3 today; the file's own cap is 32).
pub const MAX_EXPERTISE_COMMON: usize = 16;
/// The most on-hit runes (4 today).
pub const MAX_ON_HIT_RUNES: usize = 16;
/// The most damage-over-time runes (3 today).
pub const MAX_DAMAGE_OVER_TIME_RUNES: usize = 16;
/// The most perk sets (4 today; held to the file's own cap).
pub const MAX_PERK_SETS: usize = crate::perk_sets::MAX_PERK_SETS;
/// The most difficulties (4 today; held to the file's own cap).
pub const MAX_DIFFICULTIES: usize = crate::difficulties::MAX_DIFFICULTIES;
/// The most waves of the `time_limit` mode (2 today; held to the file's own cap).
pub const MAX_TIME_LIMIT_WAVES: usize = crate::time_limit::MAX_WAVES;
/// The most boss times of the `time_limit` mode (4 today; held to the file's own cap).
pub const MAX_BOSS_TICKS: usize = crate::time_limit::MAX_BOSS_TIMES;
/// The most tiers in the `time_limit` mode's elite table (3 today; held to the file's own cap).
pub const MAX_ELITE_TABLE_TIERS: usize = crate::time_limit::MAX_ELITE_TIERS;
/// The most elite types the `time_limit` mode defines (3 today; held to the file's own cap).
pub const MAX_ELITE_TYPE_DEFINITIONS: usize = crate::time_limit::MAX_ELITE_DEFINITIONS;
/// The most bosses in the `time_limit` mode's mid-boss pool (3 today; held to the file's own
/// cap).
pub const MAX_MID_BOSS_POOL: usize = crate::time_limit::MAX_POOL;
/// The most IDs one list of the starting unlocks holds (6 across the lists today; held to the
/// file's own cap).
pub const MAX_STARTING_UNLOCKS: usize = crate::starting_unlocks::MAX_STARTING_UNLOCKS;
/// The most sets of named stars (none today; held to the file's own cap).
pub const MAX_CONSTELLATIONS: usize = crate::constellations::MAX_CONSTELLATIONS;
/// The most IDs one weapon's expertise introduces: its styles, their choices and its common
/// options (15 today).
pub const MAX_EXPERTISE_IDS: usize = 64;

/// The pack file. Each content file is the JSON object its own schema describes.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackFile {
    pub schema_version: u32,
    /// The `sim_version` the pack needs: every file has the same.
    pub sim_version: u32,
    pub introduced: Introduced,
    #[schemars(with = "RunStatsFile")]
    pub run_stats: Box<RawValue>,
    /// Checked against the pack's own stage skills.
    #[schemars(with = "RewardsFile")]
    pub rewards: Box<RawValue>,
    #[schemars(with = "StageSkillsFile")]
    pub stage_skills: Box<RawValue>,
    #[schemars(with = "ExpertiseFile")]
    pub expertise: Box<RawValue>,
    #[schemars(with = "MatchUpsFile")]
    pub match_ups: Box<RawValue>,
    #[schemars(with = "RunesFile")]
    pub runes: Box<RawValue>,
    #[schemars(with = "PerkSetsFile")]
    pub perk_sets: Box<RawValue>,
    #[schemars(with = "DifficultiesFile")]
    pub difficulties: Box<RawValue>,
    /// What a new save begins with (DS5 Q10). Optional: a pack released before it has none, and
    /// reports the starting unlocks as unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<StartingUnlocksFile>")]
    pub starting_unlocks: Option<Box<RawValue>>,
    /// The sets of named stars (core#NS3), checked against the pack's own rewards. Optional: a
    /// pack released before it has no sets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<ConstellationsFile>")]
    pub constellations: Option<Box<RawValue>>,
    pub modes: PackModes,
}

/// The `sim_version` of every ID the pack's files introduce, per file. Each list names each of
/// its file's IDs once, in any order, and nothing else; the run stats', rewards' and stage
/// skills' lists share their file's count budget.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Introduced {
    pub run_stats: Vec<IntroducedId>,
    pub rewards: Vec<IntroducedId>,
    pub stage_skills: Vec<IntroducedId>,
    /// The perk sets (ADR-105).
    pub perk_sets: Vec<IntroducedId>,
    /// The difficulties (ADR-110).
    pub difficulties: Vec<IntroducedId>,
    /// One entry for each weapon of the expertise.
    pub expertise: Vec<WeaponIds>,
    /// The elite types of the `time_limit` mode file (ADR-090).
    pub elite_types: Vec<IntroducedId>,
}

/// An ID and the `sim_version` that introduced it: at most the pack's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntroducedId {
    pub id: ContentId,
    pub sim_version: u32,
}

/// The IDs one weapon's expertise introduces: its styles', their choices' and its common
/// options', at most `MAX_EXPERTISE_IDS`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeaponIds {
    pub weapon: ContentId,
    pub ids: Vec<IntroducedId>,
}

/// The modes' files.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackModes {
    /// The `trial` mode's stage, checked as any stage is.
    #[schemars(with = "Stage")]
    pub trial: Box<RawValue>,
    #[schemars(with = "TimeLimitFile")]
    pub time_limit: Box<RawValue>,
    /// The `tutorial` mode's stage, checked as any stage is (ADR-086).
    #[schemars(with = "Stage")]
    pub tutorial: Box<RawValue>,
}

/// Why a pack was refused. Paths and numbers only, never text from the pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackError {
    /// Over `Pack::MAX_BYTES`; the pack is not parsed.
    TooLarge { size: usize, limit: usize },
    /// Not JSON, or not the pack format (unknown or missing fields, wrong types).
    Parse { line: usize, column: usize },
    /// Made for a newer game.
    UnsupportedVersion {
        schema_version: u32,
        sim_version: u32,
    },
    /// A list over its count budget.
    Budget {
        path: String,
        limit: usize,
        found: usize,
    },
    /// A content file the pack holds was refused by its own parser.
    File { file: &'static str, error: Error },
    /// The `introduced` table does not match the files, or names an ID newer than the pack.
    Introduced(Vec<Issue>),
}

impl PackError {
    fn in_file(file: &'static str) -> impl Fn(Error) -> PackError {
        move |error| PackError::File { file, error }
    }
}

/// Where a file sits in the pack, as a path.
fn location(file: &str) -> String {
    match file {
        "trial" | "time_limit" | "tutorial" => format!("$.modes.{file}"),
        _ => format!("$.{file}"),
    }
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackError::TooLarge { size, limit } => {
                write!(f, "the pack is {size} bytes; the limit is {limit}")
            }
            PackError::Parse { line, column } => {
                write!(f, "not a valid pack (line {line}, column {column})")
            }
            PackError::UnsupportedVersion {
                schema_version,
                sim_version,
            } => write!(
                f,
                "the pack needs a newer version of the game (schema_version {schema_version}, \
                 sim_version {sim_version})"
            ),
            PackError::Introduced(issues) => {
                for (i, issue) in issues.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{}: {}", issue.path, issue.message)?;
                }
                Ok(())
            }
            PackError::Budget { path, limit, found } => {
                write!(f, "{path}: at most {limit} entries (found {found})")
            }
            PackError::File { file, error } => {
                let at = location(file);
                match error {
                    Error::TooLarge { size, limit } => {
                        write!(f, "{at}: {size} bytes; the limit is {limit}")
                    }
                    Error::Parse { line, column } => write!(
                        f,
                        "{at}: not a valid file (line {line}, column {column} of the file)"
                    ),
                    Error::UnsupportedVersion {
                        schema_version,
                        sim_version,
                    } => write!(
                        f,
                        "{at}: not the pack's version (schema_version {schema_version}, \
                         sim_version {sim_version})"
                    ),
                    Error::Invalid(issues) => {
                        for (i, issue) in issues.iter().enumerate() {
                            if i > 0 {
                                writeln!(f)?;
                            }
                            let path = issue.path.strip_prefix('$').unwrap_or(&issue.path);
                            write!(f, "{at}{path}: {}", issue.message)?;
                        }
                        Ok(())
                    }
                }
            }
        }
    }
}

impl std::error::Error for PackError {}

/// A pack that passed every check: its content, ready for a run, and the SHA-256 of its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pack {
    schema_version: u32,
    sim_version: u32,
    content: RunContent,
    sha256: [u8; 32],
}

impl Pack {
    /// The largest pack read (a tuning value; ADR-020 decides that one exists): about 30 times
    /// today's 36 KB of content.
    pub const MAX_BYTES: usize = 1024 * 1024;

    /// The bundled pack file's bytes.
    pub const BUNDLED: &'static [u8] = include_bytes!("../content/pack.json");

    /// Checks the size, the versions, parses strictly, checks the budgets, then reads each file
    /// with its own parser: the one call for a pack's bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Pack, PackError> {
        if bytes.len() > Pack::MAX_BYTES {
            return Err(PackError::TooLarge {
                size: bytes.len(),
                limit: Pack::MAX_BYTES,
            });
        }
        // Versions first: a pack for a newer game may have fields this one rejects.
        #[derive(Deserialize)]
        struct Versions {
            schema_version: u32,
            sim_version: u32,
        }
        let parse_error = |e: serde_json::Error| PackError::Parse {
            line: e.line(),
            column: e.column(),
        };
        let versions: Versions = serde_json::from_slice(bytes).map_err(parse_error)?;
        if versions.schema_version != SCHEMA_VERSION || versions.sim_version != SIM_VERSION {
            return Err(PackError::UnsupportedVersion {
                schema_version: versions.schema_version,
                sim_version: versions.sim_version,
            });
        }
        let pack: PackFile = serde_json::from_slice(bytes).map_err(parse_error)?;
        check_budgets(&pack)?;

        let run_stats = RunStats::parse(&file_bytes(&pack.run_stats))
            .map_err(PackError::in_file("run_stats"))?;
        let stage_skills = StageSkills::parse(&file_bytes(&pack.stage_skills))
            .map_err(PackError::in_file("stage_skills"))?;
        let rewards = Rewards::parse_with(&file_bytes(&pack.rewards), &stage_skills)
            .map_err(PackError::in_file("rewards"))?;
        let rewards = match pack.constellations.as_deref() {
            Some(raw) => {
                let sets = Constellations::parse_with(&file_bytes(raw), &rewards)
                    .map_err(PackError::in_file("constellations"))?;
                rewards.with_constellations(sets)
            }
            None => rewards,
        };
        let expertise = Expertise::parse(&file_bytes(&pack.expertise))
            .map_err(PackError::in_file("expertise"))?;
        let match_ups = MatchUps::parse(&file_bytes(&pack.match_ups))
            .map_err(PackError::in_file("match_ups"))?;
        let runes = Runes::parse(&file_bytes(&pack.runes)).map_err(PackError::in_file("runes"))?;
        let perk_sets = PerkSets::parse(&file_bytes(&pack.perk_sets))
            .map_err(PackError::in_file("perk_sets"))?;
        let difficulties = Difficulties::parse(&file_bytes(&pack.difficulties))
            .map_err(PackError::in_file("difficulties"))?;
        let starting_unlocks = pack
            .starting_unlocks
            .as_deref()
            .map(|raw| {
                StartingUnlocks::parse_with(&file_bytes(raw), &perk_sets, &stage_skills, &runes)
            })
            .transpose()
            .map_err(PackError::in_file("starting_unlocks"))?;
        let trial = validate::parse_and_validate(&file_bytes(&pack.modes.trial))
            .map_err(PackError::in_file("trial"))?;
        let time_limit = TimeLimit::parse(&file_bytes(&pack.modes.time_limit))
            .map_err(PackError::in_file("time_limit"))?;
        let tutorial = validate::parse_and_validate(&file_bytes(&pack.modes.tutorial))
            .map_err(PackError::in_file("tutorial"))?;
        // Its bosses are checked against the trial stage they are placed in.
        crate::run_setup::check_bosses(&trial, &time_limit)
            .map_err(PackError::in_file("time_limit"))?;

        check_introduced(
            &pack.introduced,
            pack.sim_version,
            &run_stats,
            &rewards,
            &stage_skills,
            &expertise,
            &perk_sets,
            &difficulties,
            &time_limit,
        )?;

        let sha256: [u8; 32] = Sha256::digest(bytes).into();
        let content = RunContent::from_parts(
            run_stats,
            rewards,
            expertise,
            match_ups,
            runes,
            perk_sets,
            difficulties,
            starting_unlocks,
            time_limit,
            trial,
            file_bytes(&pack.modes.trial),
            tutorial,
            file_bytes(&pack.modes.tutorial),
            Some(sha256),
        );
        Ok(Pack {
            schema_version: pack.schema_version,
            sim_version: pack.sim_version,
            content,
            sha256,
        })
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// The `sim_version` the pack needs.
    pub fn sim_version(&self) -> u32 {
        self.sim_version
    }

    /// The content a run reads.
    pub fn content(&self) -> &RunContent {
        &self.content
    }

    /// The bundled pack, `content/pack.json`: the default content of every run (ADR-080). It is
    /// our own file, tested to be what `export_bundled` writes, so a failure to read it is a
    /// bug, and stops at the first use.
    pub fn bundled() -> &'static Pack {
        static BUNDLED: OnceLock<Pack> = OnceLock::new();
        BUNDLED.get_or_init(|| Pack::from_bytes(Pack::BUNDLED).expect("the bundled pack"))
    }

    /// What a new save begins with, `None` for a pack released without the file: unknown,
    /// never a guessed list.
    pub fn starting_unlocks(&self) -> Option<&StartingUnlocks> {
        self.content.starting_unlocks()
    }

    /// The `trial` mode's stage.
    pub fn trial(&self) -> &Stage {
        self.content.trial()
    }

    /// The SHA-256 of the pack's bytes.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

impl From<Pack> for RunContent {
    fn from(pack: Pack) -> RunContent {
        pack.content
    }
}

/// A file's bytes as it has them in the pack, and a final newline: the file on its own.
fn file_bytes(raw: &RawValue) -> Vec<u8> {
    [raw.get().as_bytes(), b"\n"].concat()
}

/// The counts of a file's lists, read leniently: a file of the wrong shape is left to its own
/// parser, which refuses it.
fn counts<'a, T: Deserialize<'a>>(raw: &'a RawValue) -> Option<T> {
    serde_json::from_str(raw.get()).ok()
}

#[derive(Deserialize)]
struct ConstellationLists {
    #[serde(default)]
    constellations: Vec<IgnoredAny>,
}

#[derive(Deserialize)]
struct Lists {
    #[serde(default)]
    stats: Vec<IgnoredAny>,
    #[serde(default)]
    rewards: Vec<IgnoredAny>,
    #[serde(default)]
    stage_skills: Vec<IgnoredAny>,
    #[serde(default)]
    perk_sets: Vec<IgnoredAny>,
    #[serde(default)]
    difficulties: Vec<IgnoredAny>,
    #[serde(default)]
    weapons: Vec<WeaponLists>,
    #[serde(default)]
    on_hit: Vec<IgnoredAny>,
    #[serde(default)]
    damage_over_time: Vec<IgnoredAny>,
    #[serde(default)]
    waves: Vec<IgnoredAny>,
    #[serde(default)]
    boss_ticks: Vec<IgnoredAny>,
    #[serde(default)]
    mid_boss_pool: Vec<IgnoredAny>,
    #[serde(default)]
    elite_table: Vec<IgnoredAny>,
    #[serde(default)]
    elite_types: Vec<IgnoredAny>,
}

/// The lists of the starting unlocks file, counted.
#[derive(Deserialize)]
struct UnlockLists {
    #[serde(default)]
    weapons: Vec<IgnoredAny>,
    #[serde(default)]
    perk_sets: Vec<IgnoredAny>,
    #[serde(default)]
    stage_skills: Vec<IgnoredAny>,
    #[serde(default)]
    runes: Vec<IgnoredAny>,
}

#[derive(Deserialize)]
struct WeaponLists {
    #[serde(default)]
    common: Vec<IgnoredAny>,
}

fn budget(path: impl Into<String>, found: usize, limit: usize) -> Result<(), PackError> {
    if found > limit {
        Err(PackError::Budget {
            path: path.into(),
            limit,
            found,
        })
    } else {
        Ok(())
    }
}

fn check_budgets(pack: &PackFile) -> Result<(), PackError> {
    let introduced = &pack.introduced;
    budget(
        "$.introduced.run_stats",
        introduced.run_stats.len(),
        MAX_RUN_STATS,
    )?;
    budget(
        "$.introduced.rewards",
        introduced.rewards.len(),
        MAX_REWARDS,
    )?;
    budget(
        "$.introduced.stage_skills",
        introduced.stage_skills.len(),
        MAX_STAGE_SKILLS,
    )?;
    budget(
        "$.introduced.perk_sets",
        introduced.perk_sets.len(),
        MAX_PERK_SETS,
    )?;
    budget(
        "$.introduced.difficulties",
        introduced.difficulties.len(),
        MAX_DIFFICULTIES,
    )?;
    budget(
        "$.introduced.expertise",
        introduced.expertise.len(),
        MAX_EXPERTISE_WEAPONS,
    )?;
    for (w, weapon) in introduced.expertise.iter().enumerate() {
        budget(
            format!("$.introduced.expertise[{w}].ids"),
            weapon.ids.len(),
            MAX_EXPERTISE_IDS,
        )?;
    }
    if let Some(l) = counts::<Lists>(&pack.run_stats) {
        budget("$.run_stats.stats", l.stats.len(), MAX_RUN_STATS)?;
    }
    if let Some(l) = counts::<Lists>(&pack.rewards) {
        budget("$.rewards.rewards", l.rewards.len(), MAX_REWARDS)?;
    }
    if let Some(l) = counts::<Lists>(&pack.stage_skills) {
        budget(
            "$.stage_skills.stage_skills",
            l.stage_skills.len(),
            MAX_STAGE_SKILLS,
        )?;
    }
    if let Some(l) = counts::<Lists>(&pack.expertise) {
        budget(
            "$.expertise.weapons",
            l.weapons.len(),
            MAX_EXPERTISE_WEAPONS,
        )?;
        for (w, weapon) in l.weapons.iter().enumerate() {
            budget(
                format!("$.expertise.weapons[{w}].common"),
                weapon.common.len(),
                MAX_EXPERTISE_COMMON,
            )?;
        }
    }
    if let Some(l) = counts::<Lists>(&pack.runes) {
        budget("$.runes.on_hit", l.on_hit.len(), MAX_ON_HIT_RUNES)?;
        budget(
            "$.runes.damage_over_time",
            l.damage_over_time.len(),
            MAX_DAMAGE_OVER_TIME_RUNES,
        )?;
    }
    if let Some(l) = counts::<Lists>(&pack.perk_sets) {
        budget("$.perk_sets.perk_sets", l.perk_sets.len(), MAX_PERK_SETS)?;
    }
    if let Some(l) = counts::<Lists>(&pack.difficulties) {
        budget(
            "$.difficulties.difficulties",
            l.difficulties.len(),
            MAX_DIFFICULTIES,
        )?;
    }
    if let Some(l) = pack
        .starting_unlocks
        .as_deref()
        .and_then(counts::<UnlockLists>)
    {
        for (name, found) in [
            ("weapons", l.weapons.len()),
            ("perk_sets", l.perk_sets.len()),
            ("stage_skills", l.stage_skills.len()),
            ("runes", l.runes.len()),
        ] {
            budget(
                format!("$.starting_unlocks.{name}"),
                found,
                MAX_STARTING_UNLOCKS,
            )?;
        }
    }
    if let Some(l) = pack
        .constellations
        .as_deref()
        .and_then(counts::<ConstellationLists>)
    {
        budget(
            "$.constellations.constellations",
            l.constellations.len(),
            MAX_CONSTELLATIONS,
        )?;
    }
    if let Some(l) = counts::<Lists>(&pack.modes.time_limit) {
        budget(
            "$.modes.time_limit.waves",
            l.waves.len(),
            MAX_TIME_LIMIT_WAVES,
        )?;
        budget(
            "$.modes.time_limit.boss_ticks",
            l.boss_ticks.len(),
            MAX_BOSS_TICKS,
        )?;
        budget(
            "$.modes.time_limit.mid_boss_pool",
            l.mid_boss_pool.len(),
            MAX_MID_BOSS_POOL,
        )?;
        budget(
            "$.modes.time_limit.elite_table",
            l.elite_table.len(),
            MAX_ELITE_TABLE_TIERS,
        )?;
        budget(
            "$.modes.time_limit.elite_types",
            l.elite_types.len(),
            MAX_ELITE_TYPE_DEFINITIONS,
        )?;
    }
    Ok(())
}

/// Every ID one weapon's expertise introduces, with its path in the expertise file, in the
/// file's order.
fn expertise_ids(w: usize, weapon: &WeaponExpertise) -> Vec<(String, &ContentId)> {
    let mut ids = Vec::new();
    for (s, style) in weapon.styles.iter().enumerate() {
        let base = format!("$.expertise.weapons[{w}].styles[{s}]");
        ids.push((format!("{base}.id"), &style.id));
        for (field, choices) in [
            ("upgrade_1", &style.upgrade_1),
            ("upgrade_2", &style.upgrade_2),
            ("capstone", &style.capstone),
        ] {
            for (c, choice) in choices.iter().enumerate() {
                ids.push((format!("{base}.{field}[{c}].id"), &choice.id));
            }
        }
    }
    for (c, option) in weapon.common.iter().enumerate() {
        ids.push((
            format!("$.expertise.weapons[{w}].common[{c}].id"),
            &option.id,
        ));
    }
    ids
}

/// Checks one list of the table, at `at`, against the IDs of its file with their paths.
fn check_ids(
    issues: &mut Vec<Issue>,
    at: &str,
    listed: &[IntroducedId],
    ids: &[(String, &ContentId)],
    sim_version: u32,
) {
    for (i, entry) in listed.iter().enumerate() {
        if !ids.iter().any(|(_, id)| **id == entry.id) {
            issues.push(Issue {
                path: format!("{at}[{i}].id"),
                message: "not an ID of the file".into(),
            });
        } else if listed[..i].iter().any(|e| e.id == entry.id) {
            issues.push(Issue {
                path: format!("{at}[{i}].id"),
                message: "the same ID as an earlier entry".into(),
            });
        }
        if entry.sim_version > sim_version {
            issues.push(Issue {
                path: format!("{at}[{i}].sim_version"),
                message: format!(
                    "introduced in sim_version {}, newer than the pack's {sim_version}",
                    entry.sim_version
                ),
            });
        }
    }
    for (path, id) in ids {
        if !listed.iter().any(|e| e.id == **id) {
            issues.push(Issue {
                path: path.clone(),
                message: format!("not listed in {at}"),
            });
        }
    }
}

/// `ids` with their paths, the `i`th at `$.{list}[i].id`.
fn paths<'a>(list: &str, ids: impl Iterator<Item = &'a ContentId>) -> Vec<(String, &'a ContentId)> {
    ids.enumerate()
        .map(|(i, id)| (format!("$.{list}[{i}].id"), id))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn check_introduced(
    introduced: &Introduced,
    sim_version: u32,
    run_stats: &RunStats,
    rewards: &Rewards,
    stage_skills: &StageSkills,
    expertise: &Expertise,
    perk_sets: &PerkSets,
    difficulties: &Difficulties,
    time_limit: &TimeLimit,
) -> Result<(), PackError> {
    let mut issues = Vec::new();
    let stats = paths("run_stats.stats", run_stats.stats().iter().map(|s| &s.id));
    let reward_ids = paths("rewards.rewards", rewards.rewards().iter().map(|r| &r.id));
    let skills = paths(
        "stage_skills.stage_skills",
        stage_skills.skills().iter().map(|s| &s.id),
    );
    for (at, listed, ids) in [
        ("$.introduced.run_stats", &introduced.run_stats, &stats),
        ("$.introduced.rewards", &introduced.rewards, &reward_ids),
        (
            "$.introduced.stage_skills",
            &introduced.stage_skills,
            &skills,
        ),
    ] {
        check_ids(&mut issues, at, listed, ids, sim_version);
    }
    let sets = paths(
        "perk_sets.perk_sets",
        perk_sets.sets().iter().map(|s| &s.id),
    );
    check_ids(
        &mut issues,
        "$.introduced.perk_sets",
        &introduced.perk_sets,
        &sets,
        sim_version,
    );
    let difficulty_ids = paths(
        "difficulties.difficulties",
        difficulties.list().iter().map(|d| &d.id),
    );
    check_ids(
        &mut issues,
        "$.introduced.difficulties",
        &introduced.difficulties,
        &difficulty_ids,
        sim_version,
    );
    let elites = paths(
        "modes.time_limit.elite_types",
        time_limit.elite_types().iter().map(|t| &t.id),
    );
    check_ids(
        &mut issues,
        "$.introduced.elite_types",
        &introduced.elite_types,
        &elites,
        sim_version,
    );
    for (k, entry) in introduced.expertise.iter().enumerate() {
        let at = format!("$.introduced.expertise[{k}]");
        let found = expertise
            .weapons()
            .iter()
            .position(|w| w.weapon == entry.weapon);
        if introduced.expertise[..k]
            .iter()
            .any(|e| e.weapon == entry.weapon)
        {
            issues.push(Issue {
                path: format!("{at}.weapon"),
                message: "the same weapon as an earlier entry".into(),
            });
        } else if let Some(w) = found {
            let ids = expertise_ids(w, &expertise.weapons()[w]);
            check_ids(
                &mut issues,
                &format!("{at}.ids"),
                &entry.ids,
                &ids,
                sim_version,
            );
        } else {
            issues.push(Issue {
                path: format!("{at}.weapon"),
                message: "not a weapon of the expertise".into(),
            });
        }
    }
    for (w, weapon) in expertise.weapons().iter().enumerate() {
        if !introduced
            .expertise
            .iter()
            .any(|e| e.weapon == weapon.weapon)
        {
            issues.push(Issue {
                path: format!("$.expertise.weapons[{w}].weapon"),
                message: "not listed in $.introduced.expertise".into(),
            });
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(PackError::Introduced(issues))
    }
}

/// The table of `content`'s IDs, each introduced in `sim_version`.
fn introduced_at(content: &RunContent, sim_version: u32) -> Introduced {
    let list = |ids: Vec<&ContentId>| -> Vec<IntroducedId> {
        ids.into_iter()
            .map(|id| IntroducedId {
                id: id.clone(),
                sim_version,
            })
            .collect()
    };
    Introduced {
        run_stats: list(content.run_stats().stats().iter().map(|s| &s.id).collect()),
        rewards: list(content.rewards().rewards().iter().map(|r| &r.id).collect()),
        stage_skills: list(
            content
                .stage_skills()
                .skills()
                .iter()
                .map(|s| &s.id)
                .collect(),
        ),
        expertise: content
            .expertise()
            .weapons()
            .iter()
            .enumerate()
            .map(|(w, weapon)| WeaponIds {
                weapon: weapon.weapon.clone(),
                ids: list(
                    expertise_ids(w, weapon)
                        .into_iter()
                        .map(|(_, id)| id)
                        .collect(),
                ),
            })
            .collect(),
        perk_sets: list(content.perk_sets().sets().iter().map(|s| &s.id).collect()),
        difficulties: list(
            content
                .difficulties()
                .list()
                .iter()
                .map(|d| &d.id)
                .collect(),
        ),
        elite_types: list(
            content
                .time_limit()
                .elite_types()
                .iter()
                .map(|t| &t.id)
                .collect(),
        ),
    }
}

/// The bundled content files as a pack: what the content repository's first pack is made
/// from. Each file is written as it is, without its final newline, so that the pack gives the
/// bundled files' hashes (see the module's documentation).
pub fn export_bundled() -> Vec<u8> {
    let file = |bytes: &[u8]| String::from_utf8_lossy(bytes.trim_ascii_end()).into_owned();
    let trial: &[u8] = include_bytes!("../modes/trial.json");
    let tutorial: &[u8] = include_bytes!("../modes/tutorial.json");
    let mut out = String::new();
    out.push_str(&format!(
        "{{\n  \"schema_version\": {SCHEMA_VERSION},\n  \"sim_version\": {SIM_VERSION},\n"
    ));
    // Every bundled ID is version 0's (ADR-035).
    let starting_unlocks = StartingUnlocks::parse_with(
        StartingUnlocks::BUNDLED,
        &PerkSets::parse(PerkSets::BUNDLED).expect("the bundled perk sets"),
        &StageSkills::parse(StageSkills::BUNDLED).expect("the bundled stage skills"),
        &Runes::parse(Runes::BUNDLED).expect("the bundled runes"),
    )
    .expect("the bundled starting unlocks");
    let content = RunContent::from_parts(
        RunStats::parse(RunStats::BUNDLED).expect("the bundled run stats"),
        Rewards::parse_with(
            Rewards::BUNDLED,
            &StageSkills::parse(StageSkills::BUNDLED).expect("the bundled stage skills"),
        )
        .expect("the bundled rewards"),
        Expertise::parse(Expertise::BUNDLED).expect("the bundled expertise"),
        MatchUps::parse(MatchUps::BUNDLED).expect("the bundled match-ups"),
        Runes::parse(Runes::BUNDLED).expect("the bundled runes"),
        PerkSets::parse(PerkSets::BUNDLED).expect("the bundled perk sets"),
        Difficulties::parse(Difficulties::BUNDLED).expect("the bundled difficulties"),
        Some(starting_unlocks),
        TimeLimit::parse(TimeLimit::BUNDLED).expect("the bundled mode"),
        validate::parse_and_validate(trial).expect("the bundled trial"),
        trial.to_vec(),
        validate::parse_and_validate(tutorial).expect("the bundled tutorial"),
        tutorial.to_vec(),
        None,
    );
    let introduced = introduced_at(&content, SIM_VERSION);
    let table = serde_json::to_string_pretty(&introduced).expect("the table serialises");
    out.push_str(&format!(
        "  \"introduced\": {},\n",
        table.replace('\n', "\n  ")
    ));
    for (name, bytes) in [
        ("run_stats", RunStats::BUNDLED),
        ("rewards", Rewards::BUNDLED),
        ("stage_skills", StageSkills::BUNDLED),
        ("expertise", Expertise::BUNDLED),
        ("match_ups", MatchUps::BUNDLED),
        ("runes", Runes::BUNDLED),
        ("perk_sets", PerkSets::BUNDLED),
        ("difficulties", Difficulties::BUNDLED),
    ] {
        out.push_str(&format!("  \"{name}\": {},\n", file(bytes)));
    }
    out.push_str(&format!(
        "  \"starting_unlocks\": {},\n",
        file(StartingUnlocks::BUNDLED)
    ));
    out.push_str(&format!(
        "  \"modes\": {{\n    \"trial\": {},\n    \"time_limit\": {},\n    \"tutorial\": {}\n  }}\n}}\n",
        file(trial),
        file(TimeLimit::BUNDLED),
        file(tutorial)
    ));
    out.into_bytes()
}
