//! Stun (ADR-085): what a smash's stun does to the enemy it hits. The status is `add_stun`'s
//! (`entity.rs`). Only an enemy takes a stun: a boss and its parts resist it, and the main
//! character's and the companions' hits never stun anything. An elite takes its share of the
//! ticks (ADR-085 point 6), and a stun of no ticks does nothing.

use super::World;
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::sim::state::{State, StatusKind, StatusState};
use crate::validate::limits::MAX_STUN_TICKS;

use super::attacks;
use super::entity::Who;

/// Whether `statuses` holds a stun.
pub(super) fn stunned(statuses: &[StatusState]) -> bool {
    statuses.iter().any(|st| st.kind == StatusKind::Stun)
}

/// Stuns `target` for a smash's `ticks`: all of them on a regular enemy, the elite's share on an
/// elite, nothing on any other target.
pub(super) fn apply(
    stage: &World,
    s: &mut State,
    target: Who,
    source: EntityId,
    ticks: u32,
    events: &mut Vec<DomainEvent>,
) {
    let Who::Enemy(i) = target else {
        return;
    };
    if s.enemies[i].stun_immune > 0 {
        return;
    }
    let ticks = ticks.min(MAX_STUN_TICKS);
    let elite = stage.defs.agent(s.enemies[i].def).elite.is_some();
    let ticks = if elite {
        ticks.saturating_mul(s.stun.elite_pct) / 100
    } else {
        ticks
    };
    if ticks > 0 {
        // The stun cancels what the enemy was doing: its running attack, in any phase. Orbs it
        // has spawned fly on (ADR-085 point 2).
        let id = s.id_of(target);
        s.add_stun(target, source, ticks, events);
        attacks::end_runs_of(stage, s, id);
    }
}
