//! Attack presets (ADR-036 tier 3): the developer's attacks, written in the same format as the
//! attacks creators write inline, and registered as content IDs (ADR-026). A stage uses one by
//! ID with arguments, or holds an inline copy. **Taking a preset apart** copies its definition
//! inline with each argument written in as a constant; the stage then edits it freely and no
//! longer depends on the ID. The engine runs a preset by that same expansion, so a preset and
//! its inline copy run the same program and reach the same state hashes.
//!
//! Under version 0 (ADR-035) these definitions are placeholders and change in place.

use std::sync::{Arc, OnceLock};

use crate::attack::{AttackRef, Direction, Emitter, Expr};
use crate::registry::{self, Kind};

/// `(id, definition)`: the definitions are JSON, the stage format's own, with `{"arg": n}` for
/// the preset's arguments. Angles are in 1/256 degree and speeds in 1/256 pixel a tick.
const DEFINITIONS: &[(&str, &str)] = &[
    // One bullet at the target; argument 0 is its speed.
    (
        "aimed_single",
        r#"[{"type":"fire","speed":{"arg":0},"direction":{"type":"aimed","offset":0}}]"#,
    ),
    // Seven bullets in a 30 degree fan at the target; argument 0 is their speed.
    (
        "aimed_spread",
        r#"[{"type":"spread","count":7,"angle":7680,"body":[
            {"type":"fire","speed":{"arg":0},"direction":{"type":"aimed","offset":0}}]}]"#,
    ),
    // Forty bullets, one every 3 ticks, each turned 6 degrees from the one before; argument 0
    // is their speed.
    (
        "spiral",
        r#"[{"type":"repeat","count":40,"interval_ticks":3,"body":[
            {"type":"fire","speed":{"arg":0},"direction":{"type":"sequential","step":1536}}]}]"#,
    ),
    // Five bullets in a 20 degree fan at the target.
    (
        "spread_5",
        r#"[{"type":"spread","count":5,"angle":5120,"body":[
            {"type":"fire","speed":1536,"direction":{"type":"aimed","offset":0}}]}]"#,
    ),
    // Two bullets side by side, 5 degrees apart, at the target.
    (
        "twin_shot",
        r#"[{"type":"spread","count":2,"angle":1280,"body":[
            {"type":"fire","speed":2048,"direction":{"type":"aimed","offset":0}}]}]"#,
    ),
];

fn parsed() -> &'static [(&'static str, Vec<Emitter>)] {
    static PARSED: OnceLock<Vec<(&'static str, Vec<Emitter>)>> = OnceLock::new();
    PARSED.get_or_init(|| {
        DEFINITIONS
            .iter()
            .map(|(id, json)| {
                let nodes = serde_json::from_str(json)
                    .unwrap_or_else(|e| panic!("the built-in preset {id} does not parse: {e}"));
                (*id, nodes)
            })
            .collect()
    })
}

/// The ID of every preset that has a definition.
pub fn ids() -> impl Iterator<Item = &'static str> {
    DEFINITIONS.iter().map(|(id, _)| *id)
}

/// A preset's definition, with its arguments still as `arg` nodes.
pub fn definition(id: &str) -> Option<&'static [Emitter]> {
    parsed()
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, n)| n.as_slice())
}

/// The preset `id` taken apart: its definition with each `arg` replaced by the constant in
/// `args`. `None` if there is no such preset or the number of arguments is wrong.
pub fn take_apart(id: &str, args: &[i32]) -> Option<Vec<Emitter>> {
    let arity = registry::find(registry::ENTRIES, Kind::Preset, id)?.args;
    if usize::from(arity) != args.len() {
        return None;
    }
    Some(emitters(definition(id)?, args))
}

/// The program an attack runs: a preset taken apart, or the inline nodes themselves.
pub fn program(attack: &AttackRef) -> Option<Arc<Vec<Emitter>>> {
    match attack {
        AttackRef::Preset { id, args } => take_apart(&id.0, args).map(Arc::new),
        AttackRef::Inline(nodes) => Some(Arc::new(nodes.clone())),
    }
}

