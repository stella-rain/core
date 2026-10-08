//! What a rule or a skill does once it fires: attacks, casts, summons, moves and telegraphs.
//! Main-character skills (ADR-032) use the same casts as agents' rules.

use crate::behaviour::{Action, Skill};
use crate::engine::{AgentKey, AgentSource, MoveOrder, State, StatusKind};
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::stage::Stage;

use super::agents::{agent_def, agent_mut, new_agent};
use super::attacks;
use super::entity::Who;
use super::{AGGRO_PER_BUFF, AGGRO_PER_HEAL_POINT, MAX_ENEMIES_ALIVE, count};

/// Does the action for rule `rule_index` of `me`, aimed at `target`. False if this version
/// cannot do it, in which case the rule does not fire and the next one gets its turn.
pub(super) fn perform(
    stage: &Stage,
    s: &mut State,
    me: Who,
    target: Who,
    rule_index: usize,
    action: &Action,
    events: &mut Vec<DomainEvent>,
) -> bool {
    match action {
        // An attack does as much as the main shot if an ally makes it, and 1 hit point if an
        // enemy does.
        Action::Attack { attack } => {
            let base = if State::is_ally(me) {
                stage.player.atk
            } else {
                1
            };
            // The boss's clock is its phase's, so `phase_time` is the phase's time.
            let age = match (me, s.boss.as_ref()) {
                (Who::Boss, Some(b)) => b.phase_age.saturating_sub(1),
                _ => 0,
            };
            attacks::start_aged(s, me, target, attack, base, age);
            true
        }
        Action::Cast { skill } => {
            cast(s, me, target, skill, events);
            true
        }
        Action::Summon { count: n, .. } => summon(stage, s, me, rule_index, *n),
        Action::MoveTo { to, ticks } => {
            let order = Some(MoveOrder {
                to: *to,
                ticks_left: (*ticks).max(1),
            });
            match me {
                Who::Boss => s.boss.as_mut().expect("a boss").order = order,
                _ => agent_mut(s, me).order = order,
            }
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

/// Casts `skill` from `caster` at `target`. Allies gain aggro for buffs and heals.
pub(super) fn cast(
    s: &mut State,
    caster: Who,
    target: Who,
    skill: &Skill,
    events: &mut Vec<DomainEvent>,
) {
    let source = s.id_of(caster);
    let mut apply = |s: &mut State, kind, value_pct: u16, duration: u32| {
        s.add_status(target, kind, source, value_pct, duration, events);
        s.raise_aggro(source, AGGRO_PER_BUFF);
    };
    match skill {
        Skill::Shot { pattern, damage } => attacks::start(s, caster, target, pattern, *damage),
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

/// An enemy from a wave, or the boss, summons `n` more enemies where it stands. Companions and
/// summoned agents cannot summon in this version (the validator already keeps summoned agents
/// from it).
fn summon(stage: &Stage, s: &mut State, me: Who, rule_index: usize, n: u8) -> bool {
    let rule = u32::try_from(rule_index).unwrap_or(u32::MAX);
    let (key, at) = match me {
        Who::Enemy(i) => {
            let parent = &s.enemies[i];
            if parent.key.source != AgentSource::Wave {
                return false;
            }
            let key = AgentKey {
                source: AgentSource::Summoned,
                index: parent.key.index,
                rule,
            };
            (key, parent.at)
        }
        Who::Boss => {
            let boss = s.boss.as_ref().expect("a boss");
            let key = AgentKey {
                source: AgentSource::BossRule,
                index: boss.phase,
                rule,
            };
            (key, boss.at)
        }
        _ => return false,
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
