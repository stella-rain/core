//! The render snapshot: what the frontend draws and interpolates between ticks (ADR-BRDG-4). It is
//! a view of the state, not the state; the state hash does not read it.
//!
//! An enemy's attack (`AttackView`) and zone (`ZoneView`, lanes included) may carry a
//! `threat_colour`: the index of an entry of the run's threat colour table
//! (`Engine::threat_colours`, ADR-BRDG-4 point 9) that the enemy or boss names. Absent is the key
//! missing from the JSON, and means the frontend's own hostile colour; it is never present on a
//! friendly area or on the main character's or a companion's attack.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::{DamageType, HitBox, PhaseKind, ZoneShape};
pub use crate::event::Press;
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId};
use crate::kinds::KindIndex;
use crate::rewards::Rarity;
use crate::rune::Rune;
use crate::threat_colours::ThreatColourIndex;

/// An attack in progress, for the frontend to draw its telegraph and its swing on the
/// same tick that `core` judges them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttackView {
    /// 0 for an agent's or the boss's attacks and the main character's combo.
    pub slot: u8,
    /// The preset it came from; absent for an inline attack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<ContentId>,
    /// Which phase, from 0.
    pub phase: u8,
    pub kind: PhaseKind,
    /// Ticks done in the phase: 0 on its first tick.
    pub tick: u32,
    /// How long the phase lasts.
    pub ticks: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_box: Option<HitBox>,
    /// The facing locked when the attack started; an orbit's orb's turns every tick, and its hit
    /// box with it.
    pub facing: Facing,
    /// Where its hit box is centred, offset along `facing`, when that is not its attacker: an
    /// on-hit rune's attack, at the point the combo step hit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Point>,
    /// The entry of the run's threat colour table (`Engine::threat_colours`) the enemy that
    /// made it names; absent for every other area, and for an enemy that names none: the
    /// frontend draws its own hostile colour. Never present on a friendly area. Absent is the
    /// key missing from the JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threat_colour: Option<ThreatColourIndex>,
}

/// A zone (core Z1): where it is, which way an arc or a line points, and how long its current
/// phase has left. It does not move.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ZoneView {
    pub shape: ZoneShape,
    /// The centre of a circle or an arc, the starting end of a line.
    pub at: Point,
    pub direction: Facing,
    /// `wind_up` while it warns, `active` while it hurts.
    pub phase: PhaseKind,
    /// Ticks left of that phase, at least 1.
    pub ticks_left: u32,
    /// The length of that phase in ticks, at least `ticks_left`: what a wind-up's fill is drawn
    /// against.
    pub ticks: u32,
    /// Placed by the main character or a companion, rather than an enemy.
    pub friendly: bool,
    /// A warning only: `phase` stays `wind_up` for its whole life and no hit follows.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub warn_only: bool,
    /// The entry of the run's threat colour table (`Engine::threat_colours`) the enemy that
    /// placed it names; absent for every other area, and for an enemy that names none: the
    /// frontend draws its own hostile colour. Never present on a friendly area. Absent is the
    /// key missing from the JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threat_colour: Option<ThreatColourIndex>,
}

/// A direction as a unit vector times `trig::SCALE`; `y` grows down the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Facing {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub tick: u32,
    /// The map's size, so the frontend can bound its camera (core#87).
    pub map: MapView,
    /// One per active player, indexed by `PlayerIdx`; a run of one has one.
    pub players: Vec<PlayerView>,
    /// Companions, enemies, the boss and its parts, in entity-ID order.
    pub entities: Vec<EntityView>,
    pub orbs: Vec<OrbView>,
    /// Zones placed and not yet over, in the order they were placed (core Z1); the frontend draws
    /// each as a telegraph while it warns and as a hit while it is active.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<ZoneView>,
    /// Falling stars on the map, in the order they dropped (ADR-REPL-5).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stars: Vec<StarView>,
    /// The run's constellations and how many of each set's members are held (core#NS3); empty
    /// without sets or without stars.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constellations: Vec<ConstellationView>,
    /// Heal pickups on the map, in the order they dropped; empty without healing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub heal_pickups: Vec<HealPickupView>,
    /// Experience, level, stat points; absent in a stage without levels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<ProgressView>,
    /// The expertise gauge; absent in a stage whose enemies and boss give no
    /// expertise points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expertise: Option<ExpertiseView>,
    /// The boss's big patterns announced and not yet started, the call of its elites
    /// first; the frontend draws each as a count-down bar.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<PatternView>,
    /// The Stardust earned so far, times 10; absent in a stage without a Stardust
    /// factor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stardust: Option<u32>,
    /// What Starlight costs and gives, the same for every player; absent in a stage run alone and at
    /// `sim_version` 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starlight: Option<StarlightView>,
}

