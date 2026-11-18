//! Evading of the player's tick.

use super::{counter_window_opened, move_by};
use crate::event::DomainEvent;
use crate::fixed::Point;
use crate::id::EntityId;
use crate::sim::World;
use crate::sim::entity::Who;
use crate::sim::state::{DashState, PlayerIdx, PlayerState, State};
use crate::trig::SCALE;
use crate::validate::limits::MAX_EVADE_CHARGES;

/// A near miss: the evade's own invulnerability stopped a hit on player `me` (the caller has
/// seen the hit overlap and `invulnerable > 0`). Inside the window, and when the skip is the
/// evade's (`invulnerable` is what the evade has left, a tie goes to the evade; more is a hit's or
/// a revive's residue), the window closes and, with no cooldown running, the evade is perfect: the
/// slow and its own cooldown start (ADR-SIM-8: only this player's reach is slowed). No damage, no
/// record of the hit. The counter window opens beside it, cooldown or not.
pub(in crate::sim) fn near_miss(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    events: &mut Vec<DomainEvent>,
) {
    let Some(t) = stage.tuning.perfect_evade else {
        return;
    };
    let p = &mut s.players[me];
    if p.hp == 0 || p.combat.perfect_window == 0 {
        return;
    }
    let since = u32::from(t.window_ticks.saturating_sub(p.combat.perfect_window));
    let left = stage.player.evade.invulnerable_ticks.saturating_sub(since);
    if p.invulnerable != left {
        return;
    }
    p.combat.perfect_window = 0;
    // The counter window opens on every near miss of the evade, the cooldown or not.
    if stage.has_counter_bonus {
        if let Some(g) = stage.counter_of(s, me) {
            s.players[me].combat.counter_window = g.window;
            counter_window_opened(events, me);
        }
    }
    let p = &mut s.players[me];
    if p.combat.perfect_cooldown == 0 {
        p.combat.perfect_slow = t.slow_ticks;
        p.combat.perfect_cooldown = t.cooldown_ticks;
        perfect_evade_event(events, me);
    }
}

/// The step `perfect_slow`: works out the slowed set of this tick from the positions at its
/// start, then runs the three timers of every player down one (a downed or out player's too).
/// Nothing is slowed unless the pack names the perfect evade.
pub(in crate::sim) fn perfect_slow(stage: &World, s: &mut State) {
    let Some(t) = stage.tuning.perfect_evade else {
        return;
    };
    s.scratch.slowed.clear();
    if s.players.indices().any(|me| {
        let c = &s.players[me];
        c.is_up() && c.combat.perfect_slow > 0
    }) {
        slow_set(s, t);
    }
    for me in s.players.indices() {
        let c = &mut s.players[me].combat;
        c.perfect_window = c.perfect_window.saturating_sub(1);
        c.perfect_cooldown = c.perfect_cooldown.saturating_sub(1);
        c.perfect_slow = c.perfect_slow.saturating_sub(1);
    }
}

/// The enemies (and the boss with its parts) within `reach` of an up player with a slow running,
/// each once, by the lowest such slot; `holds` is whether it stops this tick. Allies, players,
/// zones and orbs are never in it.
fn slow_set(s: &mut State, t: crate::rules::PerfectEvade) {
    let reach = i64::from(t.reach);
    let every = u16::from(t.slow_every.max(2));
    let near = |s: &State, at: Point| -> Option<bool> {
        s.players.indices().find_map(|me| {
            let p = &s.players[me];
            let c = &p.combat;
            let dx = i64::from(p.at.x.0) - i64::from(at.x.0);
            let dy = i64::from(p.at.y.0) - i64::from(at.y.0);
            (p.is_up() && c.perfect_slow > 0 && dx * dx + dy * dy <= reach * reach)
                .then_some(c.perfect_slow % every != 0)
        })
    };
    let mut set = std::mem::take(&mut s.scratch.slowed);
    // Room is made on the first slow, not at load: a run that never slows allocates nothing.
    super::super::scratch::reserve_to(&mut set, s.enemies.len() + 1 + 16);
    for e in &s.enemies {
        if e.hp > 0 {
            if let Some(hold) = near(s, e.at) {
                set.push((e.id, hold));
            }
        }
    }
    if let Some(b) = &s.boss {
        let mut found = if b.hp > 0 { near(s, b.at) } else { None };
        for (i, part) in b.parts.iter().enumerate() {
            if part.hp > 0 && found.is_none() {
                found = near(s, s.at(Who::BossPart(i)));
            }
        }
        if let Some(hold) = found {
            set.push((b.id, hold));
            set.extend(b.parts.iter().map(|part| (part.id, hold)));
        }
    }
    set.sort_unstable_by_key(|&(id, _)| id);
    s.scratch.slowed = set;
}

