//! The stage file (ADR-006): data only, parsed strictly (ADR-018, ADR-020). Unknown fields are
//! rejected everywhere. Field names follow ADR-036's shape and are placeholders until the
//! freeze (ADR-035); the validator (budgets, registry, text) is separate.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::{AttackRef, Profile};
use crate::behaviour::{Agent, Condition, Rule};
use crate::fixed::{Fx, Point};
use crate::id::ContentId;
use crate::rewards::Rarity;
use crate::validate::limits::{FIELD_H, FIELD_MARGIN, FIELD_W, MAX_ELITES_ALIVE};
use crate::weapon::{MainWeapon, SubWeapon, WeaponRef};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    /// `crate::version::SCHEMA_VERSION` when the game wrote the file.
    pub schema_version: u32,
    /// `crate::version::SIM_VERSION` when the game wrote the file.
    pub sim_version: u32,
    /// The stage ID: the file name and the first half of the version tag.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Seeds the run's SplitMix64 (ADR-033). 32 bits so that every tool reading the file as
    /// JSON, GDScript included, keeps it exact; the simulation widens it.
    pub seed: u32,
    /// Ticks until the stage ends if the boss is still alive.
    pub length_ticks: u32,
    pub player: Player,
    #[serde(default)]
    pub companions: Vec<Agent>,
    #[serde(default)]
    pub waves: Vec<Wave>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boss: Option<Boss>,
    /// The map the run plays on; without one, the portrait field (`Map::PORTRAIT`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map: Option<Map>,
    /// Experience and levels, each level-up giving a stat point (ADR-079 §2); a mode's stages
    /// have them. Without them defeated enemies give nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<Levels>,
    /// Falling stars and the offers they open (ADR-079 §2, §3, ADR-068); only with `levels`.
    /// Without them kills drop nothing and nothing is offered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stars: Option<Stars>,
    /// The most elites alive at once (ADR-068); `MAX_ELITES_ALIVE` when absent. An elite that
    /// would spawn past it does not, and a wave counts it as spawned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elite_cap: Option<u16>,
}

/// How kills drop falling stars, what rarity a star has, and what an offer allows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stars {
    /// The chance, in 1000, that a kill drops a star.
    pub drop_per_mille: u16,
    /// Kills in a row without a star after which the next kill drops one for sure.
    pub pity: u16,
    /// The most stars on the map at once. A kill with the map full drops none, and does not
    /// count toward the pity.
    pub cap: u16,
    /// How near the main character's centre a star must come to be taken.
    pub pickup_radius: Fx,
    /// Rerolls of one candidate a run gets, before the setup's extra rerolls.
    #[serde(default)]
    pub rerolls: u8,
    /// Rewards a run may banish.
    #[serde(default)]
    pub banishes: u8,
    /// The drop table: tiers by the player's level, the first from level 1, each later one from
    /// a higher level (ADR-068). A star draws its rarity from the tier of the level reached.
    pub tiers: Vec<Tier>,
}

/// One tier of the drop table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Tier {
    /// The level the tier starts at.
    pub from_level: u32,
    pub weights: RarityWeights,
}

/// The weight of each rarity in a draw; a rarity left out weighs 0. A tier's highest rarity is
/// the highest one that weighs more than 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RarityWeights {
    #[serde(default)]
    pub common: u16,
    #[serde(default)]
    pub uncommon: u16,
    #[serde(default)]
    pub rare: u16,
    #[serde(default)]
    pub epic: u16,
    #[serde(default)]
    pub legendary: u16,
    #[serde(default)]
    pub supernova: u16,
}

impl RarityWeights {
    /// The weight of `rarity`.
    pub fn get(&self, rarity: Rarity) -> u16 {
        match rarity {
            Rarity::Common => self.common,
            Rarity::Uncommon => self.uncommon,
            Rarity::Rare => self.rare,
            Rarity::Epic => self.epic,
            Rarity::Legendary => self.legendary,
            Rarity::Supernova => self.supernova,
        }
    }

    /// Every weight added up.
    pub fn total(&self) -> u32 {
        Rarity::ALL.iter().map(|r| u32::from(self.get(*r))).sum()
    }

