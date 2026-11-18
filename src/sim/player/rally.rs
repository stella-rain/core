//! Rally of the player's tick.

use crate::event::DomainEvent;
use crate::sim::World;
use crate::sim::state::{PlayerIdx, PlayerState, State};

/// A hit took `lost` hit points from `me` (after the absorb and the defence): `pct` percent of them
/// stay as rally, times the giant trigger's factor while it runs, up to the hit points `me` is
/// missing, and the rally's ticks start again (ADR-SIM-9). Nothing for a hit that took none or a
/// player at 0 hit points.
pub(in crate::sim) fn rally_from_hit(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    lost: u32,
    events: &mut Vec<DomainEvent>,
) {
    let Some(rules) = stage.tuning.rally else {
        return;
    };
    if lost == 0 || s.players[me].hp == 0 {
        return;
    }
    let factor = if s.players[me].combat.giant_trigger > 0 {
        stage.trigger_of(s, me).map_or(100, |g| g.rally_pct)
    } else {
        100
    };
    let added = u64::from(lost) * u64::from(rules.pct) * u64::from(factor) / 10_000;
    let p = &mut s.players[me];
    let was = p.combat.rally;
    let missing = p.max_hp.saturating_sub(p.hp);
    let rally = was
        .saturating_add(u32::try_from(added).unwrap_or(u32::MAX))
        .min(missing);
    if rally > 0 {
        p.combat.rally_ticks = rules.ticks;
    }
    if rally != was {
        // A heal can leave more rally than the missing points a later hit allows: it is held.
        p.combat.rally = rally;
        if rally > was {
            rally_gained(events, me, rally - was, rally);
        }
    }
}

/// A combo hit of `me` dealt `dealt` damage to a foe: `regain_pct` percent of it is won back from
/// the rally as hit points, never more than the rally or the hit points missing (ADR-SIM-9).
pub(in crate::sim) fn rally_win_back(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    dealt: u32,
    events: &mut Vec<DomainEvent>,
) {
    let Some(rules) = stage.tuning.rally else {
        return;
    };
    let p = &mut s.players[me];
    if dealt == 0 || p.combat.rally == 0 || !p.is_up() {
        return;
    }
    let back = u64::from(dealt) * u64::from(rules.regain_pct) / 100;
    let won = u32::try_from(back)
        .unwrap_or(u32::MAX)
        .min(p.combat.rally)
        .min(p.max_hp.saturating_sub(p.hp));
    if won == 0 {
        return;
    }
    p.hp += won;
    p.combat.rally -= won;
    if p.combat.rally == 0 {
        p.combat.rally_ticks = 0;
    }
    rally_won(events, me, won, p.combat.rally);
}

/// One tick of the rally's countdown: when it reaches 0 the rest is lost.
pub(super) fn rally_tick(s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    if s.players[me].combat.rally_ticks == 0 {
        return;
    }
    s.players[me].combat.rally_ticks -= 1;
    if s.players[me].combat.rally_ticks == 0 {
        rally_lapse(s, me, events);
    }
}

/// The rest of `me`'s rally is lost (its ticks ran out or it went down).
pub(super) fn rally_lapse(s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    let c = &mut s.players[me].combat;
    c.rally_ticks = 0;
    if c.rally > 0 {
        c.rally = 0;
        rally_lapsed(events, me);
    }
}

/// Keeps `p`'s rally within its most hit points, the invariant the savepoint's decode checks. A
/// player with no rally is untouched.
pub(in crate::sim) fn rally_clamp(p: &mut PlayerState) {
    if p.combat.rally > p.max_hp {
        p.combat.rally = p.max_hp;
    }
}

/// `me`'s hit left `amount` more rally, `rally` in all. Cold: a run with no rally never calls it.
#[cold]
#[inline(never)]
fn rally_gained(events: &mut Vec<DomainEvent>, me: PlayerIdx, amount: u32, rally: u32) {
    events.push(DomainEvent::RallyGained {
        player: me,
        amount,
        rally,
    });
}

/// `me`'s combo hit won `amount` hit points back, `rally` left. Cold, as `rally_gained`.
#[cold]
#[inline(never)]
fn rally_won(events: &mut Vec<DomainEvent>, me: PlayerIdx, amount: u32, rally: u32) {
    events.push(DomainEvent::RallyWon {
        player: me,
        amount,
        rally,
    });
}

/// The rest of `me`'s rally was lost. Cold, as `rally_gained`.
#[cold]
#[inline(never)]
fn rally_lapsed(events: &mut Vec<DomainEvent>, me: PlayerIdx) {
    events.push(DomainEvent::RallyLapsed { player: me });
}
