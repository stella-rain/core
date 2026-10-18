//! Rewards (ADR-079 §3, ADR-067, ADR-068): what an offer from a falling star holds. Each has a
//! rarity, a stack limit (how many times one run takes it) and an effect. They are content, not
//! code: a JSON list of any length up to `MAX_REWARDS`, read in its order. Data picks, code
//! acts: a reward's effect is one of the run stats' effect kinds (`StatEffect`), adding
//! `amount` to that effect's total, as a stat point adds its step, a stage skill (ADR-075,
//! ADR-098): one of the stage skills file's skills, `amount` levels of it, `amount` more
//! evade charges, up to the stage's `max_charges` (ADR-074), an orbit of `amount` orbs that
//! attack on their own (ADR-069), `amount` percent more range for every combo step of the Main
//! weapon (ADR-099), or an enchantment of the Main weapon's combo (ADR-070, ADR-078), taken
//! once.
//!
//! The file is untrusted (ADR-020), as the run stats are: a size limit before parsing, strict
//! parsing, the current versions only, and every field bounded. Reward IDs are the file's own,
//! unique within it; a run setup's star pool names them. A reward's skill must be in the stage
//! skills file the rewards are read with. The bundled file, `content/rewards.json`, is the
//! default; the SHA-256 of whatever bytes a run used is part of its content hash.

use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::attack::AttackRef;
use crate::id::ContentId;
use crate::registry;
use crate::run_stats::{self, StatEffect};
use crate::stage_skill::{MAX_SKILL_LEVELS, StageSkills};
use crate::trig::PER_DEGREE;
use crate::validate::limits::MAX_EVADE_CHARGES;
use crate::validate::{Error, Issue};
use crate::version::{SCHEMA_VERSION, SIM_VERSION};

/// The largest rewards file read (a tuning value; ADR-020 decides that one exists).
pub const MAX_REWARDS_BYTES: usize = 32 * 1024;
/// The most rewards a file lists.
pub const MAX_REWARDS: usize = 64;
/// The largest amount: hit points, or a percentage.
pub const MAX_AMOUNT: u32 = 100;
/// The most times one run takes one reward.
pub const MAX_STACK: u8 = 20;
/// The most orbs one orbit holds, however often its reward is taken.
pub const MAX_ORBS: u8 = 8;
/// The most orbit rewards a file lists, and so the most orbits a run holds.
pub const MAX_ORBITS: usize = 4;
/// The fastest an orbit turns: 15 degrees a tick, either way.
pub const MAX_ORBIT_TURN: i32 = 15 * PER_DEGREE;
/// The longest cooldown of an orbit: one minute.
pub const MAX_ORBIT_COOLDOWN_TICKS: u32 = 3_600;
/// The most a combo step's hit box is widened, in percent: twice as large. It bounds one range
/// reward's amount and holds what the range rewards, the expertise range options and a range
/// enchantment add up to (ADR-099, ADR-020).
pub const MAX_RANGE_PCT: u32 = 100;

/// A reward's rarity, from the most common up (ADR-079 §2).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Epic,
    Legendary,
    Supernova,
}

impl Rarity {
    /// Every rarity, from the most common up.
    pub const ALL: [Rarity; 6] = [
        Rarity::Common,
        Rarity::Uncommon,
        Rarity::Rare,
        Rarity::Epic,
        Rarity::Legendary,
        Rarity::Supernova,
    ];

    /// The rarity's place in `ALL`, from 0: what the state hash writes.
    pub fn index(self) -> u8 {
        self as u8
    }
}