/// The Starlight prices of an equipment run (placeholders until the pack carries them).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StarlightView {
    /// One per rarity, in `Rarity::ALL` order.
    pub prices: Vec<StarlightPrice>,
    /// What an extracted rune gives: a rune has no rarity.
    pub rune_extract: u32,
    /// What each player who is up gets from a boss kill.
    pub boss_bonus: u32,
}

/// What extracting a star of a rarity gives and what crafting one costs, in Starlight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StarlightPrice {
    pub rarity: Rarity,
    pub extract: u32,
    pub craft: u32,
}

/// A big pattern announced in the snapshot: its content ID, the ticks the count-down
/// started with and the ticks left until it starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatternView {
    pub id: ContentId,
    pub total: u32,
    pub remaining: u32,
}

/// The one expertise gauge of both weapons: its level, from 0, its points
/// toward the next, the style chosen and the Main style's path
/// taken (ADR-REPL-5).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpertiseView {
    /// From 0, with no highest level.
    pub level: u32,
    /// Points gained toward the next level.
    pub points: u32,
    /// Points the next level needs.
    pub next: u32,
    /// The weapon's style, chosen at the level table's style level and held for the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<ContentId>,
    /// The choices taken from the style's path, in level order: its two upgrades, then its
    /// capstone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<ContentId>,
}

/// What an expertise level offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExpertiseOfferKind {
    /// The weapon's three styles, at the level table's style level: fixed, no reroll.
    Style,
    /// The fixed upgrades of the chosen style, at the table's upgrade levels: no reroll.
    Upgrade,
    /// The fixed capstones of the chosen style, at the table's capstone level: no reroll.
    Capstone,
}

/// A weapon tree point a player holds in the equipment build (ADR-SIM-7): nothing is opened and
/// the world is not stopped; the player takes it with `TreePick`, whose `option` indexes `options`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TreeReadyView {
    /// Style, Upgrade or Capstone: the next pick's kind.
    pub kind: ExpertiseOfferKind,
    /// The level table's level of this pick.
    pub level: u32,
    /// The points held, at least 1.
    pub held: u8,
    /// The choices, in the order `TreePick.option` indexes.
    pub options: Vec<ContentId>,
    /// The mechanic kinds each option grants, one entry per option in `options` order; absent
    /// while no option names a kind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<Vec<StyleKindView>>,
}

/// Which a `replace_step` grant gives its step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepGrantKind {
    Charge,
    Hold,
}

/// Which smash effect a `smash_effect` kind sets, by name only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SmashEffectKind {
    Pull,
    Knockback,
    Bleed,
    Stun,
    Reap,
}

/// What a gauge kind spends its full gauge on, by name only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GaugeSpendKind {
    FullSmash,
}

/// A mechanic kind a style level grants, in the compact form a snapshot carries (built every
/// frame, so the grant's attacks are not inside).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StyleKindView {
    /// The step the presses reach from the root of the combo tree gets a charge or a hold.
    ReplaceStep {
        step: Vec<Press>,
        grant: StepGrantKind,
    },
    /// The weapon's smash effect is this one in place of the base's.
    SmashEffect { effect: SmashEffectKind },
    /// The style has a weapon gauge `max` long, spent as `spend`.
    Gauge { max: u32, spend: GaugeSpendKind },
}

/// The kinds one level of a style's path grants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelKindsView {
    /// Style (the style pick itself), Upgrade or Capstone.
    pub offer: ExpertiseOfferKind,
    /// The style's or the choice's ID at that level.
    pub option: ContentId,
    /// What taking it grants; empty for a level that names none.
    pub kinds: Vec<StyleKindView>,
}

