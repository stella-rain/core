//! The run setup (ADR-051 §1): what a run of the launch game starts from, and all it starts
//! from. The mode, the weapon (a content ID, ADR-026, ADR-107) and its tree (ADR-117), and the seed of the
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

use crate::difficulties::{Difficulties, Difficulty};
use crate::id::ContentId;
use crate::registry::{self, Entry, Kind};
use crate::run_content::RunContent;
use crate::rune::Rune;
use std::sync::OnceLock;

use crate::sim::HordeSpec;
use crate::sim::state::Horde;
use crate::stage::{Map, Stage, StardustFactor, Wave};
use crate::time_limit::TimeLimit;
use crate::validate::{self, Error, Issue};
use crate::version::{self, SCHEMA_VERSION, SIM_VERSION};
use crate::weapon::WeaponRef;

/// The largest setup file read; a setup is a few hundred bytes.
pub const MAX_SETUP_BYTES: usize = 4096;
/// The most rewards a star pool names: as many as a rewards file holds.
pub const MAX_STAR_POOL: usize = crate::rewards::MAX_REWARDS;
/// The most extra rerolls a setup brings.
pub const MAX_EXTRA_REROLLS: u8 = 20;
/// The perk set of a setup that names none: a setup written before perk sets existed (ADR-105).
/// New setups always name theirs.
pub const DEFAULT_PERK_SET: &str = "duelist";
pub use crate::difficulties::DEFAULT_DIFFICULTY;
/// The most runes a rune pool names: every rune once.
pub const MAX_RUNE_POOL: usize = Rune::ALL.len();
/// The most stars a run starts holding (ADR-107 point 2).
pub const MAX_CARRIED_STARS: usize = 5;
/// The largest Lumine budget, the level of the Lumine upgrade at its most (ADR-107 point 2).
pub const MAX_LUMINE: u8 = 10;
/// The most run stats a stat priority names: the run stats file's own cap.
pub const MAX_STAT_PRIORITY: usize = crate::pack::MAX_RUN_STATS;

/// A star the run starts holding: a reward of the run's rewards file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CarriedStar {
    pub reward: ContentId,
}