/// What taking a reward does: one of the run stats' effect kinds, under the same names, a stage
/// skill, more evade charges, an orbit, range, or an enchantment.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RewardEffect {
    MaxHp,
    AttackPct,
    AttackSpeedPct,
    EvadePct,
    SpeedPct,
    XpPct,
    /// The stage skill the reward's `skill` names, `amount` levels of it (ADR-075).
    StageSkill,
    /// `amount` more evade charges, never beyond the stage's `max_charges`; each arrives ready
    /// (ADR-074).
    EvadeCharge,
    /// The reward's `orbit`, with `amount` more orbs, never beyond `MAX_ORBS` (ADR-069).
    Orbit,
    /// Every combo step of the Main weapon `amount` percent wider, as the expertise range
    /// option widens it, adding up over the picks (ADR-099).
    RangePct,
    /// The reward's `enchantment` of the Main weapon's combo, `amount` its strength; taken once,
    /// and never offered again, nor another of its kind, once one of its kind is held (ADR-070,
    /// ADR-078).
    Enchantment,
}

impl RewardEffect {
    /// The run stats' effect kind it adds to, if it is one.
    pub fn stat(self) -> Option<StatEffect> {
        Some(match self {
            RewardEffect::MaxHp => StatEffect::MaxHp,
            RewardEffect::AttackPct => StatEffect::AttackPct,
            RewardEffect::AttackSpeedPct => StatEffect::AttackSpeedPct,
            RewardEffect::EvadePct => StatEffect::EvadePct,
            RewardEffect::SpeedPct => StatEffect::SpeedPct,
            RewardEffect::XpPct => StatEffect::XpPct,
            RewardEffect::StageSkill
            | RewardEffect::EvadeCharge
            | RewardEffect::Orbit
            | RewardEffect::RangePct
            | RewardEffect::Enchantment => return None,
        })
    }
}

/// One reward an offer can hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Reward {
    /// The reward's own ID, unique within the file.
    pub id: ContentId,
    pub rarity: Rarity,
    pub effect: RewardEffect,
    /// The stage skill it gives, one of the stage skills file's: with the `stage_skill` effect
    /// only, and required by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<ContentId>,
    /// The orbit it gives: with the `orbit` effect only, and required by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orbit: Option<Orbit>,
    /// The enchantment it gives: with the `enchantment` effect only, and required by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enchantment: Option<Enchantment>,
    /// What taking it adds to the effect's total, from 1 to `MAX_AMOUNT`; for a stage skill,
    /// the levels it gives, from 1 to `MAX_SKILL_LEVELS`; for evade charges, the charges it
    /// adds, from 1 to `MAX_EVADE_CHARGES`; for an orbit, the orbs it adds, from 1 to
    /// `MAX_ORBS`; for range or a range enchantment, how much wider, from 1 to `MAX_RANGE_PCT`
    /// percent.
    pub amount: u32,
    /// The most times one run takes it, from 1 to `MAX_STACK`; exactly 1 for an enchantment.
    pub limit: u8,
    /// The name the choice screen shows, 1 to `run_stats::MAX_TITLE_CHARS` characters.
    pub title: String,
    /// What it does, in words, at most `run_stats::MAX_DESCRIPTION_CHARS` characters.
    pub description: String,
}

/// Orbs that turn round the main character and attack on their own (ADR-069). Each orb runs
/// `attack` in an attack slot of its own, at the main character's attack stat, its facing
/// turned `turn_per_tick` further every tick and the orbs spaced evenly round the turn: the
/// attack's hit box offset is how far out an orb is. A round of attacks starts when the orbit
/// arrives and again `cooldown_ticks` after a round's start, once the round before has ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Orbit {
    /// The attack each orb runs, validated as any attack is.
    pub attack: AttackRef,
    /// How far the orbs turn a tick, in 1/256 degree, toward the right for positive angles,
    /// at most `MAX_ORBIT_TURN` either way.
    pub turn_per_tick: i32,
    /// Ticks from a round's start to the next one's, from 1 to `MAX_ORBIT_COOLDOWN_TICKS`.
    pub cooldown_ticks: u32,
}

