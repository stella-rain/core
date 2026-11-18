//! Tempo of the player's tick.

use super::last_press_smash;
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::sim::World;
use crate::sim::state::{PlayerIdx, State};

/// Whether the combo of `me` ends on a smash pressed at full stacks and its tempo carries them:
/// the smash is the last press, the stacks are full (a smash below full has ended them at the
/// press, and a smash's own hits add none) and the grant names `carry_at_full`.
pub(super) fn tempo_carries(stage: &World, s: &State, me: PlayerIdx) -> bool {
    stage.has_tempo
        && stage.tempo_of(s, me).is_some_and(|g| {
            g.carry_at_full
                && last_press_smash(&s.players[me].combo)
                && s.players[me].combat.combo_tempo == g.max_stacks
        })
}

/// The combo tempo of `me` ends: the stacks go to 0 and `TempoLost` says so, once. None after one
/// bool test for a run that names no combo tempo (ADR-SIM-9).
pub(in crate::sim) fn tempo_lose(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    events: &mut Vec<DomainEvent>,
) {
    if !stage.has_tempo {
        return;
    }
    let c = &mut s.players[me].combat;
    if c.combo_tempo > 0 {
        c.combo_tempo = 0;
        tempo_lost(events, me);
    }
}

/// Whether `me`'s stacks are full in a held hold and its tempo names the armour: the combo tempo's
/// condition of `super_armour`, and the one armour that shelters the stacks from a hit.
pub(in crate::sim) fn tempo_armoured(stage: &World, s: &State, me: PlayerIdx) -> bool {
    stage.has_tempo
        && s.players[me].combo.hold.is_some()
        && stage
            .tempo_of(s, me)
            .is_some_and(|g| g.hold_armour && s.players[me].combat.combo_tempo == g.max_stacks)
}

/// The attack speed the stacks of `me` give, in percent, before the run's cap: 0 after one bool
/// test for a run that names no combo tempo. The one place a later kind that reads the stacks
/// adds its line.
#[inline]
pub(in crate::sim) fn tempo_speed_pct(stage: &World, s: &State, me: PlayerIdx) -> u32 {
    if !stage.has_tempo {
        return 0;
    }
    stage.tempo_of(s, me).map_or(0, |g| {
        u32::from(s.players[me].combat.combo_tempo) * g.speed_pct
    })
}

/// The percent the stacks add to the range of an attack the combo starts: the kind's
/// `range_pct_at_full` while the stacks are full, else 0.
pub(in crate::sim) fn tempo_range_pct(stage: &World, s: &State, me: PlayerIdx) -> u32 {
    if !stage.has_tempo {
        return 0;
    }
    stage.tempo_of(s, me).map_or(0, |g| {
        if s.players[me].combat.combo_tempo == g.max_stacks {
            g.range_pct_at_full
        } else {
            0
        }
    })
}

/// A smash step is pressed: the stacks end, unless the style keeps them and they are full.
/// Called after the smash's own attack started, so that attack took the speed of the stacks
/// held at the press.
pub(super) fn tempo_on_smash(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    events: &mut Vec<DomainEvent>,
) {
    if !stage.has_tempo {
        return;
    }
    let Some(g) = stage.tempo_of(s, me) else {
        return;
    };
    if g.keep_on_smash && s.players[me].combat.combo_tempo == g.max_stacks {
        return;
    }
    tempo_lose(stage, s, me, events);
}

/// A friendly swing of player `owner`'s combo hit `n` enemies this tick: a step that hit adds one
/// stack to the combo tempo its style grants, up to `max_stacks`, however many it hit. A smash,
/// a downed player and a style without a tempo add none.
pub(in crate::sim) fn tempo_from_hits(
    stage: &World,
    s: &mut State,
    owner: EntityId,
    n: u32,
    events: &mut Vec<DomainEvent>,
) {
    let Some(me) = s.players.of_id(owner) else {
        return;
    };
    let p = &s.players[me];
    if n == 0 || !p.is_up() || last_press_smash(&p.combo) {
        return;
    }
    let Some(g) = stage.tempo_of(s, me) else {
        return;
    };
    let max = g.max_stacks;
    let c = &mut s.players[me].combat;
    if c.combo_tempo < max {
        c.combo_tempo += 1;
        tempo_raised(events, me, c.combo_tempo);
    }
}

/// `me`'s stacks rose to `stacks`. Cold: the benched packs name no combo tempo.
#[cold]
#[inline(never)]
fn tempo_raised(events: &mut Vec<DomainEvent>, me: PlayerIdx, stacks: u8) {
    events.push(DomainEvent::TempoRaised {
        player: me,
        stacks: u32::from(stacks),
    });
}

/// `me`'s stacks ended. Cold, as `tempo_raised` is.
#[cold]
#[inline(never)]
fn tempo_lost(events: &mut Vec<DomainEvent>, me: PlayerIdx) {
    events.push(DomainEvent::TempoLost { player: me });
}