/// Whether the entity `id` is slowed this tick and holds: `Some(holds)`, `None` if it is not in
/// the set. One test of "is the set empty" for every tick that slows no one.
pub(in crate::sim) fn slowed(s: &State, id: EntityId) -> Option<bool> {
    if s.scratch.slowed.is_empty() {
        return None;
    }
    let at = s
        .scratch
        .slowed
        .binary_search_by_key(&id, |&(i, _)| i)
        .ok()?;
    Some(s.scratch.slowed[at].1)
}

/// A press with a charge ready dashes along the facing, which is the stick's direction when it
/// is not centred, from where the main character is, a dash in progress or not
///. Invulnerability and the spent charge's `cooldown` count from the press.
pub(super) fn start_evade(
    stage: &World,
    p: &mut PlayerState,
    cooldown: u32,
    events: &mut Vec<DomainEvent>,
) {
    let evade = &stage.player.evade;
    p.dash = Some(DashState {
        dir: p.facing,
        done: 0,
        left: evade.ticks,
    });
    if cooldown > 0 {
        p.evade_recovering.push(cooldown);
    }
    p.invulnerable = p.invulnerable.max(evade.invulnerable_ticks);
    events.push(DomainEvent::Evaded);
}

/// A reward's `amount` more evade charges, never beyond the stage's `max_charges`.
/// The new ones are ready: the spent ones keep their timers.
pub(in crate::sim) fn add_evade_charges(stage: &World, p: &mut PlayerState, amount: u32) {
    // The validator holds `max_charges` to `charges..=3`; the clamp keeps an unvalidated stage
    // there too, and never takes a charge away.
    let most = stage
        .player
        .evade
        .most_charges()
        .min(MAX_EVADE_CHARGES)
        .max(p.evade_charges);
    let more = u8::try_from(amount).unwrap_or(u8::MAX);
    p.evade_charges = p.evade_charges.saturating_add(more).min(most);
}

/// One tick of the dash: the `k`th tick moves from `distance × (k − 1) / ticks` to
/// `distance × k / ticks` along its direction, so the whole dash covers exactly its distance.
/// Slowing and attacks do not slow it, and the stick does not steer it.
pub(super) fn dash(stage: &World, p: &mut PlayerState) {
    let Some(d) = p.dash else {
        return;
    };
    let distance = i64::from(stage.player.evade.distance.0);
    let ticks = i64::from(d.done) + i64::from(d.left);
    let at = |k: i64| distance * k / ticks;
    let step = at(i64::from(d.done) + 1) - at(i64::from(d.done));
    let scale = i64::from(SCALE);
    move_by(
        stage,
        p,
        i64::from(d.dir.x) * step / scale,
        i64::from(d.dir.y) * step / scale,
    );
    p.dash = (d.left > 1).then_some(DashState {
        dir: d.dir,
        done: d.done + 1,
        left: d.left - 1,
    });
}

/// `me`'s near miss was a perfect evade. Cold, as `gauge_filled`.
#[cold]
#[inline(never)]
fn perfect_evade_event(events: &mut Vec<DomainEvent>, me: PlayerIdx) {
    events.push(DomainEvent::PerfectEvade { player: me });
}
