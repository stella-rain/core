//! Rewards (ADR-REPL-5, ADR-CONT-1): what an offer from a falling star holds. Each has a
//! rarity, a stack limit (how many times one run takes it) and an effect. They are content, not
//! code: a JSON list of any length up to `MAX_REWARDS`, read in its order. Data picks, code
//! acts: a reward's effect is one of the run stats' effect kinds (`StatEffect`), adding
//! `amount` to that effect's total, as a stat point adds its step, a stage skill: one of the stage skills file's skills, `amount` levels of it, `amount` more
//! evade charges, up to the stage's `max_charges`, an orbit of `amount` orbs that
//! attack on their own, `amount` percent more range for every combo step of the
//! Main weapon, or a type bonus: `amount` percent more damage for the main
//! character's attacks of one damage type, of every attack type or of every element,
//! or a random rune: one of the runes the Main weapon does not hold, drawn when it is picked
//!. A random rune alone has no amount and no stack limit of its own: it leaves the
//! offers once the Main holds every rune. In a bundle with other effects (a named star's) it is
//! one entry of a reward that has its limit, and a reward holds at most one.
//!
//! The file is untrusted (ADR-SVC-1), as the run stats are: a size limit before parsing, strict
//! parsing, the current versions only, and every field bounded. Reward IDs are the file's own,
//! unique within it; a run setup's star pool names them. A reward's skill must be in the stage
//! skills file the rewards are read with. The bundled file, `content/rewards.json`, is the
//! default; the SHA-256 of whatever bytes a run used is part of its content hash.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::attack::{AttackRef, DamageType};
use crate::constellations::Constellations;
use crate::id::ContentId;
use crate::limits::{MAX_CATEGORY_SLOTS, MAX_EQUIP_CATEGORIES, MAX_EQUIP_SLOTS, MAX_SLOT_LEVEL};
use crate::pack::Pack;
use crate::registry;
use crate::run_setup::MAX_LUMINE;
use crate::run_stats::{self, StatEffect};
use crate::stage_skill::{MAX_SKILL_LEVELS, StageSkills};
use crate::trig::PER_DEGREE;
use crate::validate::limits::{DAMAGE_SCALE, MAX_EVADE_CHARGES, MAX_HEAL_AMOUNT};
use crate::validate::{Error, Issue};
use crate::version::{self, Versions};
use std::sync::Arc;

/// The largest rewards file read (a tuning value; ADR-SVC-1 decides that one exists).
pub const MAX_REWARDS_BYTES: usize = 64 * 1024;
/// The most rewards a file lists.
pub const MAX_REWARDS: usize = 192;
/// The largest amount: a percentage or a count; for hit points, `MAX_HP_AMOUNT`.
pub const MAX_AMOUNT: u32 = 100;
/// The largest amount of one type bonus reward, in percent: twice `MAX_AMOUNT`, so that the
/// supernova row (128, D6c) fits. The bonuses that apply to an attack's type saturate at the
/// match-up file's `max_type_bonus_pct` (a placeholder bound, ADR-SVC-1).
pub const MAX_TYPE_BONUS_AMOUNT: u32 = 200;
/// The largest amount of hit points: 5,000 shown hit points, scaled; Vigour (Supernova)
/// gives 1,600 shown.
pub const MAX_HP_AMOUNT: u32 = 5_000 * DAMAGE_SCALE;
/// The most times one run takes one reward.
pub const MAX_STACK: u8 = 20;
/// The most orbs one orbit holds, however often its reward is taken.
pub const MAX_ORBS: u8 = 8;
/// The most orbit rewards a file lists, and so the most orbits a run holds.
pub const MAX_ORBITS: usize = 4;
/// Where the orbit keys of constellations start: a set's orbit is keyed `ORBIT_SET_BASE` plus its
/// place in the file, above any place of a reward (`MAX_REWARDS` is 192).
pub const ORBIT_SET_BASE: u16 = 0x8000;
/// The fastest an orbit turns: 15 degrees a tick, either way.
pub const MAX_ORBIT_TURN: i32 = 15 * PER_DEGREE;
/// The longest cooldown of an orbit: one minute.
pub const MAX_ORBIT_COOLDOWN_TICKS: u32 = 3_600;
/// The most a combo step's hit box is widened, in percent: twice as large. It bounds one range
/// reward's amount and holds what the range rewards and the expertise range options add up to
/// (ADR-SVC-1).
pub const MAX_RANGE_PCT: u32 = 100;
/// The ceiling of Rune Haste, in percent: the most the rewards file's `max_rune_haste_pct` may
/// name, and so the most one reward gives and the picks add up to, a cooldown or tick interval
/// then being a ninth as long, and never less than a tick (ADR-SVC-1). The
/// file's cap (300 in the bundle, app#298) is the number; this is the bound on it.
pub const MAX_RUNE_HASTE_PCT: u32 = 1_000;
/// The cap of a rewards file that names none: the cap before it was content (app#298 raised
/// the bundle's to 500).
pub const DEFAULT_MAX_RUNE_HASTE_PCT: u32 = 300;
/// The most the type bonuses add to one attack's damage, in percent: eleven times as much. It
/// bounds the match-up file's `max_type_bonus_pct`, where the bonuses that apply to an
/// attack's type saturate (ADR-SVC-1).
pub const MAX_TYPE_BONUS_PCT: u32 = 1_000;
/// The most one reward widens the pickup circle, in percent (a placeholder bound, ADR-SVC-1).
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
/// shows 3, and each such reward adds 1 (a placeholder bound, ADR-SVC-1).
pub const MAX_OFFER_CANDIDATES: usize = 4;

