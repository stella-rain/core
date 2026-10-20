//! Agents (ADR-036 tier 1): a movement and an ordered rule list. Each tick the first rule whose
//! condition holds, whose cooldown and charges allow it, and whose selector finds a target,
//! fires; the others wait. Agents are evaluated in entity-ID order (companions, then enemies),
//! each rule list in order.

use crate::behaviour::{Action, Agent, BossCall, Condition, Elite, Guardians, Movement, Rule};
use crate::engine::{
    AgentKey, AgentSource, AgentState, EliteState, LeaveState, MoveOrder, RuleState, ShrineState,
    State, UNLIMITED,
};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::stage::Stage;
use crate::trig;

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
        AgentSource::Guardian => match &wave_enemy(key.index).movement {
            Movement::Shrine { guardians, .. } => &guardians.enemy,
            _ => unreachable!("a guardian names the shrine that rang it"),
        },
        AgentSource::Escort => match &wave_enemy(key.index).elite {
            Some(Elite {
                escort: Some(escort),
                ..
            }) => &escort.enemy,
            _ => unreachable!("an escort names the leader that brought it"),
        },
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
        defence: def.defence,
        age: 0,
        rules: new_rule_states(&def.rules),
        statuses: Vec::new(),
        aggro: 0,
        order: None,
        orbit_angle: angle,
        crowded: false,
        elite: def.elite.as_ref().map(|elite| EliteState {
            home: at,
            called: false,
            boss_call: elite.boss_call,
            disguised: elite.mimic.is_some(),
        }),
        leave: (matches!(def.movement, Movement::FleeWhenDisturbed { .. })
            || def
                .elite
                .as_ref()
                .is_some_and(|e| e.boss_call == BossCall::Leave))
        .then_some(LeaveState {
            disturbed: None,
            go: false,
        }),
        shrine: matches!(def.movement, Movement::Shrine { .. }).then(|| {
            Box::new(ShrineState {
                fight: None,
                guardians: Vec::new(),
            })
        }),
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
    // After the agents, so the guardians that appear start next tick, as summoned ones do.
    shrines(stage, s, events);
    // Enemies that have left the map are gone, whether or not they were killed.
    let holds = stage.map().holds();
    s.enemies.retain(|e| holds(e.at));
    leave(stage, s, events);
}

/// The shrines' fights (ADR-090). An untouched shrine waits for the main character's body to
/// overlap its own, then activates: its guardians are placed on the ring, in order, through the
/// usual spawn path, so the cap on enemies refuses the ones that do not fit (the ring is then
/// short of them, and the first refusal ends it), and its timer starts. Each later tick the
/// timer counts one more; when no guardian that spawned is alive the shrine is done (its hit
/// points go to 0 and the tick's hits credit the elite's kill), else when the timer reaches
/// `fight_ticks` it leaves unkilled. No random number is drawn.
fn shrines(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    let ids: Vec<EntityId> = s
        .enemies
        .iter()
        .filter(|e| e.hp > 0 && e.shrine.is_some())
        .map(|e| e.id)
        .collect();
    for id in ids {
        let Some(i) = s.enemies.iter().position(|e| e.id == id) else {
            continue;
        };
        let e = &s.enemies[i];
        let def = agent_def(stage, e.key);
        let Movement::Shrine {
            fight_ticks,
            guardians,
        } = &def.movement
        else {
            continue;
        };
        let fight = e.shrine.as_ref().and_then(|sh| sh.fight);
        let Some(fight) = fight else {
            if !super::overlaps(e.at, def.radius.0, s.player.at, super::PLAYER_RADIUS) {
                continue;
            }
            let (home, wave) = (e.at, e.key.index);
            events.push(DomainEvent::ShrineActivated { elite: id });
            let key = AgentKey {
                source: AgentSource::Guardian,
                index: wave,
                rule: 0,
            };
            let rung = ring_up(stage, s, key, guardians, home);
            if let Some(sh) = s.enemies[i].shrine.as_mut() {
                sh.fight = Some(0);
                sh.guardians = rung;
            }
            continue;
        };
        let fight = fight.saturating_add(1);
        let cleared = s.enemies[i].shrine.as_ref().is_some_and(|sh| {
            !s.enemies
                .iter()
                .any(|g| g.hp > 0 && sh.guardians.contains(&g.id))
        });
        if cleared {
            s.enemies[i].hp = 0;
            events.push(DomainEvent::Died { entity: id });
        } else if fight >= *fight_ticks {
            s.enemies.remove(i);
            events.push(DomainEvent::EliteLeft { elite: id });
        } else if let Some(sh) = s.enemies[i].shrine.as_mut() {
            sh.fight = Some(fight);
        }
    }
}

