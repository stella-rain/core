//! The boss (ADR-036 tier 2): body parts and an ordered list of phases. A phase ends when its
//! `until` condition holds, and phases only move forward; the last lasts until the boss dies.
//! The boss acts before the companions and the enemies (its entity ID is the lowest).
//!
//! Each tick, in this order: the phase may end; the timeline goes on; the phase's rules fire
//! (the same rules as an agent's, with the boss as the agent).
//!
//! - **Entering a phase** cancels the attacks the boss was carrying out (bullets already
//!   fired stay, and so do the attacks they will start), then runs its transitions:
//!   `clear_bullets` removes every enemy bullet, `invulnerable_ticks` makes the boss and its
//!   parts take no harm for that long. The first phase is entered on the first tick without a
//!   `PhaseChanged` event; later ones announce themselves with one.
//! - **The timeline** is carried out a step at a time. `move_to`, `wait` and `telegraph` hold
//!   it for their ticks (`move_to` takes the boss there in that time, `telegraph` announces
//!   itself with an event); `attack` starts an attack aimed at the main character and goes on
//!   at once, so a `wait` is what lets it play out; `loop` starts the timeline again. At most
//!   `MAX_TIMELINE_STEPS_PER_TICK` steps are taken in a tick, so a timeline with nothing to
//!   wait for does not hang the tick. A timeline that runs out is done; the rules go on.
//! - **Time** for the phase's conditions and attacks (`time_above`, `phase_time`) is the ticks
//!   since the phase began.
//!
//! After that, a boss with `call_elites` that is still alive and has not called yet calls its
//! elites (ADR-068) once its time is up or its hit points are below its share: every elite
//! alive is called, and so is every one that spawns later (`spawn_enemy`). A called elite
//! leaves its place and acts at any distance, by its own movement and rules. An elite whose
//! `boss_call` is `leave` is not called: it leaves the map; one whose `boss_call` is `stay` (the shrine)
//! is not called and stays (ADR-090).

use super::World;
use crate::attack::AttackRef;
use crate::behaviour::BossCall;
use crate::engine::{BossState, MoveOrder, Origin, State};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::snapshot::PatternView;
use crate::stage::{Step, Transition};

use super::agents::{condition_holds, fire_first_rule, new_rule_states, step_order};
use super::attacks;
use super::entity::Who;

/// Timeline steps taken in one tick, so that a `loop` with nothing to wait for ends the tick.
const MAX_TIMELINE_STEPS_PER_TICK: u32 = 16;

pub(super) fn act(stage: &World, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(def) = s.boss_def(stage) else {
        return;
    };
    {
        // An attack phase with `move_pct` 0 holds the boss where it is (ADR-055).
        let rooted = s
            .boss
            .as_ref()
            .is_some_and(|b| attacks::move_pct(stage, s, b.id) == 0);
        let Some(b) = s.boss.as_mut() else {
            return;
        };
        if b.hp == 0 {
            return;
        }
        b.age = b.age.saturating_add(1);
        b.phase_age = b.phase_age.saturating_add(1);
        b.invulnerable = b.invulnerable.saturating_sub(1);
        for r in &mut b.rules {
            r.cooldown_left = r.cooldown_left.saturating_sub(1);
        }
        if let Some(order) = b.order.filter(|_| !rooted) {
            (b.at, b.order) = step_order(b.at, order);
        }
    }
    // The first phase begins on the boss's first tick: tick 1 of a stage's boss, the tick it
    // comes of a horde's (`horde::spawn_boss`); phase 0 is at age 1 on no other tick.
    if s.boss
        .as_ref()
        .is_some_and(|b| b.phase == 0 && b.phase_age == 1)
    {
        begin_phase(stage, def, s, 0, events);
    }
    maybe_end_phase(stage, s, events);
    timeline(stage, s, events);
    let phase = s.boss.as_ref().map_or(0, |b| b.phase as usize);
    if let Some(p) = def.phases.get(phase) {
        fire_first_rule(stage, s, Who::Boss, &p.rules, events);
    }
}