/// What a run starts from: built by the app from local storage when a run starts, and stored
/// in the run's replay (ADR-051).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunSetup {
    pub schema_version: u32,
    pub sim_version: u32,
    /// A mode ID: the content the run plays.
    pub mode: ContentId,
    /// A weapon preset: the one weapon the run's main character carries (ADR-107).
    pub weapon: ContentId,
    /// Which of the weapon's trees the run uses (ADR-117), by the tree's ID: it sets the combo
    /// tree's steps and the smash effect, and is fixed for the run. Absent: the weapon's first
    /// tree, so a setup written before trees existed reads as it always did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weapon_tree: Option<ContentId>,
    /// Seeds the run's SplitMix64 (ADR-033), all 64 bits.
    pub seed: u64,
    /// The rewards an offer may hold, by the IDs of the run's rewards file: the player's choice
    /// in the Star Chart (ADR-079 §1), each at most once. Absent: every reward in the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub star_pool: Option<Vec<ContentId>>,
    /// Rerolls the player's progress adds to the stage's, at most `MAX_EXTRA_REROLLS`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub extra_rerolls: u8,
    /// The permanent upgrades the player holds (ADR-079): registered upgrade IDs, sorted, each
    /// once, at most `registry::MAX_UPGRADES`. They open no combo step any more (every smash is
    /// open, ADR-117): an old setup that names `giant_sword_aab` and the others still reads, and
    /// they are ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upgrades: Vec<ContentId>,
    /// The perk set the run applies from tick 0 (ADR-105, ADR-107), by the ID of the run's perk
    /// sets file. A setup written before perk sets existed reads as `DEFAULT_PERK_SET`.
    #[serde(default = "default_perk_set")]
    pub perk_set: ContentId,
    /// The difficulty the run is played on (ADR-110), by the ID of the run's difficulties file:
    /// it scales the enemies, bounds the rarities and may set the mode's length. A setup written
    /// before difficulties existed reads as `DEFAULT_DIFFICULTY`, `normal`, which changes nothing.
    #[serde(default = "default_difficulty")]
    pub difficulty: ContentId,
    /// The runes the random rune may draw (ADR-099 point 4a, ADR-107): rune IDs, sorted, each
    /// once, at most `MAX_RUNE_POOL`. Empty: every rune.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rune_pool: Vec<ContentId>,
    /// The stars the run starts holding (ADR-107 point 3), at most `MAX_CARRIED_STARS`: each
    /// applies at tick 0 as if picked from an offer, in this order, and counts toward its
    /// reward's stack limit. Their costs (the rewards file's `rarities`) add up to at most `lumine`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carried: Vec<CarriedStar>,
    /// The Lumine budget, the level of the Lumine upgrade, at most `MAX_LUMINE` (ADR-107
    /// point 2).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub lumine: u8,
    /// A rune the run starts holding (ADR-107 point 3): in the rune pool when there is one. It
    /// costs no Lumine, and the random rune never draws it again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starting_rune: Option<ContentId>,
    /// The run stats a level-up is spent on, most wanted first (DS18): run stat IDs, each once,
    /// at most `MAX_RUN_STATS`. Each level reached raises the first of them below its limit,
    /// then the stats this list leaves out in the run stats file's order; empty is that order
    /// alone. With `pick_stats` the stats are offered instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stat_priority: Vec<ContentId>,
    /// Each level reached opens an offer of the stats below their limit, answered through
    /// `Engine::choose`, instead of spending by `stat_priority` (DS18). Absent: false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pick_stats: bool,
}

/// The plainest setup: the trial mode with the giant sword, seed 0, the default perk set and
/// difficulty, and nothing carried, pooled or prioritised. Build a setup with
/// `RunSetup { field: .., ..RunSetup::default() }` so that a new field changes no caller.
impl Default for RunSetup {
    fn default() -> RunSetup {
        RunSetup {
            schema_version: SCHEMA_VERSION,
            sim_version: SIM_VERSION,
            mode: "trial".into(),
            weapon: "giant_sword".into(),
            weapon_tree: None,
            seed: 0,
            star_pool: None,
            extra_rerolls: 0,
            upgrades: Vec::new(),
            perk_set: default_perk_set(),
            difficulty: default_difficulty(),
            rune_pool: Vec::new(),
            carried: Vec::new(),
            lumine: 0,
            starting_rune: None,
            stat_priority: Vec::new(),
            pick_stats: false,
        }
    }
}