/// Places `guardians` (a shrine's, or a pack leader's escort) on their ring around `home`, in
/// order, through the usual spawn path: the cap on enemies refuses the ones that do not fit and
/// the first refusal ends the ring. Returns the ones that spawned. No random number is drawn.
pub(super) fn ring_up(
    stage: &Stage,
    s: &mut State,
    key: AgentKey,
    guardians: &Guardians,
    home: Point,
) -> Vec<EntityId> {
    let mut rung = Vec::new();
    for k in 0..u32::from(guardians.count) {
        let at = ring_point(stage, home, guardians.radius, k, u32::from(guardians.count));
        let next = EntityId(s.next_id);
        if !super::spawn_enemy(stage, s, key, &guardians.enemy, at) {
            break;
        }
        rung.push(next);
    }
    rung
}

/// Guardian `k` of `count`: on the ring of `radius` around `at`, evenly spaced from the east,
/// moved onto the map.
fn ring_point(stage: &Stage, at: Point, radius: Fx, k: u32, count: u32) -> Point {
    let angle = i64::from(k) * i64::from(trig::PER_TURN) / i64::from(count.max(1));
    let (sin, cos) = trig::sin_cos(i32::try_from(angle).unwrap_or(0));
    let offset = |unit: i32| {
        i32::try_from(i64::from(radius.0) * i64::from(unit) / i64::from(trig::SCALE)).unwrap_or(0)
    };
    stage.map().clamp(Point {
        x: at.x.saturating_add(Fx(offset(cos))),
        y: at.y.saturating_add(Fx(offset(sin))),
    })
}

/// Elites that leave go, unkilled: a boss came or called, or they were disturbed long enough
/// ago (ADR-090). They are gone before any hit of this tick lands, so a leaving elite gives no
/// kill reward.
fn leave(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    let mut gone = Vec::new();
    for e in &s.enemies {
        let Some(state) = e.leave else {
            continue;
        };
        let timed_out = match agent_def(stage, e.key).movement {
            Movement::FleeWhenDisturbed {
                leave_after_ticks, ..
            } => state.disturbed.is_some_and(|t| t >= leave_after_ticks),
            _ => false,
        };
        if state.go || timed_out {
            gone.push(e.id);
        }
    }
    if gone.is_empty() {
        return;
    }
    s.enemies.retain(|e| !gone.contains(&e.id));
    events.extend(
        gone.into_iter()
            .map(|elite| DomainEvent::EliteLeft { elite }),
    );
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
    // A disguised mimic does nothing but wait to be revealed; the tick it reveals it does not
    // act either, so it acts from the next tick (ADR-090).
    if agent(s, me).hidden() {
        reveal(s, me, def, events);
        return;
    }
    move_agent(stage, s, me, def);
    if waits(s, me, def) {
        return;
    }
    fire_first_rule(stage, s, me, &def.rules, events);
}

/// A disguised mimic reveals the first tick the main character's centre is within its reveal
/// radius (the edge counts), for good. A mimic that reveals after the boss's call joins it.
fn reveal(s: &mut State, me: Who, def: &Agent, events: &mut Vec<DomainEvent>) {
    let Some(mimic) = def.elite.as_ref().and_then(|e| e.mimic.as_ref()) else {
        return;
    };
    let player = s.player.at;
    let called = s.boss.as_ref().and_then(|b| b.elites_called) == Some(true);
    let id = s.id_of(me);
    let a = agent_mut(s, me);
    if !super::overlaps(a.at, mimic.reveal_radius.0, player, 0) {
        return;
    }
    if let Some(el) = a.elite.as_mut() {
        el.disguised = false;
        el.called = called;
    }
    events.push(DomainEvent::MimicRevealed { elite: id });
}

/// An elite not called yet waits while the main character is beyond its range.
fn waits(s: &State, me: Who, def: &Agent) -> bool {
    let (Some(elite), Some(st)) = (def.elite.as_ref(), agent(s, me).elite) else {
        return false;
    };
    !st.called && !super::overlaps(agent(s, me).at, elite.range.0, s.player.at, 0)
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

/// `from` moved straight toward `to` by at most `most`, landing exactly on it when it is that
/// near.
fn step_toward(from: Point, to: Point, most: i64) -> Point {
    let dx = i64::from(to.x.0) - i64::from(from.x.0);
    let dy = i64::from(to.y.0) - i64::from(from.y.0);
    let dist = i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i64::MAX);
    if dist <= most {
        return to;
    }
    Point {
        x: from.x.saturating_add(Fx((dx * most / dist) as i32)),
        y: from.y.saturating_add(Fx((dy * most / dist) as i32)),
    }
}