/// The most one reward adds to the chance of a critical hit, in percent (a placeholder bound,
/// ADR-SVC-1).
pub const MAX_CRIT_CHANCE_PCT: u32 = 50;
/// The most the picks of a run add up to: a hit is a crit three times in four at the most.
pub const MAX_CRIT_CHANCE_TOTAL_PCT: u32 = 75;
/// The most one reward adds to a crit's bonus damage, in percent. A crit does twice the damage
/// before any bonus.
pub const MAX_CRIT_DAMAGE_PCT: u32 = 200;
/// The most the picks add up to: a crit does five times the damage at the most.
pub const MAX_CRIT_DAMAGE_TOTAL_PCT: u32 = 300;
/// The most one reward lets an echo repeat of a hit's damage, in percent, and the most the
/// picks add up to.
pub const MAX_ECHO_HIT_PCT: u32 = 100;
/// The most echoes one tick resolves; the rest are dropped, first-come first-served (ADR-SIM-5).
pub const MAX_ECHOES_PER_TICK: usize = 64;
/// The most of a killed enemy's maximum hit points a kill burst deals, in percent (a reward's
/// `amount`), and the most the picks add up to.
pub const MAX_BURST_PCT: u32 = 100;
/// The widest a kill burst reaches, in pixels (a reward's `radius`), and the most the picks
/// add up to.
pub const MAX_BURST_RADIUS_PX: u32 = 240;
/// The most links of a chain of bursts (a reward's `depth`): the first burst is the first link.
pub const MAX_BURST_DEPTH: u32 = 5;
/// The most bursts one tick resolves, as ADR-SIM-5's contagion cap; the rest are dropped.
pub const MAX_BURSTS_PER_TICK: usize = 8;

/// `ticks` shortened by `haste_pct` percent of Rune Haste: `ticks × 100 /
/// (100 + h)` rounded up, at least 1. The haste is held to `MAX_RUNE_HASTE_PCT`, the ceiling,
/// so an unvalidated sum cannot pass it; a run holds its sum to the file's cap first.
pub fn rune_haste_ticks(ticks: u32, haste_pct: u32) -> u32 {
    let h = u64::from(haste_pct.min(MAX_RUNE_HASTE_PCT));
    let shortened = (u64::from(ticks) * 100).div_ceil(100 + h);
    u32::try_from(shortened).unwrap_or(u32::MAX).max(1)
}

/// A reward's rarity, from the most common up (ADR-REPL-5).
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

    /// The rarity's place in `ALL`, from 0.
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
    /// The stage skill the reward's `skill` names, `amount` levels of it.
    StageSkill,
    /// `amount` more evade charges, never beyond the stage's `max_charges`; each arrives ready
    ///.
    EvadeCharge,
    /// The reward's `orbit`, with `amount` more orbs, never beyond `MAX_ORBS`.
    Orbit,
    /// Every combo step of the Main weapon `amount` percent wider, as the expertise range
    /// option widens it, adding up over the picks.
    RangePct,
    /// The main character's attacks of the reward's `type` `amount` percent stronger.
    TypeBonusPct,
    /// Its attacks of every attack type (Slash, Pierce, Strike) `amount` percent stronger.
    AttackTypeBonusPct,
    /// Its attacks of every element `amount` percent stronger.
    ElementBonusPct,
    /// Every on-hit rune's cooldown and every damage-over-time tick interval the main character
    /// applies `amount` percent of haste shorter, adding up over the picks: see
    /// `rune_haste_ticks`.
    RuneHaste,
    /// One rune the Main weapon does not hold, drawn from the run's generator when the reward
    /// is picked, never before: an offer shows it hidden. It is offered only while
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
    /// `amount` percent more chance that a hit of the main character is a critical hit, adding
    /// up over the picks to `MAX_CRIT_CHANCE_TOTAL_PCT`.
    CritChancePct,
    /// A critical hit does `amount` percent more damage than the base twice, adding up over
    /// the picks to `MAX_CRIT_DAMAGE_TOTAL_PCT`.
    CritDamagePct,
    /// Each hit of the main character is repeated once, a tick later, on the same target, for
    /// `amount` percent of its damage; the picks add up to `MAX_ECHO_HIT_PCT`.
    EchoHitPct,
    /// A kill by the main character bursts: it damages the other enemies within `radius`
    /// pixels for `amount` percent of the dead enemy's maximum hit points, and a burst kill
    /// bursts again, down to `depth` links.
    KillBurst,
    /// `amount` more links on the chain of kill bursts, up to `MAX_BURST_DEPTH` in all: it adds
    /// to the depth of a `kill_burst` held, in whatever order they came, and does nothing
    /// alone.
    BurstDepth,
}

