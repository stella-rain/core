//! Expertise (ADR-CONT-1): one level inside one run, for the one weapon, with no highest
//! level. This is its content: the points levels 1 to 10 each need, the growth factor for every
//! level after and the most a level needs, and for each weapon its three styles (a path: the
//! style, then two upgrades and a capstone, at the levels of the mode's level table,
//! `stage::LevelTable`) and its common pool, drawn at the other levels. No option has a stack
//! limit and none leaves its pool by being taken (the attack speed total is held at
//! `run_stats::MAX_ATTACK_SPEED_PCT`); cooldown reduction is no expertise option but Rune
//! Haste, a star reward. Each enemy's points per hit and per kill are in the stage
//! (`stage::ExpertisePoints`).
//!
//! Data picks, code acts: an option's effect is one of the run stats' effect kinds
//! (`StatEffect`), under the same names, or `range_pct`, the range reward's widening.
//! The gauge reads the thresholds, the growth and the maximum, the common offers
//! draw from the common pool, and the style offers show the styles and the chosen style's
//! upgrades and capstone (`sim`). A style level may also name mechanic kinds (`StyleKind`): a
//! step's charge or hold, a delayed repeat or a proc factor of a step's hits, the weapon's smash
//! effect (`reap` among them) or a weapon gauge that fills from hits and damage taken and spends
//! on a full-gauge attack, or a spirit level that the full-gauge smash raises and a countdown
//! lowers, each level with its own effects, or a counter bonus that a near miss of the evade opens
//! and the first dash-branch step inside its window takes, or a combo tempo whose stacks, gained
//! from the swings that hit, speed the attacks the combo starts, or a giant trigger that the
//! full-gauge smash starts and that runs a time with a closed list of effects, or a breaker whose
//! closed list of effects scales the stagger and the poise damage the player deals, the damage of
//! a smash on a boss part and the burst round a target the player staggers; a path may name it at
//! several levels, the effects stacking in path order (ADR-SIM-9). A repeat and a factor are runtime-only fields of the
//! step on the style's copy of the tree.
//!
//! The file is untrusted (ADR-SVC-1), as the rewards are: a size limit before parsing, strict
//! parsing, the current versions only, and every field bounded. The thresholds rise strictly,
//! the factor is above 1, no pool is empty, every weapon preset has three complete styles, and
//! every number is capped. Option and style IDs are the file's own, unique within a weapon. The
//! bundled file, `content/expertise.json`, is the default; its styles are placeholders built
//! from the effect kinds `core` has.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::attack::AttackRef;
use crate::event::Press;
use crate::fixed::Fx;
use crate::id::ContentId;
use crate::limits::{
    MAX_COUNTER_WINDOW_TICKS, MAX_GAUGE_DAMAGE_PCT, MAX_PCT, MAX_SPIRIT_DECAY_TICKS,
    MAX_SPIRIT_LEVEL, MAX_TEMPO_STACKS, MAX_TRIGGER_TICKS, MAX_WEAPON_GAUGE,
};
use crate::pack::Pack;
use crate::registry::{self, Kind};
use crate::rewards::{MAX_AMOUNT, MAX_BURST_RADIUS_PX, MAX_HP_AMOUNT, MAX_RANGE_PCT};
use crate::run_stats::{MAX_ATTACK_SPEED_PCT, MAX_STEP, StatEffect};
use crate::validate::limits::MAX_COMBO_DEPTH;
use crate::validate::{self, Error, Issue};
use crate::version::{self, Versions};
use crate::weapon::{
    Branches, Charge, Hold, RepeatAnchorKind, SmashEffect, Step, StepRepeat, Weapon,
};

/// The largest expertise file read (a tuning value; ADR-SVC-1 decides that one exists).
pub const MAX_EXPERTISE_BYTES: usize = 64 * 1024;
/// The levels whose thresholds the file lists: 1 to 10, so far as the style path's last level
/// (`stage::MAX_STYLE_LEVEL`).
pub const TABLE_LEVELS: usize = 10;
/// The most points one level needs, a threshold or the growth's maximum.
pub const MAX_LEVEL_POINTS: u32 = 1_000_000;
/// The largest growth factor, in percent: four times the level before.
pub const MAX_GROWTH_PCT: u16 = 400;
/// The styles of each weapon: exactly three. The plain base is no style (ADR-SIM-9 point 2): it
/// is the weapon's base tree, with no entry here.
pub const STYLES: usize = 3;
/// The fixed choices a style offers at each of its two upgrades and its capstone: exactly one
/// (ADR-SIM-9 point 3).
pub const MAX_STYLE_CHOICES: usize = 1;
/// The most mechanic kinds one level names (a tuning placeholder): the style pick's, an
/// upgrade's choice's or the capstone's.
pub const MAX_LEVEL_KINDS: usize = 4;
/// The longest delay of a `repeat` kind, in ticks (a tuning placeholder: 2 seconds).
pub const MAX_REPEAT_AFTER_TICKS: u32 = 120;
/// The largest `proc_pct` of a `repeat` or `proc_factor` kind: a factor is a chance (a
/// placeholder).
pub const MAX_PROC_PCT: u32 = 100;
/// The most repeats on one step, summed over one style's path (a tuning placeholder).
pub const MAX_STEP_REPEATS: usize = 4;
/// The most extra bleed stacks a spirit level's `smash_bleed_stacks` adds to a smash hit (a
/// tuning placeholder).
pub const MAX_SPIRIT_BLEED_STACKS: u32 = 4;
/// The most options in a weapon's common pool.
pub const MAX_COMMON_OPTIONS: usize = 32;
pub use crate::limits::MAX_SET_STATS;

/// One stat of a weapon base set or a style set (the equipment build): its value at weapon level
/// 1 to 10.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetStat {
    /// One of `attack_pct`, `attack_speed_pct`, `evade_pct`, `skill_cooldown_pct`, `range_pct`
    /// and `max_hp`, once per set.
    pub effect: StatEffect,
    /// Exactly `TABLE_LEVELS` entries, Lv1 to Lv10, never falling. Tenths of a percent, except
    /// `max_hp`: hit points in the stored scale of every other `max_hp` amount.
    pub by_level: Vec<u32>,
}

/// The largest entry of a set stat of `effect`, if a set may hold the effect: a tenths effect
/// up to `MAX_STEP * 10` (100 percent), `max_hp` up to `MAX_HP_AMOUNT`.
fn set_stat_most(effect: StatEffect) -> Option<u32> {
    match effect {
        StatEffect::AttackPct
        | StatEffect::AttackSpeedPct
        | StatEffect::EvadePct
        | StatEffect::SkillCooldownPct
        | StatEffect::RangePct => Some(MAX_STEP * 10),
        StatEffect::MaxHp => Some(MAX_HP_AMOUNT),
        _ => None,
    }
}

/// What taking an option or a style does: one of the run stats' effect kinds, under the same
/// names, or a wider combo.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ExpertiseEffect {
    /// No stat: a level that grants only mechanic `kinds`. Its `amount` is absent or 0 and its
    /// `kinds` are not empty; a rule of `sim_version` 2.
    None,
    MaxHp,
    AttackPct,
    AttackSpeedPct,
    EvadePct,
    SpeedPct,
    XpPct,
    /// Every combo step's hit box `amount` percent larger, as the range reward widens it,
    /// at most `MAX_RANGE_PCT`.
    RangePct,
}

impl ExpertiseEffect {
    /// The run stats' effect kind it adds to, if it is one.
    pub fn stat(self) -> Option<StatEffect> {
        Some(match self {
            ExpertiseEffect::MaxHp => StatEffect::MaxHp,
            ExpertiseEffect::AttackPct => StatEffect::AttackPct,
            ExpertiseEffect::AttackSpeedPct => StatEffect::AttackSpeedPct,
            ExpertiseEffect::EvadePct => StatEffect::EvadePct,
            ExpertiseEffect::SpeedPct => StatEffect::SpeedPct,
            ExpertiseEffect::XpPct => StatEffect::XpPct,
            ExpertiseEffect::None | ExpertiseEffect::RangePct => return None,
        })
    }

