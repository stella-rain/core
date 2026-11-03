//! Where a player's damage meets a foe's hit points (ADR-SIM-7). `damage` (a hit, a damage over
//! time tick, a kill burst) and `combat::deal_echoes` work out their number and call `land`, which is the one place that
//! lowers a foe's, the boss's or a boss part's hit points, credits the kill, reports `Hit` and `Died` and raises the shooter's aggro.

use super::entity::Who;
use super::state::{PlayerIdx, State};
use super::{World, death_event, kind_of, record_kill};
use crate::attack::{DamageType, MatchUp};
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::rune::Rune;

/// What a landed hit reports besides its target and damage.
#[derive(Clone, Copy)]
pub(super) struct Landing {
    /// The target's entity ID (`s.id_of(target)`), which the caller has already.
    pub id: EntityId,
    /// `Hit.source`, and whose aggro rises.
    pub source: EntityId,
    pub damage_type: Option<DamageType>,
    pub match_up: MatchUp,
    pub rune: Option<Rune>,
    pub crit: bool,
    pub echo: bool,
    pub buffed_by: Option<EntityId>,
}

/// What `land` found: whether the target is now at 0 hit points.
pub(super) struct Landed {
    pub dead: bool,
}

/// `shown` at `pct` percent, truncated toward zero once, and at least 1 for a hit above 0
/// (ADR-SIM-8 point 3).
pub(super) fn party_scaled(pct: u32, shown: u32) -> u32 {
    let scaled = u32::try_from(u64::from(shown) * u64::from(pct) / 100).unwrap_or(u32::MAX);
    if shown > 0 { scaled.max(1) } else { 0 }
}

/// Lowers `target`'s hit points by a hit of `shown` damage, credits the kill to `me` when an
/// enemy falls, and reports the `Hit`, the `Died` and the source's aggro. The caller has
/// already checked the target stands and may be damaged.
///
/// `shown` is what the `Hit` event, the aggro and the echo's size carry. `applied` is the only
/// number that reaches the hit points: a foe's is `shown` times the percent of the present party
/// (ADR-SIM-8), read at this hit from `s.players`, once per hit; an ally's is `shown`.
#[inline(always)]
pub(super) fn land(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    target: Who,
    shown: u32,
    how: Landing,
    events: &mut Vec<DomainEvent>,
) -> Landed {
    let foe = !State::is_ally(target);
    let applied = if foe {
        party_scaled(stage.tuning.party_damage_pct[s.players.len() - 1], shown)
    } else {
        shown
    };
    let hp = s.hp_mut(target);
    let before = *hp;
    *hp = hp.saturating_sub(applied);
    let dead = *hp == 0;
    if dead && before > 0 {
        if let Who::Enemy(i) = target {
            record_kill(s, i, me);
        }
    }
    events.push(DomainEvent::Hit {
        source: how.source,
        target: how.id,
        target_kind: kind_of(s, target),
        damage: shown,
        damage_type: how.damage_type,
        match_up: how.match_up,
        rune: how.rune,
        crit: how.crit,
        echo: how.echo,
        player: foe.then(|| s.players.id(me)),
        buffed_by: how.buffed_by,
    });
    if dead {
        let killer = (foe && before > 0).then_some(me);
        events.push(death_event(s, target, how.id, killer));
    }
    s.raise_aggro(
        how.source,
        shown.saturating_mul(stage.tuning.aggro_per_damage),
    );
    Landed { dead }
}