/// An enchantment of the Main weapon's combo (ADR-070, ADR-078): one of a closed list built into
/// `core`, its strength the reward's `amount`. It changes every combo step started after it is
/// taken: A and B steps, the dash-strike and charged releases alike; the stage skill and orbits
/// are not combo steps.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Enchantment {
    /// Every hit box of the step is `amount` percent larger: its radius and its offset, rounded
    /// toward zero; an arc's angle stays. The bullets a step spawns are not changed.
    Range,
}

impl Enchantment {
    /// The kind's place in the list, from 0: what the state hash writes.
    pub fn index(self) -> u8 {
        match self {
            Enchantment::Range => 0,
        }
    }
}

/// The rewards file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RewardsFile {
    pub schema_version: u32,
    pub sim_version: u32,
    /// The rewards, in the order offers draw them from.
    pub rewards: Vec<Reward>,
}

/// Rewards that passed validation, the SHA-256 of the bytes they came from, and the stage
/// skills they were checked against, which a run reads with them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rewards {
    rewards: Vec<Reward>,
    stage_skills: StageSkills,
    sha256: [u8; 32],
}

impl Rewards {
    /// The bundled file's bytes.
    pub const BUNDLED: &'static [u8] = include_bytes!("../content/rewards.json");

    /// The bundled rewards, with the bundled stage skills: the default of every run.
    pub fn bundled() -> &'static Rewards {
        static BUNDLED: OnceLock<Rewards> = OnceLock::new();
        BUNDLED.get_or_init(|| Rewards::parse(Rewards::BUNDLED).expect("the bundled rewards"))
    }

    /// `parse_with` the bundled stage skills.
    pub fn parse(bytes: &[u8]) -> Result<Rewards, Error> {
        Rewards::parse_with(bytes, StageSkills::bundled())
    }

    /// Checks the size, parses strictly and validates, each reward's skill in `stage_skills`:
    /// the one call for a file's bytes.
    pub fn parse_with(bytes: &[u8], stage_skills: &StageSkills) -> Result<Rewards, Error> {
        if bytes.len() > MAX_REWARDS_BYTES {
            return Err(Error::TooLarge {
                size: bytes.len(),
                limit: MAX_REWARDS_BYTES,
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
        let file: RewardsFile = serde_json::from_slice(bytes).map_err(parse_error)?;
        validate(&file, stage_skills)?;
        Ok(Rewards {
            rewards: file.rewards,
            stage_skills: stage_skills.clone(),
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The rewards, in the file's order.
    pub fn rewards(&self) -> &[Reward] {
        &self.rewards
    }

    /// The place of the reward `id` in the file, if it has one.
    pub fn index_of(&self, id: &ContentId) -> Option<usize> {
        self.rewards.iter().position(|r| &r.id == id)
    }

    /// The stage skills its rewards were checked against.
    pub fn stage_skills(&self) -> &StageSkills {
        &self.stage_skills
    }

    /// The SHA-256 of the bytes they were parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Validates a parsed file, each reward's skill in `stage_skills`: one issue per problem, with
/// its path, never text from the file.
pub fn validate(file: &RewardsFile, stage_skills: &StageSkills) -> Result<(), Error> {
    let mut issues = Vec::new();
    let mut issue = |path: String, message: String| issues.push(Issue { path, message });
    if file.rewards.is_empty() || file.rewards.len() > MAX_REWARDS {
        issue(
            "$.rewards".into(),
            format!("1 to {MAX_REWARDS} rewards (found {})", file.rewards.len()),
        );
    }
    for (i, reward) in file.rewards.iter().enumerate() {
        let at = |field: &str| format!("$.rewards[{i}].{field}");
        if !registry::is_well_formed(&reward.id.0) {
            issue(
                at("id"),
                "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
            );
        } else if file.rewards[..i].iter().any(|r| r.id == reward.id) {
            issue(at("id"), "the same ID as an earlier reward".into());
        }
        let most = match reward.effect {
            RewardEffect::StageSkill => MAX_SKILL_LEVELS as u32,
            RewardEffect::EvadeCharge => u32::from(MAX_EVADE_CHARGES),
            RewardEffect::Orbit => u32::from(MAX_ORBS),
            RewardEffect::RangePct | RewardEffect::Enchantment => MAX_RANGE_PCT,
            _ => MAX_AMOUNT,
        };
        if !(1..=most).contains(&reward.amount) {
            issue(
                at("amount"),
                format!("1 to {most} (found {})", reward.amount),
            );
        }
        match (reward.effect, &reward.skill) {
            (RewardEffect::StageSkill, None) => {
                issue(at("skill"), "a stage skill reward names its skill".into())
            }
            (RewardEffect::StageSkill, Some(id)) => {
                if stage_skills.index_of(id).is_none() {
                    issue(at("skill"), "no such skill in the stage skills file".into());
                }
            }
            (_, Some(_)) => issue(
                at("skill"),
                "only a stage skill reward names a skill".into(),
            ),
            (_, None) => {}
        }
        match (reward.effect, &reward.orbit) {
            (RewardEffect::Orbit, None) => {
                issue(at("orbit"), "an orbit reward holds its orbit".into())
            }
            (RewardEffect::Orbit, Some(orbit)) => {
                if file.rewards[..i]
                    .iter()
                    .filter(|r| r.effect == RewardEffect::Orbit)
                    .count()
                    >= MAX_ORBITS
                {
                    issue(
                        at("effect"),
                        format!("at most {MAX_ORBITS} orbit rewards in a file"),
                    );
                }
                if !(-MAX_ORBIT_TURN..=MAX_ORBIT_TURN).contains(&orbit.turn_per_tick) {
                    issue(
                        at("orbit.turn_per_tick"),
                        format!(
                            "-{MAX_ORBIT_TURN} to {MAX_ORBIT_TURN} (found {})",
                            orbit.turn_per_tick
                        ),
                    );
                }
                if !(1..=MAX_ORBIT_COOLDOWN_TICKS).contains(&orbit.cooldown_ticks) {
                    issue(
                        at("orbit.cooldown_ticks"),
                        format!(
                            "1 to {MAX_ORBIT_COOLDOWN_TICKS} (found {})",
                            orbit.cooldown_ticks
                        ),
                    );
                }
                if let Err(found) =
                    crate::validate::validate_attack_ref(&orbit.attack, registry::ENTRIES)
                {
                    // The attack's paths start at `$`.
                    for f in found {
                        issue(format!("{}{}", at("orbit.attack"), &f.path[1..]), f.message);
                    }
                }
            }
            (_, Some(_)) => issue(at("orbit"), "only an orbit reward holds an orbit".into()),
            (_, None) => {}
        }
        match (reward.effect, &reward.enchantment) {
            (RewardEffect::Enchantment, None) => issue(
                at("enchantment"),
                "an enchantment reward holds its enchantment".into(),
            ),
            (RewardEffect::Enchantment, Some(_)) => {
                if reward.limit != 1 {
                    issue(
                        at("limit"),
                        format!("an enchantment is taken once: 1 (found {})", reward.limit),
                    );
                }
            }
            (_, Some(_)) => issue(
                at("enchantment"),
                "only an enchantment reward holds an enchantment".into(),
            ),
            (_, None) => {}
        }
        if !(1..=MAX_STACK).contains(&reward.limit) {
            issue(
                at("limit"),
                format!("1 to {MAX_STACK} (found {})", reward.limit),
            );
        }
        if let Some(problem) = run_stats::text_problem(&reward.title, 1, run_stats::MAX_TITLE_CHARS)
        {
            issue(at("title"), problem);
        }
        if let Some(problem) =
            run_stats::text_problem(&reward.description, 0, run_stats::MAX_DESCRIPTION_CHARS)
        {
            issue(at("description"), problem);
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}