impl RewardEffect {
    /// Every kind, in declaration order. Not serialized.
    pub const ALL: [RewardEffect; 27] = [
        RewardEffect::MaxHp,
        RewardEffect::AttackPct,
        RewardEffect::AttackSpeedPct,
        RewardEffect::EvadePct,
        RewardEffect::SpeedPct,
        RewardEffect::XpPct,
        RewardEffect::StageSkill,
        RewardEffect::EvadeCharge,
        RewardEffect::Orbit,
        RewardEffect::RangePct,
        RewardEffect::TypeBonusPct,
        RewardEffect::AttackTypeBonusPct,
        RewardEffect::ElementBonusPct,
        RewardEffect::RuneHaste,
        RewardEffect::RandomRune,
        RewardEffect::Regen,
        RewardEffect::PickupRadiusPct,
        RewardEffect::SkillCooldownPct,
        RewardEffect::HealPerKills,
        RewardEffect::OfferCandidates,
        RewardEffect::Rerolls,
        RewardEffect::Banishes,
        RewardEffect::CritChancePct,
        RewardEffect::CritDamagePct,
        RewardEffect::EchoHitPct,
        RewardEffect::KillBurst,
        RewardEffect::BurstDepth,
    ];

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
            | RewardEffect::CritChancePct
            | RewardEffect::CritDamagePct
            | RewardEffect::EchoHitPct
            | RewardEffect::KillBurst
            | RewardEffect::BurstDepth
            | RewardEffect::RandomRune => return None,
        })
    }

    /// The stat a Growth effect becomes in equipment mode, if it is one.
    pub fn growth_stat(self) -> Option<StatEffect> {
        match self {
            RewardEffect::Rerolls => Some(StatEffect::Rerolls),
            RewardEffect::Banishes => Some(StatEffect::Banishes),
            RewardEffect::OfferCandidates => Some(StatEffect::OfferCandidates),
            _ => None,
        }
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
    /// Haste, the percent of haste, from 1 to the file's `max_rune_haste_pct`; for a type bonus,
    /// the percent, from 1 to `MAX_TYPE_BONUS_AMOUNT`. Required by every effect but the random
    /// rune, which has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<u32>,
    /// The kills between two heals, from 1 to `MAX_HEAL_KILLS`: with the `heal_per_kills`
    /// effect only, and required by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kills: Option<u32>,
    /// How far a burst reaches, in pixels, from 1 to `MAX_BURST_RADIUS_PX`: with the
    /// `kill_burst` effect only, and required by it. Its `amount` is the percent of hit points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<u32>,
    /// The links of a chain of bursts, from 1 to `MAX_BURST_DEPTH`: with the `kill_burst`
    /// effect only, and required by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
}

/// The most effects one reward holds.
pub const MAX_REWARD_EFFECTS: usize = 4;
/// The most named rewards a file lists (a placeholder bound, ADR-SVC-1).
pub const MAX_NAMED_REWARDS: usize = 32;
/// The weight a reward has when its file gives none.
pub const DEFAULT_WEIGHT: u16 = 100;
/// The largest draw weight; the smallest is 1 (a placeholder bound, ADR-SVC-1).
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
/// reward itself, which reads as a list of one (ADR-CONT-2).
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
    pub star: Option<Arc<str>>,
    /// Its share of a draw among the rewards of its rarity, from 1 to `MAX_WEIGHT`.
    #[serde(skip_serializing_if = "is_default_weight")]
    pub weight: u16,
    /// The equipment category it goes in, one of the file's `equipment.categories`; absent for a
    /// reward that cannot be equipped (a Growth star, the random rune).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<ContentId>,
    /// Retired: still loadable and playable (a setup's pool may name it), but equipment mode's
    /// default pool leaves it out (ADR-CONT-2 points 3, 8, 9).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub deprecated: bool,
    /// Offered at `sim_version` 2 only: the default pool of a version 1 run leaves it out and a
    /// version 1 setup may not name it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub equipment_only: bool,
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
    radius: Option<u32>,
    #[serde(default)]
    depth: Option<u32>,
    #[serde(default)]
    limit: Option<u8>,
    #[serde(default)]
    named: bool,
    #[serde(default)]
    star: Option<Arc<str>>,
    #[serde(default = "default_weight")]
    weight: u16,
    #[serde(default)]
    category: Option<ContentId>,
    #[serde(default)]
    deprecated: bool,
    #[serde(default)]
    equipment_only: bool,
}

