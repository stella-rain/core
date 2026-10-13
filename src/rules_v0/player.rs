//! The main character (ADR-050): movement from the input's stick, the persistent facing
//! (ADR-055), the Main and Sub weapons that attack on their own (core#83), and the evade.

use crate::attack::AttackRef;
use crate::engine::{Aim, DashState, Dir, PlayerState, State};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::input::Input;
use crate::presets;
use crate::stage::Stage;
use crate::trig::SCALE;
use crate::validate::limits::{FIELD_H, FIELD_W};
use crate::weapon::Role;

use super::PLAYER_SPEED;
use super::attacks;
use super::entity::{Who, speed_pct};
use super::select::select;

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

/// The attack slots of the main character's weapons (core#83): each runs one attack at a time
/// (ADR-055), beside the other's. The charged Main skill and the Sub skill take slots 2 and 3.
const MAIN_SLOT: u8 = 0;
const SUB_SLOT: u8 = 1;

/// What a weapon attacks with, and how, right now. This is the one place that says so, so that
/// what the main character acquires during a run (a different attack, homing, a faster rhythm,
/// more damage) changes what this returns and nothing else: the attack runs as any other, with
/// the aim and the damage given here.
struct Shot<'a> {
    attack: &'a AttackRef,
    /// Where `aimed` directions point: along the facing, or at the target.
    aim: Aim,
    /// Hit points a bullet takes before the attack-up status.
    damage: u32,
    /// Ticks from this attack's start to the next one's.
    cooldown: u32,
}

/// Today the stage's weapon in `role`, at the stage's attack stat, aimed as its targeting says:
/// `None` when its selector finds nothing to aim at.
fn shot<'a>(stage: &'a Stage, s: &State, role: Role) -> Option<Shot<'a>> {
    let reference = match role {
        Role::Main => &stage.player.main,
        Role::Sub => &stage.player.sub,
    };
    // The validator lets only defined presets through.
    let weapon = presets::weapon(reference, role)?;
    let aim = match weapon.targeting.selector() {
        None => Aim::Fixed(s.player.facing),
        Some(selector) => {
            let target = select(stage, s, Who::Player, &selector)?;
            // Both are set to the direction to the target as the attack starts.
            Aim::Target {
                id: s.id_of(target),
                heading: s.player.facing,
                last: s.player.facing,
            }
        }
    };
    Some(Shot {
        attack: &weapon.attack,
        aim,
        damage: stage.player.atk,
        cooldown: weapon.cooldown_ticks,
    })
}

/// The Main weapon, then the Sub weapon (ADR-050, core#83).
fn shoot(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    attack_with(stage, s, Role::Main, events);
    attack_with(stage, s, Role::Sub, events);
}

/// A weapon attacks on its own once its cooldown, counted from its last attack's start, is
/// over and that attack has ended (ADR-055: one attack at a time). With nothing to aim at it
/// does not attack and spends no cooldown, so it attacks on the first tick it finds a target.
/// The attack is a task like any other: once started it runs to its end, follows the main
/// character and counts against the attack and bullet budgets.
fn attack_with(stage: &Stage, s: &mut State, role: Role, events: &mut Vec<DomainEvent>) {
    let (slot, cooldown) = match role {
        Role::Main => (MAIN_SLOT, &mut s.player.main_cooldown),
        Role::Sub => (SUB_SLOT, &mut s.player.sub_cooldown),
    };
    *cooldown = cooldown.saturating_sub(1);
    if *cooldown > 0 || attacks::busy(s, s.id_of(Who::Player), slot) {
        return;
    }
    let Some(shot) = shot(stage, s, role) else {
        return;
    };
    let started = attacks::start_with(
        s,
        Who::Player,
        slot,
        shot.aim,
        shot.attack,
        shot.damage,
        0,
        events,
    );
    if started {
        match role {
            Role::Main => s.player.main_cooldown = shot.cooldown,
            Role::Sub => s.player.sub_cooldown = shot.cooldown,
        }
    }
}
