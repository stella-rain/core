//! The expertise file's data model: the serde and schema types of the file format.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::style::step_ref;
use super::{
    MAX_CHARGE_SOONER_PCT, MAX_HOLD_MOVE_PCT, MAX_ORB_BREAK_PCT, MAX_REPEAT_BLEED_STACKS,
    MAX_SPIRIT_BLEED_STACKS, REPEAT_BLEED_INTERVAL_TICKS,
};
use crate::attack::AttackRef;
use crate::event::Press;
use crate::fixed::Fx;
use crate::id::ContentId;
use crate::limits::{
    MAX_COUNTER_WINDOW_TICKS, MAX_GAUGE_DAMAGE_PCT, MAX_PCT, MAX_SPIRIT_DECAY_TICKS,
    MAX_TEMPO_STACKS, MAX_TRIGGER_TICKS, MAX_WEAPON_GAUGE,
};
use crate::rewards::{MAX_AMOUNT, MAX_BURST_RADIUS_PX, MAX_HP_AMOUNT, MAX_RANGE_PCT};
use crate::run_stats::{MAX_ATTACK_SPEED_PCT, MAX_STEP, StatEffect};
use crate::validate::limits::{MAX_COMBO_DEPTH, MAX_DAMAGE};
use crate::weapon::{Branches, Charge, Hold, RepeatAnchorKind, SmashEffect};

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
pub(super) fn set_stat_most(effect: StatEffect) -> Option<u32> {
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

/// The step or steps a kind names: the presses from the root of the combo tree (`[main, main]`
/// is AA), or a group, `normals` or `smashes`, that stands for each member of the group the
/// weapon's base tree has (ADR-SIM-9 points 3 and 4). A path is one step; the groups are closed
/// lists fixed by the tree's depth, so a check that needs no weapon can tell two names overlap.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum StepRef {
    /// One step: 1 to `MAX_COMBO_DEPTH` presses from the root.
    Path(Vec<Press>),
    /// A named group of steps.
    Group(StepGroup),
}

/// A closed name for the steps of the base tree a kind reaches together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepGroup {
    /// The A-only steps: A, AA, AAA, AAAA and AAAAA (1 to `MAX_COMBO_DEPTH - 1` A presses; the
    /// dash branch is in neither group).
    Normals,
    /// B after 0 to `MAX_COMBO_DEPTH - 1` A presses: B, AB, AAB, AAAB, AAAAB and AAAAAB.
    Smashes,
}

// The `Debug` text of a path is the list's own, as it was before a step could be a group: the
// observables digests read `Debug` text.
impl std::fmt::Debug for StepRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Path(p) => p.fmt(f),
            Self::Group(g) => g.fmt(f),
        }
    }
}

impl StepGroup {
    /// The group's steps, shortest first, whatever the weapon's tree holds.
    pub fn members(self) -> Vec<Vec<Press>> {
        let deep = MAX_COMBO_DEPTH as usize;
        match self {
            Self::Normals => (1..deep).map(|n| vec![Press::Main; n]).collect(),
            Self::Smashes => (0..deep)
                .map(|n| {
                    let mut path = vec![Press::Main; n];
                    path.push(Press::Smash);
                    path
                })
                .collect(),
        }
    }
}

impl StepRef {
    /// The one step a path names; `None` for a group.
    pub fn as_path(&self) -> Option<&[Press]> {
        match self {
            Self::Path(p) => Some(p),
            Self::Group(_) => None,
        }
    }

    /// Every step the reference names, whether or not a tree has it: a path is itself.
    pub fn members(&self) -> Vec<Vec<Press>> {
        match self {
            Self::Path(p) => vec![p.clone()],
            Self::Group(g) => g.members(),
        }
    }

    /// The members the combo tree `branches` has, in `members` order.
    pub(crate) fn members_in(&self, branches: &Branches) -> Vec<Vec<Press>> {
        let mut members = self.members();
        members.retain(|m| step_ref(branches, m).is_some());
        members
    }
}

