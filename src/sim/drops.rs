//! Drops in equipment mode (ADR-SIM-2 point 2, ADR-SIM-5 point 8, ADR-SIM-7): a falling star taken
//! gives every member that is not out their own roll into their own inventory, and a boss kill
//! gives every member that is up personal drops. Offers mode never runs this file: `stars.rs`
//! branches on `World.equipment_mode` before its own body.
//!
//! A roll draws from the rolling member's own offer stream, in this order, fixed for
//! `sim_version` 2: the kind, `below(1000)`, a rune when it is under the share; then for a rune
//! one `below(n)` over the runes of the run's rune pool in `Rune::ALL` order, for a star one
//! `below(total)` over the eligible rewards' weights. A star roll that finds nothing eligible
//! stops after the kind draw. Past the inventory bound the draws are still made and the item is
//! not added.

use super::equipment::{Equipment, Held};
use super::state::{Career, PlayerIdx, Players, StarField, State};
use super::{World, overlaps, pickup_pct, runes, stars, widened};
use crate::event::DomainEvent;
use crate::limits::MAX_INVENTORY;
use crate::rewards::{Rarity, Rewards};
use crate::rune::Rune;

/// Personal drops each member that is up gets from a boss kill. A placeholder: the pack will
/// carry it (ADR-CONT-3 point 8).
pub(super) const BOSS_DROPS: u8 = 1;

/// Out of 1,000, the share of a drop that is a rune. A placeholder: the pack will carry it.
pub(super) const RUNE_SHARE_PERMILLE: u32 = 100;

const PER_MILLE: u32 = 1000;

/// What a roll reads of one reward of the run's file.
#[derive(Clone, Copy, Debug)]
struct DropEntry {
    rarity: Rarity,
    weight: u16,
    named: bool,
    random_rune: bool,
}

/// The run's rewards as a roll reads them, by place in the file: resolved once with the world.
#[derive(Clone, Debug, Default)]
pub(crate) struct DropTable(Vec<DropEntry>);

impl DropTable {
    pub(crate) fn of(rewards: &Rewards) -> DropTable {
        DropTable(
            rewards
                .rewards()
                .iter()
                .map(|r| DropEntry {
                    rarity: r.rarity,
                    weight: r.weight,
                    named: r.named,
                    random_rune: r.is_random_rune(),
                })
                .collect(),
        )
    }
}

/// Whether the player holds the star at `place`: in the inventory, a slot or the Lumine slot.
pub(super) fn holds(equipment: &Equipment, place: u16) -> bool {
    let is = |h: &Held| matches!(h, Held::Star(p) if *p == place);
    equipment.inventory.iter().any(is)
        || equipment
            .slots
            .iter()
            .filter_map(|s| s.held.as_ref())
            .any(is)
        || equipment
            .lumine
            .as_ref()
            .and_then(|s| s.held.as_ref())
            .is_some_and(is)
}

/// The entry at `place` when it may drop at `rarity`: a star of that rarity, not the random rune
/// and, if named, not held.
fn eligible<'a>(
    table: &'a DropTable,
    equipment: &Equipment,
    place: u16,
    rarity: Rarity,
) -> Option<&'a DropEntry> {
    table
        .0
        .get(usize::from(place))
        .filter(|e| e.rarity == rarity && !e.random_rune && !(e.named && holds(equipment, place)))
}

