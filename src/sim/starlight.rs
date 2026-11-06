//! Starlight in equipment mode (ADR-SIM-3, ADR-SIM-5 point 8, ADR-SIM-7): the in-run currency a
//! player earns by extracting an inventory star or rune and by each boss kill while up, and
//! spends by crafting a star of the setup's pool. The balance is `Equipment.starlight`, hashed
//! and saved already; nothing carries it out of the run, a new run or a joiner starts at 0.
//!
//! The values (extract, craft price, rune extract, boss bonus) are the pack's `EquipmentRules`
//! (ADR-CONT-3 point 8). Rules of this file, which the pack may refine later: a rune extracts at one flat value (it has
//! no rarity); craft takes an index of the star pool and draws nothing; a craft is not clamped to
//! the difficulty's rarity range (a drop is); an extract and a craft work for a downed or out
//! player as an equip does. Nothing here draws, emits an event or allocates inside a tick.

use super::equipment::{Equipment, Held};
use super::state::State;
use crate::limits::MAX_INVENTORY;
use crate::rewards::{EquipmentRules, Rarity, Rewards};

/// Why an extract or a craft is refused (ADR-SIM-7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StarlightError {
    /// `item` is not a place of the inventory.
    NoItem { item: u32 },
    /// `star` is past the star pool, or its pool place is past the rewards file.
    NoStar { star: u32 },
    /// The star is the lone random-rune reward.
    NotCraftable { star: u32 },
    /// The star is named and the player holds it (inventory, a slot or the Lumine slot).
    AlreadyHeld { star: u32 },
    /// The price is above the balance.
    NotEnoughStarlight { need: u32, have: u32 },
    /// The inventory is at `MAX_INVENTORY`.
    InventoryFull,
}

/// The rules' row for `rarity`: a rarity the list lacks is priced 0 (the pack lists every one).
fn priced(rules: &EquipmentRules, rarity: Rarity) -> (u32, u32) {
    rules
        .rarities
        .iter()
        .find(|r| r.rarity == rarity)
        .map_or((0, 0), |r| (r.extract, r.craft_price))
}

/// The Starlight extracting a star of `rarity` gives.
pub(super) fn extract_of(rules: &EquipmentRules, rarity: Rarity) -> u32 {
    priced(rules, rarity).0
}

/// The Starlight extracting `held` gives: a star by its rarity, a star outside the file 0, a rune
/// the flat value.
pub(super) fn extract_value(rules: &EquipmentRules, rewards: &Rewards, held: Held) -> u32 {
    match held {
        Held::Rune(_) => rules.rune_extract,
        Held::Star(place) => rewards
            .rewards()
            .get(usize::from(place))
            .map_or(0, |r| priced(rules, r.rarity).0),
    }
}

/// The Starlight a craft of a star of `rarity` costs.
pub(super) fn craft_price(rules: &EquipmentRules, rarity: Rarity) -> u32 {
    priced(rules, rarity).1
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
    rules: &EquipmentRules,
    rewards: &Rewards,
    equipment: &mut Equipment,
    item: u32,
) -> Result<(), StarlightError> {
    check_extract(equipment, item)?;
    let held = equipment.inventory.remove(item as usize);
    equipment.starlight = equipment
        .starlight
        .saturating_add(extract_value(rules, rewards, held));
    Ok(())
}

/// A boss kill: every present player that is up gets the rules' `boss_bonus`, added with `saturating_add`.
/// A downed or out player gets none, as with the boss's drops. No draw, no event, no allocation.
pub(super) fn boss_bonus(rules: &EquipmentRules, s: &mut State) {
    for me in s.players.indices() {
        if s.players[me].is_up() {
            let equipment = &mut s.players.career_mut(me).equipment;
            equipment.starlight = equipment.starlight.saturating_add(rules.boss_bonus);
        }
    }
}

/// Whether the player holds the named star at `place`: the one test both the refusal of a craft
/// and the snapshot's `pool_held` read, so the two cannot differ. An unnamed star is never held
/// in this sense (a craft of it is allowed).
pub(super) fn holds_named(rewards: &Rewards, equipment: &Equipment, place: u16) -> bool {
    rewards
        .rewards()
        .get(usize::from(place))
        .is_some_and(|r| r.named)
        && super::drops::holds(equipment, place)
}

