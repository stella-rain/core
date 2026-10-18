//! The stage skill (ADR-075, ADR-098): the slot a falling star's reward fills, the use on a
//! press of `skill`, and what each building block does. A use needs a skill in the slot and its
//! cooldown run out; it then acts at its point (`ahead` along the facing, or toward the aim
//! assist's target while `assist` is on), its level's cooldown starts, and its level's effects
//! apply in their order, at most `MAX_SKILL_EFFECTS` of them. Each block:
//!
//! - **pull:** kept in the state for its time; every tick after the enemies have moved and
//!   pushed apart, it pulls each enemy whose circle reaches within its radius toward its point,
//!   `strength` at most, never past it. The boss and its parts stay where they are.
//! - **self status:** its status on the main character.
//! - **area status:** its status on every enemy, the boss and each unbroken part whose circle
//!   reaches within its radius of the point, once, at the use.
//! - **attack:** its attack in the stage skill's slot, along the aim; a use ends the one still
//!   running there.
//! - **shield:** kept in the state for its time; damage to the main character takes from it
//!   first.
//!
//! A new pull or shield replaces the one there; statuses refresh as any status does (ADR-036).

use crate::engine::{Dir, PlayerState, PullState, ShieldState, SkillSlot, State, StatusKind};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::input::Input;
use crate::rewards::Rewards;
use crate::stage::Stage;
use crate::stage_skill::{MAX_SKILL_EFFECTS, MAX_SKILL_LEVELS, SkillEffect};
use crate::trig::SCALE;

use super::agent_def;
use super::attacks;
use super::entity::Who;
use super::levels;
use super::player;
use super::{PLAYER_ID, overlaps};

/// The main character's attack slot for a stage skill's attack (ADR-069); the combo is 0.
pub(super) const SKILL_SLOT: u8 = 1;

/// The timers of the slot, the pull and the shield run down a tick; the pull and the shield end
/// when theirs reach 0.
pub(super) fn tick(s: &mut State) {
    let p = &mut s.player;
    if let Some(slot) = &mut p.skill {
        slot.cooldown_left = slot.cooldown_left.saturating_sub(1);
    }
    if let Some(b) = &mut p.pull {
        b.ticks_left -= 1;
        if b.ticks_left == 0 {
            p.pull = None;
        }
    }
    if let Some(sh) = &mut p.shield {
        sh.ticks_left -= 1;
        if sh.ticks_left == 0 {
            p.shield = None;
        }
    }
}

/// A taken reward gives `levels` of the skill at `index` in the stage skills: another skill
/// replaces the one held and arrives ready; the same one rises, up to its highest level.
pub(super) fn take(rewards: &Rewards, p: &mut PlayerState, index: usize, levels: u32) {
    let Some(def) = rewards.stage_skills().get(index) else {
        return;
    };
    // The validator holds both to `MAX_SKILL_LEVELS`.
    let most = def.levels.len().min(MAX_SKILL_LEVELS) as u32;
    let index = index as u16;
    p.skill = Some(match p.skill {
        Some(slot) if slot.index == index => SkillSlot {
            level: (u32::from(slot.level) + levels).min(most) as u8,
            ..slot
        },
        _ => SkillSlot {
            index,
            level: levels.min(most) as u8,
            cooldown_left: 0,
        },
    });
}

/// Where the aim points: toward the target while `assist` is on and there is one, else the
/// facing.
fn aim(s: &State, input: Input) -> Dir {
    if input.assist {
        if let Some(dir) = s.player.target.and_then(|t| player::toward(s, t)) {
            return dir;
        }
    }
    s.player.facing
}

/// A press of `skill`: the skill in the slot acts, if it is ready.
pub(super) fn press(
    stage: &Stage,
    rewards: &Rewards,
    s: &mut State,
    input: Input,
    events: &mut Vec<DomainEvent>,
) {
    let Some(slot) = s.player.skill else {
        return;
    };
    if slot.cooldown_left > 0 {
        return;
    }
    let Some(def) = rewards.stage_skills().get(usize::from(slot.index)) else {
        return;
    };
    let Some(level) = def.levels.get(usize::from(slot.level).saturating_sub(1)) else {
        return;
    };
    let dir = aim(s, input);
    let scale = i64::from(SCALE);
    let ahead = i64::from(def.ahead.0);
    // `ahead` is at most 512 pixels and the direction at most `SCALE` long.
    let point = stage.map().clamp(Point {
        x: s.player
            .at
            .x
            .saturating_add(Fx((i64::from(dir.x) * ahead / scale) as i32)),
        y: s.player
            .at
            .y
            .saturating_add(Fx((i64::from(dir.y) * ahead / scale) as i32)),
    });
    s.player.skill = Some(SkillSlot {
        cooldown_left: level.cooldown_ticks,
        ..slot
    });
    events.push(DomainEvent::StageSkillUsed {
        skill: def.id.clone(),
        level: slot.level,
    });
    // The validator holds a level to `MAX_SKILL_EFFECTS`.
    for effect in level.effects.iter().take(MAX_SKILL_EFFECTS) {
        apply(stage, s, effect, point, dir, events);
    }
}

