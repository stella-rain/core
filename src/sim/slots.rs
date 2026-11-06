//! The equipment slots of a player in equipment mode (ADR-SIM-3 point 2, ADR-SIM-7): the layout,
//! what a star's category and option are until the pack says, and the build re-derived from the
//! stars equipped. The inventory does not count toward the build (only the stars in a slot and
//! the Lumine slot do); a change of what is equipped goes through `refresh`, which writes the
//! change from the build of the set before to the build of the set after.
//!
//! Placeholders (ADR-CONT-3 point 8; the pack will carry them): the layout, each star's category
//! and each star's option, the kills each level takes (`KILLS_TO_NEXT`) and the share of a star's
//! base each level adds (`PCT_PER_LEVEL`). They are derived or set here and are the one place to
//! replace.
//!
//! Expertise: every kill a player makes gives one point to each of its slots that holds a star
//! (a category slot or the Lumine slot; runes and empty slots gain nothing). A slot rises one
//! level at a time, Lv1 to `MAX_LEVEL`, and each level above 1 adds `PCT_PER_LEVEL` percent of the
//! star's base amounts to the build (`level_scaled`). A replace by a star that shares no option
//! halves the level (at least 1) and resets the kills; a same-option switch keeps both.

use super::World;
use super::build::{Build, Grants};
use super::equipment::{EquipSlot, Equipment, Held, SlotKind};
use super::offer_rating::category_of;
use super::state::{Career, HeldRune, Offers, PlayerState};
use crate::attack::DamageType;
use crate::id::ContentId;
use crate::limits::MAX_SLOT_LEVEL;
use crate::rewards::{RewardEffect, Rewards};
use crate::snapshot::Category;

/// The slots a player has, in order: the kind and how many of it. Attack 5, Defence 2,
/// Mobility 2, Rune 3.
pub(super) const SLOT_LAYOUT: [(SlotKind, u8); 4] = [
    (SlotKind::Category(0), 5),
    (SlotKind::Category(1), 2),
    (SlotKind::Category(2), 2),
    (SlotKind::Rune, 3),
];

/// The highest level a slot reaches (ADR-SIM-5).
pub(super) const MAX_LEVEL: u8 = MAX_SLOT_LEVEL;

/// Placeholder curve: entry `i` is the kills a slot at level `i + 1` needs to reach level `i + 2`.
/// Rising, so the lower levels come faster; Lv1 to Lv10 takes 870 kills.
pub(super) const KILLS_TO_NEXT: [u32; 9] = [10, 20, 35, 55, 80, 110, 145, 185, 230];

/// Placeholder: the percent of a star's base amount each level above 1 adds.
const PCT_PER_LEVEL: u32 = 3;

/// A slot's level and kills after `kills` more: one level at a time while the kills reach the
/// level's need; at `MAX_LEVEL` the kills are dropped. A level of 0 (a slot that never held a
/// star) gains nothing. Pure.
pub(super) fn gain(level: u8, expertise: u32, kills: u32) -> (u8, u32) {
    if level == 0 {
        return (0, expertise);
    }
    let (mut level, mut expertise) = (level.min(MAX_LEVEL), expertise.saturating_add(kills));
    while level < MAX_LEVEL {
        let need = KILLS_TO_NEXT[usize::from(level) - 1];
        if expertise < need {
            break;
        }
        expertise -= need;
        level += 1;
    }
    if level >= MAX_LEVEL {
        expertise = 0;
    }
    (level, expertise)
}

/// `amount` at `level`: `PCT_PER_LEVEL` percent more for each level above 1, floored. A level of
/// 0 reads as 1 and one past `MAX_LEVEL` as `MAX_LEVEL`.
pub(super) fn level_scaled(amount: u32, level: u8) -> u32 {
    let above = u64::from(level.clamp(1, MAX_LEVEL) - 1);
    let scaled = u64::from(amount) * (100 + u64::from(PCT_PER_LEVEL) * above) / 100;
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// The slot index of the first rune slot.
pub(super) fn first_rune_slot() -> usize {
    SLOT_LAYOUT
        .iter()
        .take_while(|(kind, _)| !matches!(kind, SlotKind::Rune))
        .map(|(_, n)| usize::from(*n))
        .sum()
}

/// The empty slots of the layout, level 0 and no expertise.
pub(super) fn fresh_slots() -> Vec<EquipSlot> {
    SLOT_LAYOUT
        .iter()
        .flat_map(|&(kind, n)| {
            (0..n).map(move |_| EquipSlot {
                kind,
                held: None,
                level: 0,
                expertise: 0,
            })
        })
        .collect()
}

/// The category slot a star fits, from its first effect: Attack, on-hit and rune stars go to 0,
/// Defence to 1, Mobility to 2; a Growth star (or a lone random rune) fits no category slot.
pub(super) fn star_category(rewards: &Rewards, place: u16) -> Option<u8> {
    let reward = rewards.rewards().get(usize::from(place))?;
    if reward.is_random_rune() {
        return None;
    }
    match category_of(reward.effect()) {
        Category::Attack | Category::OnHit | Category::Rune => Some(0),
        Category::Defence => Some(1),
        Category::Mobility => Some(2),
        Category::Growth => None,
    }
}

/// One option of a star: its effect, the damage type of a type bonus and the stage skill.
type OptionKey<'a> = (RewardEffect, Option<DamageType>, Option<&'a ContentId>);

