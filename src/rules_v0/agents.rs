//! Agents (ADR-036 tier 1): a movement and an ordered rule list. Each tick the first rule whose
//! condition holds, whose cooldown and charges allow it, and whose selector finds a target,
//! fires; the others wait. Agents are evaluated in entity-ID order (companions, then enemies),
//! each rule list in order.

use crate::behaviour::{Action, Agent, Condition, Movement, Rule};
use crate::engine::{AgentKey, AgentSource, AgentState, MoveOrder, RuleState, State, UNLIMITED};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::stage::Stage;
use crate::trig;
use crate::validate::limits::{FIELD_H, FIELD_W};

use super::act::perform;
use super::entity::{Who, speed_pct};
use super::select::select;

/// An orbiting agent turns this much a tick, in 1/256 degree: 1.5 degrees.
const ORBIT_STEP: i32 = 384;
/// A following agent moves at most this far a tick: 4 pixels.
const FOLLOW_SPEED: i64 = Fx::px(4).0 as i64;

/// The definition an agent runs.
pub(crate) fn agent_def(stage: &Stage, key: AgentKey) -> &Agent {
    let wave_enemy = |i: u32| &stage.waves[i as usize].enemy;
    match key.source {
        AgentSource::Companion => &stage.companions[key.index as usize],
        AgentSource::Wave => wave_enemy(key.index),
        AgentSource::BossRule => {
            let action = stage
                .boss
                .as_ref()
                .and_then(|b| b.phases.get(key.index as usize))
                .and_then(|p| p.rules.get(key.rule as usize))
                .map(|r| &r.action);
            match action {
                Some(Action::Summon { agent, .. }) => agent,
                _ => unreachable!("a boss summon names the summon rule that made it"),
            }
        }
        AgentSource::Summoned => {
            match wave_enemy(key.index)
                .rules
                .get(key.rule as usize)
                .map(|r| &r.action)
            {
                Some(Action::Summon { agent, .. }) => agent,
                _ => wave_enemy(key.index),
            }
        }
    }
}

pub(super) fn new_agent(
    id: EntityId,
    key: AgentKey,
    def: &Agent,
    at: Point,
    angle: i32,
) -> AgentState {
    AgentState {
        id,
        key,
        at,
        hp: def.hp,
        max_hp: def.hp,
        age: 0,
        rules: new_rule_states(&def.rules),
        statuses: Vec::new(),
        aggro: 0,
        order: None,
        orbit_angle: angle,
    }
}

fn agent(s: &State, who: Who) -> &AgentState {
    match who {
        Who::Companion(i) => &s.companions[i],
        Who::Enemy(i) => &s.enemies[i],
        Who::Player | Who::Boss | Who::BossPart(_) => {
            unreachable!("only companions and enemies are agents")
        }
    }
}

pub(super) fn agent_mut(s: &mut State, who: Who) -> &mut AgentState {
    match who {
        Who::Companion(i) => &mut s.companions[i],
        Who::Enemy(i) => &mut s.enemies[i],
        Who::Player | Who::Boss | Who::BossPart(_) => {
            unreachable!("only companions and enemies are agents")
        }
    }
}

/// Every agent moves, then acts, in entity-ID order. Agents summoned this tick start next tick.
pub(super) fn act(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    for i in 0..s.companions.len() {
        act_one(stage, s, Who::Companion(i), events);
    }
    for i in 0..s.enemies.len() {
        act_one(stage, s, Who::Enemy(i), events);
    }
    // Enemies that have left the field are gone, whether or not they were killed.
    s.enemies.retain(|e| super::inside_with_margin(e.at));
}

fn act_one(stage: &Stage, s: &mut State, me: Who, events: &mut Vec<DomainEvent>) {
    let def = agent_def(stage, agent(s, me).key);
    {
        let a = agent_mut(s, me);
        a.age = a.age.saturating_add(1);
        for r in &mut a.rules {
            r.cooldown_left = r.cooldown_left.saturating_sub(1);
        }
    }
    move_agent(s, me, def);
    fire_first_rule(stage, s, me, &def.rules, events);
}

/// The rule states of `me`: an agent's, or the boss's for the phase it is in.
fn rule_states(s: &mut State, me: Who) -> &mut Vec<RuleState> {
    match me {
        Who::Boss => &mut s.boss.as_mut().expect("a boss").rules,
        _ => &mut agent_mut(s, me).rules,
    }
}

/// Fires the first rule of `rules` that can (ADR-036): its condition holds, its cooldown and
/// charges allow it, its selector finds a target and its action can be done. Agents and the
/// boss's phases use the same rule lists, so they share this.
pub(super) fn fire_first_rule(
    stage: &Stage,
    s: &mut State,
    me: Who,
    rules: &[Rule],
    events: &mut Vec<DomainEvent>,
) {
    for (i, rule) in rules.iter().enumerate() {
        let st = rule_states(s, me)[i];
        if st.cooldown_left > 0 || st.charges_left == 0 || (rule.once && st.fired) {
            continue;
        }
        if !condition_holds(s, me, &rule.when, stage) {
            continue;
        }
        let Some(target) = select(stage, s, me, &rule.target) else {
            continue;
        };
        if !perform(stage, s, me, target, i, &rule.action, events) {
            continue;
        }
        let id = s.id_of(me);
        let st = &mut rule_states(s, me)[i];
        st.cooldown_left = rule.cooldown_ticks;
        if st.charges_left != UNLIMITED {
            st.charges_left -= 1;
        }
        st.fired = true;
        events.push(DomainEvent::RuleFired {
            agent: id,
            rule: u8::try_from(i).unwrap_or(u8::MAX),
            target: Some(s.id_of(target)),
        });
        break;
    }
}

