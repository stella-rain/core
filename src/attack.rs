//! Attacks (ADR-055, amending tier 3 of ADR-036): an attack is an ordered list of phases, each
//! lasting a fixed number of ticks, with its own hit box, base damage and the emitter nodes it
//! spawns. Emitter nodes are trees whose parameters are small fixed-point expressions. Presets
//! are registered content IDs; a stage refers to one with arguments, or holds an inline copy it
//! may edit freely.
//!
//! Angles are in 1/256 degree; speeds and lengths in 1/256 pixel (per tick for speeds; the
//! units of `Fx`). Field, node and operator names are placeholders until the freeze (ADR-035).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::fixed::Fx;
use crate::id::ContentId;

/// A preset by ID with its arguments, or an inline definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AttackRef {
    Preset {
        id: ContentId,
        #[serde(default)]
        args: Vec<i32>,
    },
    /// The phases, in order.
    Inline(Vec<AttackPhase>),
}

/// One phase of an attack (ADR-055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttackPhase {
    pub kind: PhaseKind,
    /// How long the phase lasts: a constant, never an expression, so that an attack's length
    /// and its telegraph are known before it runs.
    pub ticks: u32,
    /// The area the phase covers, attached to the attacker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_box: Option<HitBox>,
    /// Base damage of the phase's hits and of the bullets it spawns (ADR-056). Absent, it is
    /// what the attacker's own attack does: the main character's `atk`, a `shot` skill's
    /// `damage`, or 100 (1 at the design values, ADR-103) for an enemy or the boss.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage: Option<u32>,
    /// The damage type of the phase's hits and of the bullets it spawns (ADR-100); absent, they
    /// are neutral and never get a match-up.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub damage_type: Option<DamageType>,
    /// An active phase only: a target already hit may be hit again after this many ticks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rehit_ticks: Option<u32>,
    /// Emitter nodes started on the phase's first tick, carried out in order (see `rules_v0`'s
    /// attacks for what each one does).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spawn: Vec<Emitter>,
    /// The attacker's movement speed during the phase, in percent; 0 roots it.
    #[serde(default = "full_speed", skip_serializing_if = "is_full_speed")]
    pub move_pct: u8,
    /// A powerful attack: its hits on the main character, by hit box or by the bullets the phase
    /// spawns, break a charge in progress (core#83).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub breaks_charge: bool,
}

fn full_speed() -> u8 {
    100
}

fn is_full_speed(v: &u8) -> bool {
    *v == 100
}

/// The damage types, a closed list (ADR-100): three attack types, which are physical, and the
/// elements. A list for the prototype; changing it is a schema change in version 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DamageType {
    Slash,
    Pierce,
    Strike,
    Pyro,
    Hydro,
    Electro,
    Cryo,
    Anemo,
    Geo,
    Dendro,
}

impl DamageType {
    /// Every type, attack types first.
    pub const ALL: [DamageType; 10] = [
        DamageType::Slash,
        DamageType::Pierce,
        DamageType::Strike,
        DamageType::Pyro,
        DamageType::Hydro,
        DamageType::Electro,
        DamageType::Cryo,
        DamageType::Anemo,
        DamageType::Geo,
        DamageType::Dendro,
    ];

    /// The type's place in `ALL`, from 0.
    pub fn index(self) -> usize {
        self as usize
    }

    /// An attack type (Slash, Pierce, Strike) rather than an element.
    pub fn is_physical(self) -> bool {
        matches!(
            self,
            DamageType::Slash | DamageType::Pierce | DamageType::Strike
        )
    }
}

/// A blanket resist: every attack type, or every element (ADR-100).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Blanket {
    Physical,
    Elements,
}

impl Blanket {
    /// Whether the blanket covers `ty`.
    pub fn covers(self, ty: DamageType) -> bool {
        match self {
            Blanket::Physical => ty.is_physical(),
            Blanket::Elements => !ty.is_physical(),
        }
    }
}

/// What an enemy, an elite, a boss or a boss part is weak and strong against (ADR-100). The
/// validator holds each type at most once in each list, and never in both.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weak: Vec<DamageType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strong: Vec<DamageType>,
    /// At most one blanket resist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resist: Option<Blanket>,
}

