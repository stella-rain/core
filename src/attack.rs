//! Tier 3 of the behaviour model (ADR-036): an attack is a tree of emitter nodes whose
//! parameters are small fixed-point expressions. Presets are registered content IDs; a stage
//! refers to one with arguments, or holds an inline copy it may edit freely.
//!
//! Angles are in 1/256 degree; speeds in 1/256 pixel per tick (the units of `Fx`).
//! Node and operator names are placeholders until the freeze (ADR-035).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    /// The nodes, carried out in order (see `rules_v0`'s attacks for what each one does).
    Inline(Vec<Emitter>),
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
