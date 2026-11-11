//! Equipment actions of the live rules: the checks and the actions between steps.

use super::{SlotError, StarlightError, TreeError, World, expertise, slots, starlight};
use crate::expertise::Expertise;
use crate::rewards::{EquipmentRules, Rewards};
use crate::sim::state::PlayerIdx;
use crate::sim::state::State;

/// Whether the inventory item `item` could go into `slot` of `me`, as an equip or a replace
/// (`slots::check`). Read-only.
pub(crate) fn check_equip(
    rules: &EquipmentRules,
    rewards: &Rewards,
    s: &State,
    me: PlayerIdx,
    item: u32,
    slot: u32,
    replace: bool,
) -> Result<(), SlotError> {
    slots::check(
        rules,
        rewards,
        &s.players.career(me).equipment,
        item,
        slot,
        replace,
    )
}

/// Equips (`replace` false) or replaces: the item `item` into `slot` of `me`, or a refusal that
/// changes nothing. Made between ticks, like a pick; no event, no draw.
pub(crate) fn equip_action(
    stage: &World,
    rewards: &Rewards,
    s: &mut State,
    me: PlayerIdx,
    item: u32,
    slot: u32,
    replace: bool,
) -> Result<(), SlotError> {
    let (player, career) = s.players.both_mut(me);
    match replace {
        true => slots::replace(stage, rewards, player, career, item, slot),
        false => slots::equip(stage, rewards, player, career, item, slot),
    }
}

/// Whether `me` could take choice `option` of the next tree pick (`expertise::check_tree_pick`).
/// Read-only.
pub(crate) fn check_tree_pick(
    stage: &World,
    expertise: &Expertise,
    s: &State,
    me: PlayerIdx,
    option: u32,
) -> Result<(), TreeError> {
    expertise::check_tree_pick(stage, expertise, s, me, option)
}

/// Takes choice `option` of the next tree pick for `me`, or a refusal that changes nothing. Made
/// between ticks; no event, no draw.
pub(crate) fn tree_pick_action(
    stage: &World,
    expertise: &Expertise,
    s: &mut State,
    me: PlayerIdx,
    option: u32,
) -> Result<(), TreeError> {
    expertise::tree_pick(stage, expertise, s, me, option)
}

/// The setup's star pool of `s`, empty when the run has no star field.
pub(super) fn star_pool(s: &State) -> &[u16] {
    s.stars.as_ref().map_or(&[], |f| &f.pool)
}

/// Whether `me` could extract inventory item `item` (`starlight::check_extract`). Read-only.
pub(crate) fn check_extract(s: &State, me: PlayerIdx, item: u32) -> Result<(), StarlightError> {
    starlight::check_extract(&s.players.career(me).equipment, item)
}

/// Extracts inventory item `item` of `me` into Starlight, or a refusal that changes nothing.
/// Made between ticks; no event, no draw.
pub(crate) fn extract_action(
    rules: &EquipmentRules,
    rewards: &Rewards,
    s: &mut State,
    me: PlayerIdx,
    item: u32,
) -> Result<(), StarlightError> {
    starlight::extract(
        rules,
        rewards,
        &mut s.players.career_mut(me).equipment,
        item,
    )
}

/// Whether `me` could craft the star at index `star` of the star pool. Read-only.
pub(crate) fn check_craft(
    rules: &EquipmentRules,
    rewards: &Rewards,
    s: &State,
    me: PlayerIdx,
    star: u32,
) -> Result<(), StarlightError> {
    starlight::check_craft(
        rules,
        rewards,
        star_pool(s),
        &s.players.career(me).equipment,
        star,
    )
}

/// Crafts the star at index `star` of the star pool for `me`, or a refusal that changes nothing.
/// Made between ticks; no event, no draw.
pub(crate) fn craft_action(
    rules: &EquipmentRules,
    rewards: &Rewards,
    s: &mut State,
    me: PlayerIdx,
    star: u32,
) -> Result<(), StarlightError> {
    let State { stars, players, .. } = s;
    let pool = stars.as_ref().map_or(&[][..], |f| &f.pool);
    starlight::craft(
        rules,
        rewards,
        pool,
        &mut players.career_mut(me).equipment,
        star,
    )
}
