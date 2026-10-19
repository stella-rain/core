//! Rewards (ADR-079 §3, ADR-067, ADR-068): what an offer from a falling star holds. Each has a
//! rarity, a stack limit (how many times one run takes it) and an effect. They are content, not
//! code: a JSON list of any length up to `MAX_REWARDS`, read in its order. Data picks, code
//! acts: a reward's effect is one of the run stats' effect kinds (`StatEffect`), adding
//! `amount` to that effect's total, as a stat point adds its step, a stage skill (ADR-075,
//! ADR-098): one of the stage skills file's skills, `amount` levels of it, `amount` more
//! evade charges, up to the stage's `max_charges` (ADR-074), an orbit of `amount` orbs that
//! attack on their own (ADR-069), `amount` percent more range for every combo step of the
//! Main weapon (ADR-099), or a type bonus: `amount` percent more damage for the main
//! character's attacks of one damage type, of every attack type or of every element (ADR-100),
//! or a random rune: one of the runes the Main weapon does not hold, drawn when it is picked
//! (ADR-099). A random rune has no amount and no stack limit of its own: it leaves the offers
//! once the Main holds every rune.
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

use crate::attack::{AttackRef, DamageType};
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
/// reward's amount and holds what the range rewards and the expertise range options add up to
/// (ADR-099, ADR-020).
pub const MAX_RANGE_PCT: u32 = 100;
/// The most Rune Haste one reward gives, in percent, and the most the picks add up to: a
/// cooldown or tick interval is then a quarter as long, and never less than a tick
/// (ADR-099 point 5, ADR-020).
pub const MAX_RUNE_HASTE_PCT: u32 = 300;
/// The most the type bonuses add to one attack's damage, in percent: eleven times as much. It
/// bounds the match-up file's `max_type_bonus_pct`, where the bonuses that apply to an
/// attack's type saturate (ADR-100, ADR-020).
pub const MAX_TYPE_BONUS_PCT: u32 = 1_000;

/// `ticks` shortened by `haste_pct` percent of Rune Haste (ADR-099 point 5): `ticks × 100 /
/// (100 + h)` rounded up, at least 1. The haste is held to `MAX_RUNE_HASTE_PCT`, so an
/// unvalidated sum cannot pass it.
pub fn rune_haste_ticks(ticks: u32, haste_pct: u32) -> u32 {
    let h = u64::from(haste_pct.min(MAX_RUNE_HASTE_PCT));
    let shortened = (u64::from(ticks) * 100).div_ceil(100 + h);
    u32::try_from(shortened).unwrap_or(u32::MAX).max(1)
}

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
/// skill, more evade charges, an orbit, range, a type bonus, or a random rune.
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
    /// The main character's attacks of the reward's `type` `amount` percent stronger (ADR-100).
    TypeBonusPct,
    /// Its attacks of every attack type (Slash, Pierce, Strike) `amount` percent stronger.
    AttackTypeBonusPct,
    /// Its attacks of every element `amount` percent stronger.
    ElementBonusPct,
    /// Every on-hit rune's cooldown and every damage-over-time tick interval the main character
    /// applies `amount` percent of haste shorter, adding up over the picks (ADR-099): see
    /// `rune_haste_ticks`.
    RuneHaste,
    /// One rune the Main weapon does not hold, drawn from the run's generator when the reward
    /// is picked, never before: an offer shows it hidden (ADR-099). It is offered only while
    /// the Main lacks a rune.
    RandomRune,
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
            | RewardEffect::RuneHaste
            | RewardEffect::TypeBonusPct
            | RewardEffect::AttackTypeBonusPct
            | RewardEffect::ElementBonusPct
            | RewardEffect::RandomRune => return None,
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
    /// The damage type its bonus raises: with the `type_bonus_pct` effect only, and required
    /// by it.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub damage_type: Option<DamageType>,
    /// What taking it adds to the effect's total, from 1 to `MAX_AMOUNT`; for a stage skill,
    /// the levels it gives, from 1 to `MAX_SKILL_LEVELS`; for evade charges, the charges it
    /// adds, from 1 to `MAX_EVADE_CHARGES`; for an orbit, the orbs it adds, from 1 to
    /// `MAX_ORBS`; for range, how much wider, from 1 to `MAX_RANGE_PCT` percent; for Rune
    /// Haste, the percent of haste, from 1 to `MAX_RUNE_HASTE_PCT`; for a type bonus, the
    /// percent, from 1 to `MAX_AMOUNT`. Required by every effect but the random rune, which
    /// has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<u32>,
    /// The most times one run takes it, from 1 to `MAX_STACK`. Required by every effect but
    /// the random rune, which has none of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u8>,
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
            RewardEffect::RangePct => MAX_RANGE_PCT,
            RewardEffect::RuneHaste => MAX_RUNE_HASTE_PCT,
            _ => MAX_AMOUNT,
        };
        match (reward.effect, reward.amount) {
            (RewardEffect::RandomRune, None) => {}
            (RewardEffect::RandomRune, Some(_)) => {
                issue(at("amount"), "a random rune has no amount".into())
            }
            (_, None) => issue(at("amount"), format!("1 to {most} (missing)")),
            (_, Some(amount)) => {
                if !(1..=most).contains(&amount) {
                    issue(at("amount"), format!("1 to {most} (found {amount})"));
                }
            }
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
        match (reward.effect, reward.damage_type) {
            (RewardEffect::TypeBonusPct, None) => {
                issue(at("type"), "a type bonus reward names its type".into())
            }
            (RewardEffect::TypeBonusPct, Some(_)) | (_, None) => {}
            (_, Some(_)) => issue(at("type"), "only a type bonus reward names a type".into()),
        }
        match (reward.effect, reward.limit) {
            (RewardEffect::RandomRune, None) => {}
            (RewardEffect::RandomRune, Some(_)) => issue(
                at("limit"),
                "a random rune has no limit: it leaves the offers once every rune is held".into(),
            ),
            (_, None) => issue(at("limit"), format!("1 to {MAX_STACK} (missing)")),
            (_, Some(limit)) => {
                if !(1..=MAX_STACK).contains(&limit) {
                    issue(at("limit"), format!("1 to {MAX_STACK} (found {limit})"));
                }
            }
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
