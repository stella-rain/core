//! The equipment slots of a player in equipment mode (ADR-SIM-3 point 2, ADR-SIM-7): the layout,
//! what a star's category and option are until the pack says, and the build re-derived from the
//! stars equipped. The inventory does not count toward the build (only the stars in a slot and
//! the Lumine slot do); a change of what is equipped goes through `refresh`, which writes the
//! change from the build of the set before to the build of the set after.
//!
//! Placeholders (ADR-CONT-3 point 8; the pack will carry them): the layout, each star's category
//! and each star's option. They are derived here from the effects and are the one place to
//! replace.

use super::World;
use super::build::Build;
use super::equipment::{EquipSlot, Equipment, Held, SlotKind};
use super::offer_rating::category_of;
use super::state::{Career, Offers, PlayerState};
use crate::attack::DamageType;
use crate::id::ContentId;
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
type Option3<'a> = (RewardEffect, Option<DamageType>, Option<&'a ContentId>);

/// The options of the star at `place`: one per effect entry.
fn options(rewards: &Rewards, place: u16) -> Vec<Option3<'_>> {
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
pub(super) fn equipped_counts(rewards: &Rewards, equipment: &Equipment) -> Vec<u8> {
    let mut counts = vec![0u8; rewards.rewards().len()];
    for p in equipped(equipment) {
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

/// Writes the change of what is equipped: `before` is the equipped counts before the change
/// (empty for nothing), the equipment of `career` holds the set after. The build changes by the
/// difference (`Build::write`), and the set counts are the members equipped.
pub(super) fn refresh(
    stage: &World,
    rewards: &Rewards,
    player: &mut PlayerState,
    career: &mut Career,
    before: &[u8],
) {
    let Career {
        progress,
        offers,
        regen,
        equipment,
        ..
    } = career;
    let counts = equipped_counts(rewards, equipment);
    let mut local = None;
    let o = match offers {
        Some(o) => o,
        None => local.insert(throwaway(rewards)),
    };
    let was = Build::of(stage, rewards, before);
    let now = Build::of(stage, rewards, &counts);
    Build::write(&was, &now, player, progress, regen, o);
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
