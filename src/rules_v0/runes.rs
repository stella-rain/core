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
//! Bounds (ADR-099 point 7): one attack at a time per rune, in its slot, the file holding each
//! attack to its cooldown; a rune whose slot is still busy does not fire and keeps its cooldown
//! at 0. A cooldown is held to `MAX_RUNE_COOLDOWN_TICKS`. No rune fires a bullet.

use std::sync::Arc;

use crate::engine::{HeldRune, PlayerState, State};
use crate::event::DomainEvent;
use crate::fixed::Point;
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
        if attacks::start_rune(s, slot, at, facing, phases, damage, events) {
            // The validator holds the cooldown to the cap; the clamp keeps an unvalidated file
            // there.
            s.player.runes[i].cooldown_left = cooldown.min(MAX_RUNE_COOLDOWN_TICKS);
        }
    }
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
