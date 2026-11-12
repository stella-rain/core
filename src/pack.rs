//! The content pack (ADR-CONT-5, core#148): the game's content as one JSON file, holding its
//! `schema_version`, the `sim_version` it needs and the content files (the run stats, the
//! rewards, the stage skills, the expertise, the match-up table, the runes, the perk sets and the modes). Its
//! hash is the SHA-256 of the file's bytes.
//!
//! A pack is untrusted (ADR-SVC-1), read from the network or from the cache alike: a size limit
//! before parsing, the current versions only, strict parsing, budgets on the counts of its
//! lists, then each file through its own parser, with the bounds that file already enforces
//! (ADR-CONT-1). Anything else fails closed, with one message per problem and its path, never text
//! from the pack.
//!
//! The offer effects (`offer_candidates`, `rerolls`, `banishes`, `extra_rerolls`,
//! `extra_banishes`) are deprecated (ADR-CONT-2): a pack of `sim_version` 2 refuses them in an
//! entry its `introduced` table does not record below 2 (`check_offer_effects`).
//!
//! A content ID a pack introduces, the file's own ID of a run stat, a reward, a stage skill, or
//! a style or option of a weapon's expertise, an elite type of the `time_limit` mode, or a constellation (listed only when the pack has the list), records the `sim_version` that introduced it in
//! the pack's `introduced` table, each exactly once, and is refused when that is newer than the
//! pack's own (ADR-CONT-5, as the registry's IDs are for a stage, ADR-REPL-1). An ID the released
//! content (the bundled pack) does not hold is new and is recorded at the pack's `sim_version`,
//! and is refused when it is recorded lower (ADR-REPL-2, ADR-CONT-2). The registry's IDs a
//! file names are checked against the registry by the file's own parser.
//!
//! The content files have no versions of their own: the pack's header is the only one, and each
//! file is read at it (CS1a-2). The stages in `modes` are stages and keep theirs.
//!
//! Each file sits in the pack as a JSON object, and is read from the exact bytes it has there,
//! followed by a newline: the bytes of the same file on its own. A pack exported from the
//! bundled files (`ContentFiles::export`) so gives each file the bundled file's SHA-256, and a run of
//! its content the bundled content hash.

use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

mod check;
mod compare;
mod files;
mod introduced;
pub mod trust;

use check::{
    check_budgets, check_default_difficulty, check_default_perk_set, check_difficulty_boss_times,
    check_equipment_rules, check_holds, check_perk_categories, check_style_grants,
};
use introduced::{check_introduced, check_offer_effects, introduced_at};

pub use compare::{CompareError, compare_json_values, compare_numbers_only};
pub use files::{ContentFiles, FileSlot};

use crate::attack_presets::{AttackPresets, AttackPresetsFile};
use crate::constellations::{Constellations, ConstellationsFile};
use crate::difficulties::{Difficulties, DifficultiesFile};
use crate::expertise::{Expertise, ExpertiseFile};
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
use crate::star_chart::{StarChart, StarChartFile};
use crate::stardust::{Stardust, StardustFile};
use crate::starting_unlocks::{StartingUnlocks, StartingUnlocksFile};
use crate::time_limit::{TimeLimit, TimeLimitFile};
use crate::validate::{self, Error, Issue};
use crate::version::{self, ReadError, Versions};
use crate::weapons::{Weapons, WeaponsFile};

// The count budgets: about four times today's counts, rounded up to a multiple of 8 and held
// to the file's own cap where that is lower (tuning values; ADR-SVC-1 decides that they exist).
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
/// The trees one weapon has: exactly one, its base tree (held to the file's own cap).
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
    #[serde(borrow, default)]
    star_chart: Option<&'a RawValue>,
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
    /// The rules the app pays a failed run's Stardust by. Optional: a pack
    /// released before it yields the bundled file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<StardustFile>")]
    pub stardust: Option<Box<RawValue>>,
    /// The Star Chart's Stardust prices (ADR-SAVE-17), checked against the pack's own weapons,
    /// perk sets, stage skills, run stats and starting unlocks. Optional: a pack released before
    /// it has none, and reports no catalogue, never a guessed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<StarChartFile>")]
    pub star_chart: Option<Box<RawValue>>,
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
    /// The perk sets.
    pub perk_sets: Vec<IntroducedId>,
    /// The difficulties.
    pub difficulties: Vec<IntroducedId>,
    /// One entry for each weapon of the expertise.
    pub expertise: Vec<WeaponIds>,
    /// The elite types of the `time_limit` mode file.
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
    /// The `tutorial` mode's stage, checked as any stage is.
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
    /// The largest pack read (a tuning value; ADR-SVC-1 decides that one exists): about 30 times
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
        check_equipment_rules(pack.sim_version, &rewards)?;
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
        check_default_perk_set(&perk_sets)?;
        check_perk_categories(&perk_sets, &rewards)?;
        let difficulties = Difficulties::parse(&file_bytes(&pack.difficulties), versions)
            .map_err(PackError::in_file("difficulties"))?;
        check_default_difficulty(&difficulties)?;
        let attack_presets = AttackPresets::parse(&file_bytes(&pack.attack_presets), versions)
            .map_err(PackError::in_file("attack_presets"))?;
        let weapons = Weapons::parse(&file_bytes(&pack.weapons), versions)
            .map_err(PackError::in_file("weapons"))?;
        check_holds(&weapons, &attack_presets)?;
        check_style_grants(&expertise, &weapons, &attack_presets)?;
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
        let star_chart = pack
            .star_chart
            .as_deref()
            .map(|raw| {
                StarChart::parse_with(
                    &file_bytes(raw),
                    versions,
                    &run_stats,
                    &perk_sets,
                    &stage_skills,
                    &weapons,
                    &expertise,
                    starting_unlocks.as_ref(),
                )
            })
            .transpose()
            .map_err(PackError::in_file("star_chart"))?;
        let trial = validate::parse_and_validate(&file_bytes(&pack.modes.trial))
            .map_err(PackError::in_file("trial"))?;
        let time_limit = TimeLimit::parse(&file_bytes(&pack.modes.time_limit), versions)
            .map_err(PackError::in_file("time_limit"))?;
        check_difficulty_boss_times(&difficulties, &time_limit)?;
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
        check_offer_effects(&pack.introduced, pack.sim_version, &rewards, &perk_sets)?;

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
            star_chart,
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

    /// The bundled pack, `content/pack.json`: the default content of every run (ADR-CONT-5). It is
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

    /// The Stardust rules: the pack's file, or the bundled one for a pack
    /// released without it.
    pub fn stardust(&self) -> &Stardust {
        self.content.stardust()
    }

    /// The Star Chart's prices (ADR-SAVE-17), `None` for a pack released without the file:
    /// no catalogue, never a guessed one.
    pub fn star_chart(&self) -> Option<&StarChart> {
        self.content.star_chart()
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

    /// Checks that every shown content ID in this pack has text in every locale of `set`.
    pub fn check_texts(&self, set: &crate::text::TextSet) -> Result<(), Vec<String>> {
        set.check_pack(self)
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
