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

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::attack::{AttackRef, DamageType};
use crate::id::ContentId;
use crate::pack::Pack;
use crate::registry;
use crate::run_stats::{self, StatEffect};
use crate::stage_skill::{MAX_SKILL_LEVELS, StageSkills};
use crate::trig::PER_DEGREE;
use crate::validate::limits::{DAMAGE_SCALE, MAX_EVADE_CHARGES, MAX_HEAL_AMOUNT};
use crate::validate::{Error, Issue};
use crate::version::{SCHEMA_VERSION, SIM_VERSION};

/// The largest rewards file read (a tuning value; ADR-020 decides that one exists).
pub const MAX_REWARDS_BYTES: usize = 32 * 1024;
/// The most rewards a file lists.
pub const MAX_REWARDS: usize = 64;
/// The largest amount: a percentage or a count; for hit points, `MAX_HP_AMOUNT`.
pub const MAX_AMOUNT: u32 = 100;
/// The largest amount of hit points: 5,000 shown hit points, scaled (ADR-103); Vigour (Supernova)
/// gives 1,600 shown.
pub const MAX_HP_AMOUNT: u32 = 5_000 * DAMAGE_SCALE;
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
/// The most one reward widens the pickup circle, in percent (a placeholder bound, ADR-020).
pub const MAX_PICKUP_RADIUS_PCT: u32 = 200;
/// The most the picks of a run widen it: four times the stage's circle in all.
pub const MAX_PICKUP_RADIUS_TOTAL_PCT: u32 = 300;
/// The most one reward shortens the stage skill's cooldown, in percent.
pub const MAX_SKILL_COOLDOWN_PCT: u32 = 50;
/// The most the picks add up to: a cooldown is never below 40 percent of its base.
pub const MAX_SKILL_COOLDOWN_TOTAL_PCT: u32 = 60;
/// The most kills a heal-per-kills reward waits for between heals.
pub const MAX_HEAL_KILLS: u32 = 100;
/// The most rerolls or banishes one reward gives.
pub const MAX_COUNTER_AMOUNT: u32 = 10;
/// The most candidates an offer holds once offer-candidate rewards are taken: the base offer
/// shows 3, and each such reward adds 1 (a placeholder bound, ADR-020).
pub const MAX_OFFER_CANDIDATES: usize = 4;

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
    /// Hit points the main character regenerates each second, `amount` more (the perk
    /// `regen`'s counter).
    Regen,
    /// The pickup circle of falling stars and heal pickups `amount` percent wider, adding up
    /// over the picks to `MAX_PICKUP_RADIUS_TOTAL_PCT`.
    PickupRadiusPct,
    /// The stage skill's cooldown `amount` percent shorter, adding up over the picks to
    /// `MAX_SKILL_COOLDOWN_TOTAL_PCT`.
    SkillCooldownPct,
    /// A heal of `amount` hit points (the stage's scale) each time the main character has
    /// defeated `kills` enemies: kills that give credit only, not a self-destruct. The picks
    /// add their amounts and heal at the shortest interval among them.
    HealPerKills,
    /// One more candidate in every offer, at most `MAX_OFFER_CANDIDATES` in all. Its `amount`
    /// is 1.
    OfferCandidates,
    /// `amount` more rerolls for the run, as the perk `extra_rerolls` gives them.
    Rerolls,
    /// `amount` more banishes for the run, as the perk `extra_banishes` gives them.
    Banishes,
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
            | RewardEffect::Regen
            | RewardEffect::PickupRadiusPct
            | RewardEffect::SkillCooldownPct
            | RewardEffect::HealPerKills
            | RewardEffect::OfferCandidates
            | RewardEffect::Rerolls
            | RewardEffect::Banishes
            | RewardEffect::RandomRune => return None,
        })
    }
}

/// One effect of a reward: an effect kind with the fields that kind takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectEntry {
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
    /// The kills between two heals, from 1 to `MAX_HEAL_KILLS`: with the `heal_per_kills`
    /// effect only, and required by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kills: Option<u32>,
}