// A grant on a step of the base tree: a charge or a hold, never both (ADR-SIM-9 point 3).
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
    /// `MAX_COMBO_DEPTH` long: `[main, main, main]` is AAA, `[smash]` is B from neutral; a group
    /// names each member the tree has) gets `grant`. A grant sets the step's charge (clearing its hold) or its hold (clearing its
    /// charge), so a step never ends with both; two grants on one step inside one style are
    /// refused.
    ReplaceStep { step: StepRef, grant: StepGrant },
    /// The weapon's smash effect (every smash, the dash branch's too) is `effect` in place of the
    /// base's (ADR-SIM-9 point 4). One per style path: a second is refused.
    SmashEffect { effect: SmashEffect },
    /// The step reached by `step` (a group: each member the tree has, counted one by one) repeats:
    /// `after_ticks` after its last `active` phase ends, `repeat_pct` percent of its damage lands again on `anchor`, firing ready on-hit runes at
    /// `proc_pct` (absent: every ready rune, no draw). Not on a held step; at most
    /// `MAX_STEP_REPEATS` per step over one style's path.
    Repeat {
        step: StepRef,
        after_ticks: u32,
        repeat_pct: u32,
        anchor: RepeatAnchorKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proc_pct: Option<u32>,
    },
    /// From the level that names it on, every repeat the style queues breaks each breakable enemy
    /// orb within `radius_px` of what it strikes (round the place for a `place` anchor) with
    /// `chance_pct` percent, one draw per orb, as the main character's own hit box breaks one: no
    /// burst, the `OrbBroken` event (ADR-SIM-9 points 2 to 7). `chance_pct` 1 to
    /// `MAX_ORB_BREAK_PCT` (100 still draws), `radius_px` 1 to `MAX_BURST_RADIUS_PX`. One per
    /// style path; the pack check refuses it where the style names no `repeat` at or before its
    /// level. Read at the deal from the owner's grant (`World::repeat_break_of`); no step changes.
    RepeatBreaksOrbs {
        #[schemars(range(min = 1, max = MAX_ORB_BREAK_PCT))]
        chance_pct: u32,
        #[schemars(range(min = 1, max = MAX_BURST_RADIUS_PX))]
        radius_px: u32,
    },
    /// From the level that names it on, every delayed repeat of the style puts `stacks` stacks of
    /// the repeat bleed (`StatusKind::RepeatBleed`, a damage-over-time status of its own) on each
    /// foe it hits and leaves standing: `damage` a stack tick, every `REPEAT_BLEED_INTERVAL_TICKS`,
    /// for `duration_ticks` after the last application, up to `max_stacks`. The numbers are the
    /// kind's own, fixed when the repeat is queued (`Repeat.bleed`). `stacks` 1 to
    /// `MAX_REPEAT_BLEED_STACKS`, `duration_ticks` `REPEAT_BLEED_INTERVAL_TICKS` to
    /// `MAX_DOT_DURATION_TICKS`, `damage` 1 to `MAX_DAMAGE`, `max_stacks` `stacks` to
    /// `MAX_DOT_STACKS`. One per style path; the pack check refuses it where the style names no
    /// `repeat` at or before its level. It changes no step: the weapon copy stamps it on each
    /// repeat (`weapon_with_style`). `sim_version` 2 only.
    RepeatBleed {
        #[schemars(range(min = 1, max = MAX_REPEAT_BLEED_STACKS))]
        stacks: u32,
        #[schemars(range(min = REPEAT_BLEED_INTERVAL_TICKS, max = crate::runes::MAX_DOT_DURATION_TICKS))]
        duration_ticks: u32,
        #[schemars(range(min = 1, max = MAX_DAMAGE))]
        damage: u32,
        #[schemars(range(min = 1, max = crate::runes::MAX_DOT_STACKS))]
        max_stacks: u32,
    },
    /// Each hit of the step reached by `step` fires a ready on-hit rune only when a draw falls
    /// under `proc_pct` (0 to `MAX_PROC_PCT`); one per step per style, a group counting each of its
    /// members.
    ProcFactor { step: StepRef, proc_pct: u32 },
    /// The charge of the step reached by `step` reaches each of its levels `pct` percent sooner
    /// (1 to `MAX_CHARGE_SOONER_PCT`): each level's `at_ticks` becomes `at_ticks * (100 - pct) /
    /// 100`, floored, at least 1. The perfect windows, the longest hold, the movement and the
    /// attacks stay. It comes after the grant that gives the step its charge in the style's path
    /// (the pack check refuses a step with no charge there), and a step takes it once per style
    /// path. It is applied when the world builds the style's weapon (`weapon_with_style`): no
    /// state, no hash field. `sim_version` 2 only.
    ChargeSooner {
        step: StepRef,
        #[schemars(range(min = 1, max = MAX_CHARGE_SOONER_PCT))]
        pct: u32,
    },
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
    /// The style's breaker: a closed list of effects on the stagger fills, the length of the
    /// stagger they start, the poise damage and the boss part damage of the player's hits, and a
    /// burst round a target the player staggers (ADR-SIM-9 points 2 to 7). One per level; a path may name it at several levels,
    /// the effects stacking in path order and a later level's same effect replacing the
    /// earlier one.
    Breaker(Breaker),
}

