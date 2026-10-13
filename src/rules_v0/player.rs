//! The main character (ADR-050): movement from the input's stick, the persistent facing
//! (ADR-055), the main shot that attacks on its own along it, and the evade.

use crate::attack::AttackRef;
use crate::engine::{Aim, DashState, Dir, PlayerState, State};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::input::Input;
use crate::stage::Stage;
use crate::trig::SCALE;
use crate::validate::limits::{FIELD_H, FIELD_W};

use super::attacks;
use super::entity::{Who, speed_pct};
use super::{PLAYER_SPEED, SHOT_INTERVAL};

/// The stick's full deflection on each axis (ADR-050); −128 counts as −127.
const STICK_MAX: i8 = 127;

/// `⌈√n⌉`.
fn ceil_sqrt(n: u64) -> u64 {
    let r = n.isqrt();
    if r * r < n { r + 1 } else { r }
}

/// The move of a stick at `(x, y)` under a speed cap of `cap` units: the vector clamped to
/// length 127, then scaled so that 127 is the cap. Its length is rounded up, so a diagonal is
/// never faster than a straight line (ADR-050).
fn stick_move(x: i8, y: i8, cap: i64) -> (i64, i64) {
    let (x, y) = (i64::from(x), i64::from(y));
    let len = ceil_sqrt((x * x + y * y).unsigned_abs()).max(STICK_MAX as u64);
    // `len >= 127 > 0`, and truncation toward zero is the same on every CPU.
    let len = len as i64;
    (x * cap / len, y * cap / len)
}

/// The direction of a stick at `(x, y)` as a vector of length about `SCALE`, or `None` when the
/// stick is centred.
fn stick_direction(x: i8, y: i8) -> Option<Dir> {
    if x == 0 && y == 0 {
        return None;
    }
    let (x, y) = (i64::from(x), i64::from(y));
    let scale = i64::from(SCALE);
    // Both are at most 127 in size, so every product fits in 64 bits.
    let len = ((x * x + y * y).unsigned_abs() * (scale * scale).unsigned_abs()).isqrt() as i64;
    Some(Dir {
        x: (x * scale * scale / len) as i32,
        y: (y * scale * scale / len) as i32,
    })
}

pub(super) fn update(stage: &Stage, s: &mut State, input: Input, events: &mut Vec<DomainEvent>) {
    let p = &mut s.player;
    p.invulnerable = p.invulnerable.saturating_sub(1);
    p.evade_cooldown = p.evade_cooldown.saturating_sub(1);
    let (x, y) = (input.move_x.max(-STICK_MAX), input.move_y.max(-STICK_MAX));
    if let Some(dir) = stick_direction(x, y) {
        p.facing = dir;
    }
    if input.evade && p.evade_ready() {
        start_evade(stage, p, events);
    }
    if s.player.dash.is_some() {
        dash(stage, &mut s.player);
    } else {
        move_player(s, x, y);
    }
    shoot(stage, s, events);
}

fn move_by(p: &mut PlayerState, dx: i64, dy: i64) {
    // Both are within a speed cap or an evade's distance, which the validator bounds.
    p.at = Point {
        x: p.at
            .x
            .saturating_add(Fx(dx as i32))
            .clamp(Fx(0), Fx(FIELD_W)),
        y: p.at
            .y
            .saturating_add(Fx(dy as i32))
            .clamp(Fx(0), Fx(FIELD_H)),
    };
}

fn move_player(s: &mut State, x: i8, y: i8) {
    let keep = speed_pct(&s.player.statuses) * attacks::move_pct(s, s.id_of(Who::Player)) / 100;
    let cap = i64::from(PLAYER_SPEED) * keep / 100;
    let (dx, dy) = stick_move(x, y, cap);
    move_by(&mut s.player, dx, dy);
}

/// A ready press dashes along the facing, which is the stick's direction when it is not
/// centred (ADR-050). Invulnerability and the cooldown count from the press.
fn start_evade(stage: &Stage, p: &mut PlayerState, events: &mut Vec<DomainEvent>) {
    let evade = &stage.player.evade;
    p.dash = Some(DashState {
        dir: p.facing,
        done: 0,
        left: evade.ticks,
    });
    p.evade_cooldown = evade.cooldown_ticks;
    p.invulnerable = p.invulnerable.max(evade.invulnerable_ticks);
    events.push(DomainEvent::Evaded);
}

/// One tick of the dash: the `k`th tick moves from `distance × (k − 1) / ticks` to
/// `distance × k / ticks` along its direction, so the whole dash covers exactly its distance.
/// Slowing and attacks do not slow it, and the stick does not steer it.
fn dash(stage: &Stage, p: &mut PlayerState) {
    let Some(d) = p.dash else {
        return;
    };
    let distance = i64::from(stage.player.evade.distance.0);
    let ticks = i64::from(d.done) + i64::from(d.left);
    let at = |k: i64| distance * k / ticks;
    let step = at(i64::from(d.done) + 1) - at(i64::from(d.done));
    let scale = i64::from(SCALE);
    move_by(
        p,
        i64::from(d.dir.x) * step / scale,
        i64::from(d.dir.y) * step / scale,
    );
    p.dash = (d.left > 1).then_some(DashState {
        dir: d.dir,
        done: d.done + 1,
        left: d.left - 1,
    });
}

/// What the main character shoots, and how, right now. This is the one place that says so, so
/// that ways of shooting it can acquire during a run (a different attack, aimed at the nearest
/// enemy, homing, a faster rhythm) change what this returns and nothing else: the attack runs
/// as any other, with the aim and the damage given here.
struct MainShot<'a> {
    attack: &'a AttackRef,
    /// Where `aimed` directions point: along the facing.
    aim: Aim,
    /// Hit points a bullet takes before the attack-up status.
    damage: u32,
    /// Ticks between starts.
    interval: u32,
}

/// Today the stage's own `shot` attack, along the facing, at the stage's attack stat.
fn main_shot<'a>(stage: &'a Stage, s: &State) -> MainShot<'a> {
    MainShot {
        attack: &stage.player.shot,
        aim: Aim::Fixed(s.player.facing),
        damage: stage.player.atk,
        interval: SHOT_INTERVAL,
    }
}

/// The main shot attacks on its own every `interval` ticks (ADR-050). The attack is a task like
/// any other: once started it runs to its end, follows the main character and counts against
/// the attack and bullet budgets. The next shot waits for the last one's attack to end
/// (ADR-055: one attack at a time).
fn shoot(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    s.player.shot_cooldown = s.player.shot_cooldown.saturating_sub(1);
    if s.player.shot_cooldown > 0 || attacks::busy(s, s.id_of(Who::Player), 0) {
        return;
    }
    let shot = main_shot(stage, s);
    s.player.shot_cooldown = shot.interval;
    attacks::start_with(
        s,
        Who::Player,
        0,
        shot.aim,
        shot.attack,
        shot.damage,
        0,
        events,
    );
}