/// The most effects one reward holds.
pub const MAX_REWARD_EFFECTS: usize = 3;
/// The most named rewards a file lists (a placeholder bound, ADR-020).
pub const MAX_NAMED_REWARDS: usize = 32;
/// The weight a reward has when its file gives none.
pub const DEFAULT_WEIGHT: u16 = 100;
/// The largest draw weight; the smallest is 1 (a placeholder bound, ADR-020).
pub const MAX_WEIGHT: u16 = 1_000;
/// The fewest times an unnamed reward stacks: ordinary rewards stack.
pub const MIN_UNNAMED_LIMIT: u8 = 2;

fn default_weight() -> u16 {
    DEFAULT_WEIGHT
}

fn is_default_weight(w: &u16) -> bool {
    *w == DEFAULT_WEIGHT
}

/// One reward an offer can hold. In a file it is written with an `effects` list of 1 to
/// `MAX_REWARD_EFFECTS` entries, or in the released form, with one effect's fields on the
/// reward itself, which reads as a list of one (ADR-026).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "RawReward")]
#[schemars(with = "RawReward")]
pub struct Reward {
    /// The reward's own ID, unique within the file.
    pub id: ContentId,
    pub rarity: Rarity,
    /// What taking it does, in order: 1 to `MAX_REWARD_EFFECTS`.
    pub effects: Vec<EffectEntry>,
    /// The most times one run takes it, from 1 to `MAX_STACK`. Required by every reward but a
    /// random rune, which has none of its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u8>,
    /// A named star (decision of app#336): taken once, epic or above, shown under `star`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub named: bool,
    /// The star's display name, 1 to `run_stats::MAX_TITLE_CHARS` characters, unique within
    /// the file: required when `named`, and only then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub star: Option<String>,
    /// Its share of a draw among the rewards of its rarity, from 1 to `MAX_WEIGHT`.
    #[serde(skip_serializing_if = "is_default_weight")]
    pub weight: u16,
    /// The name the choice screen shows, 1 to `run_stats::MAX_TITLE_CHARS` characters.
    pub title: String,
    /// What it does, in words, at most `run_stats::MAX_DESCRIPTION_CHARS` characters.
    pub description: String,
}

impl Reward {
    /// The first effect's kind: the reward's one, for a reward of one effect.
    pub fn effect(&self) -> RewardEffect {
        self.effects[0].effect
    }

    /// The first effect's amount.
    pub fn amount(&self) -> Option<u32> {
        self.effects[0].amount
    }

    /// Whether it is the random rune alone.
    pub fn is_random_rune(&self) -> bool {
        matches!(&self.effects[..], [e] if e.effect == RewardEffect::RandomRune)
    }

    /// The first entry that holds an orbit, if any.
    pub fn orbit(&self) -> Option<&Orbit> {
        self.effects.iter().find_map(|e| e.orbit.as_ref())
    }
}

/// A reward as a file writes it: the `effects` list, or the released form's single effect.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawReward {
    id: ContentId,
    rarity: Rarity,
    /// The effects, 1 to `MAX_REWARD_EFFECTS`: instead of `effect` and the fields of one.
    #[serde(default)]
    effects: Option<Vec<EffectEntry>>,
    /// The released form's single effect: with its `skill`, `orbit`, `type` and `amount`.
    #[serde(default)]
    effect: Option<RewardEffect>,
    #[serde(default)]
    skill: Option<ContentId>,
    #[serde(default)]
    orbit: Option<Orbit>,
    #[serde(rename = "type", default)]
    damage_type: Option<DamageType>,
    #[serde(default)]
    amount: Option<u32>,
    #[serde(default)]
    kills: Option<u32>,
    #[serde(default)]
    limit: Option<u8>,
    #[serde(default)]
    named: bool,
    #[serde(default)]
    star: Option<String>,
    #[serde(default = "default_weight")]
    weight: u16,
    title: String,
    description: String,
}

impl TryFrom<RawReward> for Reward {
    type Error = String;