/// A weapon's stat sets as the weapon level `level` gives them (equipment mode), for the tree
/// selection window and the weapon window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeaponStatsView {
    /// The weapon level, 1 to 10 (level 0 shows 1, and levels past 10 show 10).
    pub level: u32,
    /// The tree chosen, by its style ID; absent before the style pick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<ContentId>,
    /// The base set: in force until a tree is chosen.
    pub base: Vec<StatValueView>,
    /// Every tree's set, in the weapon's style order (the order `TreeReadyView.options` lists).
    pub trees: Vec<TreeStatsView>,
}

/// One tree's stat set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TreeStatsView {
    /// The tree's style ID.
    pub style: ContentId,
    /// The stats it gives at the weapon's level.
    pub stats: Vec<StatValueView>,
    /// What the style grants besides stats: its own level, then its three path levels; absent
    /// when no level names a kind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<LevelKindsView>,
}

/// One stat of a set. Percentages are in **tenths of a percent** (165 is 16.5 percent); the
/// most hit points (`MaxHp`) are whole units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatValueView {
    pub effect: crate::run_stats::StatEffect,
    /// The value at the weapon's level.
    pub now: u32,
    /// The value at the next level (the same at level 10).
    pub next: u32,
}

/// Where the run stands in experience, levels and stat points (ADR-REPL-5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgressView {
    /// From 1.
    pub level: u32,
    /// Experience gained toward the next level.
    pub xp: u32,
    /// Experience the next level needs; 0 at the highest level.
    pub xp_next: u32,
    /// Points spent on each run stat, in the run stats' order (ADR-CONT-1).
    pub spent: Vec<u8>,
}

/// A falling star waiting on the map to be taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StarView {
    pub id: EntityId,
    pub at: Point,
    pub rarity: Rarity,
}

/// A heal pickup on the map (see `DomainEvent::HealDropped`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HealPickupView {
    pub id: EntityId,
    pub at: Point,
}

/// How far a set of named stars is (core#NS3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConstellationView {
    /// Its ID in the run's constellations file.
    pub id: ContentId,
    /// Its name.
    pub name: String,
    /// Members held, carried stars included.
    pub held: u8,
    /// The tier reached, counted from 1; 0 before the first.
    pub tier: u8,
    /// Members the next tier needs; the whole set once the top tier is reached.
    pub needed: u8,
}

/// What a reward is for, to show it by (ADR-REPL-5). Derived from the effect kinds of the
/// reward in one exhaustive table, never stored in the content (ADR-CONT-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    /// Damage dealt by the main character's attacks.
    Attack,
    /// Effects that follow a hit: echoes, kill bursts, orbits.
    OnHit,
    /// Hit points, regeneration and evading.
    Defence,
    /// Movement, reach and pickup.
    Mobility,
    /// Runes.
    Rune,
    /// Experience and the offers themselves: rerolls, banishes, candidates.
    Growth,
}

/// A stat a pick can move; the unit is the field's own (percent points, hit points, counts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatKind {
    MaxHp,
    Regen,
    EvadePct,
    SpeedPct,
    XpPct,
    AttackPct,
    /// As held to the run's attack speed cap.
    AttackSpeedPct,
    /// As held to the crit chance cap.
    CritChancePct,
    /// As held to the crit damage cap.
    CritDamagePct,
    /// As held to the echo cap.
    EchoHitPct,
    /// The bonus for one damage type, as held at an attack's start.
    TypeBonus(DamageType),
    RangePct,
    PickupRadiusPct,
    SkillCooldownPct,
    RuneHastePct,
    EvadeCharges,
    KillHeal,
    BurstPct,
    /// The orbs of every orbit held.
    Orbs,
    Rerolls,
    Banishes,
    OfferCandidates,
}

