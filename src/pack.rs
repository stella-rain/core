//! The content pack (ADR-080, core#148): the game's content as one JSON file, holding its
//! `schema_version`, the `sim_version` it needs and the content files (the run stats, the
//! rewards, the stage skills, the expertise, the match-up table, the runes and the modes). Its
//! hash is the SHA-256 of the file's bytes.
//!
//! A pack is untrusted (ADR-020), read from the network or from the cache alike: a size limit
//! before parsing, the current versions only, strict parsing, budgets on the counts of its
//! lists, then each file through its own parser, with the bounds that file already enforces
//! (ADR-067). Anything else fails closed, with one message per problem and its path, never text
//! from the pack.
//!
//! Each file sits in the pack as a JSON object, and is read from the exact bytes it has there,
//! followed by a newline: the bytes of the same file on its own. A pack exported from the
//! bundled files (`export_bundled`) so gives each file the bundled file's SHA-256, and a run of
//! its content the bundled content hash.

use schemars::JsonSchema;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

use crate::expertise::{Expertise, ExpertiseFile};
use crate::match_ups::{MatchUps, MatchUpsFile};
use crate::rewards::{Rewards, RewardsFile};
use crate::run_content::RunContent;
use crate::run_setup;
use crate::run_stats::{RunStats, RunStatsFile};
use crate::runes::{Runes, RunesFile};
use crate::stage::Stage;
use crate::stage_skill::{StageSkills, StageSkillsFile};
use crate::time_limit::{TimeLimit, TimeLimitFile};
use crate::validate::{self, Error};
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
/// The most waves of the `time_limit` mode (2 today; held to the file's own cap).
pub const MAX_TIME_LIMIT_WAVES: usize = crate::time_limit::MAX_WAVES;
/// The most boss times of the `time_limit` mode (4 today; held to the file's own cap).
pub const MAX_BOSS_TICKS: usize = crate::time_limit::MAX_BOSS_TIMES;
/// The most bosses in the `time_limit` mode's mid-boss pool (3 today; held to the file's own
/// cap).
pub const MAX_MID_BOSS_POOL: usize = crate::time_limit::MAX_POOL;

/// The pack file. Each content file is the JSON object its own schema describes.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackFile {
    pub schema_version: u32,
    /// The `sim_version` the pack needs: every file has the same.
    pub sim_version: u32,
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
    pub modes: PackModes,
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
}

impl PackError {
    fn in_file(file: &'static str) -> impl Fn(Error) -> PackError {
        move |error| PackError::File { file, error }
    }
}

/// Where a file sits in the pack, as a path.
fn location(file: &str) -> String {
    match file {
        "trial" | "time_limit" => format!("$.modes.{file}"),
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
    trial: Stage,
    sha256: [u8; 32],
}

impl Pack {
    /// The largest pack read (a tuning value; ADR-020 decides that one exists): about 30 times
    /// today's 36 KB of content.
    pub const MAX_BYTES: usize = 1024 * 1024;

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
        let expertise = Expertise::parse(&file_bytes(&pack.expertise))
            .map_err(PackError::in_file("expertise"))?;
        let match_ups = MatchUps::parse(&file_bytes(&pack.match_ups))
            .map_err(PackError::in_file("match_ups"))?;
        let runes = Runes::parse(&file_bytes(&pack.runes)).map_err(PackError::in_file("runes"))?;
        let trial = validate::parse_and_validate(&file_bytes(&pack.modes.trial))
            .map_err(PackError::in_file("trial"))?;
        let time_limit = TimeLimit::parse(&file_bytes(&pack.modes.time_limit))
            .map_err(PackError::in_file("time_limit"))?;

        let content = RunContent::new(run_stats, rewards)
            .with_expertise(expertise)
            .with_match_ups(match_ups)
            .with_runes(runes)
            .with_time_limit(time_limit);
        Ok(Pack {
            schema_version: pack.schema_version,
            sim_version: pack.sim_version,
            content,
            trial,
            sha256: Sha256::digest(bytes).into(),
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

    /// The `trial` mode's stage.
    pub fn trial(&self) -> &Stage {
        &self.trial
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
struct Lists {
    #[serde(default)]
    stats: Vec<IgnoredAny>,
    #[serde(default)]
    rewards: Vec<IgnoredAny>,
    #[serde(default)]
    stage_skills: Vec<IgnoredAny>,
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
    }
    Ok(())
}

/// The bundled content files as a pack: what the content repository's first pack is made
/// from. Each file is written as it is, without its final newline, so that the pack gives the
/// bundled files' hashes (see the module's documentation).
pub fn export_bundled() -> Vec<u8> {
    let file = |bytes: &[u8]| String::from_utf8_lossy(bytes.trim_ascii_end()).into_owned();
    let trial = run_setup::mode_stage("trial").expect("the trial mode is bundled");
    let mut out = String::new();
    out.push_str(&format!(
        "{{\n  \"schema_version\": {SCHEMA_VERSION},\n  \"sim_version\": {SIM_VERSION},\n"
    ));
    for (name, bytes) in [
        ("run_stats", RunStats::BUNDLED),
        ("rewards", Rewards::BUNDLED),
        ("stage_skills", StageSkills::BUNDLED),
        ("expertise", Expertise::BUNDLED),
        ("match_ups", MatchUps::BUNDLED),
        ("runes", Runes::BUNDLED),
    ] {
        out.push_str(&format!("  \"{name}\": {},\n", file(bytes)));
    }
    out.push_str(&format!(
        "  \"modes\": {{\n    \"trial\": {},\n    \"time_limit\": {}\n  }}\n}}\n",
        file(trial),
        file(TimeLimit::BUNDLED)
    ));
    out.into_bytes()
}