/// A setup as a file holds it: a `RunSetup` plus `auto_combo`, a field the game dropped (DS5 Q1):
/// auto mode is only the player's switch, recorded as input. A setup or replay written before
/// that carries `"auto_combo": false`, which is ignored; `true` asked for a run that no longer
/// exists, so it is refused (ADR-020).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunSetupFile {
    schema_version: u32,
    sim_version: u32,
    /// A mode ID: the content the run plays.
    mode: ContentId,
    /// A weapon preset: the one weapon the run's main character carries (ADR-107).
    weapon: ContentId,
    /// Which of the weapon's trees the run uses (ADR-117), by the tree's ID: it sets the combo
    /// tree's steps and the smash effect, and is fixed for the run. Absent: the weapon's first
    /// tree, so a setup written before trees existed reads as it always did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    weapon_tree: Option<ContentId>,
    /// Seeds the run's SplitMix64 (ADR-033), all 64 bits.
    seed: u64,
    /// The rewards an offer may hold, by the IDs of the run's rewards file: the player's choice
    /// in the Star Chart (ADR-079 §1), each at most once. Absent: every reward in the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    star_pool: Option<Vec<ContentId>>,
    /// Rerolls the player's progress adds to the stage's, at most `MAX_EXTRA_REROLLS`.
    #[serde(default, skip_serializing_if = "is_zero")]
    extra_rerolls: u8,
    /// The permanent upgrades the player holds (ADR-079): registered upgrade IDs, sorted, each
    /// once, at most `registry::MAX_UPGRADES`. They open no combo step any more (every smash is
    /// open, ADR-117): an old setup that names `giant_sword_aab` and the others still reads, and
    /// they are ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    upgrades: Vec<ContentId>,
    /// The perk set the run applies from tick 0 (ADR-105, ADR-107), by the ID of the run's perk
    /// sets file. A setup written before perk sets existed reads as `DEFAULT_PERK_SET`.
    #[serde(default = "default_perk_set")]
    perk_set: ContentId,
    /// The difficulty the run is played on (ADR-110), by the ID of the run's difficulties file:
    /// it scales the enemies, bounds the rarities and may set the mode's length. A setup written
    /// before difficulties existed reads as `DEFAULT_DIFFICULTY`, `normal`, which changes nothing.
    #[serde(default = "default_difficulty")]
    difficulty: ContentId,
    /// The runes the random rune may draw (ADR-099 point 4a, ADR-107): rune IDs, sorted, each
    /// once, at most `MAX_RUNE_POOL`. Empty: every rune.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rune_pool: Vec<ContentId>,
    /// The stars the run starts holding (ADR-107 point 3), at most `MAX_CARRIED_STARS`: each
    /// applies at tick 0 as if picked from an offer, in this order, and counts toward its
    /// reward's stack limit. Their costs (the rewards file's `rarities`) add up to at most `lumine`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    carried: Vec<CarriedStar>,
    /// The Lumine budget, the level of the Lumine upgrade, at most `MAX_LUMINE` (ADR-107
    /// point 2).
    #[serde(default, skip_serializing_if = "is_zero")]
    lumine: u8,
    /// A rune the run starts holding (ADR-107 point 3): in the rune pool when there is one. It
    /// costs no Lumine, and the random rune never draws it again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    starting_rune: Option<ContentId>,
    /// The run stats a level-up is spent on, most wanted first (DS18): run stat IDs, each once,
    /// at most `MAX_RUN_STATS`. Each level reached raises the first of them below its limit,
    /// then the stats this list leaves out in the run stats file's order; empty is that order
    /// alone. With `pick_stats` the stats are offered instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    stat_priority: Vec<ContentId>,
    /// Each level reached opens an offer of the stats below their limit, answered through
    /// `Engine::choose`, instead of spending by `stat_priority` (DS18). Absent: false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pick_stats: bool,
    #[serde(default)]
    auto_combo: bool,
}

/// The message for a setup that still starts with auto mode on.
const AUTO_COMBO_REMOVED: &str =
    "the field `auto_combo` was removed: auto mode is the player's switch, recorded as input";

