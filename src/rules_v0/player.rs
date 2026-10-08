//! The main character: movement, the main shot and the skill slots with hold and release
//! (ADR-032, ADR-038).

use crate::attack::AttackRef;
use crate::engine::{Aim, Dir, HoldState, State, UNLIMITED};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::input::Input;
use crate::stage::Stage;
use crate::validate::limits::{FIELD_H, FIELD_W};

use super::act::cast;
use super::attacks;
use super::entity::{Who, speed_pct};
use super::select::select;
use super::{FOCUS_SPEED, HOLD_LIMIT_TICKS, PLAYER_SPEED, SHOT_INTERVAL};

/// Limits a move to `cap` units, keeping its direction: a diagonal is no faster (ADR-038).
fn capped_move(dx: i16, dy: i16, cap: i32) -> (i32, i32) {
    let (dx, dy) = (i64::from(dx), i64::from(dy));
    let len = i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i64::MAX);
    let cap = i64::from(cap);
    if len <= cap {
        return (dx as i32, dy as i32);
    }
    // `len > cap >= 0`, and truncation toward zero is the same on every CPU.
    ((dx * cap / len) as i32, (dy * cap / len) as i32)
}

pub(super) fn update(stage: &Stage, s: &mut State, input: Input, events: &mut Vec<DomainEvent>) {
    s.player.invulnerable = s.player.invulnerable.saturating_sub(1);
    move_player(s, input);
    shoot(stage, s, input);
    skills(stage, s, input, events);
}

fn move_player(s: &mut State, input: Input) {
    let cap = if input.focus {
        FOCUS_SPEED
    } else {
        PLAYER_SPEED
    };
    let cap = (i64::from(cap) * speed_pct(&s.player.statuses) / 100) as i32;
    let (dx, dy) = capped_move(input.dx, input.dy, cap);
    let p = &mut s.player;
    p.at = Point {
        x: p.at.x.saturating_add(Fx(dx)).clamp(Fx(0), Fx(FIELD_W)),
        y: p.at.y.saturating_add(Fx(dy)).clamp(Fx(0), Fx(FIELD_H)),
    };
}

/// What the main character shoots, and how, right now. This is the one place that says so, so
/// that ways of shooting it can acquire during a run (a different attack, aimed at the nearest
/// enemy, homing, a faster rhythm) change what this returns and nothing else: the attack runs
/// as any other, with the aim and the damage given here.
struct MainShot<'a> {
    attack: &'a AttackRef,
    /// Where `aimed` directions point. Straight up: the ship faces up the screen.
    aim: Aim,
    /// Hit points a bullet takes before the attack-up status.
    damage: u32,
    /// Ticks between starts while `fire` is held.
    interval: u32,
}

/// Today the stage's own `shot` attack, straight up, at the stage's attack stat.
fn main_shot<'a>(stage: &'a Stage, _s: &State) -> MainShot<'a> {
    MainShot {
        attack: &stage.player.shot,
        aim: Aim::Fixed(Dir::UP),
        damage: stage.player.atk,
        interval: SHOT_INTERVAL,
    }
}

/// The main shot starts its attack only on ticks whose input has `fire` (ADR-038). The attack
/// is a task like any other: once started it runs to its end whether or not `fire` is still
/// held, follows the main character and counts against the attack and bullet budgets.
fn shoot(stage: &Stage, s: &mut State, input: Input) {
    s.player.shot_cooldown = s.player.shot_cooldown.saturating_sub(1);
    if !input.fire || s.player.shot_cooldown > 0 {
        return;
    }
    let shot = main_shot(stage, s);
    s.player.shot_cooldown = shot.interval;
    attacks::start_with(s, Who::Player, shot.aim, shot.attack, shot.damage, 0);
}

/// Hold and release (ADR-032). Pressing a ready skill's slot starts a hold; releasing it, or
/// holding for `HOLD_LIMIT_TICKS`, casts the skill at the target its selector picks. One hold
/// at a time, and a skill that is not ready cannot be held. The slot is the input's `held`
/// field; the frontend slows its own clock while a hold lasts, which the simulation never sees.
fn skills(stage: &Stage, s: &mut State, input: Input, events: &mut Vec<DomainEvent>) {
    for sk in &mut s.player.skills {
        sk.cooldown_left = sk.cooldown_left.saturating_sub(1);
    }
    // Only slots 1 to 3 exist; anything else counts as nothing held.
    let held = if input.held <= 3 { input.held } else { 0 };

    if let Some(hold) = s.player.hold {
        let index = hold.index as usize;
        // Ticks held since the press: pressed on tick 10, this is 30 on tick 40.
        let ticks = hold.ticks + 1;
        if held != stage.player.skills[index].slot || ticks >= HOLD_LIMIT_TICKS {
            s.player.hold = None;
            cast_slot(stage, s, index, events);
        } else {
            s.player.hold = Some(HoldState {
                index: hold.index,
                ticks,
            });
        }
    }
    if s.player.hold.is_none() && held != 0 && held != s.player.held_before {
        let slot = stage.player.skills.iter().position(|d| d.slot == held);
        if let Some(index) = slot.filter(|i| s.player.skills[*i].is_ready()) {
            s.player.hold = Some(HoldState {
                index: u32::try_from(index).unwrap_or(u32::MAX),
                ticks: 0,
            });
        }
    }
    s.player.held_before = held;
}

/// Casts the skill in slot `index` if it is still ready and has a target. With no target it
/// fizzles and costs nothing.
fn cast_slot(stage: &Stage, s: &mut State, index: usize, events: &mut Vec<DomainEvent>) {
    let def = &stage.player.skills[index];
    if !s.player.skills[index].is_ready() {
        return;
    }
    let Some(target) = select(stage, s, Who::Player, &def.target) else {
        return;
    };
    let state = &mut s.player.skills[index];
    state.cooldown_left = def.cooldown_ticks;
    if state.charges_left != UNLIMITED {
        state.charges_left -= 1;
    }
    events.push(DomainEvent::SkillCast {
        slot: def.slot,
        target: Some(s.id_of(target)),
    });
    cast(s, Who::Player, target, &def.skill, events);
}
