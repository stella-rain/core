//! Attack program evaluation of the live rules: task steps, lists and the value expressions.

use super::super::World;
use super::exec;
use crate::attack::{Emitter, Expr};
use crate::fixed::Point;
use crate::id::EntityId;
use crate::rng::SplitMix64;
use crate::sim::state::{Aim, AttackTask, Dir, Frame, Origin, PathStep, State};
use crate::trig;
use crate::validate::limits::{MAX_REPEAT, MAX_TICKS};

/// One tick of one attack. False once it has finished.
pub(super) fn step_task(
    world: &World,
    s: &mut State,
    t: &mut AttackTask,
    budget: &mut u32,
) -> bool {
    // An attack from an agent ends when the agent is gone.
    if t.origin == Origin::Owner && s.find(t.owner).is_none() {
        return false;
    }
    if t.wait_left > 0 {
        t.wait_left -= 1;
        if t.wait_left > 0 {
            t.age = t.age.saturating_add(1);
            return true;
        }
    }
    let going = exec(world, s, t, budget);
    t.age = t.age.saturating_add(1);
    going
}

// --- Walking the program ---------------------------------------------------------------------

pub(super) fn body_of(node: &Emitter) -> Option<&[Emitter]> {
    match node {
        Emitter::Repeat { body, .. }
        | Emitter::Ring { body, .. }
        | Emitter::Spread { body, .. }
        | Emitter::Rotate { body, .. } => Some(body),
        Emitter::Wait { .. } | Emitter::Zone { .. } | Emitter::Orb { .. } => None,
    }
}

/// The list a task starts in.
pub(super) fn root_list<'a>(program: &'a [Emitter], base: &[PathStep]) -> Option<&'a [Emitter]> {
    let mut list = program;
    for step in base {
        let node = list.get(usize::from(step.node))?;
        list = match (step.then, node) {
            // The burst of an orb is carried out as an attack of its own.
            (true, Emitter::Orb { burst, .. }) => burst,
            (true, _) => return None,
            (false, node) => body_of(node)?,
        };
    }
    Some(list)
}

/// The list that the frame at `depth` is in.
pub(super) fn list_at<'a>(
    root: &'a [Emitter],
    frames: &[Frame],
    depth: usize,
) -> Option<&'a [Emitter]> {
    let mut list = root;
    for f in frames.get(..depth)? {
        list = body_of(list.get(usize::from(f.idx))?)?;
    }
    Some(list)
}

/// What the nodes above a frame add up to.
pub(super) struct Above {
    /// The round of the innermost loop (`repeat`, `ring`, `spread`) round the frame.
    pub(super) loop_index: i32,
    /// The turn of every `rotate`, `ring` and `spread` round it.
    pub(super) turn: i32,
}

pub(super) fn above(root: &[Emitter], frames: &[Frame], depth: usize) -> Option<Above> {
    let mut list = root;
    let mut out = Above {
        loop_index: 0,
        turn: 0,
    };
    for f in frames.get(..depth)? {
        let node = list.get(usize::from(f.idx))?;
        match node {
            Emitter::Repeat { .. } => out.loop_index = i32::try_from(f.loop_i).unwrap_or(i32::MAX),
            Emitter::Ring { .. } | Emitter::Spread { .. } => {
                out.loop_index = i32::try_from(f.loop_i).unwrap_or(i32::MAX);
                out.turn = out.turn.saturating_add(f.offset);
            }
            Emitter::Rotate { .. } => out.turn = out.turn.saturating_add(f.offset),
            Emitter::Wait { .. } | Emitter::Zone { .. } | Emitter::Orb { .. } => {}
        }
        list = body_of(node)?;
    }
    Some(out)
}

// --- Expressions -----------------------------------------------------------------------------

pub(super) fn saturate(v: i64) -> i32 {
    i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX })
}

/// A random number from `lo` to `hi`, both included; `lo` if the range is empty.
fn rand_between(rng: &mut SplitMix64, lo: i32, hi: i32) -> i32 {
    if hi <= lo {
        return lo;
    }
    let span = u32::try_from(i64::from(hi) - i64::from(lo) + 1).unwrap_or(u32::MAX);
    saturate(i64::from(lo) + i64::from(rng.below(span)))
}

