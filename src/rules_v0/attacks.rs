//! Attacks (ADR-036 tier 3): a tree of emitter nodes carried out over time. A task keeps a stack
//! of frames, one for each list of nodes it is inside, so that it can stop at a `wait` or between
//! the rounds of a `repeat` and go on in a later tick. Presets are expanded to inline nodes
//! before a task starts (see `presets`), so a preset and its inline copy are the same task.
//!
//! What the nodes mean:
//! - `fire` makes one bullet at the attack's origin. Its direction is a unit vector: `aimed`
//!   points at the target and turns by `offset`, `absolute` is an angle from straight down,
//!   `relative` turns the direction the attack's parent was going (straight down for an attack
//!   a rule started, the bullet's own direction for an `on_bullet` child), and `sequential`
//!   turns the last bullet's direction by `step`. The first three also turn by every `rotate`,
//!   `ring` and `spread` round them. Positive angles turn from straight down toward the right.
//! - `repeat` runs its body `count` times; the next round starts `interval_ticks` ticks after
//!   the body of the last one ends. `ring` runs its body `count` times at once, round after
//!   round turned a full turn / `count` further; `spread` the same, fanned evenly across
//!   `angle` and centred on the direction. A `wait` in their bodies delays the next round.
//! - `rotate` turns everything in its body by `angle`; `wait` pauses the attack.
//! - `on_bullet` runs `body`, and every bullet the body fires starts the `then` list as an
//!   attack of its own, from where the bullet is, `after_ticks` ticks later (if the bullet is
//!   still flying). A bullet carries one such hook, the innermost.
//!
//! Expressions are evaluated left to right when the node they belong to is carried out, with
//! `rand` drawing from the run's SplitMix64 in that order. Arithmetic saturates, and dividing by
//! zero gives 0. Counts, ticks and speeds that an expression computes are clamped.
//!
//! Dynamic budgets (ADR-020): at most `MAX_ATTACK_TASKS` attacks at once, and every tick at most
//! `MAX_EMITTER_STEPS_PER_TICK` node steps across all of them, after which the rest wait for
//! the next tick, in order. Bullets are capped as everywhere else.

use std::sync::Arc;

use crate::attack::{AttackRef, Direction, Emitter, Expr};
use crate::engine::{
    AttackTask, BulletState, Dir, Frame, Hook, Origin, PathStep, State, StatusKind,
};
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::presets;
use crate::registry;
use crate::rng::SplitMix64;
use crate::trig;
use crate::validate::limits::{MAX_LEN, MAX_REPEAT, MAX_TICKS};

use super::entity::{Who, scale_damage, status_pct};
use super::{MAX_ATTACK_TASKS, MAX_EMITTER_STEPS_PER_TICK, count, spawn_bullet};

/// Starts `attack` for `owner`, aimed at `target`. Bullets do `base_damage`, raised by the
/// owner's atk-up as it is now. Nothing starts if too many attacks are going already.
pub(super) fn start(s: &mut State, owner: Who, target: Who, attack: &AttackRef, base_damage: u32) {
    if count(&s.tasks) >= MAX_ATTACK_TASKS {
        return;
    }
    let Some(program) = presets::program(attack) else {
        return;
    };
    let damage = scale_damage(
        base_damage,
        status_pct(s.statuses(owner), StatusKind::AtkUp),
        0,
    );
    s.tasks.push(AttackTask {
        owner: s.id_of(owner),
        program,
        base: Vec::new(),
        origin: Origin::Owner,
        target: s.id_of(target),
        friendly: State::is_ally(owner),
        damage,
        age: 0,
        wait_left: 0,
        frames: vec![Frame::default()],
        parent_dir: Dir::DOWN,
        last_dir: Dir::DOWN,
    });
}

/// Bullets whose `on_bullet` wait is over start their attacks, to be carried out this tick.
pub(super) fn fire_hooks(s: &mut State) {
    let mut starts = Vec::new();
    for b in &mut s.bullets {
        let Some(hook) = b.hook.as_mut() else {
            continue;
        };
        hook.left = hook.left.saturating_sub(1);
        if hook.left == 0 {
            if let Some(hook) = b.hook.take() {
                starts.push((hook, b.at, b.dir));
            }
        }
    }
    for (hook, at, dir) in starts {
        if count(&s.tasks) >= MAX_ATTACK_TASKS {
            break;
        }
        s.tasks.push(AttackTask {
            owner: hook.owner,
            program: hook.program,
            base: hook.path,
            origin: Origin::At(at),
            target: hook.target,
            friendly: hook.friendly,
            damage: hook.damage,
            age: 0,
            wait_left: 0,
            frames: vec![Frame::default()],
            parent_dir: dir,
            last_dir: dir,
        });
    }
}