    /// The largest amount of this kind.
    pub fn max_amount(self) -> u32 {
        match self {
            ExpertiseEffect::None => 0,
            ExpertiseEffect::RangePct => MAX_RANGE_PCT,
            ExpertiseEffect::MaxHp => MAX_HP_AMOUNT,
            _ => MAX_AMOUNT,
        }
    }
}

/// A grant on a step of the base tree: a charge or a hold, never both (ADR-SIM-9 point 3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum StepGrant {
    /// The step charges while its button stays held as it starts.
    Charge(Charge),
    /// The step holds past its last active phase.
    Hold(Hold),
}

/// A mechanic a style level grants: a closed list built into `core`, its numbers in JSON
/// (ADR-SIM-9 point 3, ADR-CONT-1). Every kind is bounded and validated alone and against the
/// weapon's base tree. Only the kinds that are listed exist.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StyleKind {
    /// The step reached by `step` (the presses from the root of the combo tree, 1 to
    /// `MAX_COMBO_DEPTH` long: `[main, main, main]` is AAA, `[smash]` is B from neutral) gets
    /// `grant`. A grant sets the step's charge (clearing its hold) or its hold (clearing its
    /// charge), so a step never ends with both; two grants on one step inside one style are
    /// refused.
    ReplaceStep { step: Vec<Press>, grant: StepGrant },
    /// The weapon's smash effect (every smash, the dash branch's too) is `effect` in place of the
    /// base's (ADR-SIM-9 point 4). One per style path: a second is refused.
    SmashEffect { effect: SmashEffect },
    /// The step reached by `step` repeats: `after_ticks` after its last `active` phase ends,
    /// `repeat_pct` percent of its damage lands again on `anchor`, firing ready on-hit runes at
    /// `proc_pct` (absent: every ready rune, no draw). Not on a held step; at most
    /// `MAX_STEP_REPEATS` per step over one style's path.
    Repeat {
        step: Vec<Press>,
        after_ticks: u32,
        repeat_pct: u32,
        anchor: RepeatAnchorKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proc_pct: Option<u32>,
    },
    /// Each hit of the step reached by `step` fires a ready on-hit rune only when a draw falls
    /// under `proc_pct` (0 to `MAX_PROC_PCT`); one per step per style.
    ProcFactor { step: Vec<Press>, proc_pct: u32 },
    /// The style's weapon gauge: it fills from the player's normal (A) steps that hit and from the
    /// hit points lost, and a B press with it full plays the full-gauge attack and empties it
    /// (ADR-SIM-9 points 3 and 7). One per style path: a second is refused.
    Gauge(Gauge),
    /// The style's spirit level, 0 to `levels.len()`: a full-gauge smash raises it by one, every
    /// smash restarts a countdown of `decay_ticks`, and the level drops by one when it runs out;
    /// each level carries its own effects (ADR-SIM-9 points 2 to 7). The style also names a gauge
    /// whose spend is `full_smash`. One per level; a path may name it at several levels, the
    /// numbers stacking in path order: a later level's `decay_ticks` replaces the earlier one,
    /// each of its `levels` entries lays over the same level's effects (a longer table extends
    /// the earlier one) and an effect it does not list stays.
    Spirit(Spirit),
    /// The style's counter bonus: a near miss of the player's evade opens a window of
    /// `window_ticks`, and the first dash-branch step started inside it takes the bonus's
    /// effects (ADR-SIM-9 points 2 to 7). Dormant without the run's perfect evade. One per
    /// level; a path may name it at several levels, the numbers stacking in path order: a later
    /// level's `window_ticks` replaces the earlier one and each effect it lists replaces the
    /// same effect; one it does not list stays.
    CounterBonus(CounterBonus),
    /// The style's combo tempo: each swing of a combo step that hits adds a stack, up to
    /// `max_stacks`, and each stack makes every attack the player's combo starts afterwards faster;
    /// the stacks end when the combo resets, when a smash is pressed (unless the stacks are full
    /// and the kind keeps them) and when the player is hit or downed (ADR-SIM-9 points 2 to 7).
    /// One per level; a path may name it at several levels, the numbers stacking in path order:
    /// a later level's `max_stacks` (never lower) replaces the earlier one, its `speed_pct`
    /// replaces the earlier speed, the flag effects are on once any level names them, and the
    /// folded `max_stacks` times `speed_pct` stays within the attack speed ceiling.
    ComboTempoPct(ComboTempo),
    /// The style's giant trigger: the full-gauge smash starts a state of `ticks` (a second one
    /// restarts the count) whose effects apply while it runs; being downed or picking another
    /// style ends it (ADR-SIM-9 points 2 to 7). The style also names a gauge whose spend is
    /// `full_smash`. One per level; a path may name it at several levels, the numbers stacking in
    /// path order: a later level's `ticks` replaces the earlier one and each effect it lists
    /// replaces the same effect; one it does not list stays.
    GiantTrigger(GiantTrigger),
    /// The style's breaker: a closed list of effects on the stagger fills, the poise damage and
    /// the boss part damage of the player's hits, and a burst round a target the player
    /// staggers (ADR-SIM-9 points 2 to 7). One per level; a path may name it at several levels,
    /// the effects stacking in path order and a later level's same effect replacing the
    /// earlier one.
    Breaker(Breaker),
}

/// A breaker's effects, every number bounded by the validator and clamped again when the world
/// is built (ADR-SVC-1, ADR-SIM-5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Breaker {
    /// What the style does, each variant at most once; 1 to 4 entries.
    pub effects: Vec<BreakerEffect>,
}

/// What a breaker does: a closed list built into `core`, one variant per effect, each bounded
/// alone. A factor is in percent, 100 meaning none (200 doubles it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BreakerEffect {
    /// A factor on every stagger fill credited to the player. 101 to `MAX_PCT`.
    StaggerPct {
        #[schemars(range(min = 101, max = MAX_PCT))]
        pct: u32,
    },
    /// A factor on the poise damage of the player's combo hits. 101 to `MAX_PCT`.
    PoisePct {
        #[schemars(range(min = 101, max = MAX_PCT))]
        pct: u32,
    },
    /// A factor on the damage of the player's smash step on a boss part. 101 to `MAX_PCT`.
    PartBreakPct {
        #[schemars(range(min = 101, max = MAX_PCT))]
        pct: u32,
    },
    /// A burst round an elite or boss the player's fill staggers: the other enemies within
    /// `radius_px` take the burst's effects. `radius_px` 1 to `MAX_BURST_RADIUS_PX`; 1 to 3
    /// effects, each variant at most once.
    OnStaggerBurst {
        #[schemars(range(min = 1, max = MAX_BURST_RADIUS_PX))]
        radius_px: u32,
        effects: Vec<BurstEffect>,
    },
}

impl BreakerEffect {
    fn name(&self) -> u8 {
        match self {
            BreakerEffect::StaggerPct { .. } => 0,
            BreakerEffect::PoisePct { .. } => 1,
            BreakerEffect::PartBreakPct { .. } => 2,
            BreakerEffect::OnStaggerBurst { .. } => 3,
        }
    }
}

/// What a stagger burst does to the enemies in its radius: a closed list, each bounded alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BurstEffect {
    /// The damage each enemy takes, a percent of the damage of the hit that staggered the
    /// target. 1 to `MAX_PCT`.
    DamagePct {
        #[schemars(range(min = 1, max = MAX_PCT))]
        pct: u32,
    },
    /// A stun on each regular enemy in the radius. 1 to `MAX_STUN_TICKS`.
    StunTicks {
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_STUN_TICKS))]
        ticks: u32,
    },
    /// A knockdown on each regular enemy in the radius (the rules' `down_ticks`).
    KnockDown {},
}

impl BurstEffect {
    fn name(&self) -> u8 {
        match self {
            BurstEffect::DamagePct { .. } => 0,
            BurstEffect::StunTicks { .. } => 1,
            BurstEffect::KnockDown {} => 2,
        }
    }
}

