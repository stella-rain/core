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

use crate::attack::AttackRef;
use crate::engine::{MoveOrder, Origin, State};
use crate::event::DomainEvent;
use crate::stage::Stage;
use crate::stage::{Step, Transition};

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
        if let Some(order) = b.order {
            (b.at, b.order) = step_order(b.at, order);
        }
    }
    if s.tick == 1 {
        begin_phase(def, s, 0, events);
    }
    maybe_end_phase(stage, s, events);
    timeline(stage, s, events);
    let phase = s.boss.as_ref().map_or(0, |b| b.phase as usize);
    if let Some(p) = def.phases.get(phase) {
        fire_first_rule(stage, s, Who::Boss, &p.rules, events);
    }
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
            Step::Attack { attack } => start_attack(s, attack),
            Step::Loop => b.step = 0,
        }
    }
}

/// An attack of the boss's own, aimed at the main character, one hit point a bullet, on the
/// phase's clock: `phase_time` is 0 on the first tick of the phase.
fn start_attack(s: &mut State, attack: &AttackRef) {
    let age = s.boss.as_ref().map_or(0, |b| b.phase_age.saturating_sub(1));
    attacks::start_aged(s, Who::Boss, Who::Player, attack, 1, age);
}
