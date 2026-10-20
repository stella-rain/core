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

use crate::attack::AttackRef;
use crate::behaviour::BossCall;
use crate::engine::{MoveOrder, Origin, State};
use crate::event::DomainEvent;
use crate::stage::Stage;
use crate::stage::{Step, Transition};

use super::ENEMY_ATTACK_DAMAGE;
use super::agents::{condition_holds, fire_first_rule, new_rule_states, step_order};
use super::attacks;
use super::entity::Who;

/// Timeline steps taken in one tick, so that a `loop` with nothing to wait for ends the tick.
const MAX_TIMELINE_STEPS_PER_TICK: u32 = 16;

pub(super) fn act(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(def) = stage.boss.as_ref() else {
        return;
    };
    {
        // An attack phase with `move_pct` 0 holds the boss where it is (ADR-055).
        let rooted = s
            .boss
            .as_ref()
            .is_some_and(|b| attacks::move_pct(s, b.id) == 0);
        let Some(b) = s.boss.as_mut() else {
            return;
        };
        if b.hp == 0 {
            return;
        }
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
        begin_phase(def, s, 0, events);
    }
    maybe_end_phase(stage, s, events);
    timeline(stage, s, events);
    let phase = s.boss.as_ref().map_or(0, |b| b.phase as usize);
    if let Some(p) = def.phases.get(phase) {
        fire_first_rule(stage, s, Who::Boss, &p.rules, events);
    }
}

/// Calls the elites, once, when the boss's call is due.
pub(super) fn call_elites(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(call) = stage.boss.as_ref().and_then(|b| b.call_elites) else {
        return;
    };
    let Some(b) = s.boss.as_mut() else {
        return;
    };
    if b.hp == 0 || b.elites_called != Some(false) {
        return;
    }
    // The boss's first tick is tick 1, so its time is up on tick `after_ticks + 1`.
    let time_up = call
        .after_ticks
        .is_some_and(|t| s.tick.saturating_sub(1) >= t);
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

/// Ends the phase if its `until` holds and there is a phase to go on to.
fn maybe_end_phase(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(def) = stage.boss.as_ref() else {
        return;
    };
    let Some(phase) = s.boss.as_ref().map(|b| b.phase as usize) else {
        return;
    };
    let Some(until) = def.phases.get(phase).and_then(|p| p.until.as_ref()) else {
        return;
    };
    if phase + 1 < def.phases.len() && condition_holds(s, Who::Boss, until, stage) {
        begin_phase(def, s, phase + 1, events);
    }
}

/// Starts phase `index`: the old phase's attacks end, its rules are new, and its transitions run.
fn begin_phase(
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
    attacks::end_runs_of(s, id);
    if index > 0 {
        events.push(DomainEvent::PhaseChanged {
            phase: u8::try_from(index).unwrap_or(u8::MAX),
        });
    }
    for transition in &phase.transition {
        match transition {
            Transition::ClearBullets => s.bullets.retain(|b| b.friendly),
            Transition::InvulnerableTicks { ticks } => {
                if let Some(b) = s.boss.as_mut() {
                    b.invulnerable = b.invulnerable.max(*ticks);
                }
            }
        }
    }
}

/// Carries out the timeline up to its next wait.
fn timeline(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(def) = stage.boss.as_ref() else {
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
                    to: *to,
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
            Step::Attack { attack } => {
                if start_attack(s, attack, events) {
                    return;
                }
            }
            Step::Loop => b.step = 0,
        }
    }
}

/// An attack of the boss's own, aimed at the main character, `ENEMY_ATTACK_DAMAGE` a bullet (1
/// at the design values), on the phase's clock: `phase_time` is 0 on the first tick of the phase.
fn start_attack(s: &mut State, attack: &AttackRef, events: &mut Vec<DomainEvent>) -> bool {
    let age = s.boss.as_ref().map_or(0, |b| b.phase_age.saturating_sub(1));
    attacks::start_aged(
        s,
        Who::Boss,
        0,
        Who::Player,
        attack,
        ENEMY_ATTACK_DAMAGE,
        age,
        events,
    )
}