impl StatKind {
    /// The order the stats are listed in: the declaration order, the types by `ALL`.
    fn rank(self) -> (u8, usize) {
        match self {
            StatKind::MaxHp => (0, 0),
            StatKind::Regen => (1, 0),
            StatKind::EvadePct => (2, 0),
            StatKind::SpeedPct => (3, 0),
            StatKind::XpPct => (4, 0),
            StatKind::AttackPct => (5, 0),
            StatKind::AttackSpeedPct => (6, 0),
            StatKind::CritChancePct => (7, 0),
            StatKind::CritDamagePct => (8, 0),
            StatKind::EchoHitPct => (9, 0),
            StatKind::TypeBonus(ty) => (10, ty.index()),
            StatKind::RangePct => (11, 0),
            StatKind::PickupRadiusPct => (12, 0),
            StatKind::SkillCooldownPct => (13, 0),
            StatKind::RuneHastePct => (14, 0),
            StatKind::EvadeCharges => (15, 0),
            StatKind::KillHeal => (16, 0),
            StatKind::BurstPct => (17, 0),
            StatKind::Orbs => (18, 0),
            StatKind::Rerolls => (19, 0),
            StatKind::Banishes => (20, 0),
            StatKind::OfferCandidates => (21, 0),
        }
    }
}

impl PartialOrd for StatKind {
    fn partial_cmp(&self, other: &StatKind) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StatKind {
    fn cmp(&self, other: &StatKind) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
    }
}

/// One stat a pick moves, before and after.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatChange {
    pub stat: StatKind,
    pub before: u32,
    pub after: u32,
}

/// What `Engine::swap_ratings` rates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwapItem {
    /// The inventory place `item` of `EquipAction` (a rune rates nothing: an empty result).
    Inventory(u32),
    /// A star not held, by reward ID (a craft candidate, a preview). An unknown ID gives an
    /// empty result.
    Star(ContentId),
}

/// One slot a star could go in (the slot rules accept it) and what the swap does, from
/// `Engine::swap_ratings`. The model and its limits are in `sim::offer_rating`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapRating {
    /// The slot, as `EquipAction` numbers it (the Lumine slot is `slots.len()`).
    pub slot: u32,
    /// The slot holds a star now: a replace (the displaced star returns to the inventory);
    /// otherwise an equip.
    pub replaces: bool,
    /// A replace by a star sharing an option (a grade switch): the slot keeps its whole level.
    pub switch: bool,
    /// The slot's level now (0 while it never held a star) and after the swap, read from the
    /// rules run on a clone.
    pub level_before: u8,
    pub level_after: u8,
    /// The categories of the incoming star's effects, then of a set tier it newly reaches.
    pub categories: Vec<Category>,
    /// The signed expected change of the Main weapon's damage per tick, in basis points (+1538
    /// is +15.38%, -833 is -8.33%), the ratio rounded half up; `Some(0)` when no modelled term
    /// moves; `None` when no star or set tier the swap crosses has a modelled effect, or the
    /// build has no weapon damage to rate.
    pub dps_bp: Option<i32>,
    /// The highest set tier, counted from 1, the swap newly reaches, and the highest it loses.
    pub set_gained: Option<u8>,
    pub set_lost: Option<u8>,
    /// The stats the swap moves, before and after, in `StatKind` order.
    pub changes: Vec<StatChange>,
}

/// The map's width and height, from its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MapView {
    pub width: Fx,
    pub height: Fx,
}

/// A player's weapon gauge (ADR-SIM-9).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GaugeView {
    /// The gauge now, in gauge units.
    pub value: u32,
    /// The style's maximum: the bar is this long, and the gauge is full at `value == max`.
    pub max: u32,
}

