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
use crate::weapon::{Weapon, WeaponRef};

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
    /// Healing: what a boss kill heals and the heal pickups a regular kill may drop. Without it
    /// a stage heals only by a reward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heal: Option<Healing>,
    /// The most elites alive at once (ADR-068); `MAX_ELITES_ALIVE` when absent. An elite that
    /// would spawn past it does not, and a wave counts it as spawned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elite_cap: Option<u16>,
    /// The levels of the expertise style path (ADR-106 §4); `LevelTable::NORMAL` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_table: Option<LevelTable>,
    /// How a kill's Stardust grows over the run (ADR-109): without it the run counts none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stardust: Option<StardustFactor>,
}

/// The Stardust time factor (ADR-109 decision 2): a table of time tiers, each the tick it starts
/// at and a percent. A kill's base Stardust is multiplied by the percent of the last tier that
/// started at or before the kill tick, truncated toward zero; past the last tier's start
/// (overtime included) the last tier holds. A mode's run takes the mode file's table, or its
/// difficulty's when the difficulty gives one (ADR-110 decision 2). `stardust_tiers_problem`
/// says what a valid table is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StardustFactor {
    /// 1 to `limits::MAX_STARDUST_TIERS` tiers, the first at tick 0, the ticks rising strictly.
    pub tiers: Vec<StardustTier>,
    /// The base Stardust a kill of a summoned enemy earns before the factor, times 10 (a boss's
    /// or a splitter's `summon`), at most `limits::MAX_STARDUST_BASE`; none when absent.
    /// Guardians, escorts and a boss's parts earn nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summon_base: Option<u32>,
}

/// One tier of the Stardust time factor: from `start_tick` until the next tier starts, a kill
/// earns `pct` percent of its base.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StardustTier {
    /// When the tier starts, in ticks from the start of the run.
    pub start_tick: u32,
    /// From 1 to `limits::MAX_STARDUST_PCT`.
    pub pct: u32,
}

/// Why a Stardust tier table is not valid, if it is not: 1 to `limits::MAX_STARDUST_TIERS`
/// tiers, the first at tick 0, the ticks rising strictly, each percent from 1 to
/// `limits::MAX_STARDUST_PCT` (ADR-020).
pub fn stardust_tiers_problem(tiers: &[StardustTier]) -> Option<String> {
    use crate::validate::limits::{MAX_STARDUST_PCT, MAX_STARDUST_TIERS};
    if tiers.is_empty() || tiers.len() > MAX_STARDUST_TIERS {
        Some(format!(
            "1 to {MAX_STARDUST_TIERS} tiers (found {})",
            tiers.len()
        ))
    } else if tiers[0].start_tick != 0 {
        Some("the first tier starts at tick 0".into())
    } else if tiers.windows(2).any(|w| w[0].start_tick >= w[1].start_tick) {
        Some("each tier starts later than the one before it".into())
    } else if tiers
        .iter()
        .any(|t| !(1..=MAX_STARDUST_PCT).contains(&t.pct))
    {
        Some(format!("every percent from 1 to {MAX_STARDUST_PCT}"))
    } else {
        None
    }
}

/// The highest level a level table may name: the last level whose threshold the expertise file
/// lists (ADR-106 §3, ADR-020).
pub const MAX_STYLE_LEVEL: u32 = 10;

/// The expertise levels of a run's style path (ADR-106 §4): the level of the style choice, of
/// each of the two style upgrades and of the capstone. Every other level is a common offer.
/// The levels rise in that order, strictly, each from 1 to `MAX_STYLE_LEVEL`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelTable {
    pub style: u32,
    pub upgrades: [u32; 2],
    pub capstone: u32,
}

impl LevelTable {
    /// The normal run's table (placeholder numbers).
    pub const NORMAL: LevelTable = LevelTable {
        style: 3,
        upgrades: [6, 9],
        capstone: 10,
    };
    /// The short run's table (placeholder numbers).
    pub const SHORT: LevelTable = LevelTable {
        style: 2,
        upgrades: [4, 6],
        capstone: 8,
    };

    /// The levels in path order: the style, the two upgrades, the capstone.
    pub fn levels(&self) -> [u32; 4] {
        [
            self.style,
            self.upgrades[0],
            self.upgrades[1],
            self.capstone,
        ]
    }

    /// Why the table is not valid, if it is not: the style comes first, the levels rise
    /// strictly, and all are within bounds.
    pub fn problem(&self) -> Option<String> {
        let levels = self.levels();
        if levels.iter().any(|l| !(1..=MAX_STYLE_LEVEL).contains(l)) {
            Some(format!("every level from 1 to {MAX_STYLE_LEVEL}"))
        } else if levels.windows(2).any(|w| w[0] >= w[1]) {
            Some("style, upgrades and capstone rise strictly in that order".into())
        } else {
            None
        }
    }
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

/// Healing (see `Stage::heal`). Hit points are in the stage's own scale (ADR-103).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Healing {
    /// The percent of the main character's most hit points a boss kill heals, from 0 to 100.
    pub boss_pct: u16,
    /// A regular enemy's kill drops a heal pickup with a chance of 1 in this many, from 1 up.
    pub drop_one_in: u32,
    /// The hit points a pickup heals when the main character takes it, from 1 up.
    pub amount: u32,
    /// The most pickups on the map at once, from 1 up. A kill with the map full drops none.
    pub cap: u16,
    /// How near the main character's centre a pickup must come to be taken.
    pub pickup_radius: Fx,
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
    /// The levels of the expertise style path.
    pub fn level_table(&self) -> LevelTable {
        self.level_table.unwrap_or(LevelTable::NORMAL)
    }

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
    /// The angle the main character faces at the start, as `Direction::Absolute` takes it: 0 is
    /// straight down the screen, a half turn is up. Absent means up, as it always was
    /// (core#147). Between 0 and a full turn, not including it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facing: Option<i32>,
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
        facing: None,
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
    /// The one weapon, whose combos the main character makes by hand with both buttons
    /// (ADR-084). It moves to the run's setup when the setup exists (ADR-049, ADR-051, ADR-107).
    pub weapon: WeaponRef<Weapon>,
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
    /// The wave's role in a `time_limit` schedule (R2): lowercase snake_case, unique among the
    /// schedule's waves. A stage's waves do not use it; absent, the wave has no ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<ContentId>,
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
    /// The Stardust a kill of each enemy of the wave earns before the time factor, times 10
    /// (ADR-109), at most `limits::MAX_STARDUST_BASE`; none when absent. Counts only in a stage
    /// with a `stardust` factor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stardust: Option<u32>,
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
    /// The Stardust its kill earns before the time factor, times 10; as a wave's `stardust`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stardust: Option<u32>,
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
        /// A big pattern (ADR-103): announced in the snapshot, with the ticks left until it
        /// starts, while the timed steps before it run. Only a preset can be big; its ID names
        /// it.
        #[serde(default, skip_serializing_if = "crate::behaviour::is_false")]
        big: bool,
    },
    /// Starts the timeline again from its first step.
    Loop,
}