impl TryFrom<RawReward> for Reward {
    type Error = String;

    fn try_from(raw: RawReward) -> Result<Reward, String> {
        let single = raw.effect.is_some()
            || raw.skill.is_some()
            || raw.orbit.is_some()
            || raw.damage_type.is_some()
            || raw.amount.is_some()
            || raw.kills.is_some()
            || raw.radius.is_some()
            || raw.depth.is_some();
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
                radius: raw.radius,
                depth: raw.depth,
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
            category: raw.category,
            deprecated: raw.deprecated,
            equipment_only: raw.equipment_only,
        })
    }
}

/// Orbs that turn round the main character and attack on their own. Each orb runs
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

/// The number of rarities a rewards file prices: every rarity once.
pub const MAX_RARITIES: usize = Rarity::ALL.len();

/// What carrying a star of one rarity costs, in Lumine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RarityCost {
    /// The rarity priced.
    pub rarity: Rarity,
    /// Its cost in Lumine, 1 to `run_setup::MAX_LUMINE`.
    pub lumine_cost: u8,
}

/// One slot category of the equipment layout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EquipmentCategory {
    /// The category's ID, unique within the list.
    pub id: ContentId,
    /// Its slots, 1 to `MAX_CATEGORY_SLOTS`.
    pub slots: u8,
    /// The most extra drop weight in percent the player's setup may give this category
    /// (`RunSetup::drop_weights`, ADR-SAVE-17), 0 to `MAX_DROP_WEIGHT_PCT`. Absent: 0, so a pack
    /// released before it allows none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub drop_weight_max: u32,
    /// The weight the Star Chart screen starts this category at, 0 to `drop_weight_max`: the app
    /// writes it into the setup; core never reads it (a missing key is 0 extra). Absent: 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub drop_weight_default: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// What one rarity of star is worth in Stardust (equipment mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RarityStarlight {
    /// The rarity priced.
    pub rarity: Rarity,
    /// What extracting a star of it gives, 1 to `MAX_STARLIGHT`.
    pub extract: u32,
    /// What crafting a star of it costs, `extract` to `MAX_STARLIGHT`.
    pub craft_price: u32,
}

/// The largest Starlight value of the equipment rules.
pub const MAX_STARLIGHT: u32 = 1_000_000;
/// The largest level percentage of the equipment rules.
pub const MAX_LEVEL_PCT: u32 = 50;
/// The most boss drops a boss kill pays each member.
pub const MAX_BOSS_DROPS: u8 = 8;

/// The largest `drop_weight_max` of a category, in percent: a ceiling in code (ADR-CONT-3).
pub const MAX_DROP_WEIGHT_PCT: u32 = 1000;

/// The equipment rules (ADR-CONT-3): the slot layout, the level curve, the Starlight values and
/// the drop numbers, which a designer tunes. Bounds are code.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EquipmentRules {
    /// The slot categories in slot order, 1 to `MAX_EQUIP_CATEGORIES`.
    pub categories: Vec<EquipmentCategory>,
    /// The rune slots after the category slots, 1 to `MAX_CATEGORY_SLOTS`. The category slots,
    /// the rune slots and the Lumine slot add up to at most `MAX_EQUIP_SLOTS`.
    pub rune_slots: u8,
    /// The kills a slot needs for each next level: exactly `MAX_SLOT_LEVEL - 1` entries, strictly
    /// rising, each 1 to `MAX_STARLIGHT`.
    pub kills_to_next: Vec<u32>,
    /// What each level adds to a star's amount, in percent, 1 to `MAX_LEVEL_PCT`.
    pub level_pct: u32,
    /// Every rarity once, from common up.
    pub rarities: Vec<RarityStarlight>,
    /// What extracting a rune gives, 1 to `MAX_STARLIGHT`.
    pub rune_extract: u32,
    /// What a boss kill pays, 0 to `MAX_STARLIGHT`.
    pub boss_bonus: u32,
    /// The drops a boss kill rolls for each member who is up, 0 to `MAX_BOSS_DROPS`.
    pub boss_drops: u8,
    /// The chance in a thousand that a drop is a rune, 0 to 1000.
    pub rune_share_permille: u32,
}

