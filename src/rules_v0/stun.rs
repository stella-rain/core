//! Stun (ADR-085): what a smash's stun does to the enemy it hits. The status is `add_stun`'s
//! (`entity.rs`). Only an enemy takes a stun: a boss and its parts resist it, and the main
//! character's and the companions' hits never stun anything. An elite takes its share of the
//! ticks (ADR-085 point 6), and a stun of no ticks does nothing.

use crate::engine::State;
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::stage::Stage;
use crate::validate::limits::MAX_STUN_TICKS;

use super::agents::agent_def;
use super::entity::Who;

/// Stuns `target` for a smash's `ticks`: all of them on a regular enemy, the elite's share on an
/// elite, nothing on any other target.
pub(super) fn apply(
    stage: &Stage,
    s: &mut State,
    target: Who,
    source: EntityId,
    ticks: u32,
    events: &mut Vec<DomainEvent>,
) {
    let Who::Enemy(i) = target else {
        return;
    };
    let ticks = ticks.min(MAX_STUN_TICKS);
    let elite = agent_def(stage, s.enemies[i].key).elite.is_some();
    let ticks = if elite {
        ticks.saturating_mul(s.stun.elite_pct) / 100
    } else {
        ticks
    };
    if ticks > 0 {
        s.add_stun(target, source, ticks, events);
    }
}
