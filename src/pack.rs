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
//! a style or option of a weapon's expertise, an elite type of the `time_limit` mode, or a constellation (listed only when the pack has the list), records the `sim_version` that introduced it in
//! the pack's `introduced` table, each exactly once, and is refused when that is newer than the
//! pack's own (ADR-080, as the registry's IDs are for a stage, ADR-017). An ID the released
//! content (the bundled pack) does not hold is new and is recorded at the pack's `sim_version`,
//! and is refused when it is recorded lower (ADR-REPL-7, ADR-CONT-2). The registry's IDs a
//! file names are checked against the registry by the file's own parser.
//!
//! The content files have no versions of their own: the pack's header is the only one, and each
//! file is read at it (CS1a-2). The stages in `modes` are stages and keep theirs.
//!
//! Each file sits in the pack as a JSON object, and is read from the exact bytes it has there,
//! followed by a newline: the bytes of the same file on its own. A pack exported from the
//! bundled files (`ContentFiles::export`) so gives each file the bundled file's SHA-256, and a run of
//! its content the bundled content hash.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

pub mod trust;

use crate::attack_presets::{AttackPresets, AttackPresetsFile};
use crate::constellations::{Constellations, ConstellationsFile};
use crate::difficulties::{Difficulties, DifficultiesFile};
use crate::expertise::{Expertise, ExpertiseFile, WeaponExpertise};
use crate::id::ContentId;
use crate::match_ups::{MatchUps, MatchUpsFile};
use crate::perk_sets::{PerkSets, PerkSetsFile};
use crate::rewards::{Rewards, RewardsFile};
use crate::rules::{Rules, RulesFile};
use crate::run_content::RunContent;
use crate::run_stats::{RunStats, RunStatsFile};
use crate::runes::{Runes, RunesFile};
use crate::stage::Stage;
use crate::stage_skill::{StageSkills, StageSkillsFile};
use crate::stardust::{Stardust, StardustFile};
use crate::starting_unlocks::{StartingUnlocks, StartingUnlocksFile};
use crate::time_limit::{TimeLimit, TimeLimitFile};
use crate::validate::{self, Error, Issue};
use crate::version::{self, ReadError, SCHEMA_VERSION, SIM_VERSION, Versions};
use crate::weapons::{Weapons, WeaponsFile};

// The count budgets: about four times today's counts, rounded up to a multiple of 8 and held
// to the file's own cap where that is lower (tuning values; ADR-020 decides that they exist).
// They are checked on the pack before any file is read.

/// The most run stats (5 today; the file's own cap is 32).
pub const MAX_RUN_STATS: usize = 24;
/// The most rewards (23 today; held to the file's own cap).
pub const MAX_REWARDS: usize = crate::rewards::MAX_REWARDS;
/// The rarities a rewards file prices (every rarity once).
pub const MAX_RARITIES: usize = crate::rewards::MAX_RARITIES;
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
/// The most modes the Stardust file has entries for (2 today; held to the file's own cap).
pub const MAX_STARDUST_MODES: usize = crate::stardust::MAX_MODES;
/// The most attack presets the file lists (9 today; the file's own cap is 64).
pub const MAX_ATTACK_PRESETS: usize = 40;
/// The most weapons the weapons file lists (1 today; held to the file's own cap).
pub const MAX_WEAPONS: usize = crate::weapons::MAX_WEAPONS;
/// The most trees one weapon has (1 today; held to the file's own cap).
pub const MAX_TREES: usize = crate::weapons::MAX_TREES;
/// The most sets of named stars (none today; held to the file's own cap).
pub const MAX_CONSTELLATIONS: usize = crate::constellations::MAX_CONSTELLATIONS;
/// The most IDs one weapon's expertise introduces: its styles, their choices and its common
/// options (15 today).
pub const MAX_EXPERTISE_IDS: usize = 64;

/// The members of a pack, borrowed from its bytes: `ContentFiles::bundled` reads the bundled pack
/// with it and allocates nothing but its result.
#[derive(Deserialize)]
struct BorrowedPack<'a> {
    #[serde(borrow)]
    run_stats: &'a RawValue,
    #[serde(borrow)]
    rewards: &'a RawValue,
    #[serde(borrow)]
    stage_skills: &'a RawValue,
    #[serde(borrow)]
    expertise: &'a RawValue,
    #[serde(borrow)]
    match_ups: &'a RawValue,
    #[serde(borrow)]
    runes: &'a RawValue,
    #[serde(borrow)]
    perk_sets: &'a RawValue,
    #[serde(borrow)]
    difficulties: &'a RawValue,
    #[serde(borrow)]
    attack_presets: &'a RawValue,
    #[serde(borrow)]
    weapons: &'a RawValue,
    #[serde(borrow)]
    rules: &'a RawValue,
    #[serde(borrow, default)]
    starting_unlocks: Option<&'a RawValue>,
    #[serde(borrow, default)]
    constellations: Option<&'a RawValue>,
    #[serde(borrow, default)]
    stardust: Option<&'a RawValue>,
    #[serde(borrow)]
    modes: BorrowedModes<'a>,
}