/// A breaker's effects, every number bounded by the validator and clamped again when the world
/// is built (ADR-SVC-1, ADR-SIM-5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Breaker {
    /// What the style does, each variant at most once; 1 to 5 entries.
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
    /// A factor, in percent, on the ticks of every stagger the player causes: 150 makes it 50%
    /// longer (truncating, at most `MAX_STAGGERED_TICKS`). The stagger keeps the length it was
    /// given (`Stagger::total`). 101 to `MAX_PCT`.
    StaggerTicksPct {
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
    pub(super) fn name(&self) -> u8 {
        match self {
            BreakerEffect::StaggerPct { .. } => 0,
            BreakerEffect::PoisePct { .. } => 1,
            BreakerEffect::PartBreakPct { .. } => 2,
            BreakerEffect::OnStaggerBurst { .. } => 3,
            BreakerEffect::StaggerTicksPct { .. } => 4,
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
    pub(super) fn name(&self) -> u8 {
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
    /// What the state does, each variant at most once; 1 to `MAX_TRIGGER_EFFECTS` entries (0 to
    /// `MAX_TRIGGER_EFFECTS` at a level after one that names the kind).
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
    /// While the state runs, a player holding the AAA hold moves at `pct` percent of full speed:
    /// raised to it, never lowered below the hold's own `move_pct`. 1 to `MAX_HOLD_MOVE_PCT`.
    HoldMovePct {
        #[schemars(range(min = 1, max = MAX_HOLD_MOVE_PCT))]
        pct: u32,
    },
    /// When the state runs out with the player up (not on a down), the player plays the first
    /// ring of the style's full-gauge smash once: the smash's first `active` phase as compiled,
    /// scaled by the player's range as the smash is, with no gauge, no wind-up and no combo
    /// slot. A flag with no field: the ring's numbers come from the smash.
    EndRing {},
}

impl TriggerEffect {
    pub(super) fn name(&self) -> u8 {
        match self {
            TriggerEffect::RangePct { .. } => 0,
            TriggerEffect::SmashPct { .. } => 1,
            TriggerEffect::SuperArmour {} => 2,
            TriggerEffect::RallyPct { .. } => 3,
            TriggerEffect::HoldMovePct { .. } => 4,
            TriggerEffect::EndRing {} => 5,
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
    /// What the stacks do, each variant at most once; 1 to `MAX_TEMPO_EFFECTS` entries, `speed_pct` among them
    /// (0 to `MAX_TEMPO_EFFECTS` and no `speed_pct` needed at a level after one that names the kind).
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
    /// While the stacks are full, every attack the combo starts is `pct` percent wider, through
    /// the same range term as the spirit's and the trigger's (the sum is held to
    /// `MAX_RANGE_PCT` when the attack's box is built). 1 to `MAX_RANGE_PCT`.
    RangePctAtFull {
        #[schemars(range(min = 1, max = MAX_RANGE_PCT))]
        pct: u32,
    },
    /// A combo that ends on a smash pressed at full stacks leaves the stacks standing at every
    /// reset but the combo window running out: a press that does not continue the combo, the
    /// auto combo's restart and an evade. The next combo then starts at full stacks. Needs
    /// `keep_on_smash` to have any effect (a smash below full ends the stacks at the press).
    CarryAtFull {},
}

impl TempoEffect {
    pub(super) fn name(&self) -> u8 {
        match self {
            TempoEffect::SpeedPct { .. } => 0,
            TempoEffect::KeepOnSmash {} => 1,
            TempoEffect::SuperArmourAtFull {} => 2,
            TempoEffect::RangePctAtFull { .. } => 3,
            TempoEffect::CarryAtFull {} => 4,
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
    /// What the step takes, each variant at most once; 1 to 6 entries (0 to 6 at a level after
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
    /// At the counter's first landed combo hit, one spent evade charge is given back (the
    /// most recent spend); nothing when none is recovering. A counter that whiffs pays nothing.
    RefundEvade {},
    /// At the counter's first landed combo hit, the weapon gauge gains this percent of its bar
    /// (never past the bar); inert for a style that names no gauge kind. 1 to 100.
    GaugeFillPct {
        #[schemars(range(min = 1, max = 100))]
        pct: u32,
    },
}

impl CounterEffect {
    pub(super) fn name(&self) -> u8 {
        match self {
            CounterEffect::DamagePct { .. } => 0,
            CounterEffect::StaggerPct { .. } => 1,
            CounterEffect::StunTicks { .. } => 2,
            CounterEffect::Advance { .. } => 3,
            CounterEffect::RefundEvade {} => 4,
            CounterEffect::GaugeFillPct { .. } => 5,
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
    pub(super) fn name(&self) -> u8 {
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