/// A player's perfect evade after the near miss (ADR-SIM-8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PerfectEvadeView {
    /// Ticks of the slow left; enemies within `reach` of the player's `at` are slowed.
    pub slow_left: u32,
    /// Ticks until another near miss can be perfect.
    pub cooldown_left: u32,
    /// The slow's reach around the player, in 1/256 px.
    pub reach: Fx,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlayerView {
    pub at: Point,
    pub hp: u32,
    /// The main character's hurt circle.
    pub radius: Fx,
    /// The last non-zero movement direction.
    pub facing: Facing,
    /// Ticks of invulnerability left, after a hit or from an evade; the frontend can flash it.
    pub invulnerable: u32,
    pub evade: EvadeView,
    /// The weapon gauge of its style, while the style grants one (ADR-SIM-9); absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gauge: Option<GaugeView>,
    /// The perfect evade's slow and cooldown, while either runs and the run's rules name the
    /// perfect evade (ADR-SIM-8); absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perfect_evade: Option<PerfectEvadeView>,
    /// Where it is in its Main weapon's combos.
    pub combo: ComboView,
    /// The current step charging, if it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charge: Option<ChargeView>,
    /// The level auto mode is charging the current step to, while it is: its own
    /// field, so `charge` stays the player's and the frontend slows only for that, never for auto mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_charge: Option<u8>,
    /// The current step held, if it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<HoldView>,
    /// The enemy aim assist and auto mode would face, while either is on, so the
    /// frontend can mark it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<EntityId>,
    /// The stage skill in its slot, if a reward has filled it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<SkillView>,
    /// The stage skill's pull while it pulls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull: Option<PullView>,
    /// The stage skill's shield while it holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shield: Option<ShieldView>,
    /// The orbits rewards gave, in the order they came.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub orbits: Vec<OrbitView>,
    /// What the range rewards taken add up to, in percent, before the clamp at
    /// `MAX_RANGE_PCT`. The wider hit boxes are already in `attacks`.
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub range_pct: u32,
    /// What the Rune Haste rewards taken add up to, in percent, before the clamp at
    /// the rewards file's `max_rune_haste_pct`. The shorter cooldowns and intervals are already in the
    /// runes' cooldowns and the statuses' timers.
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub rune_haste_pct: u32,
    /// The pickup circle of stars and heal pickups is that much wider, in percent: the pickup
    /// radius rewards taken plus the `PickupRadiusPct` stat, held to `MAX_PICKUP_RADIUS_TOTAL_PCT`.
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub pickup_radius_pct: u32,
    /// What the skill cooldown rewards taken add up to, in percent, before the clamp at
    /// `MAX_SKILL_COOLDOWN_TOTAL_PCT`: the stage skill's cooldown is already that much shorter.
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub skill_cooldown_pct: u32,
    /// Hit points (the stage's scale) the heal-per-kills rewards taken heal together every
    /// `kill_heal_every` kills; both 0 while none is held.
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub kill_heal: u32,
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub kill_heal_every: u32,
    /// What the combat rewards taken add up to (core#NS2b): the chance of a crit in percent
    /// (at most 75), a crit's bonus damage beyond twice (at most 300), an echo's percent of a
    /// hit (at most 100), and a kill burst's percent of max hit points, radius in pixels and
    /// chain depth; each 0 while none is held.
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub crit_chance_pct: u32,
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub crit_damage_pct: u32,
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub echo_hit_pct: u32,
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub burst_pct: u32,
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub burst_radius: u32,
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub burst_depth: u32,
    /// What the type bonus rewards taken add up to, for each damage type that has any, in the
    /// order of `DamageType::ALL`, before the clamp at the match-up file's `max_type_bonus_pct`
    ///.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_bonuses: Vec<TypeBonusView>,
    /// The runes its Main weapon holds, in the order drawn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runes: Vec<RuneView>,
    /// What its inventory holds, in drop order (equipment mode, `sim_version` 2; empty at 1). The app
    /// sorts it by rarity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inventory: Vec<HeldView>,
    /// Its Starlight balance (equipment mode; absent reads as 0).
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub starlight: u32,
    /// For each star of the setup's star pool, in pool order (the index `Craft` takes): whether
    /// a craft of it is refused because this player already holds that named star. Empty in a
    /// stage run alone and at `sim_version` 1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pool_held: Vec<bool>,
    /// The weapon tree point it holds (equipment mode); absent at `sim_version` 1 and while none is
    /// held.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_ready: Option<TreeReadyView>,
    /// The weapon's stat sets (equipment mode): the base set and every tree's set at the weapon's
    /// level; absent at `sim_version` 1, without an expertise gauge and for an inline weapon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weapon_stats: Option<WeaponStatsView>,
    /// Its equipment slots in layout order, then the Lumine slot last when the run has one
    /// (equipment mode; empty at `sim_version` 1). An action addresses them by this order: an index
    /// into the list, the Lumine slot one past the others.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<SlotView>,
    /// Its attacks in progress, by slot: 0 the combo, 1 the stage skill, then 8 for each orbit,
    /// one for each orb, in the order the orbits came, then from 34 one for each
    /// on-hit rune, by its place in `Rune::ALL`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attacks: Vec<AttackView>,
    /// Its statuses, sorted by ID: the timed buffs and debuffs it holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<StatusView>,
    /// Downed state when the player has 0 HP in a party (ADR-SIM-7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down: Option<DownView>,
    /// Out state when a downed player's out timer expired (ADR-SIM-7).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub out: bool,
}