    /// The highest rarity that weighs more than 0.
    pub fn highest(&self) -> Option<Rarity> {
        Rarity::ALL.iter().rev().copied().find(|r| self.get(*r) > 0)
    }
}

/// How much experience each level needs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Levels {
    /// The experience each next level needs, the first from level 1 to 2; the last entry
    /// repeats for every level after it.
    pub xp: Vec<u32>,
}

impl Stage {
    /// The most elites alive at once.
    pub fn elite_cap(&self) -> u16 {
        self.elite_cap
            .unwrap_or(MAX_ELITES_ALIVE)
            .min(MAX_ELITES_ALIVE)
    }

    /// The map the run plays on (core#87).
    pub fn map(&self) -> Map {
        self.map.unwrap_or(Map::PORTRAIT)
    }
}

/// The rectangle the run plays on, from its top-left corner, and where the main character
/// starts. The main character and companions are clamped to it; enemies and bullets leave once
/// they are more than `FIELD_MARGIN` outside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Map {
    pub width: Fx,
    pub height: Fx,
    /// The centre of the map when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Point>,
}

impl Map {
    /// The portrait field of the vertical shooter: 360 by 640 pixels, the start 64 pixels above
    /// the middle of the bottom edge.
    pub const PORTRAIT: Map = Map {
        width: Fx(FIELD_W),
        height: Fx(FIELD_H),
        start: Some(Point {
            x: Fx(FIELD_W / 2),
            y: Fx(FIELD_H - Fx::px(64).0),
        }),
    };

    /// Where the main character starts.
    pub fn start(&self) -> Point {
        self.start.unwrap_or(Point {
            x: Fx(self.width.0 / 2),
            y: Fx(self.height.0 / 2),
        })
    }

    /// `p` moved onto the map.
    pub fn clamp(&self, p: Point) -> Point {
        Point {
            x: p.x.clamp(Fx(0), self.width),
            y: p.y.clamp(Fx(0), self.height),
        }
    }

    /// Whether a point is on the map or at most `FIELD_MARGIN` outside it. It runs for every
    /// bullet every tick, so each axis is one unsigned comparison, as the compiler made it when
    /// the bounds were constants: `v + FIELD_MARGIN` wraps below 0 to above any span, and the
    /// validator keeps the spans far below `i32::MAX`.
    pub fn holds(&self) -> impl Fn(Point) -> bool + use<> {
        let span = |side: Fx| (side.0 + 2 * FIELD_MARGIN) as u32;
        let (w, h) = (span(self.width), span(self.height));
        let inside = |v: Fx, span: u32| (v.0.wrapping_add(FIELD_MARGIN) as u32) <= span;
        move |p| inside(p.x, w) && inside(p.y, h)
    }
}

/// The main character, as the creator configured it; challengers get exactly this (ADR-012).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Player {
    pub base: ContentId,
    pub hp: u32,
    pub atk: u32,
    /// Reduces every hit the main character takes (ADR-056); 0 when absent. It moves to the
    /// run's setup when the setup exists (ADR-049).
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub defence: u32,
    /// The Main weapon, whose combos the main character makes by hand (ADR-070). It moves to
    /// the run's setup when the setup exists (ADR-049, ADR-051).
    pub main: WeaponRef<MainWeapon>,
    /// The Sub weapon, whose bonus joins every combo step reached by `sub` (ADR-070).
    pub sub: WeaponRef<SubWeapon>,
    /// Skill 1 (ADR-050). It moves to the run's setup with the character when the setup exists
    /// (ADR-049).
    pub evade: Evade,
}

/// A dash with invulnerability (ADR-050), on charges (ADR-074). A press with a charge ready
/// dashes `distance` over `ticks`, along the movement direction, or along the facing when the
/// stick is centred; the stick is ignored until the dash ends, and a press during it with a
/// charge ready starts the next dash at once. Invulnerability counts from the press, and so
/// does the cooldown of the charge it spent: each spent charge recovers on its own timer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evade {
    pub distance: Fx,
    pub ticks: u32,
    pub invulnerable_ticks: u32,
    pub cooldown_ticks: u32,
    /// The charges a run starts with: 1 when absent.
    #[serde(default = "one_charge", skip_serializing_if = "is_one_charge")]
    pub charges: u8,
    /// The most charges rewards can raise it to (ADR-074): `charges` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_charges: Option<u8>,
}