/// An attack phase's `move_pct` slows the agent like a slow status does; at 0 it holds still,
/// a `move_to` order included (ADR-055).
///
/// A companion with `Hold` or `Orbit` is set on its place. An enemy moves to it, at most 4
/// pixels a tick (`Orbit`: its orbit step plus 4 pixels), so that a push carries over to the
/// next tick (ADR-058). An enemy that the last push-apart found crowded does not take that step;
/// `Straight` and a `move_to` order are not held back.
fn move_agent(stage: &Stage, s: &mut State, me: Who, def: &Agent) {
    let map = stage.map();
    let player_at = s.player.at;
    let attacking = super::attacks::move_pct(s, s.id_of(me));
    if attacking == 0 {
        return;
    }
    let keep = speed_pct(s.statuses(me)) * attacking / 100;
    let a = agent_mut(s, me);
    if let Some(order) = a.order {
        (a.at, a.order) = step_order(a.at, order);
        a.at = map.clamp(a.at);
        return;
    }
    // A crowded enemy yields: it does not walk into the neighbours the push just parted it from.
    // Only enemies are ever crowded, and one that flies straight goes to no place, so it flies on.
    let yields = a.crowded;
    if let Movement::FleeWhenDisturbed {
        disturb_radius,
        speed,
        ..
    } = def.movement
    {
        let Some(leave) = a.leave.as_mut() else {
            return;
        };
        leave.disturbed = match leave.disturbed {
            Some(t) => Some(t.saturating_add(1)),
            None if a.hp < a.max_hp || super::overlaps(a.at, disturb_radius.0, player_at, 0) => {
                Some(0)
            }
            None => None,
        };
        if leave.disturbed.is_none() {
            // Undisturbed it holds where it spawned, as an elite not called does.
            if let (false, Some(st)) = (yields, a.elite) {
                a.at = step_toward(a.at, st.home, FOLLOW_SPEED * keep / 100);
            }
            return;
        }
        // Directly away from the main character; on top of it, up.
        let (dx, mut dy) = (
            i64::from(a.at.x.0) - i64::from(player_at.x.0),
            i64::from(a.at.y.0) - i64::from(player_at.y.0),
        );
        if dx == 0 && dy == 0 {
            dy = -1;
        }
        let dist = i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i64::MAX);
        let step = i64::from(speed.0) * keep / 100;
        a.at = map.clamp(Point {
            x: a.at.x.saturating_add(Fx((dx * step / dist) as i32)),
            y: a.at.y.saturating_add(Fx((dy * step / dist) as i32)),
        });
        return;
    }
    // An elite not called yet holds where it spawned, whatever its movement.
    if let Some(st) = a.elite.filter(|st| !st.called) {
        if !yields {
            a.at = step_toward(a.at, st.home, FOLLOW_SPEED * keep / 100);
        }
        return;
    }
    match def.movement {
        // A shrine is a stay elite: never called, so it held its place above.
        Movement::FleeWhenDisturbed { .. } | Movement::Shrine { .. } => {}
        Movement::Hold { at } => {
            a.at = match me {
                Who::Enemy(_) if yields => a.at,
                Who::Enemy(_) => step_toward(a.at, at, FOLLOW_SPEED * keep / 100),
                _ => at,
            }
        }
        Movement::Straight { vx, vy } => {
            let scaled = |v: Fx| (i64::from(v.0) * keep / 100) as i32;
            a.at.x = a.at.x.saturating_add(Fx(scaled(vx)));
            a.at.y = a.at.y.saturating_add(Fx(scaled(vy)));
        }
        Movement::Orbit { radius } => {
            let on_circle = |angle: i32| {
                let (sin, cos) = trig::sin_cos(angle);
                let offset = |component: i32| {
                    (i64::from(radius.0) * i64::from(component) / i64::from(trig::SCALE)) as i32
                };
                (offset(cos), offset(sin))
            };
            let was = on_circle(a.orbit_angle);
            let step = (i64::from(ORBIT_STEP) * keep / 100) as i32;
            a.orbit_angle = (a.orbit_angle + step).rem_euclid(trig::PER_TURN);
            let now = on_circle(a.orbit_angle);
            let place = Point {
                x: player_at.x.saturating_add(Fx(now.0)),
                y: player_at.y.saturating_add(Fx(now.1)),
            };
            a.at = match me {
                // Its orbit step, the chord its place moved along the circle, plus the 4 pixels.
                // Its place circles on while it yields.
                Who::Enemy(_) if yields => a.at,
                Who::Enemy(_) => {
                    let (dx, dy) = (i64::from(now.0 - was.0), i64::from(now.1 - was.1));
                    let chord =
                        i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(0);
                    step_toward(a.at, place, chord + FOLLOW_SPEED * keep / 100)
                }
                _ => place,
            };
        }
        Movement::Follow { distance } => {
            let dx = i64::from(player_at.x.0) - i64::from(a.at.x.0);
            let dy = i64::from(player_at.y.0) - i64::from(a.at.y.0);
            let dist =
                i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i64::MAX);
            let gap = dist - i64::from(distance.0);
            if gap > 0 && !yields {
                let step = gap.min(FOLLOW_SPEED * keep / 100);
                a.at.x = a.at.x.saturating_add(Fx((dx * step / dist) as i32));
                a.at.y = a.at.y.saturating_add(Fx((dy * step / dist) as i32));
            }
        }
    }
    // Enemies may leave the map (and are removed when they do); companions stay on it.
    if matches!(me, Who::Companion(_)) {
        a.at = map.clamp(a.at);
    }
}