/// A giant trigger's numbers, every one bounded by the validator and clamped again when the
/// world is built (ADR-SVC-1, ADR-SIM-5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GiantTrigger {
    /// How long the state runs, in ticks. 1 to `MAX_TRIGGER_TICKS`.
    #[schemars(range(min = 1, max = MAX_TRIGGER_TICKS))]
    pub ticks: u32,
    /// What the state does, each variant at most once; 1 to 4 entries (0 to 4 at a level after
    /// one that names the kind).
    pub effects: Vec<TriggerEffect>,
}

/// What a giant trigger does while it runs: a closed list built into `core`, one variant per
/// effect, each bounded alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TriggerEffect {
    /// Added to the range percent fixed when a combo step starts, the total held to
    /// `MAX_RANGE_PCT`. 1 to `MAX_RANGE_PCT`.
    RangePct {
        #[schemars(range(min = 1, max = MAX_RANGE_PCT))]
        pct: u32,
    },
    /// Added to the attack-up percent of a smash step when it starts. 1 to `MAX_PCT`.
    SmashPct {
        #[schemars(range(min = 1, max = MAX_PCT))]
        pct: u32,
    },
    /// The player has super armour while the state runs.
    SuperArmour {},
    /// A factor, in percent, on the hit points a hit adds to the rally while the state runs
    /// (200 doubles it). 101 to `MAX_PCT`.
    RallyPct {
        #[schemars(range(min = 101, max = MAX_PCT))]
        pct: u32,
    },
}

impl TriggerEffect {
    fn name(&self) -> u8 {
        match self {
            TriggerEffect::RangePct { .. } => 0,
            TriggerEffect::SmashPct { .. } => 1,
            TriggerEffect::SuperArmour {} => 2,
            TriggerEffect::RallyPct { .. } => 3,
        }
    }
}

/// A combo tempo's numbers, every one bounded by the validator and clamped again when the world
/// is built (ADR-SVC-1, ADR-SIM-5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComboTempo {
    /// The most stacks the player holds. 1 to `MAX_TEMPO_STACKS`.
    #[schemars(range(min = 1, max = MAX_TEMPO_STACKS))]
    pub max_stacks: u32,
    /// What the stacks do, each variant at most once; 1 to 3 entries, `speed_pct` among them (0
    /// to 3 and no `speed_pct` needed at a level after one that names the kind).
    pub effects: Vec<TempoEffect>,
}

/// What a combo tempo's stacks do: a closed list built into `core`, one variant per effect,
/// each bounded alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TempoEffect {
    /// Each stack's share of attack speed, added to the player's attack speed when an attack
    /// starts, in percent. 1 to `MAX_ATTACK_SPEED_PCT`; `max_stacks` times it at most that.
    SpeedPct {
        #[schemars(range(min = 1, max = MAX_ATTACK_SPEED_PCT))]
        pct: u32,
    },
    /// A smash pressed with the stacks full leaves them; below full it ends them as it always does.
    KeepOnSmash {},
    /// The player has super armour while its stacks are full and a hold is held, and only this
    /// armour shelters the stacks from a hit.
    SuperArmourAtFull {},
}

impl TempoEffect {
    fn name(&self) -> u8 {
        match self {
            TempoEffect::SpeedPct { .. } => 0,
            TempoEffect::KeepOnSmash {} => 1,
            TempoEffect::SuperArmourAtFull {} => 2,
        }
    }
}

/// A counter bonus's numbers, every one bounded by the validator and clamped again when the
/// world is built (ADR-SVC-1, ADR-SIM-5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CounterBonus {
    /// Ticks after the near miss in which a dash-branch step takes the bonus. 1 to
    /// `MAX_COUNTER_WINDOW_TICKS`.
    #[schemars(range(min = 1, max = MAX_COUNTER_WINDOW_TICKS))]
    pub window_ticks: u32,
    /// What the step takes, each variant at most once; 1 to 4 entries (0 to 4 at a level after
    /// one that names the kind).
    pub effects: Vec<CounterEffect>,
}

/// What a counter bonus gives its step: a closed list built into `core`, one variant per
/// effect, each bounded alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CounterEffect {
    /// Added to the attack-up percent fixed when the step starts. 1 to `MAX_PCT`.
    DamagePct {
        #[schemars(range(min = 1, max = MAX_PCT))]
        pct: u32,
    },
    /// A factor on every gauge fill the step's hits credit to its player, in percent. 101 to
    /// `MAX_PCT`.
    StaggerPct {
        #[schemars(range(min = 101, max = MAX_PCT))]
        pct: u32,
    },
    /// The stun each target the step hits takes. 1 to `MAX_STUN_TICKS`.
    StunTicks {
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_STUN_TICKS))]
        ticks: u32,
    },
    /// The slide toward the target over the step's first phase, when that phase names no
    /// `advance` (1/256 px, 1 px to `MAX_ADVANCE_DISTANCE`).
    Advance {
        #[schemars(range(min = 256, max = crate::limits::MAX_ADVANCE_DISTANCE * 256))]
        distance: Fx,
    },
}

impl CounterEffect {
    fn name(&self) -> u8 {
        match self {
            CounterEffect::DamagePct { .. } => 0,
            CounterEffect::StaggerPct { .. } => 1,
            CounterEffect::StunTicks { .. } => 2,
            CounterEffect::Advance { .. } => 3,
        }
    }
}

/// A spirit's numbers, every one bounded by the validator and clamped again when the world is
/// built (ADR-SVC-1, ADR-SIM-5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Spirit {
    /// Ticks a level lasts without a smash. 1 to `MAX_SPIRIT_DECAY_TICKS`.
    #[schemars(range(min = 1, max = MAX_SPIRIT_DECAY_TICKS))]
    pub decay_ticks: u32,
    /// Entry `i` is level `i + 1`'s effects, its own and no running sum; the length is the
    /// style's top level, 1 to `MAX_SPIRIT_LEVEL`. An empty entry is a level with no effect, and
    /// a level names each effect at most once. At a later level of the path an entry lays over
    /// the earlier level's same entry, and a longer table extends it.
    pub levels: Vec<Vec<SpiritEffect>>,
}

/// What a spirit level does: a closed list built into `core`, one variant per effect, each
/// bounded alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpiritEffect {
    /// Added to the attack-up percent fixed when a combo step starts. 1 to `MAX_PCT`.
    DamagePct {
        #[schemars(range(min = 1, max = MAX_PCT))]
        pct: u32,
    },
    /// Added to the step's range percent, the total held to `MAX_RANGE_PCT`. 1 to
    /// `MAX_RANGE_PCT`.
    RangePct {
        #[schemars(range(min = 1, max = MAX_RANGE_PCT))]
        pct: u32,
    },
    /// Bleed stacks added to a smash hit on top of the one the smash's own effect adds; needs a
    /// bleed smash. 1 to `MAX_SPIRIT_BLEED_STACKS`.
    SmashBleedStacks {
        #[schemars(range(min = 1, max = MAX_SPIRIT_BLEED_STACKS))]
        stacks: u32,
    },
}

impl SpiritEffect {
    fn name(&self) -> u8 {
        match self {
            SpiritEffect::DamagePct { .. } => 0,
            SpiritEffect::RangePct { .. } => 1,
            SpiritEffect::SmashBleedStacks { .. } => 2,
        }
    }
}

/// A weapon gauge's numbers, every one bounded by the validator and clamped again when the
/// world is built (ADR-SVC-1, ADR-SIM-5). Units are gauge units; the bar is `max` long.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Gauge {
    /// The gauge never passes it; full is `max` reached. 1 to `MAX_WEAPON_GAUGE`.
    #[schemars(range(min = 1, max = MAX_WEAPON_GAUGE))]
    pub max: u32,
    /// Added once to a normal step the first time one of its hit boxes hits an enemy; absent
    /// (0): no rule. At most `max`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub per_step: u32,
    /// Added for each enemy a normal step's hit box hits on a tick; absent (0): no rule. At most
    /// `max`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub per_hit: u32,
    /// A hit taken adds `max * lost * pct / (max_hp * 100)` (floor), `lost` the hit points really
    /// lost; absent (0): no rule. At most `MAX_GAUGE_DAMAGE_PCT`.
    #[serde(default, skip_serializing_if = "is_zero")]
    #[schemars(range(min = 0, max = MAX_GAUGE_DAMAGE_PCT))]
    pub from_damage_pct: u32,
    /// The most one step's hits add (the bound on a horde, ADR-SIM-5): 1 to `max`. The damage
    /// fill is not held to it.
    pub step_cap: u32,
    /// What a full gauge buys.
    pub spend: GaugeSpend,
}