/// The downed state of a party player (ADR-SIM-7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DownView {
    pub ticks_left: u32,
    pub revive_hold: u32,
}

/// The type bonus the main character holds for one damage type: the one-type
/// bonuses of that type and its group's bonus, added up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TypeBonusView {
    #[serde(rename = "type")]
    pub damage_type: DamageType,
    pub pct: u32,
}

/// A rune the Main weapon holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuneView {
    pub rune: Rune,
    /// For an on-hit rune, ticks until it may fire again; 0 for a damage-over-time rune.
    pub cooldown_left: u32,
}

/// What an inventory place holds: a star, by its reward ID and rarity, or a rune.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HeldView {
    Star { id: ContentId, rarity: Rarity },
    Rune { rune: Rune },
}

/// Which equipment slot a `SlotView` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SlotKindView {
    /// A category slot: the category's index in the layout.
    Category {
        index: u8,
    },
    Rune,
    /// The Lumine slot, present only when the run's setup names a Lumine star.
    Lumine,
}

/// One equipment slot: what it holds, its level (0 while it never held anything) and the kills
/// toward its next level.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlotView {
    pub kind: SlotKindView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<HeldView>,
    pub level: u8,
    pub expertise: u32,
}

/// An orbit held. Its orbs' attacks are in the main character's `attacks`, in slots
/// `2 + 8 × its place in orbits` and on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OrbitView {
    /// The reward that gave it, for its name.
    pub reward: ContentId,
    /// Its orbs: those of the next round.
    pub orbs: u8,
    /// Ticks until the next round may start, counted from the last one's start; 0 while a
    /// round that outlasts it goes on.
    pub cooldown_left: u32,
}

/// The stage skill held: what the skill button draws.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillView {
    /// Its ID in the stage skills file, for its name.
    pub skill: ContentId,
    /// The asset the button draws.
    pub icon: ContentId,
    /// From 1.
    pub level: u8,
    /// A press on the next tick would use it.
    pub ready: bool,
    /// Ticks until it is ready again, counted from its use.
    pub cooldown_left: u32,
}

/// A stage skill's pull pulling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PullView {
    pub at: Point,
    pub radius: Fx,
    /// Ticks it still pulls, this one included.
    pub ticks_left: u32,
}

/// A shield held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShieldView {
    /// The damage it still absorbs.
    pub amount: u32,
    /// Ticks it still holds, this one included.
    pub ticks_left: u32,
}

/// The evade and its charges.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvadeView {
    /// The charges held, ready or recovering: from the stage's `charges`, raised by rewards up
    /// to its `max_charges`.
    pub charges: u8,
    /// The charges ready: a press on the next tick dashes when it is more than 0, even during a
    /// dash.
    pub ready: u8,
    /// Ticks left on each spent charge, in the order they were spent; each counts from the
    /// press that spent it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recovering: Vec<u32>,
    /// Ticks of the dash still to move: 0 when not dashing.
    pub dash_left: u32,
}

/// Where the main character is in its combos: what the frontend's combo trail draws,
/// with the next possible presses read from the weapon's tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComboView {
    /// The presses from the root to the current step; empty at the root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presses: Vec<Press>,
    /// The presses began in the dash branch (a dash-strike).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dash: bool,
    /// The press waiting for the next step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffered: Option<Press>,
    /// Ticks left in which a press continues the combo, once a step has run out.
    pub window_left: u32,
    /// The auto combo: the presses auto mode plays from the root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auto: Vec<Press>,
}

/// A charge in progress. `core` judges the perfect window on this tick, so what the
/// frontend shows is what a release on the next tick gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChargeView {
    /// The level reached: 0 before level 1, when letting go is a plain step and the frontend
    /// keeps its clock at full speed.
    pub level: u8,
    /// Ticks held since the step began to charge, 0 on its first tick.
    pub held: u32,
    /// Inside the level's perfect window: letting go on the next tick is a perfect release.
    pub perfect: bool,
}