/// The draw weight of a star: its own, at least 1; a named star's is scaled by the member's
/// `drop_weight` percent (floored, at least 1).
fn weight(entry: &DropEntry, drop_weight_pct: u32) -> u32 {
    let base = u64::from(entry.weight).max(1);
    let scaled = match entry.named {
        true => (base * (100 + u64::from(drop_weight_pct)) / 100).max(1),
        false => base,
    };
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// One roll of a member at `rarity` (the rarity a fall or a boss fixed): the item, if any, goes
/// into the inventory unless it is at its bound.
pub(super) fn roll(
    table: &DropTable,
    f: &StarField,
    career: &mut Career,
    share: u32,
    drop_weight_pct: u32,
    rarity: Rarity,
) {
    let Career {
        offer, equipment, ..
    } = career;
    let item = match offer.below(PER_MILLE) < share {
        true => rune(f, offer).map(Held::Rune),
        false => star(table, f, equipment, offer, drop_weight_pct, rarity).map(Held::Star),
    };
    if let Some(item) = item
        && equipment.inventory.len() < MAX_INVENTORY
    {
        equipment.inventory.push(item);
    }
}

/// A rune of the run's rune pool: one draw.
fn rune(f: &StarField, rng: &mut crate::rng::SplitMix64) -> Option<Rune> {
    let pool = || {
        Rune::ALL
            .into_iter()
            .filter(|r| runes::in_pool(f.rune_pool, *r))
    };
    let n: u32 = pool().map(|_| 1).sum();
    if n == 0 {
        return None;
    }
    let mut left = rng.below(n);
    pool().find(|_| {
        let hit = left == 0;
        left = left.wrapping_sub(1);
        hit
    })
}

/// A star at the rarest rarity from `rarity` down with a reward to drop: one draw, none when
/// no reward is eligible.
fn star(
    table: &DropTable,
    f: &StarField,
    equipment: &Equipment,
    rng: &mut crate::rng::SplitMix64,
    drop_weight_pct: u32,
    rarity: Rarity,
) -> Option<u16> {
    let any = |r: Rarity| {
        f.pool
            .iter()
            .any(|&p| eligible(table, equipment, p, r).is_some())
    };
    let at = Rarity::ALL
        .into_iter()
        .rev()
        .filter(|r| *r <= rarity && *r >= f.rarity.0)
        .find(|r| any(*r))?;
    let weights = || {
        f.pool.iter().filter_map(move |&p| {
            eligible(table, equipment, p, at).map(|e| (p, weight(e, drop_weight_pct)))
        })
    };
    let total: u32 = weights().map(|w| w.1).fold(0, u32::saturating_add);
    let mut left = rng.below(total.max(1));
    weights().find_map(|(p, w)| {
        if left < w {
            return Some(p);
        }
        left -= w;
        None
    })
}

/// Every member that is not out rolls once for a star taken, in slot order.
fn star_taken(stage: &World, f: &StarField, players: &mut Players, rarity: Rarity) {
    for me in players.indices() {
        if players[me].is_out() {
            continue;
        }
        let career = players.career_mut(me);
        let pct = stage.drop_weight_pct;
        roll(&stage.drops, f, career, RUNE_SHARE_PERMILLE, pct, rarity);
    }
}

/// `me` takes every star inside its circle, in the order they dropped; each gives every member
/// that is not out a roll of their own, and nobody an offer.
pub(super) fn pick_up(stage: &World, s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    let Some(cfg) = &stage.stars else {
        return;
    };
    if s.players[me].hp == 0 {
        return;
    }
    let at = s.players[me].at;
    let radius = widened(cfg.pickup_radius.0, pickup_pct(s, me));
    let State { stars, players, .. } = s;
    let Some(f) = stars.as_mut() else {
        return;
    };
    // The stars wait in a list the field also lends to the rolls: move it out and back, which
    // allocates nothing.
    let mut on_map = std::mem::take(&mut f.on_map);
    on_map.retain(|star| {
        let taken = overlaps(at, radius, star.at, 0);
        if taken {
            events.push(DomainEvent::StarTaken {
                star: star.id,
                rarity: star.rarity,
            });
            star_taken(stage, f, players, star.rarity);
        }
        !taken
    });
    f.on_map = on_map;
}

/// The boss died this tick: every present member that is up gets `BOSS_DROPS` rolls of its own,
/// at one rarity for the party, the same one a boss offer would have (the highest of the drop
/// table's tier for the highest level among those up, held to the difficulty's range). A downed
/// or out member gets none.
pub(super) fn boss_killed(stage: &World, s: &mut State) {
    let Some(level) = stars::top_up_level(s) else {
        return;
    };
    let Some(rarity) = stage
        .stars
        .as_ref()
        .and_then(|cfg| stars::tier_of(cfg, level))
        .and_then(|t| t.weights.highest())
    else {
        return;
    };
    let State { stars, players, .. } = s;
    let Some(f) = stars.as_ref() else {
        return;
    };
    let rarity = rarity.clamp(f.rarity.0, f.rarity.1);
    for me in players.indices() {
        if !players[me].is_up() {
            continue;
        }
        for _ in 0..BOSS_DROPS {
            let career = players.career_mut(me);
            let pct = stage.drop_weight_pct;
            roll(&stage.drops, f, career, RUNE_SHARE_PERMILLE, pct, rarity);
        }
    }
}
