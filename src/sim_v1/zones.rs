//! Zones (core Z1, DS10): areas that a `zone` node of an attack places and that do not move.
//! A zone warns for its wind-up ticks (the snapshot shows it), then hurts for its active ticks,
//! then is gone. Placed on tick T with a wind-up of W, it warns on ticks T to T + W - 1 and is
//! active from T + W for its active ticks; a wind-up of 0 hurts on the tick it is placed.
//! A `warn_only` zone (Z3a-4) has no active ticks: it warns on ticks T to T + W - 1 (W at least
//! 1), is removed by the advance of tick T + W, and never hurts.
//!
//! - A zone hurts each target at most once, when it first overlaps one while active. What
//!   hurts is decided with the hit boxes' (`hit_boxes`): a friendly zone hurts the boss, its
//!   parts and the enemies, an enemy's the main character and the companions.
//! - A zone is not tied to its placer: it stays when the placer dies or a boss leaves the phase
//!   that placed it, until its time is up or a phase's `clear_zones` removes the enemy ones.
//! - At most `MAX_ZONES_ALIVE` are alive; one placed over the cap is dropped, in the order the
//!   nodes are carried out, so which are dropped is deterministic.
//!
//! What it does not do yet: a zone has no combo credit or smash of its own; a friendly zone
//! hurts as a plain hit.

use crate::sim_v1::state::{State, ZoneState};

use super::scratch::Scratch;
use super::{MAX_ZONES_ALIVE, count};

/// Places `zone`, unless there are `MAX_ZONES_ALIVE` already. A zone's hit list comes from, and
/// goes back to, the pool of the attack runs' (`Scratch::take_hit_list`), so that a steady tick
/// allocates nothing when an enemy's zone hurts the main character (T1b).
pub(super) fn place(s: &mut State, zone: ZoneState) {
    if count(&s.zones) >= MAX_ZONES_ALIVE {
        Scratch::recycle_hits(&mut s.scratch.hit_lists, zone.hits);
        return;
    }
    s.zones.push(zone);
}

/// Every zone goes on by a tick: its warning runs down, then its hurting, and it is gone when
/// both have. Called before the attacks are carried out, so a zone placed this tick is not
/// advanced until the next.
pub(super) fn advance(s: &mut State) {
    let pool = &mut s.scratch.hit_lists;
    s.zones.retain_mut(|z| {
        if z.wind_left > 0 {
            z.wind_left -= 1;
        } else {
            z.active_left = z.active_left.saturating_sub(1);
        }
        let alive = z.wind_left > 0 || z.active_left > 0;
        if !alive {
            Scratch::recycle_hits(pool, std::mem::take(&mut z.hits));
        }
        alive
    });
}