impl Profile {
    /// The one match-up a hit of `ty` gets: a listed weakness, else a listed strength, else the
    /// blanket, else none. A neutral hit gets none.
    pub fn match_up(&self, ty: Option<DamageType>) -> MatchUp {
        let Some(ty) = ty else {
            return MatchUp::None;
        };
        if self.weak.contains(&ty) {
            MatchUp::Weak
        } else if self.strong.contains(&ty) {
            MatchUp::Strong
        } else if self.resist.is_some_and(|b| b.covers(ty)) {
            MatchUp::Resisted
        } else {
            MatchUp::None
        }
    }
}

/// The match-up a hit got (ADR-100), which the `Hit` event reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MatchUp {
    Weak,
    Strong,
    Resisted,
    #[default]
    None,
}

impl MatchUp {
    pub fn is_none(&self) -> bool {
        *self == MatchUp::None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PhaseKind {
    /// The telegraph: its hit box is the warned area, and it deals no damage.
    WindUp,
    /// Its hit box deals damage.
    Active,
    Recovery,
}

/// A hit box, in the attacker's own frame: `offset` is forward along the attacker's facing
/// and to its right.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HitBox {
    Circle {
        #[serde(default)]
        offset: Offset,
        radius: Fx,
    },
    /// A circle sector centred on the facing, `half_angle` to each side.
    Arc {
        #[serde(default)]
        offset: Offset,
        radius: Fx,
        half_angle: i32,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Offset {
    #[serde(default)]
    pub forward: Fx,
    #[serde(default)]
    pub right: Fx,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Emitter {
    /// One bullet.
    Fire {
        speed: Expr,
        direction: Direction,
        /// Bullet asset; absent means the default bullet.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bullet: Option<ContentId>,
        /// How many ticks the bullet lives, at least 1 and at most `limits::MAX_BULLET_TICKS`;
        /// absent means it lives until it is past the map's margin (core#147).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ticks: Option<Expr>,
    },
    /// Runs `body` `count` times (at most 100). The next round starts `interval_ticks` ticks
    /// after the body of the last one ends.
    Repeat {
        count: Expr,
        interval_ticks: Expr,
        body: Vec<Emitter>,
    },
    /// Runs `body` `count` times at once, each round turned a full turn / `count` further than
    /// the one before.
    Ring {
        count: Expr,
        body: Vec<Emitter>,
    },
    /// Runs `body` `count` times at once, fanned evenly across `angle` and centred on the
    /// direction of the bullets.
    Spread {
        count: Expr,
        angle: Expr,
        body: Vec<Emitter>,
    },
    /// Turns the direction of everything in `body` by `angle`.
    Rotate {
        angle: Expr,
        body: Vec<Emitter>,
    },
    Wait {
        ticks: Expr,
    },
    /// Each bullet fired by `body` starts `then` as an attack of its own, from where the bullet
    /// is, `after_ticks` ticks later (at the next tick if 0), if it is still flying.
    OnBullet {
        after_ticks: Expr,
        body: Vec<Emitter>,
        then: Vec<Emitter>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Direction {
    /// At the target, turned by `offset`.
    Aimed { offset: Expr },
    /// A fixed angle: 0 points down the screen, and positive angles turn toward the right.
    Absolute { angle: Expr },
    /// Turned by `angle` from the parent's direction: straight down for an attack that a rule
    /// started, the direction of the bullet for a `then` list of `on_bullet`.
    Relative { angle: Expr },
    /// Turned by `step` from the previous bullet's direction; not affected by the turns of
    /// `rotate`, `ring` and `spread`.
    Sequential { step: Expr },
}

/// A fixed-point expression (ADR-036): a literal, an input, or an operator. No variables,
/// loops or recursion beyond the tree itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Expr {
    /// The attack's argument at this index.
    Arg(u8),
    /// Index of the innermost `repeat`, `ring` or `spread`.
    LoopIndex,
    /// Ticks since the attack started: 0 on its first tick.
    PhaseTime,
    /// A draw from the run's SplitMix64 (ADR-033), from `lo` to `hi` inclusive; `lo` if the
    /// range is empty. Expressions are evaluated left to right, so the order of draws is fixed.
    Rand(Box<(Expr, Expr)>),
    Add(Box<(Expr, Expr)>),
    Sub(Box<(Expr, Expr)>),
    Mul(Box<(Expr, Expr)>),
    Div(Box<(Expr, Expr)>),
    Min(Box<(Expr, Expr)>),
    Max(Box<(Expr, Expr)>),
    /// `[value, lo, hi]`.
    Clamp(Box<(Expr, Expr, Expr)>),
    /// A literal; written as a plain integer.
    #[serde(untagged)]
    Const(i32),
}
