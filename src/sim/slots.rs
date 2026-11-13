//! The equipment slots of a player in equipment mode (ADR-SIM-3 point 2, ADR-SIM-7): the layout,
//! what a star's category and option are until the pack says, and the build re-derived from the
//! stars equipped. The inventory does not count toward the build (only the stars in a slot and
//! the Lumine slot do); a change of what is equipped goes through `refresh`, which writes the
//! change from the build of the set before to the build of the set after.
//!
//! The layout, the kills each level takes and the share of a star's base each level adds are
//! the pack's `EquipmentRules` (ADR-CONT-3 point 8), held by the `World`. A star's category is
//! its reward's `category`; its option is derived from its effect entries.
//!
//! Expertise: every kill a player makes gives one point to each of its slots that holds a star
//! (a category slot or the Lumine slot; runes and empty slots gain nothing). A slot rises one
//! level at a time, Lv1 to `MAX_LEVEL`, and each level above 1 adds `PCT_PER_LEVEL` percent of the
//! star's base amounts to the build (`level_scaled`). A replace by a star that shares no option
//! halves the level (at least 1) and resets the kills; a same-option switch keeps both.

use super::World;
use super::build::{Build, Grants};
use super::equipment::{EquipSlot, Equipment, Held, SlotKind};
use super::skill;
use super::state::{Career, HeldRune, PlayerState, State};
use crate::attack::DamageType;
use crate::id::ContentId;
use crate::limits::MAX_SLOT_LEVEL;
use crate::rewards::{EquipmentRules, RewardEffect, Rewards};

#[cfg(test)]
pub(super) use super::starlight::bundled_rules;

/// The highest level a slot reaches (ADR-SIM-5).
pub(super) const MAX_LEVEL: u8 = MAX_SLOT_LEVEL;