/// `p`, a point of the boss's definition, where it is for this arrival: moved by the boss's
/// origin, in `i64` as the move was made when a `time_limit` boss was cloned and moved. A sum
/// outside `i32` leaves `p` where it was.
pub(super) fn boss_point(b: &BossState, p: Point) -> Point {
    let at = |v: Fx, d: Fx| Fx(i32::try_from(i64::from(v.0) + i64::from(d.0)).unwrap_or(v.0));
    Point {
        x: at(p.x, b.origin.x),
        y: at(p.y, b.origin.y),
    }
}

/// Calls the elites, once, when the boss's call is due.
pub(super) fn call_elites(stage: &World, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(call) = s.boss_def(stage).and_then(|b| b.call_elites) else {
        return;
    };
    let Some(b) = s.boss.as_mut() else {
        return;
    };
    if b.hp == 0 || b.no_call || b.elites_called != Some(false) {
        return;
    }
    // The boss's first tick is its age 1, so its time is up on its tick `after_ticks + 1`:
    // counted from its arrival, not from the run's first tick (ADR-103).
    let time_up = call
        .after_ticks
        .is_some_and(|t| b.age.saturating_sub(1) >= t);
    let low = call
        .below_pct
        .is_some_and(|pct| u64::from(b.hp) * 100 < u64::from(pct) * u64::from(b.max_hp));
    if !(time_up || low) {
        return;
    }
    b.elites_called = Some(true);
    let mut elites = 0u32;
    for e in &mut s.enemies {
        match (e.elite.as_mut(), e.leave.as_mut()) {
            (Some(el), Some(l)) if el.boss_call == BossCall::Leave => l.go = true,
            // An elite that stays is not called, and is not counted.
            (Some(el), _) if el.boss_call == BossCall::Stay => {}
            // A mimic still in disguise is not called; it joins when it reveals.
            (Some(el), _) if el.disguised => {}
            (Some(el), _) => {
                el.called = true;
                elites = elites.saturating_add(1);
            }
            _ => {}
        }
    }
    events.push(DomainEvent::ElitesCalled { elites });
}

/// The content ID the boss's call of its elites is announced under (ADR-103).
pub(crate) const CALL_ELITES: &str = "call_elites";

/// The big patterns announced now, the call of the elites first (ADR-103): each with the ticks
/// its count-down started with and the ticks left until it starts. It is a view of the state;
/// nothing is stored.
pub(crate) fn announced(stage: &World, s: &State) -> Vec<PatternView> {
    let Some((def, b)) = s.boss_pair(stage) else {
        return Vec::new();
    };
    if b.hp == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let (Some(total), Some(false)) =
        (def.call_elites.and_then(|c| c.after_ticks), b.elites_called)
    {
        let remaining = total.saturating_sub(b.age.saturating_sub(1));
        if remaining > 0 {
            out.push(PatternView {
                id: CALL_ELITES.into(),
                total,
                remaining,
            });
        }
    }
    out.extend(next_big_attack(def, b));
    out
}

/// The next attack step of the phase's timeline when it is a big preset and only timed steps
/// come before it: its count-down is the ticks of those steps, the one in progress counted
/// from what is left of it.
fn next_big_attack(def: &crate::stage::Boss, b: &BossState) -> Option<PatternView> {
    let timeline = &def.phases.get(b.phase as usize)?.timeline;
    let held = |step: &Step| match step {
        Step::MoveTo { ticks, .. } => Some((*ticks).max(1)),
        Step::Wait { ticks } | Step::Telegraph { ticks } => Some(*ticks),
        Step::Attack { .. } | Step::Loop => None,
    };
    let at = (b.step as usize..timeline.len()).find(|&i| held(&timeline[i]).is_none())?;
    let Step::Attack {
        attack: AttackRef::Preset { id, .. },
        big: true,
    } = &timeline[at]
    else {
        return None;
    };
    let ahead: u32 = timeline[b.step as usize..at]
        .iter()
        .filter_map(held)
        .fold(0, u32::saturating_add);
    let remaining = ahead.saturating_add(b.wait_left);
    let total = timeline[..at]
        .iter()
        .rev()
        .map_while(held)
        .fold(0, u32::saturating_add);
    (remaining > 0).then(|| PatternView {
        id: id.clone(),
        total: total.max(remaining),
        remaining,
    })
}