impl Evade {
    /// The most charges it can hold: `max_charges`, or `charges` when absent.
    pub fn most_charges(&self) -> u8 {
        self.max_charges.unwrap_or(self.charges)
    }
}

fn one_charge() -> u8 {
    1
}

fn is_one_charge(n: &u8) -> bool {
    *n == 1
}

/// `count` copies of `enemy`, the first at `at_tick`, then one every `every_ticks`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Wave {
    pub at_tick: u32,
    pub enemy: Agent,
    pub spawn: Point,
    /// A distance, `1` to `MAX_LEN`: each enemy of the wave spawns that far from the main
    /// character, at an angle the run's generator draws, kept on the map; `spawn` is then not
    /// used. Absent, every enemy spawns at `spawn`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub around: Option<Fx>,
    pub count: u16,
    #[serde(default)]
    pub every_ticks: u32,
    /// The experience each enemy of the wave gives when defeated, in a stage with `levels`;
    /// 1 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xp: Option<u32>,
    /// The expertise points each enemy of the wave gives (ADR-078 §2); none when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expertise: Option<ExpertisePoints>,
}

/// What a hit and a kill by an A or a B step of a combo add to the one expertise gauge
/// (ADR-078 §2): elites, bosses and the Star-Hunter give far more. Other damage gives none.
/// A hit on a boss part gives the boss's points per hit; its break is not a kill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpertisePoints {
    /// Points per hit, at most `MAX_EXPERTISE_PER_HIT`.
    pub hit: u32,
    /// Points for the kill, at most `MAX_EXPERTISE_PER_KILL`.
    pub kill: u32,
}

/// Tier 2 (ADR-036): body parts and an ordered list of phases that only move forward.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Boss {
    pub base: ContentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<ContentId>,
    pub hp: u32,
    pub radius: Fx,
    /// Reduces every hit it takes to `K / (K + defence)` of it (ADR-056); 0 when absent.
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub defence: u32,
    /// What it is weak and strong against (ADR-100); without one, every type is neutral against
    /// it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
    pub spawn: Point,
    #[serde(default)]
    pub parts: Vec<BossPart>,
    pub phases: Vec<Phase>,
    /// When every elite alive joins the boss fight (ADR-068); without it the boss calls none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_elites: Option<CallElites>,
    /// The expertise points the boss gives (ADR-078 §2); none when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expertise: Option<ExpertisePoints>,
}

/// The boss calls its elites at whichever comes first: `after_ticks` ticks after its first
/// tick (0 calls on it), or its hit points falling below `below_pct` percent of its maximum.
/// At least one of the two is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallElites {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_ticks: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub below_pct: Option<u8>,
}

/// A breakable part with its own HP and hurt circle (ADR-007, ADR-036).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BossPart {
    /// Unique within the boss; `part_broken` and the `part` selector use it.
    pub id: ContentId,
    /// Asset ID of the part's sprite.
    pub asset: ContentId,
    pub hp: u32,
    /// Offset from the boss's centre.
    pub offset: Point,
    pub radius: Fx,
    /// Reduces every hit it takes to `K / (K + defence)` of it (ADR-056); 0 when absent.
    #[serde(default, skip_serializing_if = "crate::behaviour::is_zero")]
    pub defence: u32,
    /// Its own profile (ADR-100), not the boss's; without one, every type is neutral against it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Phase {
    /// Ends the phase; absent on the last phase, which lasts until the boss dies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<Condition>,
    /// Runs when the phase begins.
    #[serde(default)]
    pub transition: Vec<Transition>,
    /// The choreography, one step at a time.
    #[serde(default)]
    pub timeline: Vec<Step>,
    /// Reactions, evaluated like an agent's rules.
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transition {
    ClearBullets,
    InvulnerableTicks { ticks: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    MoveTo {
        to: Point,
        ticks: u32,
    },
    Wait {
        ticks: u32,
    },
    Telegraph {
        ticks: u32,
    },
    Attack {
        attack: AttackRef,
    },
    /// Starts the timeline again from its first step.
    Loop,
}