/// One effect of a use, at `point`, along the aim `dir`.
fn apply(
    stage: &Stage,
    s: &mut State,
    effect: &SkillEffect,
    point: Point,
    dir: Dir,
    events: &mut Vec<DomainEvent>,
) {
    match effect {
        SkillEffect::Pull {
            radius,
            strength,
            duration_ticks,
        } => {
            s.player.pull = Some(PullState {
                at: point,
                radius: radius.0,
                strength: strength.0,
                ticks_left: *duration_ticks,
            });
        }
        SkillEffect::SelfStatus {
            status,
            value_pct,
            duration_ticks,
        } => {
            // The validator holds the status to the registry's, which are `StatusKind`'s.
            if let Some(kind) = StatusKind::from_name(&status.0) {
                s.add_status(
                    Who::Player,
                    kind,
                    PLAYER_ID,
                    *value_pct,
                    *duration_ticks,
                    events,
                );
            }
        }
        SkillEffect::AreaStatus {
            status,
            radius,
            value_pct,
            duration_ticks,
        } => {
            let Some(kind) = StatusKind::from_name(&status.0) else {
                return;
            };
            for who in within(stage, s, point, radius.0) {
                s.add_status(who, kind, PLAYER_ID, *value_pct, *duration_ticks, events);
            }
        }
        SkillEffect::Attack { attack } => {
            attacks::end_run(s, PLAYER_ID, SKILL_SLOT);
            let strike = attacks::Strike {
                bonus_pct: 0,
                speed_pct: 0,
                pull: None,
            };
            let damage = levels::attack(s, stage.player.atk);
            attacks::start_player(s, SKILL_SLOT, dir, attack, damage, strike, events);
        }
        SkillEffect::Shield {
            amount,
            duration_ticks,
        } => {
            s.player.shield = Some(ShieldState {
                amount: *amount,
                ticks_left: *duration_ticks,
            });
        }
    }
}

/// The live enemies, then the boss and its unbroken parts, whose circle reaches within
/// `radius` of `at`, in that order.
fn within(stage: &Stage, s: &State, at: Point, radius: i32) -> Vec<Who> {
    let mut out: Vec<Who> = s
        .enemies
        .iter()
        .enumerate()
        .filter(|(_, e)| e.hp > 0 && overlaps(at, radius, e.at, agent_def(stage, e.key).radius.0))
        .map(|(i, _)| Who::Enemy(i))
        .collect();
    if let Some((def, boss)) = stage.boss.as_ref().zip(s.boss.as_ref()) {
        if boss.hp > 0 && overlaps(at, radius, boss.at, def.radius.0) {
            out.push(Who::Boss);
        }
        for (i, part) in boss.parts.iter().enumerate() {
            if part.hp > 0 && overlaps(at, radius, s.at(Who::BossPart(i)), part.radius) {
                out.push(Who::BossPart(i));
            }
        }
    }
    out
}

/// The pull's step for this tick, after the enemies have moved and pushed apart.
pub(super) fn pull(stage: &Stage, s: &mut State) {
    let Some(b) = s.player.pull else {
        return;
    };
    let map = stage.map();
    for e in s.enemies.iter_mut() {
        if e.hp == 0 || !overlaps(b.at, b.radius, e.at, agent_def(stage, e.key).radius.0) {
            continue;
        }
        let dx = i64::from(b.at.x.0) - i64::from(e.at.x.0);
        let dy = i64::from(b.at.y.0) - i64::from(e.at.y.0);
        // Both are within the map and its margin, so the squares fit in 64 bits.
        let dist = (dx * dx + dy * dy).unsigned_abs().isqrt() as i64;
        if dist == 0 {
            continue;
        }
        let step = dist.min(i64::from(b.strength));
        e.at = map.clamp(Point {
            x: e.at.x.saturating_add(Fx((dx * step / dist) as i32)),
            y: e.at.y.saturating_add(Fx((dy * step / dist) as i32)),
        });
    }
}

/// The part of `damage` to the main character the shield absorbs; what is left of it. A
/// shield with nothing left is gone.
pub(super) fn absorb(s: &mut State, damage: u32) -> u32 {
    let p = &mut s.player;
    let Some(sh) = &mut p.shield else {
        return damage;
    };
    let absorbed = damage.min(sh.amount);
    sh.amount -= absorbed;
    if sh.amount == 0 {
        p.shield = None;
    }
    damage - absorbed
}
