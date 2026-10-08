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
    /// The root nodes, run together.
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
    /// Runs `body` `count` times, `interval_ticks` apart.
    Repeat {
        count: Expr,
        interval_ticks: Expr,
        body: Vec<Emitter>,
    },
    /// Runs `body` `count` times, spread evenly round a full circle.
    Ring {
        count: Expr,
        body: Vec<Emitter>,
    },
    /// Runs `body` `count` times, spread evenly across `angle`.
    Spread {
        count: Expr,
        angle: Expr,
        body: Vec<Emitter>,
    },
    /// Turns the direction of everything in `body`.
    Rotate {
        angle: Expr,
        body: Vec<Emitter>,
    },
    Wait {
        ticks: Expr,
    },
    /// Each bullet fired by `body` starts `then` after `after_ticks`.
    OnBullet {
        after_ticks: Expr,
        body: Vec<Emitter>,
        then: Vec<Emitter>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Direction {
    /// At the target, plus an offset.
    Aimed { offset: Expr },
    /// A fixed angle; 0 points down the screen.
    Absolute { angle: Expr },
    /// Relative to the parent's direction.
    Relative { angle: Expr },
    /// The previous bullet's direction plus a step.
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
    /// Ticks since the boss phase, or the agent, began.
    PhaseTime,
    /// A draw from the run's SplitMix64 (ADR-033), from `lo` to `hi` inclusive.
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
