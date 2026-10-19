//! Runes (ADR-099): the Main weapon's set, each rune at most once, the random rune's draw, and
//! the on-hit runes' fire.
//!
//! When a combo step's hit lands (by hit box or bullet, as the expertise gauge counts it), each
//! on-hit rune held whose cooldown is 0 fires its attack around the point hit, in the order
//! held, and its cooldown is set to the runes file's; each cooldown counts down by 1 at the
//! start of every tick. The attack runs in the rune's own slot, as the main character's but no
//! combo step's: its hits give no expertise and fire no rune, so a rune never feeds itself or
//! another (point 3). It starts in the collision step, after that tick's hit boxes have
//! landed, so its first phase counts that tick and its hit boxes land from the next. Nothing
//! here draws from the generator. Damage-over-time runes do nothing yet.
//!
//! Boomerang is thrown (`fly`): its attack starts on the main character, and its hit boxes fly
//! out over its first phase to the point hit and back over its second toward where the main
//! character is now. Each tick, after the main character moves, the centre goes a straight
//! step, the distance left over the ticks left in the phase, so that it is on the point hit on
//! the first phase's last tick and on the main character on the second's; no steering, no
//! bullet. Each phase is active and keeps its own hit list, so an enemy on the way is hit once
//! out and once back. The path is a placeholder.
//!
//! Bounds (ADR-099 point 7): one attack at a time per rune, in its slot, the file holding each
//! attack to its cooldown; a rune whose slot is still busy does not fire and keeps its cooldown
//! at 0. A cooldown is held to `MAX_RUNE_COOLDOWN_TICKS`. No rune fires a bullet, a thrown one
//! included: the bullets alive from runes stay at 0.

use std::sync::Arc;

use crate::engine::{HeldRune, PlayerState, State};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::rng::SplitMix64;
use crate::rune::{MAX_RUNES, Rune};
use crate::runes::MAX_RUNE_COOLDOWN_TICKS;

use super::attacks;

/// The first on-hit rune's slot: after the combo (0), the stage skill (1) and the orbits' orbs
/// (2 to 33); each rune's is this plus its place in `Rune::ALL`.
pub(super) const FIRST_SLOT: u8 = 34;

/// Every on-hit rune's cooldown counts down by a tick.
pub(super) fn tick(s: &mut State) {
    for r in &mut s.player.runes {
        r.cooldown_left = r.cooldown_left.saturating_sub(1);
    }
}

/// A combo step hit at `at`: each on-hit rune held and ready fires its attack there, at its
/// base damage, and waits out its cooldown.
pub(super) fn on_hit(s: &mut State, at: Point, events: &mut Vec<DomainEvent>) {
    for i in 0..s.player.runes.len() {
        let HeldRune {
            rune,
            cooldown_left,
        } = s.player.runes[i];
        if cooldown_left > 0 {
            continue;
        }
        let Some(entry) = s.runes.on_hit(rune) else {
            continue;
        };
        let (cooldown, damage) = (entry.cooldown_ticks, entry.damage);
        let phases = Arc::new(entry.attack.clone());
        let slot = FIRST_SLOT.saturating_add(rune.index());
        let facing = s.player.facing;
        let thrown = rune == Rune::Boomerang;
        let from = if thrown { s.player.at } else { at };
        if attacks::start_rune(s, slot, from, facing, phases, damage, events) {
            if thrown {
                // It was pushed last; its first step counts the tick it is thrown on.
                if let Some(r) = s.runs.last_mut() {
                    r.throw = Some(at);
                }
                if let Some(r) = s.runs.len().checked_sub(1) {
                    fly_one(s, r);
                }
            }
            // The validator holds the cooldown to the cap; the clamp keeps an unvalidated file
            // there.
            s.player.runes[i].cooldown_left = cooldown.min(MAX_RUNE_COOLDOWN_TICKS);
        }
    }
}

/// Every thrown rune's hit boxes go a step: out to its turn point in its first phase, back to
/// the main character in the others.
pub(super) fn fly(s: &mut State) {
    for i in 0..s.runs.len() {
        if s.runs[i].throw.is_some() {
            fly_one(s, i);
        }
    }
}

/// Run `i`'s centre goes the distance left to its goal over the phase's ticks left, this one
/// counted, so that it is there on the phase's last tick.
fn fly_one(s: &mut State, i: usize) {
    let to_player = s.player.at;
    let r = &mut s.runs[i];
    let (Some(at), Some(turn)) = (r.at, r.throw) else {
        return;
    };
    let goal = if r.phase == 0 { turn } else { to_player };
    let ticks = r.phases.get(r.phase as usize).map_or(1, |p| p.ticks);
    let left = i64::from(ticks.saturating_sub(r.tick).max(1));
    let step = |from: Fx, to: Fx| {
        let d = (i64::from(to.0) - i64::from(from.0)) / left;
        Fx(i32::try_from(i64::from(from.0) + d).unwrap_or(to.0))
    };
    r.at = Some(Point {
        x: step(at.x, goal.x),
        y: step(at.y, goal.y),
    });
}

/// The Main weapon holds every rune: the random rune has nothing left to give.
pub(super) fn all_held(runes: &[HeldRune]) -> bool {
    Rune::ALL.iter().all(|r| runes.iter().any(|h| h.rune == *r))
}

/// The random rune is picked: draws one of the runes the Main does not hold, in `Rune::ALL`'s
/// order, with one draw from the run's generator, and adds it. None, drawing nothing, once it
/// holds them all.
pub(super) fn draw(rng: &mut SplitMix64, p: &mut PlayerState) -> Option<Rune> {
    let left: Vec<Rune> = Rune::ALL
        .iter()
        .copied()
        .filter(|r| !p.runes.iter().any(|h| h.rune == *r))
        .collect();
    if left.is_empty() || p.runes.len() >= MAX_RUNES {
        return None;
    }
    let at = rng.below(u32::try_from(left.len()).unwrap_or(u32::MAX));
    let rune = *left.get(usize::try_from(at).unwrap_or(usize::MAX))?;
    p.runes.push(HeldRune {
        rune,
        cooldown_left: 0,
    });
    Some(rune)
}