impl<'de> Deserialize<'de> for RunSetup {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<RunSetup, D::Error> {
        RunSetup::try_from(RunSetupFile::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

impl TryFrom<RunSetupFile> for RunSetup {
    type Error = &'static str;

    fn try_from(f: RunSetupFile) -> Result<RunSetup, Self::Error> {
        if f.auto_combo {
            return Err(AUTO_COMBO_REMOVED);
        }
        Ok(RunSetup {
            schema_version: f.schema_version,
            sim_version: f.sim_version,
            mode: f.mode,
            weapon: f.weapon,
            weapon_tree: f.weapon_tree,
            seed: f.seed,
            star_pool: f.star_pool,
            extra_rerolls: f.extra_rerolls,
            upgrades: f.upgrades,
            perk_set: f.perk_set,
            difficulty: f.difficulty,
            rune_pool: f.rune_pool,
            carried: f.carried,
            lumine: f.lumine,
            starting_rune: f.starting_rune,
            stat_priority: f.stat_priority,
            pick_stats: f.pick_stats,
        })
    }
}

fn default_perk_set() -> ContentId {
    ContentId::from(DEFAULT_PERK_SET)
}

fn default_difficulty() -> ContentId {
    ContentId::from(DEFAULT_DIFFICULTY)
}

fn is_zero(n: &u8) -> bool {
    *n == 0
}

/// Checks the size, parses strictly and validates: the one call for a setup file's bytes.
pub fn parse_and_validate(bytes: &[u8]) -> Result<RunSetup, Error> {
    let file: RunSetupFile = version::read_current(bytes, MAX_SETUP_BYTES)?;
    if file.auto_combo {
        return Err(Error::Invalid(vec![Issue {
            path: "$.auto_combo".into(),
            message: AUTO_COMBO_REMOVED.into(),
        }]));
    }
    let setup = RunSetup::try_from(file).expect("auto_combo is false");
    validate(&setup)?;
    Ok(setup)
}

/// Validates a parsed setup against the registry.
pub fn validate(setup: &RunSetup) -> Result<(), Error> {
    validate_with(setup, registry::ENTRIES)
}

/// As `validate`, against a given registry (tests use one with newer IDs).
pub fn validate_with(setup: &RunSetup, registry: &[Entry]) -> Result<(), Error> {
    version::check(setup.schema_version, setup.sim_version)?;
    let mut issues = Vec::new();
    let mut id = |key: &str, id: &ContentId, kind: Kind| {
        let problem = if !registry::is_well_formed(id.as_str()) {
            Some("not a valid content ID (lowercase snake_case, at most 32 characters)".to_owned())
        } else {
            match registry::find(registry, kind, id.as_str()) {
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
    id("weapon", &setup.weapon, Kind::Weapon);
    // Whether the weapon has that tree is the weapons file's to say: the run checks it against
    // its own content (`World::new`).
    if setup
        .weapon_tree
        .as_ref()
        .is_some_and(|tree| !registry::is_well_formed(tree.as_str()))
    {
        issues.push(Issue {
            path: "$.weapon_tree".into(),
            message: validate::TREE_ID_PROBLEM.into(),
        });
    }
    if let Some(pool) = &setup.star_pool {
        if pool.len() > MAX_STAR_POOL {
            issues.push(Issue {
                path: "$.star_pool".into(),
                message: format!("at most {MAX_STAR_POOL} rewards (found {})", pool.len()),
            });
        }
        for (i, id) in pool.iter().enumerate().take(MAX_STAR_POOL) {
            let message = if !registry::is_well_formed(id.as_str()) {
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
    if setup.upgrades.len() > registry::MAX_UPGRADES {
        issues.push(Issue {
            path: "$.upgrades".into(),
            message: format!(
                "at most {} upgrades (found {})",
                registry::MAX_UPGRADES,
                setup.upgrades.len()
            ),
        });
    }
    for (i, upgrade) in setup
        .upgrades
        .iter()
        .enumerate()
        .take(registry::MAX_UPGRADES)
    {
        let in_order = i == 0 || setup.upgrades[i - 1].as_str() < upgrade.as_str();
        let message = if !in_order {
            "not after the entry before it (sorted, each once)"
        } else {
            let known = registry::find(registry, Kind::Upgrade, upgrade.as_str())
                .is_some_and(|e| e.introduced <= setup.sim_version);
            if known {
                continue;
            }
            "no such Upgrade ID in the registry"
        };
        issues.push(Issue {
            path: format!("$.upgrades[{i}]"),
            message: message.into(),
        });
    }
    validate_loadout(setup, &mut issues);
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}

/// The perk set, rune pool, carried stars, Lumine and starting rune, against what needs no
/// content file: well-formed IDs, known runes, sorted and unique, and the counts.
fn validate_loadout(setup: &RunSetup, issues: &mut Vec<Issue>) {
    let mut issue = |path: String, message: String| issues.push(Issue { path, message });
    let ill_formed = "not a valid ID (lowercase snake_case, at most 32 characters)";
    if !registry::is_well_formed(setup.perk_set.as_str()) {
        issue("$.perk_set".into(), ill_formed.into());
    }
    if !registry::is_well_formed(setup.difficulty.as_str()) {
        issue("$.difficulty".into(), ill_formed.into());
    }
    if setup.rune_pool.len() > MAX_RUNE_POOL {
        issue(
            "$.rune_pool".into(),
            format!(
                "at most {MAX_RUNE_POOL} runes (found {})",
                setup.rune_pool.len()
            ),
        );
    }
    for (i, rune) in setup.rune_pool.iter().enumerate().take(MAX_RUNE_POOL) {
        let message = if Rune::from_id(rune.as_str()).is_none() {
            "no such rune"
        } else if i > 0 && setup.rune_pool[i - 1].as_str() >= rune.as_str() {
            "not after the entry before it (sorted, each once)"
        } else {
            continue;
        };
        issue(format!("$.rune_pool[{i}]"), message.into());
    }
    if setup.carried.len() > MAX_CARRIED_STARS {
        issue(
            "$.carried".into(),
            format!(
                "at most {MAX_CARRIED_STARS} stars (found {})",
                setup.carried.len()
            ),
        );
    }
    for (i, star) in setup.carried.iter().enumerate().take(MAX_CARRIED_STARS) {
        if !registry::is_well_formed(star.reward.as_str()) {
            issue(format!("$.carried[{i}].reward"), ill_formed.into());
        }
    }
    if setup.stat_priority.len() > MAX_STAT_PRIORITY {
        issue(
            "$.stat_priority".into(),
            format!(
                "at most {MAX_STAT_PRIORITY} stats (found {})",
                setup.stat_priority.len()
            ),
        );
    }
    for (i, stat) in setup
        .stat_priority
        .iter()
        .enumerate()
        .take(MAX_STAT_PRIORITY)
    {
        if !registry::is_well_formed(stat.as_str()) {
            issue(format!("$.stat_priority[{i}]"), ill_formed.into());
        } else if setup.stat_priority[..i].contains(stat) {
            issue(
                format!("$.stat_priority[{i}]"),
                "the same stat as an earlier entry".into(),
            );
        }
    }
    if setup.lumine > MAX_LUMINE {
        issue(
            "$.lumine".into(),
            format!("at most {MAX_LUMINE} (found {})", setup.lumine),
        );
    }
    if let Some(rune) = &setup.starting_rune {
        let in_pool = setup.rune_pool.is_empty() || setup.rune_pool.contains(rune);
        if Rune::from_id(rune.as_str()).is_none() {
            issue("$.starting_rune".into(), "no such rune".into());
        } else if !in_pool {
            issue("$.starting_rune".into(), "not in the rune pool".into());
        }
    }
}

/// As `validate`, and against the run's content files: the perk set is one of the perk sets
/// file's, the difficulty one of the difficulties file's, every star pool entry a reward of the rewards file, every carried star a reward of the rewards file (and of the star pool, when the
/// setup names one) that is not the random rune, a reward carried no more times than it
/// stacks, and the carried stars' costs within the Lumine budget (ADR-107 point 4). The call
/// `Engine::start_with` makes.
pub fn validate_in(setup: &RunSetup, content: &RunContent) -> Result<(), Error> {
    validate(setup)?;
    let mut issues = Vec::new();
    if content.perk_sets().index_of(&setup.perk_set).is_none() {
        issues.push(Issue {
            path: "$.perk_set".into(),
            message: "no such perk set in the run's perk sets file".into(),
        });
    }
    if content.difficulties().get(&setup.difficulty).is_none() {
        issues.push(Issue {
            path: "$.difficulty".into(),
            message: "no such difficulty in the run's difficulties file".into(),
        });
    }
    for (i, id) in setup.stat_priority.iter().enumerate() {
        if !content.run_stats().stats().iter().any(|s| s.id == *id) {
            issues.push(Issue {
                path: format!("$.stat_priority[{i}]"),
                message: "no such stat in the run's run stats file".into(),
            });
        }
    }
    let rewards = content.rewards();
    for (i, id) in setup.star_pool.iter().flatten().enumerate() {
        if rewards.index_of(id).is_none() {
            issues.push(Issue {
                path: format!("$.star_pool[{i}]"),
                message: "no such reward in the run's rewards file".into(),
            });
        }
    }
    let mut cost: u32 = 0;
    for (i, star) in setup.carried.iter().enumerate() {
        let path = format!("$.carried[{i}].reward");
        let Some(at) = rewards.index_of(&star.reward) else {
            issues.push(Issue {
                path,
                message: "no such reward in the run's rewards file".into(),
            });
            continue;
        };
        let reward = &rewards.rewards()[at];
        let in_pool = setup
            .star_pool
            .as_ref()
            .is_none_or(|pool| pool.contains(&star.reward));
        let times = setup.carried[..=i]
            .iter()
            .filter(|c| c.reward == star.reward)
            .count();
        let message = if reward.is_random_rune() {
            Some("the random rune is not carried; start with a rune instead")
        } else if !in_pool {
            Some("not in the star pool")
        } else if reward.limit.is_some_and(|limit| times > usize::from(limit)) {
            Some("carried more times than the reward stacks")
        } else {
            None
        };
        match message {
            Some(message) => issues.push(Issue {
                path,
                message: message.into(),
            }),
            None => cost += u32::from(rewards.lumine_cost(reward.rarity)),
        }
    }
    if issues.is_empty() && cost > u32::from(setup.lumine) {
        issues.push(Issue {
            path: "$.carried".into(),
            message: format!(
                "cost {cost} Lumine, more than the budget of {}",
                setup.lumine
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
    [mode.bytes(), time_limit_base()].concat()
}

/// The bundled `trial` mode's file: the stage the `time_limit` run takes its main character,
/// companions, levels, stars and boss from (placeholders until the mode has its own).
fn time_limit_base() -> &'static [u8] {
    RunContent::bundled().trial_bytes()
}

/// The bundled content of mode `id` when it is a stage file; `time_limit` is not one (its run
/// is built from its mode file, `time_limit_run`).
pub fn mode_stage(id: &str) -> Option<&'static [u8]> {
    match id {
        "trial" => Some(time_limit_base()),
        "tutorial" => Some(RunContent::bundled().tutorial_bytes()),
        _ => None,
    }
}

/// The stage a setup plays: its mode's stage, validated, with the setup's weapon presets in
/// the Main and Sub roles, validated again. The stage's own `seed` is left as it is:
/// `Engine::start` seeds the run with the setup's. For `time_limit`, the stage of its run
/// (`time_limit_run`).
pub fn stage(setup: &RunSetup) -> Result<Stage, Error> {
    stage_in(setup, RunContent::bundled())
}

/// As `stage`, with `time_limit` in place of the bundled `time_limit` mode file.
pub fn stage_with(setup: &RunSetup, time_limit: &TimeLimit) -> Result<Stage, Error> {
    let bundled = RunContent::bundled();
    stage_on(
        setup,
        bundled.trial(),
        bundled.tutorial(),
        time_limit,
        bundled.difficulties(),
    )
}

/// As `stage`, from the `trial` and `tutorial` stages and the `time_limit` mode file of
/// `content` (a pack's).
pub fn stage_in(setup: &RunSetup, content: &RunContent) -> Result<Stage, Error> {
    stage_on(
        setup,
        content.trial(),
        content.tutorial(),
        content.time_limit(),
        content.difficulties(),
    )
}

fn stage_on(
    setup: &RunSetup,
    trial: &Stage,
    tutorial: &Stage,
    time_limit: &TimeLimit,
    difficulties: &Difficulties,
) -> Result<Stage, Error> {
    validate(setup)?;
    if setup.mode.as_str() == "time_limit" {
        let difficulty = difficulty_of(setup, difficulties)?;
        return Ok(time_limit_run(setup, time_limit, difficulty, trial)?.0);
    }
    let base = match setup.mode.as_str() {
        "trial" => trial,
        "tutorial" => tutorial,
        _ => {
            return Err(Error::Invalid(vec![Issue {
                path: "$.mode".into(),
                message: "the mode has no stage".into(),
            }]));
        }
    };
    let mut stage = base.clone();
    stage.player.weapon = WeaponRef::Preset {
        id: setup.weapon.clone(),
        tree: setup.weapon_tree.clone(),
    };
    validate::validate(&stage)?;
    Ok(stage)
}

/// The difficulty `setup` names, in `difficulties`.
pub(crate) fn difficulty_of<'a>(
    setup: &RunSetup,
    difficulties: &'a Difficulties,
) -> Result<&'a Difficulty, Error> {
    difficulties.get(&setup.difficulty).ok_or_else(|| {
        Error::Invalid(vec![Issue {
            path: "$.difficulty".into(),
            message: "no such difficulty in the run's difficulties file".into(),
        }])
    })
}

/// Checks each boss definition of the mode as the placeholder stage with that boss is (its
/// presets, its parts, its numbers), so a run never starts on a boss the stage would refuse.
/// A run start checks them, and so does a pack's load, so a pack that loads starts a run.
pub(crate) fn check_bosses(placeholder: &Stage, mode: &TimeLimit) -> Result<(), Error> {
    for (k, def) in mode.bosses().iter().enumerate() {
        let mut with_boss = placeholder.clone();
        with_boss.boss = Some(def.clone());
        validate::validate(&with_boss).map_err(|e| match e {
            Error::Invalid(issues) => Error::Invalid(
                issues
                    .into_iter()
                    .map(|i| Issue {
                        path: format!("$.bosses[{k}]{}", i.path.trim_start_matches("$.boss")),
                        message: i.message,
                    })
                    .collect(),
            ),
            other => other,
        })?;
    }
    Ok(())
}

/// The stage and the bosses of a `time_limit` run of `mode` (core#212): the placeholder stage
/// with the setup's weapons, its map, schedule of waves and length from the mode file, the
/// length being the run and its overtime (a clear is the final boss's death; running out of
/// ticks fails), and no boss until their times (`Horde`). Only the base is validated as a
/// stage: the mode file validated its own waves, which a stage's caps would refuse.
pub(crate) fn time_limit_run(
    setup: &RunSetup,
    mode: &TimeLimit,
    difficulty: &Difficulty,
    trial: &Stage,
) -> Result<(Stage, HordeSpec, Horde), Error> {
    if setup.mode.as_str() != "time_limit" {
        return Err(Error::Invalid(vec![Issue {
            path: "$.mode".into(),
            message: "not the time_limit mode".into(),
        }]));
    }
    let mut stage = trial.clone();
    stage.player.weapon = WeaponRef::Preset {
        id: setup.weapon.clone(),
        tree: setup.weapon_tree.clone(),
    };
    validate::validate(&stage)?;
    check_bosses(&stage, mode)?;
    let template = stage.boss.take().ok_or_else(|| {
        Error::Invalid(vec![Issue {
            path: "$.boss".into(),
            message: "the placeholder stage has no boss".into(),
        }])
    })?;
    stage.id = "time-limit".into();
    stage.title = "Time Limit".into();
    stage.description = String::new();
    let length_ticks = difficulty
        .length_ticks
        .unwrap_or_else(|| mode.length_ticks());
    let boss_ticks = difficulty
        .boss_ticks
        .clone()
        .unwrap_or_else(|| mode.boss_ticks().to_vec());
    // One mid-boss for each boss time but the last, from the pool, each entry once.
    if boss_ticks.len().saturating_sub(1) > mode.mid_boss_pool().len() {
        return Err(Error::Invalid(vec![Issue {
            path: "$.difficulty".into(),
            message: "more mid-boss times than the mode's pool has bosses".into(),
        }]));
    }
    stage.length_ticks = length_ticks.saturating_add(mode.overtime_ticks());
    stage.map = Some(Map {
        width: mode.map().width,
        height: mode.map().height,
        start: Some(mode.map().start),
        facing: None,
    });
    stage.waves = mode.waves().to_vec();
    stage.elite_cap = Some(mode.elite_cap());
    stage.crowd_target = mode.crowd_target();
    stage.levels = Some(mode.levels().clone());
    stage.stars = Some(mode.stars().clone());
    stage.level_table = Some(difficulty.level_table.unwrap_or(*mode.level_table()));
    stage.stardust = Some(StardustFactor {
        tiers: difficulty
            .stardust_tiers
            .clone()
            .unwrap_or_else(|| mode.stardust_tiers().to_vec()),
        summon_base: mode.stardust_summon_base(),
    });
    // Each elite type is a wave that never spawns (count 0): the lookups of an enemy's
    // definition, experience and expertise go by the stage's waves.
    let mut elite_waves: Vec<(ContentId, u32)> = Vec::new();
    for t in mode.elite_types() {
        elite_waves.push((
            t.id.clone(),
            u32::try_from(stage.waves.len()).unwrap_or(u32::MAX),
        ));
        stage.waves.push(Wave {
            id: None,
            at_tick: 0,
            enemy: t.enemy.clone(),
            spawn: Some(mode.map().start),
            edge: None,
            count: 0,
            every_ticks: 0,
            xp: t.xp,
            expertise: t.expertise,
            stardust: t.stardust,
        });
    }
    let spec = HordeSpec {
        pool: mode.mid_boss_pool().to_vec(),
        final_boss: mode.final_boss().clone(),
        template,
        bosses: mode.bosses().to_vec(),
        length_ticks,
        elite_table: mode.elite_table().to_vec(),
        elite_waves,
    };
    let horde = Horde {
        boss_ticks,
        no_call_ticks: mode.no_call_ticks().to_vec(),
        mid_boss_hp_pct: mode.mid_boss_hp_pct().to_vec(),
        spawned: 0,
        used: vec![false; mode.mid_boss_pool().len()],
        held: vec![0; stage.waves.len()],
    };
    Ok((stage, spec, horde))
}

/// The most ticks a replay of `stage` may hold. A `time_limit` run's stage ends at the length
/// and the overtime; its replay may also hold the Star-Hunter's extra time (ADR-079).
pub fn max_replay_ticks(setup: &RunSetup, stage: &Stage, time_limit: &TimeLimit) -> u32 {
    if setup.mode.as_str() == "time_limit" {
        stage
            .length_ticks
            .saturating_add(time_limit.star_hunter_extra_ticks())
    } else {
        stage.length_ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_setup_is_valid_and_starts_a_run() {
        let setup = RunSetup::default();
        assert_eq!(validate(&setup), Ok(()));
        assert_eq!(validate_in(&setup, RunContent::bundled()), Ok(()));
        assert!(crate::engine::Engine::start(&setup).is_ok());
    }

    #[test]
    fn a_setup_built_from_the_default_changes_only_what_it_names() {
        let setup = RunSetup {
            seed: 7,
            weapon: "giant_sword".into(),
            ..RunSetup::default()
        };
        assert_eq!((setup.seed, setup.mode.as_str()), (7, "trial"));
        let bytes = serde_json::to_vec(&setup).unwrap();
        assert_eq!(serde_json::from_slice::<RunSetup>(&bytes).unwrap(), setup);
    }
}
