//! The render snapshot: what the frontend draws and interpolates between ticks (ADR-005). It is
//! a view of the state, not the state; the state hash does not read it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::{DamageType, HitBox, PhaseKind};
use crate::event::Press;
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId};
use crate::rewards::Rarity;
use crate::rune::Rune;

/// An attack in progress (ADR-055), for the frontend to draw its telegraph and its swing on the
/// same tick that `core` judges them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttackView {
    /// 0 for an agent's or the boss's attacks and the main character's combo (ADR-069).
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
    /// on-hit rune's attack, at the point the combo step hit (ADR-099).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Point>,
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
    pub player: PlayerView,
    /// Companions, enemies, the boss and its parts, in entity-ID order.
    pub entities: Vec<EntityView>,
    pub bullets: Vec<BulletView>,
    /// Falling stars on the map, in the order they dropped (ADR-079 §2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stars: Vec<StarView>,
    /// Heal pickups on the map, in the order they dropped; empty without healing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub heal_pickups: Vec<HealPickupView>,
    /// Experience, level, stat points and the open offer; absent in a stage without levels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<ProgressView>,
    /// The expertise gauge (ADR-078); absent in a stage whose enemies and boss give no
    /// expertise points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expertise: Option<ExpertiseView>,
    /// The boss's big patterns announced and not yet started (ADR-103), the call of its elites
    /// first; the frontend draws each as a count-down bar.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<PatternView>,
    /// The Stardust earned so far, times 10 (ADR-109); absent in a stage without a Stardust
    /// factor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stardust: Option<u32>,
}

/// A big pattern announced in the snapshot (ADR-103): its content ID, the ticks the count-down
/// started with and the ticks left until it starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatternView {
    pub id: ContentId,
    pub total: u32,
    pub remaining: u32,
}

/// The one expertise gauge of both weapons (ADR-078 §1 to §3): its level, from 0, its points
/// toward the next, the common options taken, the styles chosen and the Main style's path
/// taken (ADR-078 §5), and the open expertise offer (ADR-079 §3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpertiseView {
    /// From 0, with no highest level.
    pub level: u32,
    /// Points gained toward the next level.
    pub points: u32,
    /// Points the next level needs.
    pub next: u32,
    /// Times each option of the weapon's common pool was taken, in the pool's order; empty for
    /// a weapon with no expertise content.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub taken: Vec<u8>,
    /// The weapon's style, chosen at the level table's style level and held for the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<ContentId>,
    /// The choices taken from the style's path, in level order: its two upgrades, then its
    /// capstone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<ContentId>,
    /// The open expertise offer: while it is open the world is stopped, and `Engine::choose`
    /// takes one of its options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offer: Option<ExpertiseOfferView>,
}

/// What an expertise level offers (ADR-078 §4, §5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExpertiseOfferKind {
    /// Options drawn from the weapon's common pool, with a reroll of each.
    Common,
    /// The weapon's three styles, at the level table's style level: fixed, no reroll.
    Style,
    /// The fixed upgrades of the chosen style, at the table's upgrade levels: no reroll.
    Upgrade,
    /// The fixed capstones of the chosen style, at the table's capstone level: no reroll.
    Capstone,
}

/// An expertise offer (ADR-078 §4 to §6; ADR-079 §3). A common one is drawn from one weapon's
/// common pool, with a reroll of each candidate; a style, upgrade or capstone one shows its
/// fixed choices, with no reroll. None has a skip or a banish.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpertiseOfferView {
    /// The expertise level that opened it.
    pub level: u32,
    pub kind: ExpertiseOfferKind,
    pub candidates: Vec<ExpertiseCandidateView>,
    /// What `Engine::choose` takes, by index: the candidates, then, in a common offer, a reroll
    /// of each while rerolls are left and another option could come.
    pub options: Vec<OfferOption>,
    /// Rerolls left in the run, shared with the stars' offers.
    pub rerolls_left: u32,
}

/// An option an expertise offer holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpertiseCandidateView {
    /// Its ID in the weapon's expertise content: a common option, a style, or a choice of the
    /// chosen style's path.
    pub option: ContentId,
}

