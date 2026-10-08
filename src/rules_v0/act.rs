//! What a rule or a skill does once it fires: attacks, casts, summons, moves and telegraphs.
//! Main-character skills (ADR-032) use the same casts as agents' rules.

use crate::behaviour::{Action, Skill};
use crate::engine::{AgentKey, AgentSource, BulletState, MoveOrder, State, StatusKind};
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::stage::Stage;

use super::agents::{agent_def, agent_mut, new_agent};
use super::entity::{Who, scale_damage, status_pct};
use super::{
    AGGRO_PER_BUFF, AGGRO_PER_HEAL_POINT, ENEMY_BULLET_SPEED, MAX_ENEMIES_ALIVE,
    PLAYER_BULLET_SPEED, aimed, count, spawn_bullet,
};

/// Does the action for rule `rule_index` of `me`, aimed at `target`. False if this version
/// cannot do it, in which case the rule does not fire and the next one gets its turn.
#[allow(clippy::too_many_arguments)]
pub(super) fn perform(
    stage: &Stage,
    s: &mut State,
    me: Who,
    target: Who,
    rule_index: usize,
    action: &Action,
    events: &mut Vec<DomainEvent>,
    spawns_left: &mut u32,
) -> bool {
    match action {
        // Until attack contents arrive (core#18) every attack is one aimed bullet: an ally's
        // is as strong as the main shot, an enemy's takes 1 hit point.
        Action::Attack { .. } => {
            let base = if State::is_ally(me) {
                stage.player.atk
            } else {
                1
            };
            fire(s, me, target, base, spawns_left);
            true
        }
        Action::Cast { skill } => {
            cast(s, me, target, skill, events, spawns_left);
            true
        }
        Action::Summon { count: n, .. } => summon(stage, s, me, rule_index, *n),
        Action::MoveTo { to, ticks } => {
            agent_mut(s, me).order = Some(MoveOrder {
                to: *to,
                ticks_left: (*ticks).max(1),
            });
            true
        }
        Action::Telegraph { ticks } => {
            events.push(DomainEvent::Telegraph {
                source: s.id_of(me),
                ticks: *ticks,
            });
            true
        }
    }
}

/// One bullet from `from` toward `target`, `base` damage before the shooter's atk-up.
pub(super) fn fire(s: &mut State, from: Who, target: Who, base: u32, spawns_left: &mut u32) {
    let friendly = State::is_ally(from);
    let speed = if friendly {
        PLAYER_BULLET_SPEED
    } else {
        ENEMY_BULLET_SPEED
    };
    let at = s.at(from);
    let (vx, vy) = aimed(at, s.at(target), speed);
    let damage = scale_damage(base, status_pct(s.statuses(from), StatusKind::AtkUp), 0);
    let bullet = BulletState {
        at,
        vx,
        vy,
        owner: s.id_of(from),
        friendly,
        damage,
    };
    spawn_bullet(s, bullet, spawns_left);
}

/// Casts `skill` from `caster` at `target`. Allies gain aggro for buffs and heals.
pub(super) fn cast(
    s: &mut State,
    caster: Who,
    target: Who,
    skill: &Skill,
    events: &mut Vec<DomainEvent>,
    spawns_left: &mut u32,
) {
    let source = s.id_of(caster);
    let mut apply = |s: &mut State, kind, value_pct: u16, duration: u32| {
        s.add_status(target, kind, source, value_pct, duration, events);
        s.raise_aggro(source, AGGRO_PER_BUFF);
    };
    match skill {
        Skill::Shot { damage, .. } => fire(s, caster, target, *damage, spawns_left),
        Skill::AtkUp {
            value_pct,
            duration_ticks,
        } => apply(s, StatusKind::AtkUp, *value_pct, *duration_ticks),
        Skill::Vulnerable {
            value_pct,
            duration_ticks,
        } => apply(s, StatusKind::Vulnerable, *value_pct, *duration_ticks),
        Skill::Slow {
            value_pct,
            duration_ticks,
        } => apply(s, StatusKind::Slow, *value_pct, *duration_ticks),
        Skill::Heal { amount } => {
            let room = s.max_hp(target).saturating_sub(s.hp(target));
            let healed = (*amount).min(room);
            *s.hp_mut(target) += healed;
            if healed > 0 {
                events.push(DomainEvent::Healed {
                    target: s.id_of(target),
                    amount: healed,
                });
                s.raise_aggro(source, healed.saturating_mul(AGGRO_PER_HEAL_POINT));
            }
        }
    }
}

/// An enemy from a wave summons `n` more enemies where it stands. Companions and summoned
/// agents cannot summon in this version (the validator already keeps summoned agents from it).
fn summon(stage: &Stage, s: &mut State, me: Who, rule_index: usize, n: u8) -> bool {
    let Who::Enemy(i) = me else {
        return false;
    };
    let parent = &s.enemies[i];
    if parent.key.source != AgentSource::Wave {
        return false;
    }
    let (wave, at) = (parent.key.index, parent.at);
    let key = AgentKey {
        source: AgentSource::Summoned,
        index: wave,
        rule: u32::try_from(rule_index).unwrap_or(u32::MAX),
    };
    let def = agent_def(stage, key);
    for _ in 0..n {
        if count(&s.enemies) >= MAX_ENEMIES_ALIVE {
            break;
        }
        let id = EntityId(s.next_id);
        s.next_id += 1;
        s.enemies.push(new_agent(id, key, def, at, 0));
    }
    true
}