struct Eval<'a> {
    rng: &'a mut SplitMix64,
    loop_index: i32,
    phase_time: i32,
}

fn eval(e: &Expr, c: &mut Eval) -> i32 {
    let two = |b: &(Expr, Expr), c: &mut Eval| {
        let x = i64::from(eval(&b.0, c));
        (x, i64::from(eval(&b.1, c)))
    };
    match e {
        Expr::Const(v) => *v,
        // Presets are taken apart before they run; an `arg` that is left reads as 0.
        Expr::Arg(_) => 0,
        Expr::LoopIndex => c.loop_index,
        Expr::PhaseTime => c.phase_time,
        Expr::Rand(b) => {
            let lo = eval(&b.0, c);
            let hi = eval(&b.1, c);
            rand_between(c.rng, lo, hi)
        }
        Expr::Add(b) => {
            let (x, y) = two(b, c);
            saturate(x + y)
        }
        Expr::Sub(b) => {
            let (x, y) = two(b, c);
            saturate(x - y)
        }
        Expr::Mul(b) => {
            let (x, y) = two(b, c);
            saturate(x * y)
        }
        Expr::Div(b) => {
            let (x, y) = two(b, c);
            if y == 0 { 0 } else { saturate(x / y) }
        }
        Expr::Min(b) => {
            let (x, y) = two(b, c);
            saturate(x.min(y))
        }
        Expr::Max(b) => {
            let (x, y) = two(b, c);
            saturate(x.max(y))
        }
        Expr::Clamp(b) => {
            let v = eval(&b.0, c);
            let lo = eval(&b.1, c);
            let hi = eval(&b.2, c);
            v.max(lo).min(hi)
        }
    }
}

/// Evaluates `e` for a node whose frame is `top`.
pub(super) fn value(s: &mut State, t: &AttackTask, ab: &Above, e: &Expr) -> i32 {
    let mut c = Eval {
        rng: &mut s.rng.attack,
        loop_index: ab.loop_index,
        phase_time: i32::try_from(t.age).unwrap_or(i32::MAX),
    };
    eval(e, &mut c)
}

pub(super) fn ticks(v: i32) -> u32 {
    u32::try_from(v.clamp(0, i32::try_from(MAX_TICKS).unwrap_or(i32::MAX))).unwrap_or(0)
}

pub(super) fn rounds(v: i32) -> u32 {
    u32::try_from(v.clamp(0, MAX_REPEAT)).unwrap_or(0)
}

// --- Directions ------------------------------------------------------------------------------

/// The unit direction of the vector `(x, y)`; straight down if it has no length.
pub(in crate::sim) fn normalize(x: i64, y: i64) -> Dir {
    let len = i64::try_from((x * x + y * y).unsigned_abs().isqrt()).unwrap_or(i64::MAX);
    if len == 0 {
        return Dir::DOWN;
    }
    let scale = i64::from(trig::SCALE);
    Dir {
        x: saturate(x * scale / len),
        y: saturate(y * scale / len),
    }
}

/// `d` turned by `angle` (1/256 degree), toward the right for positive angles.
pub(in crate::sim) fn rotate(d: Dir, angle: i32) -> Dir {
    let (sin, cos) = trig::sin_cos(angle);
    let (x, y) = (i64::from(d.x), i64::from(d.y));
    let (sin, cos) = (i64::from(sin), i64::from(cos));
    let scale = i64::from(trig::SCALE);
    normalize((x * cos + y * sin) / scale, (-x * sin + y * cos) / scale)
}

/// The direction from `from` to the entity `target`, if it is there.
fn toward(s: &State, from: Point, target: EntityId) -> Option<Dir> {
    let to = s.at(s.find(target)?);
    Some(normalize(
        i64::from(to.x.0) - i64::from(from.x.0),
        i64::from(to.y.0) - i64::from(from.y.0),
    ))
}

/// The direction an `aimed` orb takes from `from`: at the entity (straight down if it is
/// gone), or along the fixed direction.
pub(super) fn aim(s: &State, from: Point, aim: Aim) -> Dir {
    match aim {
        Aim::Entity(target) => toward(s, from, target).unwrap_or(Dir::DOWN),
        Aim::Fixed(d) => d,
    }
}