#[derive(Deserialize)]
struct BorrowedModes<'a> {
    #[serde(borrow)]
    trial: &'a RawValue,
    #[serde(borrow)]
    time_limit: &'a RawValue,
    #[serde(borrow)]
    tutorial: &'a RawValue,
}

/// The pack file. Each content file is the JSON object its own schema describes.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackFile {
    pub schema_version: u32,
    /// The `sim_version` the pack needs: the version of every file it holds; the stages in
    /// `modes` also carry it, as every stage does.
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
    /// The attack presets (CS1b-1). Required: the rules read them.
    #[schemars(with = "AttackPresetsFile")]
    pub attack_presets: Box<RawValue>,
    /// The weapons' combo trees (WP1).
    #[schemars(with = "WeaponsFile")]
    pub weapons: Box<RawValue>,
    /// The crowd target and the cap's headroom (DIR1). Required: the simulation reads it.
    #[schemars(with = "RulesFile")]
    pub rules: Box<RawValue>,
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
    /// The rules the app pays a failed run's Stardust by (ADR-101 point 4). Optional: a pack
    /// released before it yields the bundled file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<StardustFile>")]
    pub stardust: Option<Box<RawValue>>,
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
    /// The constellations (core#NS4). Optional: a pack released with the file but before the
    /// list has none, stays valid and records no version for its sets. A list names each set of
    /// the pack's file once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constellations: Option<Vec<IntroducedId>>,
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
    /// A file the pack needs is not among the files (`ContentFiles::export` only).
    Missing { file: &'static str },
    /// The pack is valid but is not the bytes `ContentFiles::export` writes for its files
    /// (`ContentFiles::from_pack`); `path` is the first member that differs.
    NotExportForm { path: String },
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

impl From<ReadError> for PackError {
    fn from(e: ReadError) -> PackError {
        match e {
            ReadError::TooLarge { size, limit } => PackError::TooLarge { size, limit },
            ReadError::Parse { line, column } => PackError::Parse { line, column },
            ReadError::UnsupportedVersion {
                schema_version,
                sim_version,
            } => PackError::UnsupportedVersion {
                schema_version,
                sim_version,
            },
        }
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
            PackError::Missing { file } => write!(f, "{}: the file is missing", location(file)),
            PackError::NotExportForm { path } => write!(
                f,
                "{path}: not written as `content-pack export` writes it (LF, the files as they are)"
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
        let pack: PackFile = version::read_current(bytes, Pack::MAX_BYTES)?;
        check_budgets(&pack)?;
        let versions = Versions {
            schema_version: pack.schema_version,
            sim_version: pack.sim_version,
        };

        let run_stats = RunStats::parse(&file_bytes(&pack.run_stats), versions)
            .map_err(PackError::in_file("run_stats"))?;
        let stage_skills = StageSkills::parse(&file_bytes(&pack.stage_skills), versions)
            .map_err(PackError::in_file("stage_skills"))?;
        let rewards = Rewards::parse_with(&file_bytes(&pack.rewards), versions, &stage_skills)
            .map_err(PackError::in_file("rewards"))?;
        let rewards = match pack.constellations.as_deref() {
            Some(raw) => {
                let sets = Constellations::parse_with(&file_bytes(raw), versions, &rewards)
                    .map_err(PackError::in_file("constellations"))?;
                rewards.with_constellations(sets)
            }
            None => rewards,
        };
        let expertise = Expertise::parse(&file_bytes(&pack.expertise), versions)
            .map_err(PackError::in_file("expertise"))?;
        let match_ups = MatchUps::parse(&file_bytes(&pack.match_ups), versions)
            .map_err(PackError::in_file("match_ups"))?;
        let runes = Runes::parse(&file_bytes(&pack.runes), versions)
            .map_err(PackError::in_file("runes"))?;
        let perk_sets = PerkSets::parse(&file_bytes(&pack.perk_sets), versions)
            .map_err(PackError::in_file("perk_sets"))?;
        let difficulties = Difficulties::parse(&file_bytes(&pack.difficulties), versions)
            .map_err(PackError::in_file("difficulties"))?;
        let attack_presets = AttackPresets::parse(&file_bytes(&pack.attack_presets), versions)
            .map_err(PackError::in_file("attack_presets"))?;
        let weapons = Weapons::parse(&file_bytes(&pack.weapons), versions)
            .map_err(PackError::in_file("weapons"))?;
        check_holds(&weapons, &attack_presets)?;
        let rules = Rules::parse(&file_bytes(&pack.rules), versions)
            .map_err(PackError::in_file("rules"))?;
        let starting_unlocks = pack
            .starting_unlocks
            .as_deref()
            .map(|raw| {
                StartingUnlocks::parse_with(
                    &file_bytes(raw),
                    versions,
                    &perk_sets,
                    &stage_skills,
                    &runes,
                    &weapons,
                )
            })
            .transpose()
            .map_err(PackError::in_file("starting_unlocks"))?;
        let stardust = pack
            .stardust
            .as_deref()
            .map(|raw| Stardust::parse(&file_bytes(raw), versions))
            .transpose()
            .map_err(PackError::in_file("stardust"))?
            // Never re-entered: the bundled pack carries `stardust`, so loading it does not
            // reach `Pack::bundled()`, whose `OnceLock` would deadlock.
            .unwrap_or_else(|| Stardust::bundled().clone());
        let trial = validate::parse_and_validate(&file_bytes(&pack.modes.trial))
            .map_err(PackError::in_file("trial"))?;
        let time_limit = TimeLimit::parse(&file_bytes(&pack.modes.time_limit), versions)
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
            attack_presets,
            weapons,
            rules,
            starting_unlocks,
            stardust,
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
    /// our own file, tested to be what `ContentFiles::read_dir` of `content/` exports, so a failure to read it is a
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

    /// The Stardust rules (ADR-101 point 4): the pack's file, or the bundled one for a pack
    /// released without it.
    pub fn stardust(&self) -> &Stardust {
        self.content.stardust()
    }

    /// The `trial` mode's stage.
    pub fn trial(&self) -> &Stage {
        self.content.trial()
    }

    /// The SHA-256 of the pack's bytes.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }

    /// Checks that every shown content ID in this pack has a corresponding entry in `text`.
    pub fn check_text(&self, text: &crate::text::TextTable) -> Result<(), Vec<String>> {
        text.check_pack(self)
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
struct ModeLists {
    #[serde(default)]
    modes: Vec<IgnoredAny>,
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
    rarities: Vec<IgnoredAny>,
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

/// The lists of the weapons file, counted.
#[derive(Deserialize)]
struct AttackPresetsLists {
    #[serde(default)]
    presets: Vec<IgnoredAny>,
}

/// The lists of the weapons file, counted.
#[derive(Deserialize)]
struct WeaponsLists {
    #[serde(default)]
    weapons: Vec<TreeLists>,
}

#[derive(Deserialize)]
struct TreeLists {
    #[serde(default)]
    trees: Vec<IgnoredAny>,
}

#[derive(Deserialize)]
struct WeaponLists {
    #[serde(default)]
    common: Vec<IgnoredAny>,
}

/// The held-step checks that read a preset's phases, for every tree of the pack's weapons against
/// the pack's own attack presets (ADR-CONT-7 point 6, CS1b-1): the weapons file alone cannot say.
fn check_holds(weapons: &Weapons, presets: &AttackPresets) -> Result<(), PackError> {
    let mut issues = Vec::new();
    for (w, entry) in weapons.list().iter().enumerate() {
        for (t, tree) in entry.trees.iter().enumerate() {
            for found in crate::presets::hold_problems(&tree.weapon, presets) {
                issues.push(validate::Issue {
                    path: format!(
                        "$.weapons[{w}].trees[{t}].weapon{}",
                        found.path.trim_start_matches('$')
                    ),
                    message: found.message,
                });
            }
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(PackError::File {
            file: "weapons",
            error: validate::Error::Invalid(issues),
        })
    }
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
    if let Some(listed) = &introduced.constellations {
        budget(
            "$.introduced.constellations",
            listed.len(),
            MAX_CONSTELLATIONS,
        )?;
    }
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
        budget("$.rewards.rarities", l.rarities.len(), MAX_RARITIES)?;
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
    if let Some(l) = counts::<AttackPresetsLists>(&pack.attack_presets) {
        budget(
            "$.attack_presets.presets",
            l.presets.len(),
            MAX_ATTACK_PRESETS,
        )?;
    }
    if let Some(l) = counts::<WeaponsLists>(&pack.weapons) {
        budget("$.weapons.weapons", l.weapons.len(), MAX_WEAPONS)?;
        for (w, weapon) in l.weapons.iter().enumerate() {
            budget(
                format!("$.weapons.weapons[{w}].trees"),
                weapon.trees.len(),
                MAX_TREES,
            )?;
        }
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
    if let Some(l) = pack.stardust.as_deref().and_then(counts::<ModeLists>) {
        budget("$.stardust.modes", l.modes.len(), MAX_STARDUST_MODES)?;
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

/// The `introduced` table of the bundled pack: the IDs the released content holds. Read without
/// the version check (`Pack::bundled` calls `check_introduced`, which calls this).
fn released() -> &'static Introduced {
    static RELEASED: OnceLock<Introduced> = OnceLock::new();
    RELEASED.get_or_init(|| {
        serde_json::from_slice::<PackFile>(Pack::BUNDLED)
            .expect("the bundled pack parses")
            .introduced
    })
}

/// The released entries of one weapon's expertise; none for a weapon the release lacks.
fn released_expertise(weapon: &ContentId) -> &'static [IntroducedId] {
    released()
        .expertise
        .iter()
        .find(|e| e.weapon == *weapon)
        .map_or(&[], |e| e.ids.as_slice())
}

/// Checks one list of the table, at `at`, against the IDs of its file with their paths. An ID
/// the `released` entries (the bundled pack's) do not hold is new, and is recorded at the pack's
/// own `sim_version` (ADR-REPL-7, ADR-CONT-2); a released ID keeps the version it was released at.
fn check_ids(
    issues: &mut Vec<Issue>,
    at: &str,
    listed: &[IntroducedId],
    ids: &[(String, &ContentId)],
    sim_version: u32,
    released: &[IntroducedId],
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
        } else if entry.sim_version < sim_version && !released.iter().any(|r| r.id == entry.id) {
            issues.push(Issue {
                path: format!("{at}[{i}].sim_version"),
                message: format!(
                    "a new ID is introduced in the pack's sim_version {sim_version}, not {}",
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
    for (at, listed, ids, released) in [
        (
            "$.introduced.run_stats",
            &introduced.run_stats,
            &stats,
            &released().run_stats,
        ),
        (
            "$.introduced.rewards",
            &introduced.rewards,
            &reward_ids,
            &released().rewards,
        ),
        (
            "$.introduced.stage_skills",
            &introduced.stage_skills,
            &skills,
            &released().stage_skills,
        ),
    ] {
        check_ids(&mut issues, at, listed, ids, sim_version, released);
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
        &released().perk_sets,
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
        &released().difficulties,
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
        &released().elite_types,
    );
    if let Some(listed) = &introduced.constellations {
        let sets = paths(
            "constellations.constellations",
            rewards.constellations().sets().iter().map(|c| &c.id),
        );
        check_ids(
            &mut issues,
            "$.introduced.constellations",
            listed,
            &sets,
            sim_version,
            released().constellations.as_deref().unwrap_or(&[]),
        );
    }
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
                released_expertise(&entry.weapon),
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
        constellations: Some(list(
            content
                .constellations()
                .sets()
                .iter()
                .map(|c| &c.id)
                .collect(),
        ))
        .filter(|l| !l.is_empty()),
    }
}

/// Where a content file sits in core's tree, and whether a pack may leave it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileSlot {
    /// The pack's key for the file, and its name there: `<name>.json`.
    pub name: &'static str,
    /// `modes/` (beside `content/`) rather than `content/`.
    pub in_modes: bool,
    /// A pack released before the file may lack it.
    pub optional: bool,
}

impl FileSlot {
    /// Where the file is in core's tree, given its `content/` directory: `<name>.json` there, or
    /// in `modes/` beside it.
    pub fn path(&self, content_dir: &Path) -> PathBuf {
        let dir = if self.in_modes {
            content_dir.join("..").join("modes")
        } else {
            content_dir.to_path_buf()
        };
        dir.join(format!("{}.json", self.name))
    }
}

const fn slot(name: &'static str, in_modes: bool, optional: bool) -> FileSlot {
    FileSlot {
        name,
        in_modes,
        optional,
    }
}

/// The content files of a pack, as the bytes of the file on its own (a final newline included):
/// what `content/` and `modes/` hold, and what a pack is made from and unpacked into (ADR-131).
#[derive(Clone, Debug, Default)]
pub struct ContentFiles {
    files: BTreeMap<&'static str, Cow<'static, [u8]>>,
}

impl ContentFiles {
    /// Every file, in the order the pack lists them.
    pub const LAYOUT: [FileSlot; 17] = [
        slot("run_stats", false, false),
        slot("rewards", false, false),
        slot("stage_skills", false, false),
        slot("expertise", false, false),
        slot("match_ups", false, false),
        slot("runes", false, false),
        slot("perk_sets", false, false),
        slot("difficulties", false, false),
        slot("attack_presets", false, false),
        slot("weapons", false, false),
        slot("rules", false, false),
        slot("starting_unlocks", false, true),
        slot("constellations", false, true),
        slot("stardust", false, true),
        slot("trial", true, false),
        slot("time_limit", true, false),
        slot("tutorial", true, false),
    ];

    /// Sets a file; false for a name the layout does not have.
    pub fn insert(&mut self, name: &str, bytes: Vec<u8>) -> bool {
        match Self::LAYOUT.iter().find(|s| s.name == name) {
            Some(s) => {
                self.files.insert(s.name, Cow::Owned(bytes));
                true
            }
            None => false,
        }
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.files.get(name).map(AsRef::as_ref)
    }

    /// The files of core's tree: each slot of `LAYOUT` read from `content_dir` (core's
    /// `content/`; the mode files from `modes/` beside it). A file that is not there leaves its
    /// slot empty, so that an optional one stays out of the pack and a required one is
    /// `PackError::Missing` from `export`. Reads nothing else (`pack.json` is not a slot).
    pub fn read_dir(content_dir: &Path) -> std::io::Result<ContentFiles> {
        let mut files = ContentFiles::default();
        for slot in Self::LAYOUT {
            match std::fs::read(slot.path(content_dir)) {
                Ok(bytes) => {
                    files.insert(slot.name, bytes);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(files)
    }

    /// The bundled pack's files, each the bytes of the file on its own (a final newline
    /// included): equal to `content/` and `modes/` (tested). Built once from `Pack::BUNDLED`.
    pub fn bundled() -> &'static ContentFiles {
        static BUNDLED: OnceLock<ContentFiles> = OnceLock::new();
        // Split, not re-checked: `Pack::bundled()` reads the same bytes with every check, and
        // tests prove that `export` writes them back. The files share one buffer, made with
        // no temporaries, so that the first call leaves the heap as it found it save one block
        // (the bench cases count the instructions of a tick, which a different heap layout moves).
        BUNDLED.get_or_init(|| {
            let pack: BorrowedPack =
                serde_json::from_slice(Pack::BUNDLED).expect("the bundled pack");
            let members = [
                ("run_stats", Some(pack.run_stats)),
                ("rewards", Some(pack.rewards)),
                ("stage_skills", Some(pack.stage_skills)),
                ("expertise", Some(pack.expertise)),
                ("match_ups", Some(pack.match_ups)),
                ("runes", Some(pack.runes)),
                ("perk_sets", Some(pack.perk_sets)),
                ("difficulties", Some(pack.difficulties)),
                ("attack_presets", Some(pack.attack_presets)),
                ("weapons", Some(pack.weapons)),
                ("rules", Some(pack.rules)),
                ("starting_unlocks", pack.starting_unlocks),
                ("constellations", pack.constellations),
                ("stardust", pack.stardust),
                ("trial", Some(pack.modes.trial)),
                ("time_limit", Some(pack.modes.time_limit)),
                ("tutorial", Some(pack.modes.tutorial)),
            ];
            // Each file, then its final newline.
            let total: usize = members
                .iter()
                .flat_map(|(_, raw)| raw)
                .map(|r| r.get().len() + 1)
                .sum();
            let mut buffer = Vec::with_capacity(total);
            for raw in members.iter().flat_map(|(_, raw)| raw) {
                buffer.extend_from_slice(raw.get().as_bytes());
                buffer.push(b'\n');
            }
            let mut rest: &'static [u8] = Box::leak(buffer.into_boxed_slice());
            let mut files = ContentFiles::default();
            for (name, raw) in members {
                if let Some(raw) = raw {
                    let (file, tail) = rest.split_at(raw.get().len() + 1);
                    files.files.insert(name, Cow::Borrowed(file));
                    rest = tail;
                }
            }
            files
        })
    }

    /// The bytes of one bundled file: the one call for library code and tests that need a
    /// content file on its own. Panics for a name the layout lacks: the bundled pack has every
    /// slot (tested).
    pub fn bundled_file(name: &str) -> &'static [u8] {
        Self::bundled()
            .get(name)
            .unwrap_or_else(|| panic!("the bundled pack has no file `{name}`"))
    }

    /// The files of a pack, read with the checks of `Pack::from_bytes`. It is refused when
    /// `export` would not write these bytes back (other whitespace or order, a different
    /// `introduced` table, CRLF), so that the files give the same pack and so the same hash.
    pub fn from_pack(bytes: &[u8]) -> Result<ContentFiles, PackError> {
        Pack::from_bytes(bytes)?;
        let parse_error = |e: serde_json::Error| PackError::Parse {
            line: e.line(),
            column: e.column(),
        };
        let pack: PackFile = serde_json::from_slice(bytes).map_err(parse_error)?;
        let mut files = ContentFiles::default();
        for (name, raw) in members(&pack) {
            files.insert(name, file_bytes(raw));
        }
        for slot in Self::LAYOUT {
            if files.get(slot.name).is_some_and(|b| b.contains(&b'\r')) {
                return Err(PackError::NotExportForm {
                    path: location(slot.name),
                });
            }
        }
        let exported = files.export()?;
        if exported != bytes {
            let other: PackFile = serde_json::from_slice(&exported).map_err(parse_error)?;
            let same_table = serde_json::to_string(&pack.introduced).ok()
                == serde_json::to_string(&other.introduced).ok();
            let path = if !same_table {
                "$.introduced".to_string()
            } else {
                members(&pack)
                    .into_iter()
                    .zip(members(&other))
                    .find(|((_, a), (_, b))| a.get() != b.get())
                    .map_or("$".to_string(), |((name, _), _)| location(name))
            };
            return Err(PackError::NotExportForm { path });
        }
        Ok(files)
    }

    /// The files as a pack: what the content repository's first pack is made from. Each file is
    /// written as it is, without its final newline, so that the pack gives the files' hashes
    /// (see the module's documentation). A file an optional slot lacks is left out of the
    /// pack. The result is checked as `Pack::from_bytes` checks any pack.
    pub fn export(&self) -> Result<Vec<u8>, PackError> {
        let get = |name: &'static str| self.get(name).ok_or(PackError::Missing { file: name });
        let optional = |name: &'static str| self.get(name);
        let file = |bytes: &[u8]| String::from_utf8_lossy(bytes.trim_ascii_end()).into_owned();

        let stage_skills = StageSkills::parse(get("stage_skills")?, Versions::CURRENT)
            .map_err(PackError::in_file("stage_skills"))?;
        let rewards = Rewards::parse_with(get("rewards")?, Versions::CURRENT, &stage_skills)
            .map_err(PackError::in_file("rewards"))?;
        let rewards = match optional("constellations") {
            Some(bytes) => {
                let sets = Constellations::parse_with(bytes, Versions::CURRENT, &rewards)
                    .map_err(PackError::in_file("constellations"))?;
                rewards.with_constellations(sets)
            }
            None => rewards,
        };
        let perk_sets = PerkSets::parse(get("perk_sets")?, Versions::CURRENT)
            .map_err(PackError::in_file("perk_sets"))?;
        let runes =
            Runes::parse(get("runes")?, Versions::CURRENT).map_err(PackError::in_file("runes"))?;
        let attack_presets = AttackPresets::parse(get("attack_presets")?, Versions::CURRENT)
            .map_err(PackError::in_file("attack_presets"))?;
        let weapons = Weapons::parse(get("weapons")?, Versions::CURRENT)
            .map_err(PackError::in_file("weapons"))?;
        check_holds(&weapons, &attack_presets)?;
        let rules =
            Rules::parse(get("rules")?, Versions::CURRENT).map_err(PackError::in_file("rules"))?;
        let starting_unlocks = optional("starting_unlocks")
            .map(|bytes| {
                StartingUnlocks::parse_with(
                    bytes,
                    Versions::CURRENT,
                    &perk_sets,
                    &stage_skills,
                    &runes,
                    &weapons,
                )
            })
            .transpose()
            .map_err(PackError::in_file("starting_unlocks"))?;
        let stardust = optional("stardust")
            .map(|b| Stardust::parse(b, Versions::CURRENT))
            .transpose()
            .map_err(PackError::in_file("stardust"))?
            .unwrap_or_else(|| Stardust::bundled().clone());
        let trial = get("trial")?;
        let tutorial = get("tutorial")?;
        let content = RunContent::from_parts(
            RunStats::parse(get("run_stats")?, Versions::CURRENT)
                .map_err(PackError::in_file("run_stats"))?,
            rewards,
            Expertise::parse(get("expertise")?, Versions::CURRENT)
                .map_err(PackError::in_file("expertise"))?,
            MatchUps::parse(get("match_ups")?, Versions::CURRENT)
                .map_err(PackError::in_file("match_ups"))?,
            runes,
            perk_sets,
            Difficulties::parse(get("difficulties")?, Versions::CURRENT)
                .map_err(PackError::in_file("difficulties"))?,
            attack_presets,
            weapons,
            rules,
            starting_unlocks,
            stardust,
            TimeLimit::parse(get("time_limit")?, Versions::CURRENT)
                .map_err(PackError::in_file("time_limit"))?,
            validate::parse_and_validate(trial).map_err(PackError::in_file("trial"))?,
            trial.to_vec(),
            validate::parse_and_validate(tutorial).map_err(PackError::in_file("tutorial"))?,
            tutorial.to_vec(),
            None,
        );
        let introduced = introduced_at(&content, SIM_VERSION);
        let table = serde_json::to_string_pretty(&introduced).expect("the table serialises");

        let mut out = String::new();
        out.push_str(&format!(
            "{{\n  \"schema_version\": {SCHEMA_VERSION},\n  \"sim_version\": {SIM_VERSION},\n"
        ));
        out.push_str(&format!(
            "  \"introduced\": {},\n",
            table.replace('\n', "\n  ")
        ));
        for slot in Self::LAYOUT.iter().filter(|s| !s.in_modes) {
            if let Some(bytes) = self.get(slot.name) {
                out.push_str(&format!("  \"{}\": {},\n", slot.name, file(bytes)));
            }
        }
        out.push_str(&format!(
            "  \"modes\": {{\n    \"trial\": {},\n    \"time_limit\": {},\n    \"tutorial\": {}\n  }}\n}}\n",
            file(trial),
            file(get("time_limit")?),
            file(tutorial)
        ));
        let out = out.into_bytes();
        Pack::from_bytes(&out)?;
        Ok(out)
    }
}

/// A pack's content files by name, in the layout's order.
fn members(pack: &PackFile) -> Vec<(&'static str, &RawValue)> {
    let mut out: Vec<(&'static str, &RawValue)> = vec![
        ("run_stats", &pack.run_stats),
        ("rewards", &pack.rewards),
        ("stage_skills", &pack.stage_skills),
        ("expertise", &pack.expertise),
        ("match_ups", &pack.match_ups),
        ("runes", &pack.runes),
        ("perk_sets", &pack.perk_sets),
        ("difficulties", &pack.difficulties),
        ("attack_presets", &pack.attack_presets),
        ("weapons", &pack.weapons),
        ("rules", &pack.rules),
    ];
    for (name, raw) in [
        ("starting_unlocks", &pack.starting_unlocks),
        ("constellations", &pack.constellations),
        ("stardust", &pack.stardust),
    ] {
        if let Some(raw) = raw {
            out.push((name, raw));
        }
    }
    out.push(("trial", &pack.modes.trial));
    out.push(("time_limit", &pack.modes.time_limit));
    out.push(("tutorial", &pack.modes.tutorial));
    out
}

/// An error comparing two content directories with numbers-only comparison (RP2-1, ADR-CONT-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompareError {
    /// Command line usage / invalid directory error (exit status 2).
    Usage(String),
    /// Content difference or validation failure (exit status 1).
    NotValid(String),
}

impl std::fmt::Display for CompareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(msg) => write!(f, "{msg}"),
            Self::NotValid(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for CompareError {}

fn walk_json(
    dir: &Path,
    root: &Path,
    out: &mut BTreeMap<String, PathBuf>,
) -> Result<(), CompareError> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| {
        CompareError::NotValid(format!("cannot read {}: {}", dir.display(), e.kind()))
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            CompareError::NotValid(format!(
                "cannot read entry in {}: {}",
                dir.display(),
                e.kind()
            ))
        })?;
        let path = entry.path();
        if path.is_dir() {
            walk_json(&path, root, out)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let rel = path
                .strip_prefix(root)
                .map_err(|e| CompareError::NotValid(format!("path strip prefix failed: {e}")))?
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, path);
        }
    }
    Ok(())
}

fn collect_json_files(dir: &Path) -> Result<BTreeMap<String, PathBuf>, CompareError> {
    let mut files = BTreeMap::new();
    walk_json(dir, dir, &mut files)?;
    let has_modes = files.keys().any(|k| k.starts_with("modes/"));
    if !has_modes {
        let sibling_modes = dir.join("..").join("modes");
        if sibling_modes.is_dir() {
            let mut modes_files = BTreeMap::new();
            walk_json(&sibling_modes, &sibling_modes, &mut modes_files)?;
            for (rel, path) in modes_files {
                files.insert(format!("modes/{rel}"), path);
            }
        }
    }
    Ok(files)
}

/// Compares two JSON values structurally, returning `Ok(())` if they are identical or differ
/// only in numeric values. Returns `Err(CompareError::NotValid)` on type mismatches, differing
/// strings, booleans, nulls, array lengths, or object keys.
pub fn compare_json_values(
    base: &serde_json::Value,
    head: &serde_json::Value,
    path: &str,
) -> Result<(), CompareError> {
    match (base, head) {
        (serde_json::Value::Number(_), serde_json::Value::Number(_)) => Ok(()),
        (serde_json::Value::String(s1), serde_json::Value::String(s2)) => {
            if s1 == s2 {
                Ok(())
            } else {
                Err(CompareError::NotValid(format!("{path}: string differs")))
            }
        }
        (serde_json::Value::Bool(b1), serde_json::Value::Bool(b2)) => {
            if b1 == b2 {
                Ok(())
            } else {
                Err(CompareError::NotValid(format!("{path}: boolean differs")))
            }
        }
        (serde_json::Value::Null, serde_json::Value::Null) => Ok(()),
        (serde_json::Value::Array(a1), serde_json::Value::Array(a2)) => {
            if a1.len() != a2.len() {
                return Err(CompareError::NotValid(format!(
                    "{path}: array length differs ({} vs {})",
                    a1.len(),
                    a2.len()
                )));
            }
            for (i, (item1, item2)) in a1.iter().zip(a2.iter()).enumerate() {
                compare_json_values(item1, item2, &format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        (serde_json::Value::Object(m1), serde_json::Value::Object(m2)) => {
            if m1.len() != m2.len() {
                return Err(CompareError::NotValid(format!(
                    "{path}: key count differs ({} vs {})",
                    m1.len(),
                    m2.len()
                )));
            }
            for (k, v1) in m1 {
                match m2.get(k) {
                    Some(v2) => compare_json_values(v1, v2, &format!("{path}.{k}"))?,
                    None => {
                        return Err(CompareError::NotValid(format!("{path}: missing key {k:?}")));
                    }
                }
            }
            Ok(())
        }
        _ => Err(CompareError::NotValid(format!("{path}: type differs"))),
    }
}

/// Compares two content directories, exiting 0 if only numeric field values differ.
/// Fails with `CompareError::NotValid` if any key, ID, string, boolean, or array length differs,
/// or if any file cannot be read or parsed.
pub fn compare_numbers_only(base_dir: &Path, head_dir: &Path) -> Result<(), CompareError> {
    if !base_dir.is_dir() {
        return Err(CompareError::Usage("BASE_DIR is not a directory".into()));
    }
    if !head_dir.is_dir() {
        return Err(CompareError::Usage("HEAD_DIR is not a directory".into()));
    }

    let base_files = collect_json_files(base_dir)?;
    let head_files = collect_json_files(head_dir)?;

    if !base_files.keys().eq(head_files.keys()) {
        let missing_in_head: Vec<_> = base_files
            .keys()
            .filter(|k| !head_files.contains_key(*k))
            .collect();
        let extra_in_head: Vec<_> = head_files
            .keys()
            .filter(|k| !base_files.contains_key(*k))
            .collect();
        return Err(CompareError::NotValid(format!(
            "content files differ: missing {missing_in_head:?}, extra {extra_in_head:?}"
        )));
    }

    for (rel_path, base_path) in &base_files {
        let head_path = &head_files[rel_path];
        let base_bytes = std::fs::read(base_path).map_err(|e| {
            CompareError::NotValid(format!("cannot read base {rel_path}: {}", e.kind()))
        })?;
        let head_bytes = std::fs::read(head_path).map_err(|e| {
            CompareError::NotValid(format!("cannot read head {rel_path}: {}", e.kind()))
        })?;

        let base_val: serde_json::Value = serde_json::from_slice(&base_bytes)
            .map_err(|e| CompareError::NotValid(format!("cannot parse base {rel_path}: {e}")))?;
        let head_val: serde_json::Value = serde_json::from_slice(&head_bytes)
            .map_err(|e| CompareError::NotValid(format!("cannot parse head {rel_path}: {e}")))?;

        compare_json_values(&base_val, &head_val, rel_path)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, sim_version: u32) -> IntroducedId {
        IntroducedId {
            id: id.into(),
            sim_version,
        }
    }

    /// The issues `check_ids` raises for `listed` (each ID also an ID of the file).
    fn run(listed: &[IntroducedId], sim_version: u32, released: &[IntroducedId]) -> Vec<Issue> {
        let file_ids: Vec<ContentId> = listed.iter().map(|e| e.id.clone()).collect();
        let ids: Vec<(String, &ContentId)> = file_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (format!("$.rewards.rewards[{i}].id"), id))
            .collect();
        let mut issues = Vec::new();
        check_ids(
            &mut issues,
            "$.introduced.rewards",
            listed,
            &ids,
            sim_version,
            released,
        );
        issues
    }

    #[test]
    fn a_new_id_recorded_below_the_pack_is_refused() {
        let issues = run(&[entry("a", 0)], 1, &[]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].path, "$.introduced.rewards[0].sim_version");
        assert_eq!(
            issues[0].message,
            "a new ID is introduced in the pack's sim_version 1, not 0"
        );
    }

    #[test]
    fn a_released_id_keeps_its_version() {
        assert!(run(&[entry("a", 0)], 1, &[entry("a", 0)]).is_empty());
    }

    #[test]
    fn a_new_id_at_the_pack_version_passes() {
        assert!(run(&[entry("a", 1)], 1, &[]).is_empty());
        assert!(run(&[entry("a", 0)], 0, &[]).is_empty());
    }

    #[test]
    fn a_newer_entry_gets_only_the_newer_issue() {
        let issues = run(&[entry("a", 2)], 1, &[]);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("newer than the pack's 1"));
    }

    #[test]
    fn only_the_new_entry_among_released_ones_is_refused() {
        let listed = [entry("r", 0), entry("n", 1), entry("m", 0)];
        let issues = run(&listed, 1, &[entry("r", 0)]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].path, "$.introduced.rewards[2].sim_version");
    }

    #[test]
    fn the_baseline_is_the_bundled_pack_and_holds_all_its_ids() {
        let file: PackFile = serde_json::from_slice(Pack::BUNDLED).expect("parses");
        assert_eq!(released(), &file.introduced);
        assert!(!released().rewards.is_empty());
        let _ = Pack::bundled();
    }

    #[test]
    fn released_expertise_is_per_weapon() {
        let weapon = released().expertise[0].weapon.clone();
        assert!(!released_expertise(&weapon).is_empty());
        assert!(released_expertise(&"no_such_weapon".into()).is_empty());
    }
}
