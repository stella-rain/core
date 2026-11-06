//! Starlight in equipment mode (ADR-SIM-3, ADR-SIM-5 point 8, ADR-SIM-7): the in-run currency a
//! player earns by extracting an inventory star or rune and by each boss kill while up, and
//! spends by crafting a star of the setup's pool. The balance is `Equipment.starlight`, hashed
//! and saved already; nothing carries it out of the run, a new run or a joiner starts at 0.
//!
//! Rules of this file, which the pack may refine later: a rune extracts at one flat value (it has
//! no rarity); craft takes an index of the star pool and draws nothing; a craft is not clamped to
//! the difficulty's rarity range (a drop is); an extract and a craft work for a downed or out
//! player as an equip does. Nothing here draws, emits an event or allocates inside a tick.

use super::equipment::{Equipment, Held};
use super::state::State;
use crate::rewards::{Rarity, Rewards};

/// The Starlight an extracted star gives, by `Rarity::index()`. A placeholder: the pack will
/// carry it (ADR-CONT-3 point 8).
pub(super) const EXTRACT_VALUE: [u32; 6] = [5, 10, 20, 40, 80, 160];

/// The Starlight an extracted rune gives: a rune has no rarity. A placeholder.
pub(super) const RUNE_EXTRACT: u32 = 10;

/// The Starlight a craft of a star costs, by `Rarity::index()`: five times its extract value, so
/// extracting is a loss. A placeholder.
pub(super) const CRAFT_PRICE: [u32; 6] = [25, 50, 100, 200, 400, 800];

/// The Starlight each player who is up gets from a boss kill. A placeholder.
pub(super) const BOSS_BONUS: u32 = 50;

/// Why an extract or a craft is refused (ADR-SIM-7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StarlightError {
    /// `item` is not a place of the inventory.
    NoItem { item: u32 },
}

/// The Starlight extracting `held` gives: a star by its rarity, a star outside the file 0, a rune
/// the flat value.
pub(super) fn extract_value(rewards: &Rewards, held: Held) -> u32 {
    match held {
        Held::Rune(_) => RUNE_EXTRACT,
        Held::Star(place) => rewards
            .rewards()
            .get(usize::from(place))
            .map_or(0, |r| EXTRACT_VALUE[usize::from(r.rarity.index())]),
    }
}

/// The Starlight a craft of a star of `rarity` costs.
pub(super) fn craft_price(rarity: Rarity) -> u32 {
    CRAFT_PRICE[usize::from(rarity.index())]
}

/// Whether inventory item `item` could be extracted. Pure.
pub(super) fn check_extract(equipment: &Equipment, item: u32) -> Result<(), StarlightError> {
    usize::try_from(item)
        .ok()
        .filter(|&i| i < equipment.inventory.len())
        .map(|_| ())
        .ok_or(StarlightError::NoItem { item })
}

/// Extracts inventory item `item`: it leaves the inventory (later items shift down, as an equip
/// does) and its value is added with `saturating_add`. A refusal changes nothing.
pub(super) fn extract(
    rewards: &Rewards,
    equipment: &mut Equipment,
    item: u32,
) -> Result<(), StarlightError> {
    check_extract(equipment, item)?;
    let held = equipment.inventory.remove(item as usize);
    equipment.starlight = equipment
        .starlight
        .saturating_add(extract_value(rewards, held));
    Ok(())
}

/// A boss kill: every present player that is up gets `BOSS_BONUS`, added with `saturating_add`.
/// A downed or out player gets none, as with the boss's drops. No draw, no event, no allocation.
pub(super) fn boss_bonus(s: &mut State) {
    for me in s.players.indices() {
        if s.players[me].is_up() {
            let equipment = &mut s.players.career_mut(me).equipment;
            equipment.starlight = equipment.starlight.saturating_add(BOSS_BONUS);
        }
    }
}