/// Carries out every attack up to its next wait, in the order they started.
pub(super) fn run(s: &mut State, spawns_left: &mut u32) {
    let mut tasks = std::mem::take(&mut s.tasks);
    let mut budget = MAX_EMITTER_STEPS_PER_TICK;
    tasks.retain_mut(|t| step_task(s, t, &mut budget, spawns_left));
    s.tasks = tasks;
}

/// One tick of one attack. False once it has finished.
fn step_task(s: &mut State, t: &mut AttackTask, budget: &mut u32, spawns_left: &mut u32) -> bool {
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
    let going = exec(s, t, budget, spawns_left);
    t.age = t.age.saturating_add(1);
    going
}

// --- Walking the program ---------------------------------------------------------------------

fn body_of(node: &Emitter) -> Option<&[Emitter]> {
    match node {
        Emitter::Repeat { body, .. }
        | Emitter::Ring { body, .. }
        | Emitter::Spread { body, .. }
        | Emitter::Rotate { body, .. }
        | Emitter::OnBullet { body, .. } => Some(body),
        Emitter::Fire { .. } | Emitter::Wait { .. } => None,
    }
}

/// The list a task starts in.
fn root_list<'a>(program: &'a [Emitter], base: &[PathStep]) -> Option<&'a [Emitter]> {
    let mut list = program;
    for step in base {
        let node = list.get(usize::from(step.node))?;
        list = match (step.then, node) {
            (true, Emitter::OnBullet { then, .. }) => then,
            (true, _) => return None,
            (false, node) => body_of(node)?,
        };
    }
    Some(list)
}

/// The list that the frame at `depth` is in.
fn list_at<'a>(root: &'a [Emitter], frames: &[Frame], depth: usize) -> Option<&'a [Emitter]> {
    let mut list = root;
    for f in frames.get(..depth)? {
        list = body_of(list.get(usize::from(f.idx))?)?;
    }
    Some(list)
}

/// What the nodes above a frame add up to.
struct Above {
    /// The round of the innermost loop (`repeat`, `ring`, `spread`) round the frame.
    loop_index: i32,
    /// The turn of every `rotate`, `ring` and `spread` round it.
    turn: i32,
    /// The innermost `on_bullet` round it: its frame, and how long its bullets wait.
    hook: Option<(usize, u32)>,
}

fn above(root: &[Emitter], frames: &[Frame], depth: usize) -> Option<Above> {
    let mut list = root;
    let mut out = Above {
        loop_index: 0,
        turn: 0,
        hook: None,
    };
    for (k, f) in frames.get(..depth)?.iter().enumerate() {
        let node = list.get(usize::from(f.idx))?;
        match node {
            Emitter::Repeat { .. } => out.loop_index = i32::try_from(f.loop_i).unwrap_or(i32::MAX),
            Emitter::Ring { .. } | Emitter::Spread { .. } => {
                out.loop_index = i32::try_from(f.loop_i).unwrap_or(i32::MAX);
                out.turn = out.turn.saturating_add(f.offset);
            }
            Emitter::Rotate { .. } => out.turn = out.turn.saturating_add(f.offset),
            Emitter::OnBullet { .. } => out.hook = Some((k, f.hook_after)),
            Emitter::Fire { .. } | Emitter::Wait { .. } => {}
        }
        list = body_of(node)?;
    }
    Some(out)
}

// --- Expressions -----------------------------------------------------------------------------