/// The options of the star at `place`: one per effect entry.
fn options(rewards: &Rewards, place: u16) -> Vec<OptionKey<'_>> {
    rewards
        .rewards()
        .get(usize::from(place))
        .map(|r| {
            r.effects
                .iter()
                .map(|e| (e.effect, e.damage_type, e.skill.as_ref()))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether two stars share an option: their option sets intersect.
pub(super) fn shares_option(rewards: &Rewards, a: u16, b: u16) -> bool {
    let (a, b) = (options(rewards, a), options(rewards, b));
    a.iter().any(|o| b.contains(o))
}

/// The stars equipped, in slot order and the Lumine slot last.
pub(super) fn equipped(equipment: &Equipment) -> impl Iterator<Item = u16> + '_ {
    equipment
        .slots
        .iter()
        .chain(equipment.lumine.as_ref())
        .filter_map(|s| match s.held {
            Some(Held::Star(p)) => Some(p),
            _ => None,
        })
}

/// How many of each star of the file are equipped (the inventory does not count).
#[cfg(test)]
pub(super) fn equipped_counts(rewards: &Rewards, equipment: &Equipment) -> Vec<u8> {
    counts_of(rewards, equipped(equipment))
}

/// How many of each star of the file `places` hold.
fn counts_of(rewards: &Rewards, places: impl IntoIterator<Item = u16>) -> Vec<u8> {
    let mut counts = vec![0u8; rewards.rewards().len()];
    for p in places {
        if let Some(c) = counts.get_mut(usize::from(p)) {
            *c = c.saturating_add(1);
        }
    }
    counts
}

/// The offers a stage without stars lacks, for the writes that need one: thrown away.
fn throwaway(rewards: &Rewards) -> Offers {
    Offers {
        due: Vec::new(),
        offer: None,
        rerolls_left: 0,
        banishes_left: 0,
        extra_candidates: 0,
        taken: vec![0; rewards.rewards().len()],
        banished: vec![false; rewards.rewards().len()],
        set_counts: vec![0; rewards.constellations().sets().len()],
    }
}

/// Writes the change of what is equipped: `before` is the stars equipped before the change in
/// slot order (empty for nothing), the equipment of `career` holds the set after. The build and
/// the grants change by the difference (`Build::write`, `Grants::write`), and the set counts are
/// the members equipped.
pub(super) fn refresh(
    stage: &World,
    rewards: &Rewards,
    player: &mut PlayerState,
    career: &mut Career,
    before: &[u16],
) {
    let Career {
        progress,
        offers,
        regen,
        equipment,
        ..
    } = career;
    let after: Vec<u16> = equipped(equipment).collect();
    let counts = counts_of(rewards, after.iter().copied());
    let mut local = None;
    let o = match offers {
        Some(o) => o,
        None => local.insert(throwaway(rewards)),
    };
    let was = Build::of(stage, rewards, &counts_of(rewards, before.iter().copied()));
    let now = Build::of(stage, rewards, &counts);
    Build::write(&was, &now, player, progress, regen, o);
    Grants::write(
        &Grants::of(rewards, before),
        &Grants::of(rewards, &after),
        stage,
        player,
        o,
    );
    for (set, c) in rewards.constellations().sets().iter().enumerate() {
        let members = c
            .members
            .iter()
            .filter(|m| {
                rewards
                    .index_of(m)
                    .is_some_and(|p| counts.get(p).copied().unwrap_or(0) > 0)
            })
            .count();
        if let Some(n) = o.set_counts.get_mut(set) {
            *n = u8::try_from(members).unwrap_or(u8::MAX);
        }
    }
}

/// Why an equip or a replace is refused (ADR-SIM-7), in the order the checks run: the item, the
/// slot, what fits it, then the option and the rune.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlotError {
    /// `item` is not a place of the inventory.
    NoItem { item: u32 },
    /// `slot` addresses no slot: `0..slots.len()`, and `slots.len()` for the Lumine slot.
    NoSlot { slot: u32 },
    /// Equip into an occupied slot.
    SlotFilled { slot: u32 },
    /// Replace into an empty slot.
    SlotEmpty { slot: u32 },
    /// A rune in a star slot, a star in a rune slot, or a star in a slot of another category.
    WrongSlot { slot: u32 },
    /// The item is the lone random rune.
    NotEquippable { item: u32 },
    /// Another slot (the Lumine slot included) holds a star sharing an option with the item.
    OptionHeld { slot: u32 },
    /// That rune is in another rune slot.
    RuneHeld { slot: u32 },
}