/// Where the run stands in experience, levels and stat points (ADR-079 §2, §4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgressView {
    /// From 1.
    pub level: u32,
    /// Experience gained toward the next level.
    pub xp: u32,
    /// Experience the next level needs; 0 at the highest level.
    pub xp_next: u32,
    /// Stat points not spent yet.
    pub points: u32,
    /// Points spent on each run stat, in the run stats' order (ADR-067).
    pub spent: Vec<u8>,
    /// The open offer: while it is open the world is stopped, and `Engine::choose` takes one
    /// of its options (ADR-079 §3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offer: Option<OfferView>,
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

/// An offer from a falling star (ADR-079 §3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OfferView {
    pub kind: OfferKind,
    /// The star's rarity, or for a boss offer the highest of the player's tier. A candidate is
    /// of a lower one when the pool has too few left of it.
    pub rarity: Rarity,
    pub candidates: Vec<CandidateView>,
    /// What `Engine::choose` takes, by index: the candidates, then a reroll of each while
    /// rerolls are left and another reward could come, then the skip (not in a boss offer),
    /// then a banish of each while banishes are left.
    pub options: Vec<OfferOption>,
    /// Rerolls and banishes left in the run.
    pub rerolls_left: u32,
    pub banishes_left: u32,
}

/// What opened an offer (ADR-079 §2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OfferKind {
    /// A falling star the main character took; it can be skipped.
    Star,
    /// A boss kill: at the highest rarity of the player's tier, with no skip.
    Boss,
}

/// A reward an offer holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateView {
    /// Its ID in the run's rewards file.
    pub reward: ContentId,
    pub rarity: Rarity,
    /// A random rune (ADR-099): which rune it gives is drawn only when it is taken, so the
    /// frontend shows it face down.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

/// One option of an open offer; `candidate` is a place in its candidates, from 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OfferOption {
    /// Takes the candidate's reward and closes the offer.
    Take { candidate: u8 },
    /// Draws that candidate again, spending a reroll; the offer stays open.
    Reroll { candidate: u8 },
    /// Closes the offer, taking nothing.
    Skip,
    /// Takes the candidate's reward out of the run's pool for good, spending a banish, and
    /// draws its place again; the offer stays open, or closes when nothing is left to offer.
    Banish { candidate: u8 },
}

/// The map's width and height, from its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MapView {
    pub width: Fx,
    pub height: Fx,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlayerView {
    pub at: Point,
    pub hp: u32,
    /// The main character's hurt circle.
    pub radius: Fx,
    /// The last non-zero movement direction (ADR-055).
    pub facing: Facing,
    /// Ticks of invulnerability left, after a hit or from an evade; the frontend can flash it.
    pub invulnerable: u32,
    pub evade: EvadeView,
    /// Where it is in its Main weapon's combos (ADR-070).
    pub combo: ComboView,
    /// The current step charging, if it is (ADR-070).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charge: Option<ChargeView>,
    /// The current step held, if it is (ADR-084 point 6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<HoldView>,
    /// The enemy aim assist and auto mode would face, while either is on (ADR-072), so the
    /// frontend can mark it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<EntityId>,
    /// The stage skill in its slot, if a reward has filled it (ADR-075).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<SkillView>,
    /// The stage skill's pull while it pulls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull: Option<PullView>,
    /// The stage skill's shield while it holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shield: Option<ShieldView>,
    /// The orbits rewards gave, in the order they came (ADR-069).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub orbits: Vec<OrbitView>,
    /// What the range rewards taken add up to, in percent, before the clamp at
    /// `MAX_RANGE_PCT` (ADR-099). The wider hit boxes are already in `attacks`.
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub range_pct: u32,
    /// What the Rune Haste rewards taken add up to, in percent, before the clamp at
    /// `MAX_RUNE_HASTE_PCT` (ADR-099). The shorter cooldowns and intervals are already in the
    /// runes' cooldowns and the statuses' timers.
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub rune_haste_pct: u32,
    /// What the type bonus rewards taken add up to, for each damage type that has any, in the
    /// order of `DamageType::ALL`, before the clamp at the match-up file's `max_type_bonus_pct`
    /// (ADR-100).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_bonuses: Vec<TypeBonusView>,
    /// The runes its Main weapon holds, in the order drawn (ADR-099).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runes: Vec<RuneView>,
    /// Its attacks in progress, by slot: 0 the combo, 1 the stage skill, then 8 for each orbit,
    /// one for each orb, in the order the orbits came (ADR-069), then from 34 one for each
    /// on-hit rune, by its place in `Rune::ALL` (ADR-099).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attacks: Vec<AttackView>,
    /// Its statuses, sorted by ID (ADR-103 decision 3): the timed buffs and debuffs it holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<StatusView>,
}

