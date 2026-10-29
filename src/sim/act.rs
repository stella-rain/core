//! What a rule or a skill does once it fires: attacks, casts, summons, moves and telegraphs.

use super::World;
use crate::behaviour::{Action, Skill};
use crate::engine::{AgentKey, AgentSource, MoveOrder, State, StatusKind};
use crate::event::DomainEvent;

use super::agents::agent_mut;
use super::attacks;
use super::boss;
use super::entity::Who;
use super::spawn_enemy;

/// Does the action for rule `rule_index` of `me`, aimed at `target`. False if this version
/// cannot do it, in which case the rule does not fire and the next one gets its turn.
pub(super) fn perform(
    stage: &World,
    s: &mut State,
    me: Who,
    target: Who,
    rule_index: usize,
    action: &Action,
    events: &mut Vec<DomainEvent>,
) -> bool {
    match action {
        // An attack does as much as the main character's weapons (its attack stat) if an ally
        // makes it, and the run stats file's `enemy_attack_damage` if an enemy does.
        Action::Attack { attack, ranged } => {
            // A ranged start without a grant, or over the pool's tokens, is refused here, so the
            // rule keeps its cooldown and retries next tick (core Z2, RA1). Only regular
            // enemies are held to the tokens: an elite's definition has no pools.
            let pool = match me {
                Who::Enemy(i) if *ranged => stage
                    .defs
                    .get(s.enemies[i].def)
                    .ranged_pools
                    .get(rule_index)
                    .copied()
                    .flatten(),
                _ => None,
            };
            let ranged = pool.is_some();
            if let Some(pool) = pool
                && !attacks::ranged_may_start(stage, s, s.id_of(me), pool)
            {
                return false;
            }
            let base = if State::is_ally(me) {
                stage.player.atk
            } else {
                s.enemy_attack_damage
            };
            // The boss's clock is its phase's, so `phase_time` is the phase's time.
            let age = match (me, s.boss.as_ref()) {
                (Who::Boss, Some(b)) => b.phase_age.saturating_sub(1),
                _ => 0,
            };
            // Not ready while its last attack runs (ADR-055): the next rule gets its turn.
            let started = attacks::start_aged(stage, s, me, 0, target, attack, base, age, events);
            if started && ranged {
                attacks::mark_ranged(s);
            }
            started
        }
        Action::Cast { skill } => {
            if matches!(skill, Skill::Shot { .. }) && attacks::busy(s, s.id_of(me), 0) {
                return false;
            }
            cast(stage, s, me, target, skill, events);
            true
        }
        Action::Summon { count: n, .. } => summon(stage, s, me, rule_index, *n),
        Action::MoveTo { to, ticks } => {
            let mut order = MoveOrder {
                to: *to,
                ticks_left: (*ticks).max(1),
            };
            match me {
                Who::Boss => {
                    let b = s.boss.as_mut().expect("a boss");
                    order.to = boss::boss_point(b, order.to);
                    b.order = Some(order);
                }
                _ => agent_mut(s, me).order = Some(order),
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
    stage: &World,
    s: &mut State,
    caster: Who,
    target: Who,
    skill: &Skill,
    events: &mut Vec<DomainEvent>,
) {
    let source = s.id_of(caster);
    let mut apply = |s: &mut State, kind, value_pct: u16, duration: u32| {
        s.add_status(target, kind, source, value_pct, duration, events);
        s.raise_aggro(source, stage.tuning.aggro_per_buff);
    };
    match skill {
        Skill::Shot { pattern, damage } => {
            attacks::start(stage, s, caster, 0, target, pattern, *damage, events);
        }
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
                s.raise_aggro(
                    source,
                    healed.saturating_mul(stage.tuning.aggro_per_heal_point),
                );
            }
        }
    }
}

/// An enemy from a wave, or the boss, summons `n` more enemies where it stands. Companions and
/// summoned agents cannot summon in this version (the validator already keeps summoned agents
/// from it).
fn summon(stage: &World, s: &mut State, me: Who, rule_index: usize, n: u8) -> bool {
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
    for _ in 0..n {
        if !spawn_enemy(stage, s, key, at) {
            break;
        }
    }
    true
}