/// A slot's level and kills after `kills` more: one level at a time while the kills reach the
/// level's need; at `MAX_LEVEL` the kills are dropped. A level of 0 (a slot that never held a
/// star) gains nothing. Pure.
pub(super) fn gain(rules: &EquipmentRules, level: u8, expertise: u32, kills: u32) -> (u8, u32) {
    if level == 0 {
        return (0, expertise);
    }
    let (mut level, mut expertise) = (level.min(MAX_LEVEL), expertise.saturating_add(kills));
    while level < MAX_LEVEL {
        // The pack's list has `MAX_LEVEL - 1` entries (validated); a missing one stops the climb.
        let Some(&need) = rules.kills_to_next.get(usize::from(level) - 1) else {
            break;
        };
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

/// `amount` at `level`: `level_pct` percent more for each level above 1, floored. A level of
/// 0 reads as 1 and one past `MAX_LEVEL` as `MAX_LEVEL`.
pub(super) fn level_scaled(rules: &EquipmentRules, amount: u32, level: u8) -> u32 {
    let above = u64::from(level.clamp(1, MAX_LEVEL) - 1);
    let scaled = u64::from(amount) * (100 + u64::from(rules.level_pct) * above) / 100;
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// The slot index of the first rune slot: the category slots come first.
pub(super) fn first_rune_slot(rules: &EquipmentRules) -> usize {
    rules.categories.iter().map(|c| usize::from(c.slots)).sum()
}

/// The empty slots of the layout, level 0 and no expertise: each category's slots in list order,
/// then the rune slots.
pub(super) fn fresh_slots(rules: &EquipmentRules) -> Vec<EquipSlot> {
    let slot = |kind| EquipSlot {
        kind,
        held: None,
        level: 0,
        expertise: 0,
    };
    let mut slots = Vec::with_capacity(first_rune_slot(rules) + usize::from(rules.rune_slots));
    for (i, c) in rules.categories.iter().enumerate() {
        let kind = SlotKind::Category(u8::try_from(i).unwrap_or(u8::MAX));
        slots.extend((0..c.slots).map(|_| slot(kind)));
    }
    slots.extend((0..rules.rune_slots).map(|_| slot(SlotKind::Rune)));
    slots
}

/// The category slot a star fits: the place of its reward's `category` in the rules' list. A
/// reward with none (a Growth star, the lone random rune) fits no category slot.
pub(super) fn star_category(rules: &EquipmentRules, rewards: &Rewards, place: u16) -> Option<u8> {
    let category = rewards
        .rewards()
        .get(usize::from(place))?
        .category
        .as_ref()?;
    let at = rules.categories.iter().position(|c| &c.id == category)?;
    u8::try_from(at).ok()
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

/// Whether a star at `place` fits a slot of `kind`: a category slot takes the stars of its
/// category, the Lumine slot any star. A rune slot takes no star.
pub(super) fn fits_slot(
    rules: &EquipmentRules,
    rewards: &Rewards,
    kind: SlotKind,
    place: u16,
) -> bool {
    match kind {
        SlotKind::Category(c) => star_category(rules, rewards, place) == Some(c),
        SlotKind::Lumine => true,
        SlotKind::Rune => false,
    }
}

/// The first slot other than `at` (the category slots, then the Lumine slot) holding a star that
/// shares an option with the star at `place`.
pub(super) fn clashes(
    rewards: &Rewards,
    equipment: &Equipment,
    place: u16,
    at: usize,
) -> Option<usize> {
    let others = equipment.slots.iter().chain(equipment.lumine.as_ref());
    others
        .enumerate()
        .find_map(|(i, other)| match (i != at, other.held) {
            (true, Some(Held::Star(q))) if shares_option(rewards, place, q) => Some(i),
            _ => None,
        })
}

/// What is wrong with the stars an equipment holds (a savepoint is refused for it): a star in a
/// slot of another category, or two stars sharing an option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Misfit {
    /// A category slot holds a star of another category, or one with no category.
    Category,
    /// Two equipped stars (the Lumine slot included) share an option.
    SharedOption,
}

/// The first `Misfit` of `equipment`: a category fault in slot order (the Lumine slot last), else
/// the first option clash in the same order. Pure; the rune slots are `holds_right_kinds`'
/// business, and the stars' places must be in range.
pub(crate) fn misfit(
    rules: &EquipmentRules,
    rewards: &Rewards,
    equipment: &Equipment,
) -> Option<Misfit> {
    let stars = || {
        equipment
            .slots
            .iter()
            .chain(equipment.lumine.as_ref())
            .enumerate()
            .filter_map(|(i, s)| match s.held {
                Some(Held::Star(p)) => Some((i, s.kind, p)),
                _ => None,
            })
    };
    if stars()
        .any(|(_, kind, p)| !matches!(kind, SlotKind::Rune) && !fits_slot(rules, rewards, kind, p))
    {
        return Some(Misfit::Category);
    }
    stars()
        .any(|(i, _, p)| clashes(rewards, equipment, p, i).is_some())
        .then_some(Misfit::SharedOption)
}

/// The stars equipped, in slot order and the Lumine slot last.
#[cfg(test)]
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

/// The stars equipped with their slot's level, in slot order and the Lumine slot last.
pub(super) fn equipped_levels(equipment: &Equipment) -> Vec<(u16, u8)> {
    equipment
        .slots
        .iter()
        .chain(equipment.lumine.as_ref())
        .filter_map(|s| match s.held {
            Some(Held::Star(p)) => Some((p, s.level)),
            _ => None,
        })
        .collect()
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

/// The hit points a swap leaves when the most hit points go from `was` to `now`: the same
/// fraction, rounded down, so swapping a star in and out never heals. A player at 0 stays at 0 and
/// a living one never reaches 0; `was` of 0 only clamps.
pub(super) fn keep_fraction(hp: u32, was: u32, now: u32) -> u32 {
    if hp == 0 {
        return 0;
    }
    if was == 0 {
        return hp.min(now);
    }
    let kept = u64::from(hp) * u64::from(now) / u64::from(was);
    u32::try_from(kept).unwrap_or(u32::MAX).max(1).min(now)
}

/// How a change of the equipped set arrives: at the start of a run, or as a swap in play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Arrival {
    /// Tick 0: loading the run is no equip, so a skill starts ready.
    Start,
    /// An equip, a replace or a level-up in play: a skill that takes the slot starts its full
    /// cooldown.
    Swap,
}

/// Writes the change of what is equipped: `before` is the stars equipped before the change, with
/// their levels, in slot order (empty for nothing), the equipment of `career` holds the set after. The build and
/// the grants change by the difference (`Build::write`, `Grants::write`), and the set counts are
/// the members equipped. A change of the most hit points leaves the hit points at the same
/// fraction (`keep_fraction`), not the heal or clamp the build's write did. A stage skill that
/// arrives as a `Swap` starts the full cooldown a press would set (`skill::cooldown_ticks`).
pub(super) fn refresh(
    stage: &World,
    rewards: &Rewards,
    player: &mut PlayerState,
    career: &mut Career,
    before: &[(u16, u8)],
    arrival: Arrival,
) {
    let Career {
        progress,
        offers,
        regen,
        equipment,
        ..
    } = career;
    let after_levels = equipped_levels(equipment);
    let after: Vec<u16> = after_levels.iter().map(|s| s.0).collect();
    let before_places: Vec<u16> = before.iter().map(|s| s.0).collect();
    let counts = counts_of(rewards, after.iter().copied());
    let was = Build::of_levelled(stage, rewards, before);
    let now = Build::of_levelled(stage, rewards, &after_levels);
    let (hp_was, max_was) = (player.hp, player.max_hp);
    Build::write(&was, &now, player, progress, regen);
    if player.max_hp != max_was {
        player.hp = keep_fraction(hp_was, max_was, player.max_hp);
        super::player::rally_clamp(player);
    }
    let grants = Grants::of(rewards, &after);
    let was_grants = Grants::of(rewards, &before_places);
    // Only a skill that changes takes the entry cooldown (a grade switch keeps its own).
    let entry = match (arrival, grants.skill()) {
        (Arrival::Swap, Some((index, level))) if was_grants.skill() != grants.skill() => rewards
            .stage_skills()
            .get(usize::from(index))
            .and_then(|def| def.levels.get(usize::from(level).saturating_sub(1)))
            .map_or(0, |l| {
                skill::cooldown_ticks(l.cooldown_ticks, progress.as_ref(), player)
            }),
        _ => 0,
    };
    Grants::write(&was_grants, &grants, stage, player, entry);
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
        if let Some(n) = offers.as_mut().and_then(|o| o.set_counts.get_mut(set)) {
            *n = u8::try_from(members).unwrap_or(u8::MAX);
        }
    }
}

/// Counts the enemies defeated this tick for each player into `Scratch::kills_by`: an enemy at 0
/// hit points that is not a kill burst's, for the player that defeated it (the rule of
/// `heal::kills_heal`). Called by `collide` before the dead are removed, in equipment mode only.
pub(super) fn count_kills(s: &mut State) {
    for i in 0..s.enemies.len() {
        if s.enemies[i].hp == 0 && !s.enemies[i].burst {
            let who = super::killer_of(s, i).get();
            if let Some(n) = s.scratch.kills_by.get_mut(who) {
                *n = n.saturating_add(1);
            }
        }
    }
}

/// Gives each player its kills of the tick as expertise: every slot holding a star (a category
/// slot or the Lumine slot) gains them all (`gain`); a player whose slot rose a level has its
/// build rewritten from the levels before and after (`refresh`). Empties `Scratch::kills_by`
/// (`collide` alone, as a test calls it, leaves the counts for the next `credit_kills`).
pub(super) fn credit_kills(stage: &World, rewards: &Rewards, s: &mut State) {
    let kills = std::mem::take(&mut s.scratch.kills_by);
    for me in s.players.indices() {
        let n = kills[me.get()];
        if n == 0 {
            continue;
        }
        let (player, career) = s.players.both_mut(me);
        let eq = &career.equipment;
        let rises = |slot: &EquipSlot| {
            matches!(slot.held, Some(Held::Star(_)))
                && gain(&stage.equipment_rules, slot.level, slot.expertise, n).0 != slot.level
        };
        // A level-up is rare: the levels before are listed only when one happens.
        let before = eq
            .slots
            .iter()
            .chain(eq.lumine.as_ref())
            .any(rises)
            .then(|| equipped_levels(eq));
        let eq = &mut career.equipment;
        for slot in eq.slots.iter_mut().chain(eq.lumine.as_mut()) {
            if matches!(slot.held, Some(Held::Star(_))) {
                (slot.level, slot.expertise) =
                    gain(&stage.equipment_rules, slot.level, slot.expertise, n);
            }
        }
        if let Some(before) = before {
            refresh(stage, rewards, player, career, &before, Arrival::Swap);
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
    rules: &EquipmentRules,
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
            if !fits_slot(rules, rewards, target.kind, place) {
                return wrong;
            }
            if let Some(i) = clashes(rewards, equipment, place, at) {
                let slot = u32::try_from(i).unwrap_or(u32::MAX);
                return Err(SlotError::OptionHeld { slot });
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
/// share an option. A switch costs no expertise: `apply` keeps the whole level and kills.
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
    check(
        &stage.equipment_rules,
        rewards,
        &career.equipment,
        item,
        slot,
        false,
    )?;
    apply(stage, rewards, player, career, item, slot, false);
    Ok(())
}

/// Replaces the star or rune in the occupied `slot` by the inventory item `item`: the displaced
/// one takes the item's place in the inventory. The slot keeps its level and kills for a same-option
/// switch (`is_switch`) and otherwise keeps half the level, at least 1, and none of the kills.
pub(super) fn replace(
    stage: &World,
    rewards: &Rewards,
    player: &mut PlayerState,
    career: &mut Career,
    item: u32,
    slot: u32,
) -> Result<(), SlotError> {
    check(
        &stage.equipment_rules,
        rewards,
        &career.equipment,
        item,
        slot,
        true,
    )?;
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
    let switch = replace && is_switch(rewards, &career.equipment, item, slot);
    let (item, at) = (item as usize, slot as usize);
    let before = equipped_levels(&career.equipment);
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
    if replace && !switch {
        // A replace by a star sharing no option keeps half the level and none of the kills; a
        // switch keeps both. (A rune slot sits at Lv1 with no kills, so the rule leaves it there.)
        target.level = (target.level / 2).max(1);
        target.expertise = 0;
    }
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
        refresh(stage, rewards, player, career, &before, Arrival::Swap);
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