/// What a full gauge is spent on: a closed list, one variant now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum GaugeSpend {
    /// A B press with the gauge full plays `attack` in place of the step's own and empties the
    /// gauge. It is checked as a player's attack and differs from every attack of the weapon's
    /// base tree (the orb clear tells it by its compiled index).
    FullSmash { attack: AttackRef },
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// One option: a candidate of a common offer, or a fixed choice of a style.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpertiseOption {
    /// The option's own ID, unique within its weapon.
    pub id: ContentId,
    pub effect: ExpertiseEffect,
    /// What taking it adds to the effect's total, from 1 to the effect's most; absent (0) for
    /// the effect `none`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub amount: u32,
    /// The mechanic kinds it grants, at most `MAX_LEVEL_KINDS`; absent or empty for none
    /// (`sim_version` 2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<StyleKind>,
}

/// A style: a path chosen at the level table's style level, held for the run. Its
/// own effect applies when it is chosen at `sim_version` 1 (a version 2 run applies the style's
/// `stats` set instead: the set is everything); it then offers the fixed choices of `upgrade_1` at the
/// table's first upgrade level, `upgrade_2` at the second and `capstone` at the capstone's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Style {
    /// The style's own ID, unique within its weapon.
    pub id: ContentId,
    pub effect: ExpertiseEffect,
    /// What choosing it adds to the effect's total, from 1 to the effect's most; absent (0) for
    /// the effect `none`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub amount: u32,
    /// The mechanic kinds the style pick grants, at most `MAX_LEVEL_KINDS`; absent or empty for
    /// none (`sim_version` 2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<StyleKind>,
    /// The set the style grants in the equipment build, replacing the weapon's base set when
    /// the style is picked: at most `MAX_SET_STATS` stats. Absent or empty: the empty set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stats: Vec<SetStat>,
    /// The first upgrade's one choice.
    pub upgrade_1: Vec<ExpertiseOption>,
    /// The second upgrade's one choice.
    pub upgrade_2: Vec<ExpertiseOption>,
    /// The capstone's one choice.
    pub capstone: Vec<ExpertiseOption>,
}

/// One weapon's expertise.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeaponExpertise {
    /// A weapon preset of the registry.
    pub weapon: ContentId,
    /// Exactly `STYLES` styles, in the order the style offer shows them.
    pub styles: Vec<Style>,
    /// The common pool, 1 to `MAX_COMMON_OPTIONS` options, in the order offers draw from.
    pub common: Vec<ExpertiseOption>,
    /// The set the weapon grants in the equipment build until a style is picked: at most
    /// `MAX_SET_STATS` stats. Absent or empty: the empty set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base: Vec<SetStat>,
}

/// The expertise file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpertiseFile {
    /// The points each of levels 1 to 10 needs from the level before, the first from 0:
    /// exactly `TABLE_LEVELS`, rising strictly, each from 1 to `max_points`.
    pub thresholds: Vec<u32>,
    /// Each level after 10 needs the level before's points times `growth_pct / 100`: above 100,
    /// at most `MAX_GROWTH_PCT`.
    pub growth_pct: u16,
    /// The most points any level needs, where the growth saturates: at least the last
    /// threshold, at most `MAX_LEVEL_POINTS`.
    pub max_points: u32,
    /// Every weapon preset of the registry, each once.
    pub weapons: Vec<WeaponExpertise>,
}

/// Expertise content that passed validation, and the SHA-256 of the bytes it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expertise {
    thresholds: Vec<u32>,
    growth_pct: u16,
    max_points: u32,
    weapons: Vec<WeaponExpertise>,
    sha256: [u8; 32],
}

