//! Orbits (ADR-069): what an orbit reward gives. Each orbit held turns its orbs round the main
//! character; each orb runs the orbit's attack in an attack slot of its own, so an orb never
//! waits for or cancels a combo step, the stage skill or another orb. Every tick, for each
//! orbit in the order they came:
//!
//! - its cooldown runs down a tick; once it has run out and no orb of the round before is still
//!   attacking, a round starts: every orb starts the attack at once, at the main character's
//!   attack stat, and the cooldown starts again, so it counts from the round's start;
//! - each orb of the round going on faces `angle + j × full turn / orbs of the round` from
//!   straight down the screen, toward the right: the attack's hit box turns with it;
//! - the angle turns `turn_per_tick` further.
//!
//! An orbit arrives ready, at angle 0. Taking it again adds its orbs, up to `MAX_ORBS`; they join
//! at the next round, and the round going on keeps its own.

use super::World;
use crate::event::DomainEvent;
use crate::rewards::{MAX_ORBITS, MAX_ORBS, Rewards};
use crate::sim::state::PlayerIdx;
use crate::sim::state::{Dir, OrbitState, PlayerState, State};
use crate::trig::PER_TURN;

use super::PLAYER_ID;
use super::attacks;
use super::levels;

/// The attack slot of the first orbit's first orb (ADR-069): 0 is the combo and 1 the stage
/// skill. Orbit `k`'s orb `j` attacks in `FIRST_SLOT + k × MAX_ORBS + j`.
pub(super) const FIRST_SLOT: u8 = 2;

/// A taken reward, at `reward` in the rewards file, gives `orbs` more orbs of its orbit: the
/// orbit held, or a new one, which arrives ready.
pub(super) fn take(p: &mut PlayerState, reward: u16, orbs: u32) {
    let more = u8::try_from(orbs).unwrap_or(u8::MAX);
    if let Some(o) = p.orbits.iter_mut().find(|o| o.reward == reward) {
        o.orbs = o.orbs.saturating_add(more).min(MAX_ORBS);
        return;
    }
    // The validator holds a file to `MAX_ORBITS` orbit rewards; the clamp keeps an unvalidated
    // one there too.
    if p.orbits.len() >= MAX_ORBITS {
        return;
    }
    p.orbits.push(OrbitState {
        reward,
        orbs: more.min(MAX_ORBS),
        round: 0,
        cooldown_left: 0,
        angle: 0,
    });
}

/// Orbit `k`'s orb `j`'s slot: `k` below `MAX_ORBITS` and `j` below `MAX_ORBS`, so at most 33.
fn slot(k: usize, j: u8) -> u8 {
    FIRST_SLOT + u8::try_from(k).expect("at most MAX_ORBITS orbits") * MAX_ORBS + j
}

/// Orb `j` of a round of `round` orbs, with orb 0 at `angle`.
fn facing(angle: i32, j: u8, round: u8) -> Dir {
    // `j` is below `round`, at most `MAX_ORBS`, so this stays within two turns.
    let spaced = angle + PER_TURN * i32::from(j) / i32::from(round.max(1));
    attacks::rotate(Dir::DOWN, spaced)
}

/// One tick of every orbit held.
pub(super) fn act(stage: &World, rewards: &Rewards, s: &mut State, events: &mut Vec<DomainEvent>) {
    for k in 0..s.players[PlayerIdx::SOLO].orbits.len() {
        let mut o = s.players[PlayerIdx::SOLO].orbits[k];
        let Some(def) = rewards.orbit_def(o.reward) else {
            continue;
        };
        o.cooldown_left = o.cooldown_left.saturating_sub(1);
        let attacking = (0..MAX_ORBS).any(|j| attacks::busy(s, PLAYER_ID, slot(k, j)));
        if o.cooldown_left == 0 && !attacking {
            let damage = levels::attack(s, stage.player.atk);
            let strike = attacks::Strike {
                bonus_pct: 0,
                speed_pct: 0,
                range_pct: 0,
                smash: None,
            };
            for j in 0..o.orbs {
                let dir = facing(o.angle, j, o.orbs);
                attacks::start_player(
                    stage,
                    s,
                    slot(k, j),
                    dir,
                    &def.attack,
                    damage,
                    strike,
                    events,
                );
            }
            o.round = o.orbs;
            o.cooldown_left = def.cooldown_ticks;
        }
        let first = slot(k, 0);
        for r in s.runs.iter_mut() {
            if r.owner == PLAYER_ID && (first..first + MAX_ORBS).contains(&r.slot) {
                r.facing = facing(o.angle, r.slot - first, o.round);
            }
        }
        // The validator holds the turn to `MAX_ORBIT_TURN`, far below `i32`'s range.
        o.angle = o
            .angle
            .saturating_add(def.turn_per_tick)
            .rem_euclid(PER_TURN);
        s.players[PlayerIdx::SOLO].orbits[k] = o;
    }
}