/// A hold in progress: the ticks held so far, 0 on the tick it began, and the
/// most it can last. The release comes on the tick the button is let go or the maximum is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HoldView {
    pub held: u32,
    pub max_ticks: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntityView {
    pub id: EntityId,
    pub kind: EntityKind,
    pub asset: ContentId,
    /// Its kind, the one the Bestiary keys on: an index into the run's `KindTable`
    /// (`Engine::kinds`) of an enemy's, an elite's, a boss's or a companion's sprite ID, or of a
    /// boss part's own ID. Absent for a mimic still in disguise (`asset` is then its disguise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_index: Option<KindIndex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<ContentId>,
    pub at: Point,
    /// The hurt circle that hits use.
    pub radius: Fx,
    pub hp: u32,
    /// The attack it is making, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attack: Option<AttackView>,
    /// A mimic that is still disguised: `asset` is its disguise and `elite` is false,
    /// so the app draws an object and the minimap shows nothing. Absent when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disguised: bool,
    /// An elite enemy, which the minimap shows wherever it is.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub elite: bool,
    /// A pack leader's aura: the radius, around `at`, within which the non-elite
    /// enemies deal more damage for as long as this entity is there to be drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aura_radius: Option<Fx>,
    /// Its damage-over-time statuses, one of a kind at most.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub damage_over_time: Vec<DamageOverTimeView>,
    /// Its statuses and auras, sorted by ID. An enemy can show `atk_up`, `vulnerable`, `slow`,
    /// `bleed`, `poison`, `burn`, `stun`, `flinch` and `aura`, each with `remaining` and `total`
    /// ticks; `flinch` is the enemy's own control state, shown as a status.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<StatusView>,
    /// A shrine's running fight: the ticks left before it leaves unkilled.
    /// Absent for every other entity, and for a shrine nobody has touched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fight: Option<Countdown>,
    /// The stagger gauge of an elite or the boss whose definition names a `stagger_max`: what it
    /// holds now (0 while it is staggered) and its maximum. Absent for every other entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stagger: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stagger_max: Option<u32>,
}

/// A count-down in the snapshot: the ticks left and the ticks it started with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Countdown {
    /// Ticks left.
    pub remaining: u32,
    /// The ticks it started with.
    pub total: u32,
}

/// A status on an entity, or a pack leader's aura: what the app shows as
/// an icon with a count-down, or as a row while it lasts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusView {
    /// The status's content ID: `atk_up`, `vulnerable`, `slow`, `flinch`, `staggered`, `bleed`, `poison`, `burn`, or
    /// `aura` for a pack leader's aura, which has no content ID of its own.
    pub id: ContentId,
    /// Ticks left. An aura has none: it lasts while its entity is in reach, so both this and
    /// `total` are 0 for it.
    pub remaining: u32,
    /// The ticks the status was applied for, or 0 for an aura.
    pub total: u32,
}

/// A damage-over-time status on an entity: bleed, poison or burn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DamageOverTimeView {
    /// The status's ID: `bleed`, `poison` or `burn`.
    pub status: ContentId,
    pub stacks: u16,
    /// Ticks until it runs out, unless it is applied again.
    pub ticks_left: u32,
    /// Ticks until it next deals its damage, at least 1.
    pub next_tick_in: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Companion,
    Enemy,
    Boss,
    BossPart,
    /// An elite object that is not an enemy: it cannot be damaged and fights through its
    /// guardians, who are `Enemy`s. Its `elite` flag is set.
    Shrine,
}

/// Orbs carry no ID or asset name: thousands are drawn per tick, batched by `style`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OrbView {
    pub at: Point,
    /// How far the orb moves each tick, in 1/256 pixel: it moved by this to reach `at`, and
    /// moves by it again on the next tick. The frontend draws it at `at` plus this times the
    /// fraction of the tick that has passed.
    pub vx: Fx,
    pub vy: Fx,
    pub radius: Fx,
    /// The orb asset, as an index into the orb asset IDs the stage uses, in order of
    /// first appearance.
    pub style: u16,
    /// An orb the main character's hit box breaks.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub breakable: bool,
    /// The index, in this list, of the orb a live beam runs to from this one (a cage's `link`);
    /// the beam is as wide as `link.width` and hurts the main character.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_to: Option<u32>,
    /// The width of that beam, in 1/256 pixel (`link.width` of the content); set exactly when
    /// `link_to` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_width: Option<Fx>,
}