impl Expertise {
    /// The bundled expertise content.
    pub fn bundled() -> &'static Expertise {
        Pack::bundled().content().expertise()
    }

    /// Checks the size and the versions given (the pack's), parses strictly and validates: the one
    /// call for a file's bytes.
    pub fn parse(bytes: &[u8], versions: Versions) -> Result<Expertise, Error> {
        let file: ExpertiseFile = version::read_member(bytes, MAX_EXPERTISE_BYTES, versions)?;
        validate(&file, versions.sim_version)?;
        Ok(Expertise {
            thresholds: file.thresholds,
            growth_pct: file.growth_pct,
            max_points: file.max_points,
            weapons: file.weapons,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The points each of levels 1 to 10 needs from the level before.
    pub fn thresholds(&self) -> &[u32] {
        &self.thresholds
    }

    /// The growth factor after level 10, in percent.
    pub fn growth_pct(&self) -> u16 {
        self.growth_pct
    }

    /// The most points any level needs.
    pub fn max_points(&self) -> u32 {
        self.max_points
    }

    /// Every weapon's expertise, in the file's order.
    pub fn weapons(&self) -> &[WeaponExpertise] {
        &self.weapons
    }

    /// The expertise of the weapon preset `id`.
    pub fn weapon(&self, id: &ContentId) -> Option<&WeaponExpertise> {
        self.weapons.iter().find(|w| &w.weapon == id)
    }

    /// The SHA-256 of the bytes it was parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// The weapon presets a file must cover: the registry's, no newer than this `sim_version`.
fn weapon_presets(sim_version: u32) -> impl Iterator<Item = &'static str> {
    registry::ENTRIES
        .iter()
        .filter(move |e| e.kind == Kind::Weapon && e.introduced.le(&sim_version))
        .map(|e| e.id)
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &ExpertiseFile, sim_version: u32) -> Result<(), Error> {
    let mut issues = Vec::new();
    let mut issue = |path: String, message: String| issues.push(Issue { path, message });

    if file.thresholds.len() != TABLE_LEVELS {
        issue(
            "$.thresholds".into(),
            format!(
                "exactly {TABLE_LEVELS} thresholds (found {})",
                file.thresholds.len()
            ),
        );
    }
    for (i, &points) in file.thresholds.iter().enumerate() {
        let at = format!("$.thresholds[{i}]");
        if !(1..=file.max_points).contains(&points) {
            issue(
                at,
                format!("1 to max_points, {} (found {points})", file.max_points),
            );
        } else if i > 0 && points <= file.thresholds[i - 1] {
            issue(at, "more than the threshold before".into());
        }
    }
    let last = file.thresholds.last().copied().unwrap_or(1);
    if !(last..=MAX_LEVEL_POINTS).contains(&file.max_points) {
        issue(
            "$.max_points".into(),
            format!(
                "the last threshold to {MAX_LEVEL_POINTS} (found {})",
                file.max_points
            ),
        );
    }
    if !(101..=MAX_GROWTH_PCT).contains(&file.growth_pct) {
        issue(
            "$.growth_pct".into(),
            format!("101 to {MAX_GROWTH_PCT} (found {})", file.growth_pct),
        );
    }

    let missing: Vec<&str> = weapon_presets(sim_version)
        .filter(|id| !file.weapons.iter().any(|w| w.weapon.as_str() == *id))
        .collect();
    if !missing.is_empty() {
        issue(
            "$.weapons".into(),
            format!("every weapon preset, missing {}", missing.join(", ")),
        );
    }
    for (w, weapon) in file.weapons.iter().enumerate() {
        let at = |field: &str| format!("$.weapons[{w}].{field}");
        if !weapon_presets(sim_version).any(|id| id == weapon.weapon.as_str()) {
            issue(at("weapon"), "not a weapon preset".into());
        } else if file.weapons[..w].iter().any(|x| x.weapon == weapon.weapon) {
            issue(at("weapon"), "the same weapon as an earlier entry".into());
        }
        if weapon.styles.len() != STYLES {
            issue(
                at("styles"),
                format!("exactly {STYLES} styles (found {})", weapon.styles.len()),
            );
        }
        check_set(&mut issue, &at("base"), &weapon.base);
        if weapon.common.is_empty() || weapon.common.len() > MAX_COMMON_OPTIONS {
            issue(
                at("common"),
                format!(
                    "1 to {MAX_COMMON_OPTIONS} options (found {})",
                    weapon.common.len()
                ),
            );
        }

        // Every ID of the weapon, styles and options alike, in the file's order.
        let mut seen: Vec<&ContentId> = Vec::new();
        for (s, style) in weapon.styles.iter().enumerate() {
            let base = at(&format!("styles[{s}]"));
            check_id(&mut issue, &mut seen, &base, &style.id);
            check_level(
                &mut issue,
                &base,
                style.effect,
                style.amount,
                &style.kinds,
                sim_version,
                Earlier::default(),
            );
            check_set(&mut issue, &format!("{base}.stats"), &style.stats);
            let mut earlier = Earlier::default().with(&style.kinds);
            for (field, choices) in [
                ("upgrade_1", &style.upgrade_1),
                ("upgrade_2", &style.upgrade_2),
                ("capstone", &style.capstone),
            ] {
                let path = format!("{base}.{field}");
                if choices.len() != MAX_STYLE_CHOICES {
                    issue(
                        path.clone(),
                        format!("exactly 1 choice (found {})", choices.len()),
                    );
                }
                let before = earlier;
                for (c, choice) in choices.iter().enumerate() {
                    let path = format!("{path}[{c}]");
                    check_option(&mut issue, &mut seen, &path, choice, sim_version, before);
                    earlier = earlier.with(&choice.kinds);
                }
            }
            check_step_once(&mut issue, &base, style);
        }
        for (c, option) in weapon.common.iter().enumerate() {
            let path = at(&format!("common[{c}]"));
            check_option(
                &mut issue,
                &mut seen,
                &path,
                option,
                sim_version,
                Earlier::default(),
            );
            // A common option is a stat: the kinds are a style path's (ADR-SIM-9 point 3).
            if !option.kinds.is_empty() {
                issue(
                    format!("{path}.kinds"),
                    "a common option names no kind".into(),
                );
            }
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}

/// The kinds an earlier level of the same style path already names: a later level naming one
/// again states only what it changes (ADR-SIM-9 points 2 to 7).
#[derive(Clone, Copy, Default)]
struct Earlier {
    spirit: bool,
    counter: bool,
    tempo: bool,
    trigger: bool,
}

impl Earlier {
    /// This set and every kind `kinds` names.
    fn with(mut self, kinds: &[StyleKind]) -> Self {
        for kind in kinds {
            match kind {
                StyleKind::Spirit(_) => self.spirit = true,
                StyleKind::CounterBonus(_) => self.counter = true,
                StyleKind::ComboTempoPct(_) => self.tempo = true,
                StyleKind::GiantTrigger(_) => self.trigger = true,
                _ => {}
            }
        }
        self
    }
}

fn check_option<'a>(
    issue: &mut impl FnMut(String, String),
    seen: &mut Vec<&'a ContentId>,
    path: &str,
    option: &'a ExpertiseOption,
    sim_version: u32,
    earlier: Earlier,
) {
    check_id(issue, seen, path, &option.id);
    check_level(
        issue,
        path,
        option.effect,
        option.amount,
        &option.kinds,
        sim_version,
        earlier,
    );
}

/// A level's stat and kinds: a stat effect has an amount from 1 to its most; `none` has none
/// and names at least one kind; the kinds, and `none`, are `sim_version` 2's, at most
/// `MAX_LEVEL_KINDS`, each bounded alone; `earlier` names the kinds a level before it on the
/// style path already named.
fn check_level(
    issue: &mut impl FnMut(String, String),
    path: &str,
    effect: ExpertiseEffect,
    amount: u32,
    kinds: &[StyleKind],
    sim_version: u32,
    earlier: Earlier,
) {
    if effect == ExpertiseEffect::None {
        if sim_version < 2 {
            issue(
                format!("{path}.effect"),
                "the effect none needs sim_version 2".into(),
            );
        } else if kinds.is_empty() {
            issue(
                format!("{path}.effect"),
                "the effect none needs at least one kind".into(),
            );
        }
        if amount != 0 {
            issue(
                format!("{path}.amount"),
                format!("none takes no amount (found {amount})"),
            );
        }
    } else {
        check_amount(issue, path, effect, amount);
    }
    if kinds.is_empty() {
        return;
    }
    let at = format!("{path}.kinds");
    if sim_version < 2 {
        issue(at.clone(), "kinds need sim_version 2".into());
    }
    if kinds.len() > MAX_LEVEL_KINDS {
        issue(
            at.clone(),
            format!("at most {MAX_LEVEL_KINDS} kinds (found {})", kinds.len()),
        );
    }
    for (k, kind) in kinds.iter().enumerate().take(MAX_LEVEL_KINDS) {
        let at = format!("{at}[{k}]");
        match kind {
            StyleKind::ReplaceStep { step, grant } => {
                if step.is_empty() || step.len() > MAX_COMBO_DEPTH as usize {
                    issue(
                        format!("{at}.step"),
                        format!("1 to {MAX_COMBO_DEPTH} presses (found {})", step.len()),
                    );
                }
                if let Err(found) = validate::validate_step_grant(grant, registry::ENTRIES) {
                    for f in found {
                        issue(
                            format!("{at}.grant{}", f.path.trim_start_matches('$')),
                            f.message,
                        );
                    }
                }
            }
            StyleKind::SmashEffect { effect } => {
                if let Err(found) = validate::validate_smash_effect(effect, registry::ENTRIES) {
                    for f in found {
                        issue(
                            format!("{at}.effect{}", f.path.trim_start_matches('$')),
                            f.message,
                        );
                    }
                }
            }
            StyleKind::Repeat {
                step,
                after_ticks,
                repeat_pct,
                proc_pct,
                ..
            } => {
                check_step_len(issue, &at, step);
                if !(1..=MAX_REPEAT_AFTER_TICKS).contains(after_ticks) {
                    issue(
                        format!("{at}.after_ticks"),
                        format!("1 to {MAX_REPEAT_AFTER_TICKS} (found {after_ticks})"),
                    );
                }
                if !(1..=crate::limits::MAX_PCT).contains(repeat_pct) {
                    issue(
                        format!("{at}.repeat_pct"),
                        format!("1 to {} (found {repeat_pct})", crate::limits::MAX_PCT),
                    );
                }
                check_proc_pct(issue, &at, *proc_pct);
            }
            StyleKind::ProcFactor { step, proc_pct } => {
                check_step_len(issue, &at, step);
                check_proc_pct(issue, &at, Some(*proc_pct));
            }
            StyleKind::Gauge(g) => check_gauge(issue, &at, g),
            StyleKind::Spirit(s) => check_spirit(issue, &at, s),
            StyleKind::CounterBonus(c) => check_counter_bonus(issue, &at, c, earlier.counter),
            StyleKind::ComboTempoPct(t) => check_combo_tempo(issue, &at, t, earlier.tempo),
            StyleKind::GiantTrigger(t) => check_giant_trigger(issue, &at, t, earlier.trigger),
            StyleKind::Breaker(b) => check_breaker(issue, &at, b),
        }
    }
}

/// A gauge's numbers, each in its own range, and its full-gauge attack as a player's attack.
fn check_gauge(issue: &mut impl FnMut(String, String), at: &str, g: &Gauge) {
    if !(1..=MAX_WEAPON_GAUGE).contains(&g.max) {
        issue(
            format!("{at}.max"),
            format!("1 to {MAX_WEAPON_GAUGE} (found {})", g.max),
        );
    }
    // The fill bounds are read against the gauge's own length; a `max` out of range is its own issue.
    let most = g.max.min(MAX_WEAPON_GAUGE);
    for (key, v) in [("per_step", g.per_step), ("per_hit", g.per_hit)] {
        if v > most {
            issue(
                format!("{at}.{key}"),
                format!("0 to the gauge's max {most} (found {v})"),
            );
        }
    }
    if g.from_damage_pct > MAX_GAUGE_DAMAGE_PCT {
        issue(
            format!("{at}.from_damage_pct"),
            format!("0 to {MAX_GAUGE_DAMAGE_PCT} (found {})", g.from_damage_pct),
        );
    }
    if g.step_cap == 0 || g.step_cap > most {
        issue(
            format!("{at}.step_cap"),
            format!("1 to the gauge's max {most} (found {})", g.step_cap),
        );
    }
    let GaugeSpend::FullSmash { attack } = &g.spend;
    if let Err(found) = validate::validate_attack_ref(attack, registry::ENTRIES) {
        for f in found {
            issue(
                format!("{at}.spend.attack{}", f.path.trim_start_matches('$')),
                f.message,
            );
        }
    }
}

/// A spirit's numbers, each in its own range; a level names an effect once.
fn check_spirit(issue: &mut impl FnMut(String, String), at: &str, s: &Spirit) {
    if !(1..=u32::from(MAX_SPIRIT_DECAY_TICKS)).contains(&s.decay_ticks) {
        issue(
            format!("{at}.decay_ticks"),
            format!("1 to {MAX_SPIRIT_DECAY_TICKS} (found {})", s.decay_ticks),
        );
    }
    if s.levels.is_empty() || s.levels.len() > usize::from(MAX_SPIRIT_LEVEL) {
        issue(
            format!("{at}.levels"),
            format!("1 to {MAX_SPIRIT_LEVEL} levels (found {})", s.levels.len()),
        );
    }
    for (l, level) in s
        .levels
        .iter()
        .enumerate()
        .take(usize::from(MAX_SPIRIT_LEVEL))
    {
        let mut seen = [false; 3];
        for (e, effect) in level.iter().enumerate() {
            let at = format!("{at}.levels[{l}][{e}]");
            let (key, v, most) = match effect {
                SpiritEffect::DamagePct { pct } => ("pct", *pct, MAX_PCT),
                SpiritEffect::RangePct { pct } => ("pct", *pct, MAX_RANGE_PCT),
                SpiritEffect::SmashBleedStacks { stacks } => {
                    ("stacks", *stacks, MAX_SPIRIT_BLEED_STACKS)
                }
            };
            if !(1..=most).contains(&v) {
                issue(format!("{at}.{key}"), format!("1 to {most} (found {v})"));
            }
            let slot = &mut seen[usize::from(effect.name())];
            if *slot {
                issue(at, "the level already names this effect".into());
            }
            *slot = true;
        }
    }
}

/// A combo tempo's numbers, each in its own range; it names an effect once and, unless an
/// earlier level of the path named the kind (`later`), names `speed_pct` and keeps the stacks'
/// total speed within the attack speed ceiling (a later level's fold is `check_step_once`'s).
fn check_combo_tempo(
    issue: &mut impl FnMut(String, String),
    at: &str,
    t: &ComboTempo,
    later: bool,
) {
    let max = u64::from(MAX_TEMPO_STACKS);
    if !(1..=max).contains(&u64::from(t.max_stacks)) {
        issue(
            format!("{at}.max_stacks"),
            format!("1 to {MAX_TEMPO_STACKS} (found {})", t.max_stacks),
        );
    }
    let least = usize::from(!later);
    if t.effects.len() < least || t.effects.len() > 3 {
        issue(
            format!("{at}.effects"),
            format!("{least} to 3 effects (found {})", t.effects.len()),
        );
    }
    let mut seen = [false; 3];
    let mut speed = None;
    for (e, effect) in t.effects.iter().enumerate().take(3) {
        let at = format!("{at}.effects[{e}]");
        if let TempoEffect::SpeedPct { pct } = effect {
            if !(1..=MAX_ATTACK_SPEED_PCT).contains(pct) {
                issue(
                    format!("{at}.pct"),
                    format!("1 to {MAX_ATTACK_SPEED_PCT} (found {pct})"),
                );
            }
            speed.get_or_insert(*pct);
        }
        let slot = &mut seen[usize::from(effect.name())];
        if *slot {
            issue(at, "the kind already names this effect".into());
        }
        *slot = true;
    }
    if !later && !t.effects.is_empty() && t.effects.len() <= 3 {
        match speed {
            None => issue(format!("{at}.effects"), "names no speed_pct".into()),
            Some(pct) if (1..=MAX_ATTACK_SPEED_PCT).contains(&pct) => {
                let total = u64::from(t.max_stacks) * u64::from(pct);
                if total > u64::from(MAX_ATTACK_SPEED_PCT) {
                    issue(
                        format!("{at}.effects"),
                        format!(
                            "max_stacks times speed_pct is at most {MAX_ATTACK_SPEED_PCT} (found {total})"
                        ),
                    );
                }
            }
            Some(_) => {}
        }
    }
}

/// A giant trigger's numbers, each in its own range; it names an effect once, and names at
/// least one unless an earlier level of the path named the kind (`later`).
fn check_giant_trigger(
    issue: &mut impl FnMut(String, String),
    at: &str,
    t: &GiantTrigger,
    later: bool,
) {
    if !(1..=u32::from(MAX_TRIGGER_TICKS)).contains(&t.ticks) {
        issue(
            format!("{at}.ticks"),
            format!("1 to {MAX_TRIGGER_TICKS} (found {})", t.ticks),
        );
    }
    let least = usize::from(!later);
    if t.effects.len() < least || t.effects.len() > 4 {
        issue(
            format!("{at}.effects"),
            format!("{least} to 4 effects (found {})", t.effects.len()),
        );
    }
    let mut seen = [false; 4];
    for (e, effect) in t.effects.iter().enumerate().take(4) {
        let at = format!("{at}.effects[{e}]");
        let bound = match effect {
            TriggerEffect::RangePct { pct } => Some((*pct, 1, MAX_RANGE_PCT)),
            TriggerEffect::SmashPct { pct } => Some((*pct, 1, MAX_PCT)),
            TriggerEffect::RallyPct { pct } => Some((*pct, 101, MAX_PCT)),
            TriggerEffect::SuperArmour {} => None,
        };
        if let Some((pct, lo, hi)) = bound
            && !(lo..=hi).contains(&pct)
        {
            issue(format!("{at}.pct"), format!("{lo} to {hi} (found {pct})"));
        }
        let slot = &mut seen[usize::from(effect.name())];
        if *slot {
            issue(at, "the kind already names this effect".into());
        }
        *slot = true;
    }
}

/// A breaker's numbers, each in its own range; it names an effect once, its burst too.
fn check_breaker(issue: &mut impl FnMut(String, String), at: &str, b: &Breaker) {
    if b.effects.is_empty() || b.effects.len() > 4 {
        issue(
            format!("{at}.effects"),
            format!("1 to 4 effects (found {})", b.effects.len()),
        );
    }
    let mut seen = [false; 4];
    for (e, effect) in b.effects.iter().enumerate().take(4) {
        let at = format!("{at}.effects[{e}]");
        match effect {
            BreakerEffect::StaggerPct { pct }
            | BreakerEffect::PoisePct { pct }
            | BreakerEffect::PartBreakPct { pct } => {
                if !(101..=MAX_PCT).contains(pct) {
                    issue(
                        format!("{at}.pct"),
                        format!("101 to {MAX_PCT} (found {pct})"),
                    );
                }
            }
            BreakerEffect::OnStaggerBurst { radius_px, effects } => {
                if !(1..=MAX_BURST_RADIUS_PX).contains(radius_px) {
                    issue(
                        format!("{at}.radius_px"),
                        format!("1 to {MAX_BURST_RADIUS_PX} (found {radius_px})"),
                    );
                }
                check_burst(issue, &at, effects);
            }
        }
        let slot = &mut seen[usize::from(effect.name())];
        if *slot {
            issue(at, "the kind already names this effect".into());
        }
        *slot = true;
    }
}

/// A burst's effects: 1 to 3, each in its own range, each variant once.
fn check_burst(issue: &mut impl FnMut(String, String), at: &str, effects: &[BurstEffect]) {
    if effects.is_empty() || effects.len() > 3 {
        issue(
            format!("{at}.effects"),
            format!("1 to 3 effects (found {})", effects.len()),
        );
    }
    let mut seen = [false; 3];
    for (e, effect) in effects.iter().enumerate().take(3) {
        let at = format!("{at}.effects[{e}]");
        match effect {
            BurstEffect::DamagePct { pct } => {
                if !(1..=MAX_PCT).contains(pct) {
                    issue(format!("{at}.pct"), format!("1 to {MAX_PCT} (found {pct})"));
                }
            }
            BurstEffect::StunTicks { ticks } => {
                let max = validate::limits::MAX_STUN_TICKS;
                if !(1..=max).contains(ticks) {
                    issue(format!("{at}.ticks"), format!("1 to {max} (found {ticks})"));
                }
            }
            BurstEffect::KnockDown {} => {}
        }
        let slot = &mut seen[usize::from(effect.name())];
        if *slot {
            issue(at, "the burst already names this effect".into());
        }
        *slot = true;
    }
}

/// A counter bonus's numbers, each in its own range; it names an effect once, and names at
/// least one unless an earlier level of the path named the kind (`later`).
fn check_counter_bonus(
    issue: &mut impl FnMut(String, String),
    at: &str,
    c: &CounterBonus,
    later: bool,
) {
    if !(1..=u32::from(MAX_COUNTER_WINDOW_TICKS)).contains(&c.window_ticks) {
        issue(
            format!("{at}.window_ticks"),
            format!("1 to {MAX_COUNTER_WINDOW_TICKS} (found {})", c.window_ticks),
        );
    }
    let least = usize::from(!later);
    if c.effects.len() < least || c.effects.len() > 4 {
        issue(
            format!("{at}.effects"),
            format!("{least} to 4 effects (found {})", c.effects.len()),
        );
    }
    let mut seen = [false; 4];
    for (e, effect) in c.effects.iter().enumerate().take(4) {
        let at = format!("{at}.effects[{e}]");
        let (key, v, lo, hi) = match effect {
            CounterEffect::DamagePct { pct } => ("pct", i64::from(*pct), 1, i64::from(MAX_PCT)),
            CounterEffect::StaggerPct { pct } => ("pct", i64::from(*pct), 101, i64::from(MAX_PCT)),
            CounterEffect::StunTicks { ticks } => (
                "ticks",
                i64::from(*ticks),
                1,
                i64::from(validate::limits::MAX_STUN_TICKS),
            ),
            CounterEffect::Advance { distance } => (
                "distance",
                i64::from(distance.0),
                i64::from(Fx::px(1).0),
                i64::from(Fx::px(crate::limits::MAX_ADVANCE_DISTANCE).0),
            ),
        };
        if !(lo..=hi).contains(&v) {
            issue(format!("{at}.{key}"), format!("{lo} to {hi} (found {v})"));
        }
        let slot = &mut seen[usize::from(effect.name())];
        if *slot {
            issue(at, "the kind already names this effect".into());
        }
        *slot = true;
    }
}

fn check_step_len(issue: &mut impl FnMut(String, String), at: &str, step: &[Press]) {
    if step.is_empty() || step.len() > MAX_COMBO_DEPTH as usize {
        issue(
            format!("{at}.step"),
            format!("1 to {MAX_COMBO_DEPTH} presses (found {})", step.len()),
        );
    }
}

fn check_proc_pct(issue: &mut impl FnMut(String, String), at: &str, proc_pct: Option<u32>) {
    if let Some(p) = proc_pct.filter(|&p| p > MAX_PROC_PCT) {
        issue(
            format!("{at}.proc_pct"),
            format!("0 to {MAX_PROC_PCT} (found {p})"),
        );
    }
}

/// The levels of a style's path in order, as the simulation takes them: the style pick's own
/// kinds, then the one choice of each upgrade and the capstone.
pub(crate) fn style_levels(style: &Style) -> [(&'static str, Option<&ExpertiseOption>); 3] {
    [
        ("upgrade_1", style.upgrade_1.first()),
        ("upgrade_2", style.upgrade_2.first()),
        ("capstone", style.capstone.first()),
    ]
}

/// One grant on a step per style, and one smash effect and one gauge: a second grant on a step an
/// earlier level of the same style granted, or a second smash effect or gauge, is refused at the
/// later one. The spirit, counter bonus, combo tempo and giant trigger are named once per level
/// and may be named at every level of the path.
fn check_step_once(issue: &mut impl FnMut(String, String), base: &str, style: &Style) {
    let mut seen: Vec<&Vec<Press>> = Vec::new();
    let mut factored: Vec<&Vec<Press>> = Vec::new();
    let mut smash_set = false;
    let mut gauge_set = false;
    // The tempo's folded `max_stacks` and `speed_pct` over the levels so far, once one named it.
    let mut tempo: Option<(u32, u32)> = None;
    let mut levels: Vec<(String, &[StyleKind])> = vec![(format!("{base}.kinds"), &style.kinds)];
    for (field, option) in style_levels(style) {
        if let Some(o) = option {
            levels.push((format!("{base}.{field}[0].kinds"), &o.kinds));
        }
    }
    for (path, kinds) in levels {
        // A path may name the breaker, spirit, counter bonus, combo tempo and giant trigger at
        // every level (their numbers stack in path order), a level only once.
        let mut breaker_here = false;
        let mut spirit_here = false;
        let mut counter_here = false;
        let mut tempo_here = false;
        let mut trigger_here = false;
        for (k, kind) in kinds.iter().enumerate().take(MAX_LEVEL_KINDS) {
            match kind {
                StyleKind::Breaker(_) => {
                    if breaker_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names a breaker".into(),
                        );
                    }
                    breaker_here = true;
                }
                StyleKind::ReplaceStep { step, .. } => {
                    if seen.contains(&step) {
                        issue(
                            format!("{path}[{k}].step"),
                            "the step is already granted by an earlier level of the style".into(),
                        );
                    } else {
                        seen.push(step);
                    }
                }
                StyleKind::SmashEffect { .. } => {
                    if smash_set {
                        issue(
                            format!("{path}[{k}]"),
                            "the style already sets the smash effect".into(),
                        );
                    }
                    smash_set = true;
                }
                StyleKind::Repeat { .. } => {}
                StyleKind::ProcFactor { step, .. } => {
                    if factored.contains(&step) {
                        issue(
                            format!("{path}[{k}].step"),
                            "the step already has a proc factor from an earlier level of the style"
                                .into(),
                        );
                    } else {
                        factored.push(step);
                    }
                }
                StyleKind::Gauge(_) => {
                    if gauge_set {
                        issue(
                            format!("{path}[{k}]"),
                            "the style already names a gauge".into(),
                        );
                    }
                    gauge_set = true;
                }
                StyleKind::Spirit(_) => {
                    if spirit_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names a spirit".into(),
                        );
                    }
                    spirit_here = true;
                }
                StyleKind::CounterBonus(_) => {
                    if counter_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names a counter bonus".into(),
                        );
                    }
                    counter_here = true;
                }
                StyleKind::ComboTempoPct(t) => {
                    if tempo_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names a combo tempo".into(),
                        );
                    }
                    tempo_here = true;
                    check_tempo_fold(issue, &format!("{path}[{k}]"), t, &mut tempo);
                }
                StyleKind::GiantTrigger(_) => {
                    if trigger_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names a giant trigger".into(),
                        );
                    }
                    trigger_here = true;
                }
            }
        }
    }
}