/// The slot `index` addresses: `0..slots.len()` for `slots`, `slots.len()` for the Lumine slot.
fn slot_at(equipment: &Equipment, index: usize) -> Option<&EquipSlot> {
    equipment.slots.get(index).or_else(|| {
        equipment
            .lumine
            .as_ref()
            .filter(|_| index == equipment.slots.len())
    })
}

/// Whether `item` could go into `slot` as an equip (`replace` false) or a replace. Pure. The
/// order of the checks is the item, the slot (and whether it is empty or filled), what fits
/// the slot (the random rune fits nowhere, a rune a rune slot, a star its category slot or the
/// Lumine slot), then the option (no other slot holds a star sharing one) or the rune (no other
/// rune slot holds it).
pub(super) fn check(
    rewards: &Rewards,
    equipment: &Equipment,
    item: u32,
    slot: u32,
    replace: bool,
) -> Result<(), SlotError> {
    let held = usize::try_from(item)
        .ok()
        .and_then(|i| equipment.inventory.get(i))
        .copied()
        .ok_or(SlotError::NoItem { item })?;
    let at = usize::try_from(slot).unwrap_or(usize::MAX);
    let target = slot_at(equipment, at).ok_or(SlotError::NoSlot { slot })?;
    match (replace, target.held.is_some()) {
        (false, true) => return Err(SlotError::SlotFilled { slot }),
        (true, false) => return Err(SlotError::SlotEmpty { slot }),
        _ => {}
    }
    let wrong = Err(SlotError::WrongSlot { slot });
    match held {
        Held::Star(place) => {
            let lone_rune = rewards
                .rewards()
                .get(usize::from(place))
                .is_none_or(|r| r.is_random_rune());
            if lone_rune {
                return Err(SlotError::NotEquippable { item });
            }
            match target.kind {
                SlotKind::Category(c) if star_category(rewards, place) == Some(c) => {}
                SlotKind::Lumine => {}
                SlotKind::Category(_) | SlotKind::Rune => return wrong,
            }
            let others = equipment.slots.iter().chain(equipment.lumine.as_ref());
            for (i, other) in others.enumerate() {
                if let (true, Some(Held::Star(q))) = (i != at, other.held)
                    && shares_option(rewards, place, q)
                {
                    let slot = u32::try_from(i).unwrap_or(u32::MAX);
                    return Err(SlotError::OptionHeld { slot });
                }
            }
        }
        Held::Rune(rune) => {
            if !matches!(target.kind, SlotKind::Rune) {
                return wrong;
            }
            for (i, other) in equipment.slots.iter().enumerate() {
                if i != at && matches!(other.held, Some(Held::Rune(r)) if r == rune) {
                    let slot = u32::try_from(i).unwrap_or(u32::MAX);
                    return Err(SlotError::RuneHeld { slot });
                }
            }
        }
    }
    Ok(())
}

/// Whether replacing the star in `slot` by the star `item` is a switch: both are stars and
/// share an option. A switch costs no expertise: the expertise rules read this (a later task),
/// so it is tested here and unused by the live rules until then.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn is_switch(rewards: &Rewards, equipment: &Equipment, item: u32, slot: u32) -> bool {
    let item = usize::try_from(item)
        .ok()
        .and_then(|i| equipment.inventory.get(i));
    let target = usize::try_from(slot)
        .ok()
        .and_then(|i| slot_at(equipment, i));
    matches!(
        (item, target.and_then(|t| t.held)),
        (Some(&Held::Star(a)), Some(Held::Star(b))) if shares_option(rewards, a, b)
    )
}