/// The place and price of the star at index `star` of `pool`, or why it cannot be crafted. Pure.
/// The order of the checks is the star, whether it is craftable, whether a named star is held,
/// the price, the inventory.
fn craft_target(
    rules: &EquipmentRules,
    rewards: &Rewards,
    pool: &[u16],
    equipment: &Equipment,
    star: u32,
) -> Result<(u16, u32), StarlightError> {
    let place = usize::try_from(star)
        .ok()
        .and_then(|i| pool.get(i))
        .copied()
        .ok_or(StarlightError::NoStar { star })?;
    let reward = rewards
        .rewards()
        .get(usize::from(place))
        .ok_or(StarlightError::NoStar { star })?;
    if reward.is_random_rune() {
        return Err(StarlightError::NotCraftable { star });
    }
    if holds_named(rewards, equipment, place) {
        return Err(StarlightError::AlreadyHeld { star });
    }
    let need = craft_price(rules, reward.rarity);
    if equipment.starlight < need {
        return Err(StarlightError::NotEnoughStarlight {
            need,
            have: equipment.starlight,
        });
    }
    if equipment.inventory.len() >= MAX_INVENTORY {
        return Err(StarlightError::InventoryFull);
    }
    Ok((place, need))
}

/// Whether the star at index `star` of `pool` could be crafted (`craft_target`). Pure.
pub(super) fn check_craft(
    rules: &EquipmentRules,
    rewards: &Rewards,
    pool: &[u16],
    equipment: &Equipment,
    star: u32,
) -> Result<(), StarlightError> {
    craft_target(rules, rewards, pool, equipment, star).map(|_| ())
}

/// Crafts the star at index `star` of `pool`: its price leaves the balance and the star is
/// pushed at the end of the inventory, like a drop. A refusal changes nothing.
pub(super) fn craft(
    rules: &EquipmentRules,
    rewards: &Rewards,
    pool: &[u16],
    equipment: &mut Equipment,
    star: u32,
) -> Result<(), StarlightError> {
    let (place, price) = craft_target(rules, rewards, pool, equipment, star)?;
    equipment.starlight -= price;
    equipment.inventory.push(Held::Star(place));
    Ok(())
}

/// The bundled pack's equipment rules, for tests that run on the bundled pack.
#[cfg(test)]
#[allow(clippy::expect_used)]
pub(crate) fn bundled_rules() -> &'static EquipmentRules {
    Rewards::bundled()
        .equipment()
        .expect("the bundled pack holds the equipment rules")
}

/// The bundled pack's extract values by `Rarity::index()`, for tests.
#[cfg(test)]
pub(crate) fn extract_table() -> Vec<u32> {
    Rarity::ALL.map(|r| extract_of(bundled_rules(), r)).to_vec()
}

/// The bundled pack's craft prices by `Rarity::index()`, for tests.
#[cfg(test)]
pub(crate) fn craft_table() -> Vec<u32> {
    Rarity::ALL
        .map(|r| craft_price(bundled_rules(), r))
        .to_vec()
}

/// Gives a hand-written rewards file the bundled equipment rules and, to each reward with none,
/// the category its first effect names (Attack, on-hit and rune effects `attack`, Defence
/// `defence`, Mobility `mobility`; Growth and the random rune none), for tests.
#[cfg(test)]
#[allow(clippy::expect_used)]
pub(crate) fn with_equipment_rules(file: &mut serde_json::Value) {
    use crate::rewards::RewardEffect;
    use crate::snapshot::Category;
    file["equipment"] = serde_json::to_value(bundled_rules()).expect("the rules serialise");
    let Some(list) = file["rewards"].as_array_mut() else {
        return;
    };
    for r in list {
        let first = r["effects"][0]["effect"]
            .as_str()
            .or_else(|| r["effect"].as_str())
            .map(str::to_owned);
        let Some(first) = first else { continue };
        let effect: RewardEffect =
            serde_json::from_value(serde_json::Value::String(first)).expect("an effect name");
        if effect == RewardEffect::RandomRune || r.get("category").is_some() {
            continue;
        }
        let category = match super::offer_rating::category_of(effect) {
            Category::Attack | Category::OnHit | Category::Rune => Some("attack"),
            Category::Defence => Some("defence"),
            Category::Mobility => Some("mobility"),
            Category::Growth => None,
        };
        if let Some(category) = category {
            r["category"] = serde_json::Value::String(category.into());
        }
    }
}