/// The highest argument number a list of nodes reads, if it reads any.
pub fn highest_arg(nodes: &[Emitter]) -> Option<u8> {
    fn expr(e: &Expr) -> Option<u8> {
        match e {
            Expr::Arg(i) => Some(*i),
            Expr::LoopIndex | Expr::PhaseTime | Expr::Const(_) => None,
            Expr::Rand(b)
            | Expr::Add(b)
            | Expr::Sub(b)
            | Expr::Mul(b)
            | Expr::Div(b)
            | Expr::Min(b)
            | Expr::Max(b) => expr(&b.0).max(expr(&b.1)),
            Expr::Clamp(b) => expr(&b.0).max(expr(&b.1)).max(expr(&b.2)),
        }
    }
    fn direction(d: &Direction) -> Option<u8> {
        match d {
            Direction::Aimed { offset } => expr(offset),
            Direction::Absolute { angle } | Direction::Relative { angle } => expr(angle),
            Direction::Sequential { step } => expr(step),
        }
    }
    nodes
        .iter()
        .map(|n| match n {
            Emitter::Fire {
                speed,
                direction: d,
                ..
            } => expr(speed).max(direction(d)),
            Emitter::Repeat {
                count,
                interval_ticks,
                body,
            } => expr(count).max(expr(interval_ticks)).max(highest_arg(body)),
            Emitter::Ring { count, body } => expr(count).max(highest_arg(body)),
            Emitter::Spread { count, angle, body } => {
                expr(count).max(expr(angle)).max(highest_arg(body))
            }
            Emitter::Rotate { angle, body } => expr(angle).max(highest_arg(body)),
            Emitter::Wait { ticks } => expr(ticks),
            Emitter::OnBullet {
                after_ticks,
                body,
                then,
            } => expr(after_ticks)
                .max(highest_arg(body))
                .max(highest_arg(then)),
        })
        .max()
        .flatten()
}

fn pair(b: &(Expr, Expr), args: &[i32]) -> Box<(Expr, Expr)> {
    Box::new((expr(&b.0, args), expr(&b.1, args)))
}

fn expr(e: &Expr, args: &[i32]) -> Expr {
    match e {
        Expr::Arg(i) => Expr::Const(args.get(usize::from(*i)).copied().unwrap_or(0)),
        Expr::LoopIndex | Expr::PhaseTime | Expr::Const(_) => e.clone(),
        Expr::Rand(b) => Expr::Rand(pair(b, args)),
        Expr::Add(b) => Expr::Add(pair(b, args)),
        Expr::Sub(b) => Expr::Sub(pair(b, args)),
        Expr::Mul(b) => Expr::Mul(pair(b, args)),
        Expr::Div(b) => Expr::Div(pair(b, args)),
        Expr::Min(b) => Expr::Min(pair(b, args)),
        Expr::Max(b) => Expr::Max(pair(b, args)),
        Expr::Clamp(b) => Expr::Clamp(Box::new((
            expr(&b.0, args),
            expr(&b.1, args),
            expr(&b.2, args),
        ))),
    }
}

fn direction(d: &Direction, args: &[i32]) -> Direction {
    match d {
        Direction::Aimed { offset } => Direction::Aimed {
            offset: expr(offset, args),
        },
        Direction::Absolute { angle } => Direction::Absolute {
            angle: expr(angle, args),
        },
        Direction::Relative { angle } => Direction::Relative {
            angle: expr(angle, args),
        },
        Direction::Sequential { step } => Direction::Sequential {
            step: expr(step, args),
        },
    }
}

fn emitters(nodes: &[Emitter], args: &[i32]) -> Vec<Emitter> {
    nodes
        .iter()
        .map(|n| match n {
            Emitter::Fire {
                speed,
                direction: d,
                bullet,
            } => Emitter::Fire {
                speed: expr(speed, args),
                direction: direction(d, args),
                bullet: bullet.clone(),
            },
            Emitter::Repeat {
                count,
                interval_ticks,
                body,
            } => Emitter::Repeat {
                count: expr(count, args),
                interval_ticks: expr(interval_ticks, args),
                body: emitters(body, args),
            },
            Emitter::Ring { count, body } => Emitter::Ring {
                count: expr(count, args),
                body: emitters(body, args),
            },
            Emitter::Spread { count, angle, body } => Emitter::Spread {
                count: expr(count, args),
                angle: expr(angle, args),
                body: emitters(body, args),
            },
            Emitter::Rotate { angle, body } => Emitter::Rotate {
                angle: expr(angle, args),
                body: emitters(body, args),
            },
            Emitter::Wait { ticks } => Emitter::Wait {
                ticks: expr(ticks, args),
            },
            Emitter::OnBullet {
                after_ticks,
                body,
                then,
            } => Emitter::OnBullet {
                after_ticks: expr(after_ticks, args),
                body: emitters(body, args),
                then: emitters(then, args),
            },
        })
        .collect()
}