/// Equips the inventory item `item` into the empty slot `slot`, or refuses and changes nothing.
/// The item leaves the inventory; a level of 0 becomes 1 and a later level stays.
pub(super) fn equip(
    stage: &World,
    rewards: &Rewards,
    player: &mut PlayerState,
    career: &mut Career,
    item: u32,
    slot: u32,
) -> Result<(), SlotError> {
    check(rewards, &career.equipment, item, slot, false)?;
    apply(stage, rewards, player, career, item, slot, false);
    Ok(())
}

/// Replaces the star or rune in the occupied `slot` by the inventory item `item`: the displaced
/// one takes the item's place in the inventory, and the slot keeps its level and expertise.
pub(super) fn replace(
    stage: &World,
    rewards: &Rewards,
    player: &mut PlayerState,
    career: &mut Career,
    item: u32,
    slot: u32,
) -> Result<(), SlotError> {
    check(rewards, &career.equipment, item, slot, true)?;
    apply(stage, rewards, player, career, item, slot, true);
    Ok(())
}

/// The move itself, after `check`: the indices are valid.
fn apply(
    stage: &World,
    rewards: &Rewards,
    player: &mut PlayerState,
    career: &mut Career,
    item: u32,
    slot: u32,
    replace: bool,
) {
    let (item, at) = (item as usize, slot as usize);
    let before: Vec<u16> = equipped(&career.equipment).collect();
    let eq = &mut career.equipment;
    let is_rune = matches!(eq.inventory[item], Held::Rune(_));
    let new = if replace {
        eq.inventory[item]
    } else {
        eq.inventory.remove(item)
    };
    let len = eq.slots.len();
    let target = match at < len {
        true => &mut eq.slots[at],
        false => eq.lumine.as_mut().expect("checked: the Lumine slot"),
    };
    let displaced = target.held.replace(new);
    target.level = target.level.max(1);
    if replace && let Some(old) = displaced {
        eq.inventory[item] = old;
    }
    if is_rune {
        // The runes the Main fires are the rune slots filled, in slot order; a rune that stays
        // keeps its cooldown, a new one is ready.
        let old = std::mem::take(&mut player.runes);
        player.runes = eq
            .slots
            .iter()
            .filter_map(|s| match (s.kind, s.held) {
                (SlotKind::Rune, Some(Held::Rune(rune))) => Some(HeldRune {
                    rune,
                    cooldown_left: old
                        .iter()
                        .find(|h| h.rune == rune)
                        .map_or(0, |h| h.cooldown_left),
                }),
                _ => None,
            })
            .collect();
    } else {
        refresh(stage, rewards, player, career, &before);
    }
}

/// Whether `a` and `b` are the same layout: the same kinds in the same order. What a savepoint
/// checks of the slots it restores against the fresh engine's.
pub(crate) fn same_layout(a: &[EquipSlot], b: &[EquipSlot]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| match (x.kind, y.kind) {
            (SlotKind::Category(i), SlotKind::Category(j)) => i == j,
            (SlotKind::Rune, SlotKind::Rune) | (SlotKind::Lumine, SlotKind::Lumine) => true,
            _ => false,
        })
}

/// Whether every slot holds the kind of item it takes: a rune slot a rune, a category slot and the
/// Lumine slot a star; the Lumine slot is of the Lumine kind.
pub(crate) fn holds_right_kinds(equipment: &Equipment) -> bool {
    let right = |s: &EquipSlot| {
        !matches!(
            (s.kind, s.held),
            (SlotKind::Rune, Some(Held::Star(_)))
                | (
                    SlotKind::Category(_) | SlotKind::Lumine,
                    Some(Held::Rune(_))
                )
        )
    };
    equipment.slots.iter().all(right)
        && equipment
            .lumine
            .as_ref()
            .is_none_or(|l| matches!(l.kind, SlotKind::Lumine) && right(l))
}

/// Whether the runes the Main fires are the rune slots filled, in slot order (their cooldowns are
/// free).
pub(crate) fn runes_match_slots(equipment: &Equipment, runes: &[HeldRune]) -> bool {
    let filled = equipment
        .slots
        .iter()
        .filter_map(|s| match (s.kind, s.held) {
            (SlotKind::Rune, Some(Held::Rune(r))) => Some(r),
            _ => None,
        });
    filled.eq(runes.iter().map(|h| h.rune))
}