/// A later level's combo tempo laid over the folded one: `max_stacks` never falls (the stacks
/// held never pass the cap) and `max_stacks` times `speed_pct` stays within the attack speed
/// ceiling at every place of the path, refused at the level that breaks it. The first naming is
/// `check_combo_tempo`'s.
fn check_tempo_fold(
    issue: &mut impl FnMut(String, String),
    at: &str,
    t: &ComboTempo,
    folded: &mut Option<(u32, u32)>,
) {
    let speed = t.effects.iter().find_map(|e| match e {
        TempoEffect::SpeedPct { pct } => Some(*pct),
        _ => None,
    });
    let Some((max, pct)) = *folded else {
        *folded = Some((t.max_stacks, speed.unwrap_or(0)));
        return;
    };
    if t.max_stacks < max {
        issue(
            format!("{at}.max_stacks"),
            format!(
                "at least the earlier level's {max} (found {})",
                t.max_stacks
            ),
        );
    }
    let max = max.max(t.max_stacks);
    let pct = speed
        .filter(|p| (1..=MAX_ATTACK_SPEED_PCT).contains(p))
        .unwrap_or(pct);
    let total = u64::from(max) * u64::from(pct);
    if total > u64::from(MAX_ATTACK_SPEED_PCT) {
        let key = if speed.is_some() {
            "effects"
        } else {
            "max_stacks"
        };
        issue(
            format!("{at}.{key}"),
            format!(
                "max_stacks times speed_pct, folded over the path, is at most {MAX_ATTACK_SPEED_PCT} (found {total})"
            ),
        );
    }
    *folded = Some((max, pct));
}