    fn try_from(raw: RawReward) -> Result<Reward, String> {
        let single = raw.effect.is_some()
            || raw.skill.is_some()
            || raw.orbit.is_some()
            || raw.damage_type.is_some()
            || raw.amount.is_some()
            || raw.kills.is_some();
        let effects = match (raw.effects, raw.effect) {
            (Some(_), _) if single => {
                return Err("a reward has `effects` or one effect's fields, not both".into());
            }
            (Some(list), _) => list,
            (None, Some(effect)) => vec![EffectEntry {
                effect,
                skill: raw.skill,
                orbit: raw.orbit,
                damage_type: raw.damage_type,
                amount: raw.amount,
                kills: raw.kills,
            }],
            (None, None) => return Err("a reward has `effects` or an `effect`".into()),
        };
        Ok(Reward {
            id: raw.id,
            rarity: raw.rarity,
            effects,
            limit: raw.limit,
            named: raw.named,
            star: raw.star,
            weight: raw.weight,
            title: raw.title,
            description: raw.description,
        })
    }
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
        Pack::bundled().content().rewards()
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

/// The largest amount of one effect.
fn most_of(effect: RewardEffect) -> u32 {
    match effect {
        RewardEffect::StageSkill => MAX_SKILL_LEVELS as u32,
        RewardEffect::EvadeCharge => u32::from(MAX_EVADE_CHARGES),
        RewardEffect::Orbit => u32::from(MAX_ORBS),
        RewardEffect::RangePct => MAX_RANGE_PCT,
        RewardEffect::RuneHaste => MAX_RUNE_HASTE_PCT,
        RewardEffect::MaxHp | RewardEffect::Regen => MAX_HP_AMOUNT,
        RewardEffect::PickupRadiusPct => MAX_PICKUP_RADIUS_PCT,
        RewardEffect::SkillCooldownPct => MAX_SKILL_COOLDOWN_PCT,
        RewardEffect::HealPerKills => MAX_HEAL_AMOUNT,
        RewardEffect::OfferCandidates => 1,
        RewardEffect::Rerolls | RewardEffect::Banishes => MAX_COUNTER_AMOUNT,
        _ => MAX_AMOUNT,
    }
}

/// One effect entry on its own: its amount, skill, orbit, type and kills, by its kind.
fn validate_entry(
    entry: &EffectEntry,
    at: &dyn Fn(&str) -> String,
    stage_skills: &StageSkills,
    issue: &mut dyn FnMut(String, String),
) {
    let most = most_of(entry.effect);
    match (entry.effect, entry.amount) {
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
    match (entry.effect, &entry.skill) {
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
    match (entry.effect, &entry.orbit) {
        (RewardEffect::Orbit, None) => issue(at("orbit"), "an orbit reward holds its orbit".into()),
        (RewardEffect::Orbit, Some(orbit)) => {
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
    match (entry.effect, entry.damage_type) {
        (RewardEffect::TypeBonusPct, None) => {
            issue(at("type"), "a type bonus reward names its type".into())
        }
        (RewardEffect::TypeBonusPct, Some(_)) | (_, None) => {}
        (_, Some(_)) => issue(at("type"), "only a type bonus reward names a type".into()),
    }
    match (entry.effect, entry.kills) {
        (RewardEffect::HealPerKills, None) => {
            issue(at("kills"), format!("1 to {MAX_HEAL_KILLS} (missing)"))
        }
        (RewardEffect::HealPerKills, Some(kills)) => {
            if !(1..=MAX_HEAL_KILLS).contains(&kills) {
                issue(
                    at("kills"),
                    format!("1 to {MAX_HEAL_KILLS} (found {kills})"),
                );
            }
        }
        (_, Some(_)) => issue(
            at("kills"),
            "only a heal-per-kills reward names its kills".into(),
        ),
        (_, None) => {}
    }
}

/// The caps over the entries of one reward: what a kind's entries add up to stays within the
/// kind's largest amount, as the picks of a run do (ADR-020), a random rune stands alone, and
/// a reward holds one orbit.
fn validate_bundle(
    reward: &Reward,
    at: &dyn Fn(&str) -> String,
    issue: &mut dyn FnMut(String, String),
) {
    let mut sums: Vec<(RewardEffect, Option<&ContentId>, Option<DamageType>, u64)> = Vec::new();
    for e in &reward.effects {
        let amount = u64::from(e.amount.unwrap_or(0));
        let key = (e.effect, e.skill.as_ref(), e.damage_type);
        match sums.iter_mut().find(|(k, s, t, _)| (*k, *s, *t) == key) {
            Some(sum) => sum.3 += amount,
            None => sums.push((key.0, key.1, key.2, amount)),
        }
    }
    for (effect, _, _, sum) in &sums {
        let most = most_of(*effect);
        if *sum > u64::from(most) {
            issue(
                at("effects"),
                format!(
                    "the {effect:?} amounts of one reward add up to at most {most} (found {sum})"
                ),
            );
        }
    }
    let orbits = reward
        .effects
        .iter()
        .filter(|e| e.effect == RewardEffect::Orbit)
        .count();
    if orbits > 1 {
        issue(at("effects"), "a reward holds at most one orbit".into());
    }
    if reward.effects.len() > 1
        && reward
            .effects
            .iter()
            .any(|e| e.effect == RewardEffect::RandomRune)
    {
        issue(
            at("effects"),
            "a random rune is a reward of its own, with no other effect".into(),
        );
    }
}

/// The named-star rules for one reward, given the rewards before it: a named reward is taken
/// once and is epic or above, an unnamed one stacks, the star's name is bounded and unique,
/// and the draw weight is within its bounds.
fn validate_naming(
    earlier: &[Reward],
    reward: &Reward,
    at: &dyn Fn(&str) -> String,
    issue: &mut dyn FnMut(String, String),
) {
    if reward.named {
        if reward.limit != Some(1) {
            issue(
                at("limit"),
                "a named reward is taken once: its limit is 1".into(),
            );
        }
        if reward.rarity < Rarity::Epic {
            issue(
                at("rarity"),
                "a named reward is epic, legendary or supernova".into(),
            );
        }
    } else if !reward.is_random_rune() && reward.limit.is_some_and(|l| l < MIN_UNNAMED_LIMIT) {
        issue(
            at("limit"),
            format!("an unnamed reward stacks: its limit is {MIN_UNNAMED_LIMIT} or more"),
        );
    }
    match (&reward.star, reward.named) {
        (None, true) => issue(at("star"), "a named reward has a star name".into()),
        (Some(_), false) => issue(at("star"), "only a named reward has a star name".into()),
        (Some(star), true) => {
            if let Some(problem) = run_stats::text_problem(star, 1, run_stats::MAX_TITLE_CHARS) {
                issue(at("star"), problem);
            } else if earlier.iter().any(|r| r.star.as_ref() == Some(star)) {
                issue(at("star"), "the same star name as an earlier reward".into());
            }
        }
        (None, false) => {}
    }
    if !(1..=MAX_WEIGHT).contains(&reward.weight) {
        issue(
            at("weight"),
            format!("1 to {MAX_WEIGHT} (found {})", reward.weight),
        );
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
        if reward.effects.is_empty() || reward.effects.len() > MAX_REWARD_EFFECTS {
            issue(
                at("effects"),
                format!(
                    "1 to {MAX_REWARD_EFFECTS} effects (found {})",
                    reward.effects.len()
                ),
            );
        }
        for (k, entry) in reward.effects.iter().enumerate() {
            let at = |field: &str| format!("$.rewards[{i}].effects[{k}].{field}");
            validate_entry(entry, &at, stage_skills, &mut issue);
            if entry.effect == RewardEffect::Orbit && entry.orbit.is_some() {
                let earlier = file.rewards[..i]
                    .iter()
                    .filter(|r| r.effects.iter().any(|e| e.effect == RewardEffect::Orbit))
                    .count();
                if k == 0 && earlier >= MAX_ORBITS {
                    issue(
                        at("effect"),
                        format!("at most {MAX_ORBITS} orbit rewards in a file"),
                    );
                }
            }
        }
        match (reward.is_random_rune(), reward.limit) {
            (true, None) => {}
            (true, Some(_)) => issue(
                at("limit"),
                "a random rune has no limit: it leaves the offers once every rune is held".into(),
            ),
            (false, None) => issue(at("limit"), format!("1 to {MAX_STACK} (missing)")),
            (false, Some(limit)) => {
                if !(1..=MAX_STACK).contains(&limit) {
                    issue(at("limit"), format!("1 to {MAX_STACK} (found {limit})"));
                }
            }
        }
        validate_bundle(reward, &at, &mut issue);
        validate_naming(&file.rewards[..i], reward, &at, &mut issue);
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
    let named = file.rewards.iter().filter(|r| r.named).count();
    if named > MAX_NAMED_REWARDS {
        issue(
            "$.rewards".into(),
            format!("at most {MAX_NAMED_REWARDS} named rewards (found {named})"),
        );
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}
