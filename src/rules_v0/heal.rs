//! Healing (a stage with `heal`): a boss kill heals the main character `boss_pct` percent of its
//! most hit points, and a regular enemy's kill may drop a heal pickup, which the main character
//! takes on touch for `amount` hit points. Both heal as far as the most allows (`Healed`).
//!
//! The drop draws from the run's one generator (ADR-033, the same stream as the falling stars):
//! in the collisions, after the stars' drops of the tick, each regular kill in entity-ID order
//! draws once, `drop_one_in` being the range, and drops a pickup where the enemy fell when the
//! draw is 0 and the map holds fewer than `cap`. The draw happens whether or not the map has
//! room, so the stream advances once per regular kill and nothing else. An elite drops none. A
//! boss kill draws nothing. Then the main character takes every pickup inside its pickup circle,
//! in the order they dropped. Nothing heals a main character at 0 hit points.

use crate::engine::{HealPickup, PlayerState, State};
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::stage::Stage;

use super::{PLAYER_ID, overlaps, widened};

/// The boss died this tick: it heals the main character `boss_pct` percent of its most.
pub(super) fn boss_killed(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(cfg) = &stage.heal else {
        return;
    };
    if s.player.hp == 0 {
        return;
    }
    let most = u64::from(s.player.max_hp);
    let want = most * u64::from(cfg.boss_pct) / 100;
    heal(&mut s.player, want, events);
}

/// Stars and pickups for the regular enemies at 0 hp, before they are removed (see the module
/// doc): one draw each.
pub(super) fn drop_from_defeated(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(cfg) = &stage.heal else {
        return;
    };
    let State {
        rng,
        enemies,
        next_id,
        heal_pickups,
        ..
    } = s;
    let Some(pickups) = heal_pickups.as_mut() else {
        return;
    };
    for e in enemies
        .iter()
        .filter(|e| e.hp == 0 && !e.burst && e.elite.is_none())
    {
        let drops = rng.below(cfg.drop_one_in) == 0;
        if !drops || pickups.len() >= usize::from(cfg.cap) {
            continue;
        }
        let id = EntityId(*next_id);
        *next_id += 1;
        pickups.push(HealPickup { id, at: e.at });
        events.push(DomainEvent::HealDropped { pickup: id });
    }
}

/// The main character takes the pickups inside its circle, in the order they dropped.
pub(super) fn pick_up(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(cfg) = &stage.heal else {
        return;
    };
    if s.player.hp == 0 {
        return;
    }
    let at = s.player.at;
    let radius = widened(cfg.pickup_radius.0, s.player.pickup_radius_pct);
    let taken = {
        let Some(pickups) = s.heal_pickups.as_mut() else {
            return;
        };
        let (taken, kept): (Vec<HealPickup>, Vec<HealPickup>) = std::mem::take(pickups)
            .into_iter()
            .partition(|p| overlaps(at, radius, p.at, 0));
        *pickups = kept;
        taken
    };
    for _ in taken {
        heal(&mut s.player, u64::from(cfg.amount), events);
    }
}

/// Heals `player` by `want` hit points, as far as its most allows, and reports what it healed.
fn heal(player: &mut PlayerState, want: u64, events: &mut Vec<DomainEvent>) {
    let before = player.hp;
    let healed = (u64::from(before) + want).min(u64::from(player.max_hp));
    player.hp = u32::try_from(healed).unwrap_or(player.max_hp);
    events.push(DomainEvent::Healed {
        target: PLAYER_ID,
        amount: player.hp.saturating_sub(before),
    });
}