fn check_id<'a>(
    issue: &mut impl FnMut(String, String),
    seen: &mut Vec<&'a ContentId>,
    path: &str,
    id: &'a ContentId,
) {
    if !registry::is_well_formed(id.as_str()) {
        issue(
            format!("{path}.id"),
            "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
        );
    } else if seen.contains(&id) {
        issue(
            format!("{path}.id"),
            "the same ID as an earlier style or option of the weapon".into(),
        );
    }
    seen.push(id);
}

fn check_amount(
    issue: &mut impl FnMut(String, String),
    path: &str,
    effect: ExpertiseEffect,
    amount: u32,
) {
    let most = effect.max_amount();
    if !(1..=most).contains(&amount) {
        issue(
            format!("{path}.amount"),
            format!("1 to {most} (found {amount})"),
        );
    }
}

/// A set: at most `MAX_SET_STATS` stats, each effect allowed and once, each with exactly
/// `TABLE_LEVELS` entries within the effect's bound and never falling.
fn check_set(issue: &mut impl FnMut(String, String), path: &str, set: &[SetStat]) {
    if set.len() > MAX_SET_STATS {
        issue(
            path.to_string(),
            format!("at most {MAX_SET_STATS} stats (found {})", set.len()),
        );
    }
    for (i, stat) in set.iter().enumerate().take(MAX_SET_STATS) {
        let at = format!("{path}[{i}]");
        let most = set_stat_most(stat.effect);
        if most.is_none() {
            issue(
                format!("{at}.effect"),
                "a set holds attack_pct, attack_speed_pct, evade_pct, skill_cooldown_pct, range_pct or max_hp".into(),
            );
        } else if set[..i].iter().any(|x| x.effect == stat.effect) {
            issue(
                format!("{at}.effect"),
                "the same effect as an earlier stat of the set".into(),
            );
        }
        if stat.by_level.len() != TABLE_LEVELS {
            issue(
                format!("{at}.by_level"),
                format!(
                    "exactly {TABLE_LEVELS} entries (found {})",
                    stat.by_level.len()
                ),
            );
        }
        for (n, &value) in stat.by_level.iter().enumerate().take(TABLE_LEVELS) {
            let path = format!("{at}.by_level[{n}]");
            if let Some(most) = most.filter(|&m| value > m) {
                issue(path, format!("0 to {most} (found {value})"));
            } else if n > 0 && value < stat.by_level[n - 1] {
                issue(path, "not less than the level before".into());
            }
        }
    }
}