fn saturate(v: i64) -> i32 {
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
fn value(s: &mut State, t: &AttackTask, ab: &Above, e: &Expr) -> i32 {
    let mut c = Eval {
        rng: &mut s.rng,
        loop_index: ab.loop_index,
        phase_time: i32::try_from(t.age).unwrap_or(i32::MAX),
    };
    eval(e, &mut c)
}

fn ticks(v: i32) -> u32 {
    u32::try_from(v.clamp(0, i32::try_from(MAX_TICKS).unwrap_or(i32::MAX))).unwrap_or(0)
}

fn rounds(v: i32) -> u32 {
    u32::try_from(v.clamp(0, MAX_REPEAT)).unwrap_or(0)
}

// --- Directions ------------------------------------------------------------------------------

/// The unit direction of the vector `(x, y)`; straight down if it has no length.
fn normalize(x: i64, y: i64) -> Dir {
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
fn rotate(d: Dir, angle: i32) -> Dir {
    let (sin, cos) = trig::sin_cos(angle);
    let (x, y) = (i64::from(d.x), i64::from(d.y));
    let (sin, cos) = (i64::from(sin), i64::from(cos));
    let scale = i64::from(trig::SCALE);
    normalize((x * cos + y * sin) / scale, (-x * sin + y * cos) / scale)
}

/// The direction from `from` to the entity `target`; straight down if it is gone.
fn aim(s: &State, from: Point, target: EntityId) -> Dir {
    match s.find(target) {
        Some(w) => {
            let to = s.at(w);
            normalize(
                i64::from(to.x.0) - i64::from(from.x.0),
                i64::from(to.y.0) - i64::from(from.y.0),
            )
        }
        None => Dir::DOWN,
    }
}

// --- Carrying out ----------------------------------------------------------------------------

enum Advance {
    Continue,
    Wait,
    Broken,
}

/// Carries out nodes until the attack waits (true) or is over (false).
fn exec(s: &mut State, t: &mut AttackTask, budget: &mut u32, spawns_left: &mut u32) -> bool {
    let program = Arc::clone(&t.program);
    let Some(root) = root_list(&program, &t.base) else {
        return false;
    };
    loop {
        if *budget == 0 {
            return true;
        }
        *budget -= 1;
        let Some(top) = t.frames.len().checked_sub(1) else {
            return false;
        };
        let (Some(list), Some(ab)) = (list_at(root, &t.frames, top), above(root, &t.frames, top))
        else {
            return false;
        };
        let Some(node) = list.get(usize::from(t.frames[top].idx)) else {
            // The end of this list: the node round it goes on to its next round, or is done.
            t.frames.pop();
            let Some(parent) = t.frames.len().checked_sub(1) else {
                return false;
            };
            match next_round(s, t, root, parent) {
                Advance::Continue => continue,
                Advance::Wait => return true,
                Advance::Broken => return false,
            }
        };
        match node {
            Emitter::Fire { .. } => {
                if !emit(s, t, &ab, node, spawns_left) {
                    return false;
                }
                t.frames[top].idx += 1;
            }
            Emitter::Wait { ticks: n } => {
                let wait = ticks(value(s, t, &ab, n));
                t.frames[top].idx += 1;
                if wait > 0 {
                    t.wait_left = wait;
                    return true;
                }
            }
            Emitter::Repeat { count: n, body, .. } | Emitter::Ring { count: n, body } => {
                let n = rounds(value(s, t, &ab, n));
                enter_loop(t, top, node, n, 0, body.is_empty());
            }
            Emitter::Spread {
                count: n,
                angle,
                body,
            } => {
                let n = rounds(value(s, t, &ab, n));
                let angle = value(s, t, &ab, angle);
                enter_loop(t, top, node, n, angle, body.is_empty());
            }
            Emitter::Rotate { angle, body } => {
                t.frames[top].offset = value(s, t, &ab, angle);
                enter_body(t, top, body.is_empty());
            }
            Emitter::OnBullet {
                after_ticks, body, ..
            } => {
                t.frames[top].hook_after = ticks(value(s, t, &ab, after_ticks));
                enter_body(t, top, body.is_empty());
            }
        }
    }
}

/// Goes into the body of the node the frame `top` is at, or past the node if it has none.
fn enter_body(t: &mut AttackTask, top: usize, empty: bool) {
    if empty {
        t.frames[top].idx += 1;
    } else {
        t.frames.push(Frame::default());
    }
}

/// Starts a `repeat`, `ring` or `spread` of `n` rounds.
fn enter_loop(t: &mut AttackTask, top: usize, node: &Emitter, n: u32, param: i32, empty: bool) {
    if n == 0 {
        t.frames[top].idx += 1;
        return;
    }
    let f = &mut t.frames[top];
    f.loop_i = 0;
    f.loop_n = n;
    f.param = param;
    f.offset = round_turn(node, f);
    enter_body(t, top, empty);
}

/// How far round `f.loop_i` of a `ring` or `spread` turns its directions.
fn round_turn(node: &Emitter, f: &Frame) -> i32 {
    let (i, n) = (i64::from(f.loop_i), i64::from(f.loop_n));
    match node {
        Emitter::Ring { .. } => saturate(i * i64::from(trig::PER_TURN) / n),
        Emitter::Spread { .. } if n > 1 => {
            let angle = i64::from(f.param);
            saturate(i * angle / (n - 1) - angle / 2)
        }
        _ => 0,
    }
}

/// The body of the node at frame `parent` has ended: start its next round, or go on past it.
fn next_round(s: &mut State, t: &mut AttackTask, root: &[Emitter], parent: usize) -> Advance {
    let Some(list) = list_at(root, &t.frames, parent) else {
        return Advance::Broken;
    };
    let Some(node) = list.get(usize::from(t.frames[parent].idx)) else {
        return Advance::Broken;
    };
    match node {
        Emitter::Repeat { .. } | Emitter::Ring { .. } | Emitter::Spread { .. } => {
            let f = &mut t.frames[parent];
            f.loop_i += 1;
            if f.loop_i >= f.loop_n {
                f.idx += 1;
                return Advance::Continue;
            }
            f.offset = round_turn(node, f);
            t.frames.push(Frame::default());
            if let Emitter::Repeat { interval_ticks, .. } = node {
                let Some(ab) = above(root, &t.frames, parent) else {
                    return Advance::Broken;
                };
                let wait = ticks(value(s, t, &ab, interval_ticks));
                if wait > 0 {
                    t.wait_left = wait;
                    return Advance::Wait;
                }
            }
            Advance::Continue
        }
        Emitter::Rotate { .. } | Emitter::OnBullet { .. } => {
            t.frames[parent].idx += 1;
            Advance::Continue
        }
        Emitter::Fire { .. } | Emitter::Wait { .. } => Advance::Broken,
    }
}

/// Makes the bullet of a `fire` node. False if the attack's owner is gone.
fn emit(
    s: &mut State,
    t: &mut AttackTask,
    ab: &Above,
    node: &Emitter,
    spawns_left: &mut u32,
) -> bool {
    let Emitter::Fire {
        speed,
        direction,
        bullet,
    } = node
    else {
        return true;
    };
    let origin = match t.origin {
        Origin::At(p) => p,
        Origin::Owner => match s.find(t.owner) {
            Some(w) => s.at(w),
            None => return false,
        },
    };
    let speed = value(s, t, ab, speed).clamp(-MAX_LEN, MAX_LEN);
    let dir = match direction {
        Direction::Aimed { offset } => {
            let turn = value(s, t, ab, offset).saturating_add(ab.turn);
            rotate(aim(s, origin, t.target), turn)
        }
        Direction::Absolute { angle } => {
            rotate(Dir::DOWN, value(s, t, ab, angle).saturating_add(ab.turn))
        }
        Direction::Relative { angle } => {
            rotate(t.parent_dir, value(s, t, ab, angle).saturating_add(ab.turn))
        }
        Direction::Sequential { step } => rotate(t.last_dir, value(s, t, ab, step)),
    };
    let hook = ab.hook.map(|(k, after)| {
        let path = t
            .base
            .iter()
            .copied()
            .chain((0..=k).map(|j| PathStep {
                node: t.frames[j].idx,
                then: j == k,
            }))
            .collect();
        Box::new(Hook {
            program: Arc::clone(&t.program),
            path,
            left: after,
            owner: t.owner,
            target: t.target,
            friendly: t.friendly,
            damage: t.damage,
        })
    });
    let scale = i64::from(trig::SCALE);
    let velocity = |component: i32| Fx(saturate(i64::from(component) * i64::from(speed) / scale));
    spawn_bullet(
        s,
        BulletState {
            at: origin,
            vx: velocity(dir.x),
            vy: velocity(dir.y),
            dir,
            style: bullet.as_ref().map_or(0, |b| registry::bullet_style(&b.0)),
            owner: t.owner,
            friendly: t.friendly,
            damage: t.damage,
            hook,
        },
        spawns_left,
    );
    t.last_dir = dir;
    true
}