/// The type bonus the main character holds for one damage type (ADR-100): the one-type
/// bonuses of that type and its group's bonus, added up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TypeBonusView {
    #[serde(rename = "type")]
    pub damage_type: DamageType,
    pub pct: u32,
}

/// A rune the Main weapon holds (ADR-099).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuneView {
    pub rune: Rune,
    /// For an on-hit rune, ticks until it may fire again; 0 for a damage-over-time rune.
    pub cooldown_left: u32,
}

/// An orbit held (ADR-069). Its orbs' attacks are in the main character's `attacks`, in slots
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

/// The stage skill held (ADR-075): what the skill button draws.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillView {
    /// Its ID in the stage skills file, for its name.
    pub skill: ContentId,
    /// The asset the button draws (ADR-098).
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

/// The evade and its charges (ADR-074).
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

/// Where the main character is in its combos (ADR-070): what the frontend's combo trail draws,
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
    /// The auto combo: the presses auto mode plays from the root (ADR-073).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auto: Vec<Press>,
}

/// A charge in progress (ADR-070). `core` judges the perfect window on this tick, so what the
/// frontend shows is what a release on the next tick gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChargeView {
    /// The level reached: 0 before level 1, when letting go is a plain step and the frontend
    /// keeps its clock at full speed (ADR-071).
    pub level: u8,
    /// Ticks held since the step began to charge, 0 on its first tick.
    pub held: u32,
    /// Inside the level's perfect window: letting go on the next tick is a perfect release.
    pub perfect: bool,
}

/// A hold in progress (ADR-084 point 6): the ticks held so far, 0 on the tick it began, and the
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<ContentId>,
    pub at: Point,
    /// The hurt circle that hits use.
    pub radius: Fx,
    pub hp: u32,
    /// The attack it is making, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attack: Option<AttackView>,
    /// A mimic that is still disguised (ADR-090): `asset` is its disguise and `elite` is false,
    /// so the app draws an object and the minimap shows nothing. Absent when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disguised: bool,
    /// An elite enemy (ADR-068), which the minimap shows wherever it is.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub elite: bool,
    /// A pack leader's aura (ADR-090): the radius, around `at`, within which the non-elite
    /// enemies deal more damage for as long as this entity is there to be drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aura_radius: Option<Fx>,
    /// Its damage-over-time statuses (ADR-099), one of a kind at most.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub damage_over_time: Vec<DamageOverTimeView>,
    /// Its statuses and auras, sorted by ID (ADR-103 decision 3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<StatusView>,
    /// A shrine's running fight (ADR-090, ADR-103): the ticks left before it leaves unkilled.
    /// Absent for every other entity, and for a shrine nobody has touched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fight: Option<Countdown>,
}

/// A count-down in the snapshot (ADR-103): the ticks left and the ticks it started with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Countdown {
    /// Ticks left.
    pub remaining: u32,
    /// The ticks it started with.
    pub total: u32,
}

/// A status on an entity, or a pack leader's aura (ADR-103 decision 3): what the app shows as
/// an icon with a count-down, or as a row while it lasts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusView {
    /// The status's content ID: `atk_up`, `vulnerable`, `slow`, `bleed`, `poison`, `burn`, or
    /// `aura` for a pack leader's aura, which has no content ID of its own.
    pub id: ContentId,
    /// Ticks left. An aura has none: it lasts while its entity is in reach, so both this and
    /// `total` are 0 for it.
    pub remaining: u32,
    /// The ticks the status was applied for, or 0 for an aura.
    pub total: u32,
}

/// A damage-over-time status on an entity (ADR-099): bleed, poison or burn.
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
}

/// Bullets carry no ID or asset name: thousands are drawn per tick, batched by `style`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BulletView {
    pub at: Point,
    /// How far the bullet moves each tick, in 1/256 pixel: it moved by this to reach `at`, and
    /// moves by it again on the next tick. The frontend draws it at `at` plus this times the
    /// fraction of the tick that has passed.
    pub vx: Fx,
    pub vy: Fx,
    pub radius: Fx,
    /// The bullet asset, as an index into the bullet asset IDs the stage uses, in order of
    /// first appearance.
    pub style: u16,
    /// Fired by the main character or a companion, rather than an enemy.
    pub friendly: bool,
}