/// The step the `presses` reach from the root of a combo tree.
pub(crate) fn step_mut<'a>(branches: &'a mut Branches, presses: &[Press]) -> Option<&'a mut Step> {
    let (first, rest) = presses.split_first()?;
    let step = match first {
        Press::Main => branches.main.as_deref_mut(),
        Press::Smash => branches.smash.as_deref_mut(),
    }?;
    if rest.is_empty() {
        Some(step)
    } else {
        step_mut(&mut step.next, rest)
    }
}

/// Applies one `replace_step` grant to the step of `weapon` that `presses` reach from the root
/// of the combo tree: the grant sets the step's charge (clearing its hold) or its hold
/// (clearing its charge). `false`, nothing changed, when no step is there.
pub(crate) fn apply_grant(weapon: &mut Weapon, presses: &[Press], grant: &StepGrant) -> bool {
    let Some(step) = step_mut(&mut weapon.combo, presses) else {
        return false;
    };
    match grant {
        StepGrant::Charge(c) => {
            step.charge = Some(c.clone());
            step.hold = None;
        }
        StepGrant::Hold(h) => {
            step.hold = Some(h.clone());
            step.charge = None;
        }
    }
    true
}
/// `base` with the grants a style gives when it is chosen and `taken` places of its path
/// (0 to 3: the first upgrade, the second, the capstone) are taken, in path order: the style's
/// own kinds, then each taken place's one choice. A grant that finds no step is skipped (the
/// pack check refuses it before a run).
pub(crate) fn weapon_with_style(base: &Weapon, style: &Style, taken: usize) -> Weapon {
    let mut weapon = base.clone();
    let places = style_levels(style)
        .into_iter()
        .take(taken)
        .filter_map(|(_, option)| option.map(|o| o.kinds.as_slice()));
    for kinds in std::iter::once(style.kinds.as_slice()).chain(places) {
        for kind in kinds {
            match kind {
                StyleKind::ReplaceStep { step, grant } => {
                    apply_grant(&mut weapon, step, grant);
                }
                StyleKind::SmashEffect { effect } => weapon.smash = Some(*effect),
                StyleKind::Repeat {
                    step,
                    after_ticks,
                    repeat_pct,
                    anchor,
                    proc_pct,
                } => {
                    if let Some(s) = step_mut(&mut weapon.combo, step) {
                        s.repeats.push(StepRepeat {
                            after_ticks: *after_ticks,
                            repeat_pct: u16::try_from(*repeat_pct).unwrap_or(u16::MAX),
                            anchor: *anchor,
                            proc_pct: proc_pct.map(|p| u16::try_from(p).unwrap_or(u16::MAX)),
                        });
                    }
                }
                StyleKind::ProcFactor { step, proc_pct } => {
                    if let Some(s) = step_mut(&mut weapon.combo, step) {
                        s.proc_pct = Some(u16::try_from(*proc_pct).unwrap_or(u16::MAX));
                    }
                }
                // The gauge changes no step: the rules read it beside the weapon (`World::gauge_of`).
                StyleKind::Gauge(_) => {}
                // Nor does the spirit: the rules read it beside the weapon (`World::spirit_of`).
                StyleKind::Spirit(_) => {}
                // Nor does the counter bonus (`World::counter_of`).
                StyleKind::CounterBonus(_) => {}
                // Nor does the combo tempo (`World::tempo_of`).
                StyleKind::ComboTempoPct(_) => {}
                StyleKind::GiantTrigger(_) => {}
                // Nor does the breaker (`World::breaker_of`).
                StyleKind::Breaker(_) => {}
            }
        }
    }
    weapon
}

/// Every attack a weapon's combo and dash trees can start: a step's, a charge level's (and its
/// perfect one), a hold's loop and release.
pub(crate) fn weapon_attacks(weapon: &Weapon) -> Vec<&AttackRef> {
    fn branches<'a>(b: &'a Branches, out: &mut Vec<&'a AttackRef>) {
        for step in [&b.main, &b.smash].into_iter().flatten() {
            out.push(&step.attack);
            if let Some(charge) = &step.charge {
                for level in &charge.levels {
                    out.push(&level.attack);
                    out.extend(level.perfect_attack.as_ref());
                }
            }
            if let Some(hold) = &step.hold {
                out.push(&hold.loop_attack);
                out.push(&hold.release);
            }
            branches(&step.next, out);
        }
    }
    let mut out = Vec::new();
    branches(&weapon.combo, &mut out);
    branches(&weapon.dash, &mut out);
    out
}