/// The rewards file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RewardsFile {
    /// The equipment rules; absent in a file made before them, which then cannot start an
    /// equipment run (kept packs, ADR-CONT-5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equipment: Option<EquipmentRules>,
    /// The most Rune Haste the picks add up to, in percent, 1 to `MAX_RUNE_HASTE_PCT`
    /// (app#298); absent, `DEFAULT_MAX_RUNE_HASTE_PCT`. A reward gives at most this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rune_haste_pct: Option<u32>,
    /// What carrying a star of each rarity costs, in Lumine: every rarity
    /// once, from common up, each 1 to `run_setup::MAX_LUMINE`. The run setup's carried stars
    /// are priced from this list.
    pub rarities: Vec<RarityCost>,
    /// The rewards, in the order offers draw them from.
    pub rewards: Vec<Reward>,
}

/// Rewards that passed validation, the SHA-256 of the bytes they came from, and the stage
/// skills they were checked against, which a run reads with them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rewards {
    rewards: Vec<Reward>,
    max_rune_haste_pct: u32,
    /// The Lumine cost of a star of each rarity, by `Rarity::index`.
    lumine_costs: [u8; MAX_RARITIES],
    equipment: Option<EquipmentRules>,
    stage_skills: StageSkills,
    /// The sets of named stars checked against these rewards (core#NS3): none unless a pack
    /// or a caller gives them (`with_constellations`).
    constellations: Constellations,
    sha256: [u8; 32],
}

