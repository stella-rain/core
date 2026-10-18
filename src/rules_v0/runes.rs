//! Runes (ADR-099): the Main weapon's set, each rune at most once, and the random rune's draw.
//! No rune does anything yet.

use crate::engine::{HeldRune, PlayerState};
use crate::rng::SplitMix64;
use crate::rune::{MAX_RUNES, Rune};

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