/// Ends the phase if its `until` holds and there is a phase to go on to.
fn maybe_end_phase(stage: &World, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(def) = s.boss_def(stage) else {
        return;
    };
    let Some(phase) = s.boss.as_ref().map(|b| b.phase as usize) else {
        return;
    };
    let Some(until) = def.phases.get(phase).and_then(|p| p.until.as_ref()) else {
        return;
    };
    if phase + 1 < def.phases.len() && condition_holds(s, Who::Boss, until) {
        begin_phase(stage, def, s, phase + 1, events);
    }
}

/// Starts phase `index`: the old phase's attacks end, its rules are new, and its transitions run.
fn begin_phase(
    stage: &World,
    def: &crate::stage::Boss,
    s: &mut State,
    index: usize,
    events: &mut Vec<DomainEvent>,
) {
    let Some(phase) = def.phases.get(index) else {
        return;
    };
    let Some(b) = s.boss.as_mut() else {
        return;
    };
    let id = b.id;
    b.phase = u32::try_from(index).unwrap_or(u32::MAX);
    // The tick a phase begins on is its first: phase age 1, phase time 0.
    b.phase_age = 1;
    b.step = 0;
    b.wait_left = 0;
    b.order = None;
    b.rules = new_rule_states(&phase.rules);
    // The attacks the boss itself was making end with the phase.
    s.tasks
        .retain(|t| !(t.owner == id && t.origin == Origin::Owner));
    attacks::end_runs_of(stage, s, id);
    if index > 0 {
        events.push(DomainEvent::PhaseChanged {
            phase: u8::try_from(index).unwrap_or(u8::MAX),
        });
    }
    for transition in &phase.transition {
        match transition {
            Transition::ClearBullets => s.bullets.retain(|b| b.friendly),
            Transition::ClearZones => s.zones.retain(|z| z.friendly),
            Transition::InvulnerableTicks { ticks } => {
                if let Some(b) = s.boss.as_mut() {
                    b.invulnerable = b.invulnerable.max(*ticks);
                }
            }
        }
    }
}

/// Carries out the timeline up to its next wait.
fn timeline(stage: &World, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(def) = s.boss_def(stage) else {
        return;
    };
    // An `attack` step lasts as long as its attack (ADR-055), and so does any attack the
    // boss's rules started: one attack at a time.
    if s.boss.as_ref().is_some_and(|b| attacks::busy(s, b.id, 0)) {
        return;
    }
    let Some(b) = s.boss.as_mut() else {
        return;
    };
    let Some(phase) = def.phases.get(b.phase as usize) else {
        return;
    };
    // A wait of N ticks begun on tick T lets the next step run on tick T + N.
    if b.wait_left > 0 {
        b.wait_left -= 1;
        if b.wait_left > 0 {
            return;
        }
    }
    for _ in 0..MAX_TIMELINE_STEPS_PER_TICK {
        let Some(b) = s.boss.as_mut() else {
            return;
        };
        let Some(step) = phase.timeline.get(b.step as usize) else {
            return;
        };
        b.step += 1;
        match step {
            Step::MoveTo { to, ticks } => {
                let ticks = (*ticks).max(1);
                b.order = Some(MoveOrder {
                    to: boss_point(b, *to),
                    ticks_left: ticks,
                });
                b.wait_left = ticks;
                return;
            }
            Step::Wait { ticks } => {
                b.wait_left = *ticks;
                if *ticks > 0 {
                    return;
                }
            }
            Step::Telegraph { ticks } => {
                events.push(DomainEvent::Telegraph {
                    source: b.id,
                    ticks: *ticks,
                });
                b.wait_left = *ticks;
                if *ticks > 0 {
                    return;
                }
            }
            Step::Attack { attack, .. } => {
                if start_attack(stage, s, attack, events) {
                    return;
                }
            }
            Step::Loop => b.step = 0,
        }
    }
}

/// An attack of the boss's own, aimed at the main character, the run stats file's
/// `enemy_attack_damage` a bullet, on the phase's clock: `phase_time` is 0 on the first tick of the phase.
fn start_attack(
    stage: &World,
    s: &mut State,
    attack: &AttackRef,
    events: &mut Vec<DomainEvent>,
) -> bool {
    let age = s.boss.as_ref().map_or(0, |b| b.phase_age.saturating_sub(1));
    let damage = s.enemy_attack_damage;
    attacks::start_aged(
        stage,
        s,
        Who::Boss,
        0,
        Who::Player,
        attack,
        damage,
        age,
        events,
    )
}
