//! The main character: movement, the main shot and the skill slots with hold and release
//! (ADR-032, ADR-038).

use crate::engine::{HoldState, State, UNLIMITED};
use crate::event::DomainEvent;
use crate::fixed::Fx;
use crate::input::Input;
use crate::stage::Stage;
use crate::validate::limits::{FIELD_H, FIELD_W};

use super::act::cast;
use super::entity::{Who, scale_damage, speed_pct, status_pct};
use super::select::select;
use super::{
    FOCUS_SPEED, HOLD_LIMIT_TICKS, PLAYER_BULLET_SPEED, PLAYER_ID, PLAYER_SPEED, SHOT_INTERVAL,
    spawn_bullet,
};
use crate::engine::{BulletState, StatusKind};
use crate::fixed::Point;

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

pub(super) fn update(
    stage: &Stage,
    s: &mut State,
    input: Input,
    events: &mut Vec<DomainEvent>,
    spawns_left: &mut u32,
) {
    s.player.invulnerable = s.player.invulnerable.saturating_sub(1);
    move_player(s, input);
    shoot(stage, s, input, spawns_left);
    skills(stage, s, input, events, spawns_left);
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

/// The main shot fires only on ticks whose input has `fire` (ADR-038).
fn shoot(stage: &Stage, s: &mut State, input: Input, spawns_left: &mut u32) {
    s.player.shot_cooldown = s.player.shot_cooldown.saturating_sub(1);
    if !input.fire || s.player.shot_cooldown > 0 {
        return;
    }
    s.player.shot_cooldown = SHOT_INTERVAL;
    let damage = scale_damage(
        stage.player.atk,
        status_pct(&s.player.statuses, StatusKind::AtkUp),
        0,
    );
    let bullet = BulletState {
        at: s.player.at,
        vx: Fx(0),
        vy: Fx(-PLAYER_BULLET_SPEED),
        owner: PLAYER_ID,
        friendly: true,
        damage,
    };
    spawn_bullet(s, bullet, spawns_left);
}

/// Hold and release (ADR-032). Pressing a ready skill's slot starts a hold; releasing it, or
/// holding for `HOLD_LIMIT_TICKS`, casts the skill at the target its selector picks. One hold
/// at a time, and a skill that is not ready cannot be held. The slot is the input's `held`
/// field; the frontend slows its own clock while a hold lasts, which the simulation never sees.
fn skills(
    stage: &Stage,
    s: &mut State,
    input: Input,
    events: &mut Vec<DomainEvent>,
    spawns_left: &mut u32,
) {
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
            cast_slot(stage, s, index, events, spawns_left);
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
fn cast_slot(
    stage: &Stage,
    s: &mut State,
    index: usize,
    events: &mut Vec<DomainEvent>,
    spawns_left: &mut u32,
) {
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
    cast(s, Who::Player, target, &def.skill, events, spawns_left);
}