/// Fresh rule states for a list of rules.
pub(super) fn new_rule_states(rules: &[Rule]) -> Vec<RuleState> {
    rules
        .iter()
        .map(|r| RuleState {
            cooldown_left: 0,
            charges_left: r.charges.map_or(UNLIMITED, u32::from),
            fired: false,
        })
        .collect()
}

/// Ticks since an agent spawned, or since the boss began its phase.
fn time_since_start(s: &State, me: Who) -> u32 {
    match me {
        Who::Companion(_) | Who::Enemy(_) => agent(s, me).age,
        Who::Boss => s.boss.as_ref().map_or(0, |b| b.phase_age),
        Who::Player | Who::BossPart(_) => 0,
    }
}

pub(super) fn condition_holds(s: &State, me: Who, cond: &Condition, stage: &Stage) -> bool {
    match cond {
        Condition::Always => true,
        Condition::HpBelow { who, pct } => select(stage, s, me, who).is_some_and(|w| {
            // Strictly below `pct` percent of the maximum.
            u64::from(s.hp(w)) * 100 < u64::from(*pct) * u64::from(s.max_hp(w))
        }),
        Condition::HasStatus { who, status } => select(stage, s, me, who)
            .is_some_and(|w| s.statuses(w).iter().any(|st| st.kind.name() == status.0)),
        Condition::AggroAbove { who, value } => {
            select(stage, s, me, who).is_some_and(|w| s.aggro(w) > *value)
        }
        Condition::TimeAbove { ticks } => time_since_start(s, me) > *ticks,
        Condition::PlayerFiring { min_ticks } => s.player.firing_streak >= *min_ticks,
        Condition::PartBroken { part } => {
            let index = stage
                .boss
                .iter()
                .flat_map(|b| &b.parts)
                .position(|p| p.id == *part);
            match (index, &s.boss) {
                (Some(i), Some(boss)) => boss.parts.get(i).is_some_and(|p| p.hp == 0),
                _ => false,
            }
        }
    }
}

/// One tick of a `move_to`: straight to the point, in equal steps, the last landing exactly on
/// it. Returns the new place and what is left of the order.
pub(super) fn step_order(at: Point, order: MoveOrder) -> (Point, Option<MoveOrder>) {
    if order.ticks_left <= 1 {
        return (order.to, None);
    }
    let left = i64::from(order.ticks_left);
    let step =
        |from: Fx, to: Fx| Fx(((i64::from(to.0) - i64::from(from.0)) / left) as i32 + from.0);
    let next = Point {
        x: step(at.x, order.to.x),
        y: step(at.y, order.to.y),
    };
    (
        next,
        Some(MoveOrder {
            to: order.to,
            ticks_left: order.ticks_left - 1,
        }),
    )
}

fn clamp_to_field(p: Point) -> Point {
    Point {
        x: p.x.clamp(Fx(0), Fx(FIELD_W)),
        y: p.y.clamp(Fx(0), Fx(FIELD_H)),
    }
}

fn move_agent(s: &mut State, me: Who, def: &Agent) {
    let player_at = s.player.at;
    let keep = speed_pct(s.statuses(me));
    let a = agent_mut(s, me);
    if let Some(order) = a.order {
        (a.at, a.order) = step_order(a.at, order);
        a.at = clamp_to_field(a.at);
        return;
    }
    match def.movement {
        Movement::Hold { at } => a.at = at,
        Movement::Straight { vx, vy } => {
            let scaled = |v: Fx| (i64::from(v.0) * keep / 100) as i32;
            a.at.x = a.at.x.saturating_add(Fx(scaled(vx)));
            a.at.y = a.at.y.saturating_add(Fx(scaled(vy)));
        }
        Movement::Orbit { radius } => {
            let step = (i64::from(ORBIT_STEP) * keep / 100) as i32;
            a.orbit_angle = (a.orbit_angle + step).rem_euclid(trig::PER_TURN);
            let (sin, cos) = trig::sin_cos(a.orbit_angle);
            let offset = |component: i32| {
                (i64::from(radius.0) * i64::from(component) / i64::from(trig::SCALE)) as i32
            };
            a.at = Point {
                x: player_at.x.saturating_add(Fx(offset(cos))),
                y: player_at.y.saturating_add(Fx(offset(sin))),
            };
        }
        Movement::Follow { distance } => {
            let dx = i64::from(player_at.x.0) - i64::from(a.at.x.0);
            let dy = i64::from(player_at.y.0) - i64::from(a.at.y.0);
            let dist =
                i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i64::MAX);
            let gap = dist - i64::from(distance.0);
            if gap > 0 {
                let step = gap.min(FOLLOW_SPEED * keep / 100);
                a.at.x = a.at.x.saturating_add(Fx((dx * step / dist) as i32));
                a.at.y = a.at.y.saturating_add(Fx((dy * step / dist) as i32));
            }
        }
    }
    // Enemies may leave the field (and are removed when they do); companions stay in it.
    if matches!(me, Who::Companion(_)) {
        a.at = clamp_to_field(a.at);
    }
}
