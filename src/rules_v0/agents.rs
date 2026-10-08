//! Agents (ADR-036 tier 1): a movement and an ordered rule list. Each tick the first rule whose
//! condition holds, whose cooldown and charges allow it, and whose selector finds a target,
//! fires; the others wait. Agents are evaluated in entity-ID order (companions, then enemies),
//! each rule list in order.

use crate::behaviour::{Action, Agent, Condition, Movement};
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
        rules: def
            .rules
            .iter()
            .map(|r| RuleState {
                cooldown_left: 0,
                charges_left: r.charges.map_or(UNLIMITED, u32::from),
                fired: false,
            })
            .collect(),
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
        Who::Player | Who::Boss => unreachable!("only companions and enemies are agents"),
    }
}

pub(super) fn agent_mut(s: &mut State, who: Who) -> &mut AgentState {
    match who {
        Who::Companion(i) => &mut s.companions[i],
        Who::Enemy(i) => &mut s.enemies[i],
        Who::Player | Who::Boss => unreachable!("only companions and enemies are agents"),
    }
}

/// Every agent moves, then acts, in entity-ID order. Agents summoned this tick start next tick.
pub(super) fn act(
    stage: &Stage,
    s: &mut State,
    events: &mut Vec<DomainEvent>,
    spawns_left: &mut u32,
) {
    for i in 0..s.companions.len() {
        act_one(stage, s, Who::Companion(i), events, spawns_left);
    }
    for i in 0..s.enemies.len() {
        act_one(stage, s, Who::Enemy(i), events, spawns_left);
    }
    // Enemies that have left the field are gone, whether or not they were killed.
    s.enemies.retain(|e| super::inside_with_margin(e.at));
}

fn act_one(
    stage: &Stage,
    s: &mut State,
    me: Who,
    events: &mut Vec<DomainEvent>,
    spawns_left: &mut u32,
) {
    let def = agent_def(stage, agent(s, me).key);
    {
        let a = agent_mut(s, me);
        a.age = a.age.saturating_add(1);
        for r in &mut a.rules {
            r.cooldown_left = r.cooldown_left.saturating_sub(1);
        }
    }
    move_agent(s, me, def);
    for (i, rule) in def.rules.iter().enumerate() {
        let st = agent(s, me).rules[i];
        if st.cooldown_left > 0 || st.charges_left == 0 || (rule.once && st.fired) {
            continue;
        }
        if !condition_holds(s, me, &rule.when, stage) {
            continue;
        }
        let Some(target) = select(stage, s, me, &rule.target) else {
            continue;
        };
        if !perform(stage, s, me, target, i, &rule.action, events, spawns_left) {
            continue;
        }
        let id = s.id_of(me);
        let st = &mut agent_mut(s, me).rules[i];
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

fn condition_holds(s: &State, me: Who, cond: &Condition, stage: &Stage) -> bool {
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
        Condition::TimeAbove { ticks } => agent(s, me).age > *ticks,
        Condition::PlayerFiring { min_ticks } => s.player.firing_streak >= *min_ticks,
        // Boss parts arrive with boss timelines (core#17).
        Condition::PartBroken { .. } => false,
    }
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
        // Straight to the point, in equal steps; the last step lands exactly on it.
        if order.ticks_left <= 1 {
            a.at = order.to;
            a.order = None;
        } else {
            let left = i64::from(order.ticks_left);
            let step = |from: Fx, to: Fx| Fx((i64::from(to.0 - from.0) / left) as i32 + from.0);
            a.at = Point {
                x: step(a.at.x, order.to.x),
                y: step(a.at.y, order.to.y),
            };
            a.order = Some(MoveOrder {
                to: order.to,
                ticks_left: order.ticks_left - 1,
            });
        }
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