impl Rewards {
    /// The bundled rewards, with the bundled stage skills: the default of every run.
    pub fn bundled() -> &'static Rewards {
        Pack::bundled().content().rewards()
    }

    /// `parse_with` the bundled stage skills.
    pub fn parse(bytes: &[u8], versions: Versions) -> Result<Rewards, Error> {
        Rewards::parse_with(bytes, versions, StageSkills::bundled())
    }

    /// Checks the size and the versions given (the pack's), parses strictly and validates, each
    /// reward's skill in `stage_skills`: the one call for a file's bytes.
    pub fn parse_with(
        bytes: &[u8],
        versions: Versions,
        stage_skills: &StageSkills,
    ) -> Result<Rewards, Error> {
        let file: RewardsFile = version::read_member(bytes, MAX_REWARDS_BYTES, versions)?;
        validate(&file, stage_skills)?;
        let mut lumine_costs = [0; MAX_RARITIES];
        for cost in &file.rarities {
            lumine_costs[usize::from(cost.rarity.index())] = cost.lumine_cost;
        }
        Ok(Rewards {
            rewards: file.rewards,
            lumine_costs,
            equipment: file.equipment,
            max_rune_haste_pct: file
                .max_rune_haste_pct
                .unwrap_or(DEFAULT_MAX_RUNE_HASTE_PCT),
            stage_skills: stage_skills.clone(),
            constellations: Constellations::default(),
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The most Rune Haste the picks add up to, in percent: the file's, or the default.
    pub fn max_rune_haste_pct(&self) -> u32 {
        self.max_rune_haste_pct
    }

    /// What carrying a star of `rarity` costs, in Lumine: the file's `rarities`.
    pub fn lumine_cost(&self, rarity: Rarity) -> u8 {
        self.lumine_costs[usize::from(rarity.index())]
    }

    /// The equipment rules, if the file holds them.
    pub fn equipment(&self) -> Option<&EquipmentRules> {
        self.equipment.as_ref()
    }

    /// The places of the run's star pool when its setup names none (ADR-CONT-3 point 8): at
    /// `sim_version` 2 and later every reward that has a category and is not deprecated, at 1
    /// every reward that is not equipment-only.
    pub fn default_pool(&self, sim_version: u32) -> Vec<u16> {
        self.rewards
            .iter()
            .enumerate()
            .filter(|(_, r)| match sim_version {
                1 => !r.equipment_only,
                _ => r.category.is_some() && !r.deprecated,
            })
            .map(|(i, _)| u16::try_from(i).unwrap_or(u16::MAX))
            .collect()
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

    /// The same rewards with `constellations`, which were read against them
    /// (`Constellations::parse_with`).
    pub fn with_constellations(self, constellations: Constellations) -> Rewards {
        Rewards {
            constellations,
            ..self
        }
    }

    /// The sets of named stars; empty for rewards read without the file.
    pub fn constellations(&self) -> &Constellations {
        &self.constellations
    }

    /// The SHA-256 of the bytes they were parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }

    /// The orbit a run's orbit key names: the key of an orbit reward is its place in the file,
    /// that of a constellation's orbit is `ORBIT_SET_BASE` and the set's place.
    pub fn orbit_def(&self, key: u16) -> Option<&Orbit> {
        match key.checked_sub(ORBIT_SET_BASE) {
            None => self.rewards.get(usize::from(key))?.orbit(),
            Some(set) => self
                .constellations
                .sets()
                .get(usize::from(set))?
                .tiers
                .iter()
                .flat_map(|t| &t.effects)
                .find_map(|e| e.orbit.as_ref()),
        }
    }

    /// Every orbit the rewards and the constellations give, for compiling their attacks.
    pub fn orbits(&self) -> impl Iterator<Item = &Orbit> {
        let sets = self.constellations.sets().iter();
        self.rewards.iter().filter_map(Reward::orbit).chain(
            sets.flat_map(|s| &s.tiers)
                .flat_map(|t| &t.effects)
                .filter_map(|e| e.orbit.as_ref()),
        )
    }

    /// The ID an orbit key shows under: its reward's, or its set's.
    pub fn orbit_id(&self, key: u16) -> Option<ContentId> {
        match key.checked_sub(ORBIT_SET_BASE) {
            None => Some(self.rewards.get(usize::from(key))?.id.clone()),
            Some(set) => Some(self.constellations.sets().get(usize::from(set))?.id.clone()),
        }
    }
}

/// The largest amount of one effect; for Rune Haste, the file's cap `haste_cap`.
fn most_of(effect: RewardEffect, haste_cap: u32) -> u32 {
    match effect {
        RewardEffect::StageSkill => u32::try_from(MAX_SKILL_LEVELS).expect("a few levels"),
        RewardEffect::EvadeCharge => u32::from(MAX_EVADE_CHARGES),
        RewardEffect::Orbit => u32::from(MAX_ORBS),
        RewardEffect::RangePct => MAX_RANGE_PCT,
        RewardEffect::TypeBonusPct => MAX_TYPE_BONUS_AMOUNT,
        RewardEffect::RuneHaste => haste_cap.min(MAX_RUNE_HASTE_PCT),
        RewardEffect::MaxHp | RewardEffect::Regen => MAX_HP_AMOUNT,
        RewardEffect::PickupRadiusPct => MAX_PICKUP_RADIUS_PCT,
        RewardEffect::SkillCooldownPct => MAX_SKILL_COOLDOWN_PCT,
        RewardEffect::HealPerKills => MAX_HEAL_AMOUNT,
        RewardEffect::OfferCandidates => 1,
        RewardEffect::Rerolls | RewardEffect::Banishes => MAX_COUNTER_AMOUNT,
        RewardEffect::CritChancePct => MAX_CRIT_CHANCE_PCT,
        RewardEffect::CritDamagePct => MAX_CRIT_DAMAGE_PCT,
        RewardEffect::EchoHitPct => MAX_ECHO_HIT_PCT,
        RewardEffect::KillBurst => MAX_BURST_PCT,
        RewardEffect::BurstDepth => MAX_BURST_DEPTH,
        _ => MAX_AMOUNT,
    }
}

/// One effect entry on its own: its amount, skill, orbit, type, kills, radius and depth, by its
/// kind.
pub(crate) fn validate_entry(
    entry: &EffectEntry,
    at: &dyn Fn(&str) -> String,
    stage_skills: &StageSkills,
    haste_cap: u32,
    issue: &mut dyn FnMut(String, String),
) {
    let most = most_of(entry.effect, haste_cap);
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
    for (field, value, most) in [
        ("radius", entry.radius, MAX_BURST_RADIUS_PX),
        ("depth", entry.depth, MAX_BURST_DEPTH),
    ] {
        match (entry.effect, value) {
            (RewardEffect::KillBurst, None) => issue(at(field), format!("1 to {most} (missing)")),
            (RewardEffect::KillBurst, Some(v)) => {
                if !(1..=most).contains(&v) {
                    issue(at(field), format!("1 to {most} (found {v})"));
                }
            }
            (_, Some(_)) => issue(at(field), "only a kill burst reward names it".into()),
            (_, None) => {}
        }
    }
}

/// The caps over the entries of one reward: what a kind's entries add up to stays within the
/// kind's largest amount, as the picks of a run do (ADR-SVC-1), and a reward holds one orbit and
/// one random rune.
pub(crate) fn validate_bundle(
    effects: &[EffectEntry],
    haste_cap: u32,
    at: &dyn Fn(&str) -> String,
    issue: &mut dyn FnMut(String, String),
) {
    let mut sums: Vec<(RewardEffect, Option<&ContentId>, Option<DamageType>, u64)> = Vec::new();
    for e in effects {
        let amount = u64::from(e.amount.unwrap_or(0));
        let key = (e.effect, e.skill.as_ref(), e.damage_type);
        match sums.iter_mut().find(|(k, s, t, _)| (*k, *s, *t) == key) {
            Some(sum) => sum.3 += amount,
            None => sums.push((key.0, key.1, key.2, amount)),
        }
    }
    for (effect, _, _, sum) in &sums {
        let most = most_of(*effect, haste_cap);
        if *sum > u64::from(most) {
            issue(
                at("effects"),
                format!(
                    "the {effect:?} amounts of one reward add up to at most {most} (found {sum})"
                ),
            );
        }
    }
    let orbits = effects
        .iter()
        .filter(|e| e.effect == RewardEffect::Orbit)
        .count();
    if orbits > 1 {
        issue(at("effects"), "a reward holds at most one orbit".into());
    }
    let runes = effects
        .iter()
        .filter(|e| e.effect == RewardEffect::RandomRune)
        .count();
    if runes > 1 {
        issue(
            at("effects"),
            "a reward holds at most one random rune".into(),
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
    let haste_cap = file
        .max_rune_haste_pct
        .unwrap_or(DEFAULT_MAX_RUNE_HASTE_PCT);
    if !(1..=MAX_RUNE_HASTE_PCT).contains(&haste_cap) {
        issue(
            "$.max_rune_haste_pct".into(),
            format!("1 to {MAX_RUNE_HASTE_PCT} (found {haste_cap})"),
        );
    }
    if file.rarities.len() != MAX_RARITIES {
        issue(
            "$.rarities".into(),
            format!(
                "every rarity once ({MAX_RARITIES}, found {})",
                file.rarities.len()
            ),
        );
    }
    for (k, cost) in file.rarities.iter().enumerate().take(MAX_RARITIES) {
        let at = |field: &str| format!("$.rarities[{k}].{field}");
        if cost.rarity != Rarity::ALL[k] {
            issue(
                at("rarity"),
                format!(
                    "not {:?}: every rarity once, from common up",
                    Rarity::ALL[k]
                ),
            );
        }
        if !(1..=MAX_LUMINE).contains(&cost.lumine_cost) {
            issue(
                at("lumine_cost"),
                format!("1 to {MAX_LUMINE} (found {})", cost.lumine_cost),
            );
        }
    }
    if let Some(rules) = &file.equipment {
        validate_equipment(rules, &mut issue);
    }
    if file.rewards.is_empty() || file.rewards.len() > MAX_REWARDS {
        issue(
            "$.rewards".into(),
            format!("1 to {MAX_REWARDS} rewards (found {})", file.rewards.len()),
        );
    }
    for (i, reward) in file.rewards.iter().enumerate() {
        let at = |field: &str| format!("$.rewards[{i}].{field}");
        if !registry::is_well_formed(reward.id.as_str()) {
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
            validate_entry(entry, &at, stage_skills, haste_cap, &mut issue);
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
        validate_bundle(&reward.effects, haste_cap, &at, &mut issue);
        validate_naming(&file.rewards[..i], reward, &at, &mut issue);
        validate_flags(file.equipment.as_ref(), reward, &at, &mut issue);
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

/// A reward's `category` and its two flags: the category is one of the file's, the random rune
/// has none, and a reward is not both deprecated and equipment-only.
fn validate_flags(
    rules: Option<&EquipmentRules>,
    reward: &Reward,
    at: &dyn Fn(&str) -> String,
    issue: &mut dyn FnMut(String, String),
) {
    if let Some(category) = &reward.category {
        if reward.is_random_rune() {
            issue(at("category"), "a random rune has no category".into());
        } else if !rules.is_some_and(|r| r.categories.iter().any(|c| c.id == *category)) {
            issue(
                at("category"),
                "not a category of the file's equipment rules".into(),
            );
        }
    }
    if reward.deprecated && reward.equipment_only {
        issue(
            at("equipment_only"),
            "a reward is deprecated or equipment-only, not both".into(),
        );
    }
}

/// The equipment rules' bounds (code): one issue per problem, with its path.
fn validate_equipment(rules: &EquipmentRules, issue: &mut dyn FnMut(String, String)) {
    let at = |field: &str| format!("$.equipment.{field}");
    let categories = rules.categories.len();
    if categories == 0 || categories > MAX_EQUIP_CATEGORIES {
        issue(
            at("categories"),
            format!("1 to {MAX_EQUIP_CATEGORIES} categories (found {categories})"),
        );
    }
    let mut slots = usize::from(rules.rune_slots) + 1;
    for (i, category) in rules
        .categories
        .iter()
        .enumerate()
        .take(MAX_EQUIP_CATEGORIES)
    {
        let path = |field: &str| format!("$.equipment.categories[{i}].{field}");
        if !registry::is_well_formed(category.id.as_str()) {
            issue(
                path("id"),
                "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
            );
        } else if rules.categories[..i].iter().any(|c| c.id == category.id) {
            issue(path("id"), "the same ID as an earlier category".into());
        }
        if !(1..=MAX_CATEGORY_SLOTS).contains(&category.slots) {
            issue(
                path("slots"),
                format!("1 to {MAX_CATEGORY_SLOTS} (found {})", category.slots),
            );
        }
        slots += usize::from(category.slots);
        if category.drop_weight_max > MAX_DROP_WEIGHT_PCT {
            issue(
                path("drop_weight_max"),
                format!(
                    "at most {MAX_DROP_WEIGHT_PCT} (found {})",
                    category.drop_weight_max
                ),
            );
        }
        if category.drop_weight_default > category.drop_weight_max {
            issue(
                path("drop_weight_default"),
                format!(
                    "at most the category's drop_weight_max (found {})",
                    category.drop_weight_default
                ),
            );
        }
    }
    if !(1..=MAX_CATEGORY_SLOTS).contains(&rules.rune_slots) {
        issue(
            at("rune_slots"),
            format!("1 to {MAX_CATEGORY_SLOTS} (found {})", rules.rune_slots),
        );
    }
    if slots > MAX_EQUIP_SLOTS {
        issue(
            at("categories"),
            format!(
                "the category slots, the rune slots and the Lumine slot add up to at most {MAX_EQUIP_SLOTS} (found {slots})"
            ),
        );
    }
    let levels = usize::from(MAX_SLOT_LEVEL) - 1;
    if rules.kills_to_next.len() != levels {
        issue(
            at("kills_to_next"),
            format!(
                "exactly {levels} entries (found {})",
                rules.kills_to_next.len()
            ),
        );
    }
    for (i, &kills) in rules.kills_to_next.iter().enumerate() {
        let path = format!("$.equipment.kills_to_next[{i}]");
        if !(1..=MAX_STARLIGHT).contains(&kills) {
            issue(path, format!("1 to {MAX_STARLIGHT} (found {kills})"));
        } else if i > 0 && kills <= rules.kills_to_next[i - 1] {
            issue(path, "more than the entry before".into());
        }
    }
    if !(1..=MAX_LEVEL_PCT).contains(&rules.level_pct) {
        issue(
            at("level_pct"),
            format!("1 to {MAX_LEVEL_PCT} (found {})", rules.level_pct),
        );
    }
    if rules.rarities.len() != MAX_RARITIES {
        issue(
            at("rarities"),
            format!(
                "every rarity once ({MAX_RARITIES}, found {})",
                rules.rarities.len()
            ),
        );
    }
    for (k, r) in rules.rarities.iter().enumerate().take(MAX_RARITIES) {
        let path = |field: &str| format!("$.equipment.rarities[{k}].{field}");
        if r.rarity != Rarity::ALL[k] {
            issue(
                path("rarity"),
                format!(
                    "not {:?}: every rarity once, from common up",
                    Rarity::ALL[k]
                ),
            );
        }
        if !(1..=MAX_STARLIGHT).contains(&r.extract) {
            issue(
                path("extract"),
                format!("1 to {MAX_STARLIGHT} (found {})", r.extract),
            );
        }
        if !(r.extract..=MAX_STARLIGHT).contains(&r.craft_price) {
            issue(
                path("craft_price"),
                format!(
                    "{} (its extract) to {MAX_STARLIGHT} (found {})",
                    r.extract, r.craft_price
                ),
            );
        }
    }
    if !(1..=MAX_STARLIGHT).contains(&rules.rune_extract) {
        issue(
            at("rune_extract"),
            format!("1 to {MAX_STARLIGHT} (found {})", rules.rune_extract),
        );
    }
    if rules.boss_bonus > MAX_STARLIGHT {
        issue(
            at("boss_bonus"),
            format!("0 to {MAX_STARLIGHT} (found {})", rules.boss_bonus),
        );
    }
    if rules.boss_drops > MAX_BOSS_DROPS {
        issue(
            at("boss_drops"),
            format!("0 to {MAX_BOSS_DROPS} (found {})", rules.boss_drops),
        );
    }
    if rules.rune_share_permille > 1000 {
        issue(
            at("rune_share_permille"),
            format!("0 to 1000 (found {})", rules.rune_share_permille),
        );
    }
}
