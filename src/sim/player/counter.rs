//! The counter bonus and the breaker of the player's tick.

use super::{gauge_filled, last_press_smash};
use crate::event::DomainEvent;
use crate::limits::MAX_COUNTER_WINDOW_TICKS;
use crate::sim::World;
use crate::sim::entity::Who;
use crate::sim::state::{PlayerIdx, State};
use crate::sim::world::{BurstGrant, CounterGrant};

/// The payout of a taken counter, at its first landed combo hit (`counter_first_hit`'s grant):
/// the last spent evade charge comes back (nothing when none is recovering) and the weapon gauge
/// gains `gauge_fill_pct` of its bar, never past it (nothing without a gauge kind). The fill is the
/// counter's own: it ignores the step's `step_cap` and leaves `gauge_step_fill` as it was. Called once
/// per taken counter and only when the grant names a payout, and out of line so the hit's common
/// path stays as it was.
#[inline(never)]
pub(in crate::sim) fn counter_pay(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    g: &CounterGrant,
    events: &mut Vec<DomainEvent>,
) {
    if g.refund_evade {
        s.players[me].evade_recovering.pop();
    }
    if g.gauge_fill_pct == 0 {
        return;
    }
    let Some(gauge) = stage.gauge_of(s, me) else {
        return;
    };
    let c = &s.players[me].combat;
    let room = gauge.max.saturating_sub(c.weapon_gauge);
    let want = u64::from(gauge.max) * u64::from(g.gauge_fill_pct) / 100;
    let added = u32::try_from(want.min(u64::from(room))).unwrap_or(room);
    let max = gauge.max;
    if added > 0 {
        let c = &mut s.players[me].combat;
        c.weapon_gauge += added;
        gauge_filled(events, me, added, c.weapon_gauge, max);
    }
}

/// The counter window of `me` opened. Cold, as `gauge_filled`.
#[cold]
#[inline(never)]
pub(super) fn counter_window_opened(events: &mut Vec<DomainEvent>, me: PlayerIdx) {
    events.push(DomainEvent::CounterWindowOpened { player: me });
}

/// The step `me` started took the counter bonus. Cold, as `gauge_filled`.
#[cold]
#[inline(never)]
fn counter_bonus_taken(events: &mut Vec<DomainEvent>, me: PlayerIdx) {
    events.push(DomainEvent::CounterBonusTaken { player: me });
}

/// The counter bonus the step `me` is running took, read from the mark `counter_bonus_step`:
/// none after one bool test for a run that names no counter bonus. The one place a later kind
/// that reads the counter adds its line.
pub(in crate::sim) fn counter_taken<'a>(
    stage: &'a World,
    s: &State,
    me: PlayerIdx,
) -> Option<&'a CounterGrant> {
    if !stage.has_counter_bonus || s.players[me].combat.counter_bonus_step != 1 {
        return None;
    }
    stage.counter_of(s, me)
}

/// The first landed combo hit of the taken step collects its payout: when the step took the
/// counter bonus (the mark) and the payout is still owed (`counter_payout` 1), clears the flag
/// and returns the grant, once per taken counter; otherwise none. The caller pays it
/// (`counter_pay`).
pub(in crate::sim) fn counter_first_hit<'a>(
    stage: &'a World,
    s: &mut State,
    me: PlayerIdx,
) -> Option<&'a CounterGrant> {
    let c = &mut s.players[me].combat;
    if c.counter_bonus_step != 1 || c.counter_payout != 1 {
        return None;
    }
    c.counter_payout = 0;
    stage.counter_of(s, me)
}

/// `value` scaled by `pct` percent, one truncation, saturating to `u32`.
pub(in crate::sim) fn scaled(value: u32, pct: u32) -> u32 {
    u32::try_from(u64::from(value) * u64::from(pct) / 100).unwrap_or(u32::MAX)
}

/// The breaker's factor on a stagger fill credited to `me`: `value` unchanged without a breaker
/// naming `stagger_pct`, after one bool test for a run that names no breaker.
pub(in crate::sim) fn breaker_fill(stage: &World, s: &State, me: PlayerIdx, value: u32) -> u32 {
    match stage.breaker_of(s, me) {
        Some(g) if g.stagger_pct > 100 => scaled(value, g.stagger_pct),
        _ => value,
    }
}

/// The breaker's factor on the ticks of a stagger `me` causes: `ticks` unchanged without a
/// breaker naming `stagger_ticks_pct` (one bool test), else `ticks` scaled by it, one truncation,
/// held to `MAX_STAGGERED_TICKS`.
pub(in crate::sim) fn breaker_stagger_ticks(
    stage: &World,
    s: &State,
    me: PlayerIdx,
    ticks: u16,
) -> u16 {
    match stage.breaker_of(s, me) {
        Some(g) if g.stagger_ticks_pct > 100 => {
            let longer = scaled(u32::from(ticks), g.stagger_ticks_pct)
                .min(u32::from(crate::limits::MAX_STAGGERED_TICKS));
            u16::try_from(longer).unwrap_or(crate::limits::MAX_STAGGERED_TICKS)
        }
        _ => ticks,
    }
}

/// The breaker's factor on the poise damage of a combo hit of `me`.
pub(in crate::sim) fn breaker_poise(stage: &World, s: &State, me: PlayerIdx, damage: u32) -> u32 {
    match stage.breaker_of(s, me) {
        Some(g) if g.poise_pct > 100 => scaled(damage, g.poise_pct),
        _ => damage,
    }
}

/// The breaker's factor on the damage of a smash step's combo hit of `me` on a boss part: `damage`
/// unchanged for any other hit.
pub(in crate::sim) fn breaker_part(
    stage: &World,
    s: &State,
    me: PlayerIdx,
    combo: bool,
    damage: u32,
) -> u32 {
    if !stage.has_breaker || !combo || !last_press_smash(&s.players[me].combo) {
        return damage;
    }
    match stage.breaker_of(s, me) {
        Some(g) if g.part_break_pct > 100 => scaled(damage, g.part_break_pct),
        _ => damage,
    }
}

/// The breaker's burst: a fill of `me` has just staggered `target` (an elite or the boss) with a
/// hit of `hit` damage. It reports `StaggerBurst` and, for each other living enemy within the
/// radius of the target's centre that can be hurt (no shrine, no disguised mimic): `damage_pct` of
/// `hit` (at least 1) as a hit credited to `me` that is no combo hit (no rally, no expertise, no
/// fill, no flinch), then the stun and the knockdown on each regular enemy that is still alive.
/// The boss, its parts and the staggered target are never hurt, an elite takes the damage and
/// nothing else, so a burst never fills a gauge and never staggers a second target: no chain. It
/// holds no state; the staggered target's own stagger already began (ADR-SIM-9).
#[inline(never)]
pub(in crate::sim) fn stagger_burst(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    (target, hit): (Who, u32),
    burst: BurstGrant,
    events: &mut Vec<DomainEvent>,
) {
    let (at, id) = (s.at(target), s.id_of(target));
    events.push(DomainEvent::StaggerBurst {
        player: me,
        target: id,
        at,
        radius: burst.radius,
    });
    let staggered = match target {
        Who::Enemy(i) => Some(i),
        _ => None,
    };
    let owner = s.players.id(me);
    let reached = |s: &State, j: usize| {
        let e = &s.enemies[j];
        Some(j) != staggered
            && e.hp > 0
            && super::super::kills::damageable(s, Who::Enemy(j))
            && super::super::collide::overlaps(
                at,
                burst.radius,
                e.at,
                stage.defs.agent(e.def).radius.0,
            )
    };
    // Kills in the hit pass leave `s.enemies` as it is until the tick's retain, so the indexes
    // below stay valid across the damage (as in `combat::bursts`).
    if burst.damage_pct > 0 {
        let shot = super::super::Shot {
            at,
            owner,
            friendly: true,
            damage: scaled(hit, burst.damage_pct).max(1),
            damage_type: None,
            breaks_charge: false,
            smash: None,
            combo: false,
            rune: None,
            chain: true,
            dot: false,
        };
        for j in 0..s.enemies.len() {
            if reached(s, j) {
                super::super::damage(stage, s, me, shot, Who::Enemy(j), events);
            }
        }
    }
    if burst.stun_ticks == 0 && !burst.knock_down {
        return;
    }
    for j in 0..s.enemies.len() {
        if !reached(s, j) || s.enemies[j].elite.is_some() {
            continue;
        }
        if burst.stun_ticks > 0 {
            super::super::stun::apply(stage, s, Who::Enemy(j), owner, burst.stun_ticks, events);
        }
        if burst.knock_down {
            super::super::stun::knock_down(stage, s, me, j, events);
        }
    }
}

/// A step of `me` starts: the mark of the step before is gone, and the first step of the dash
/// branch pressed inside an open window takes the bonus and closes the window. A step of the
/// ordinary combo, a later step of the dash branch and an auto step take nothing and leave an
/// open window running.
pub(super) fn counter_take(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    first_dash: bool,
    events: &mut Vec<DomainEvent>,
) {
    let c = &mut s.players[me].combat;
    c.counter_bonus_step = 0;
    c.counter_payout = 0;
    if first_dash
        && (1..=MAX_COUNTER_WINDOW_TICKS).contains(&c.counter_window)
        && stage.counter_of(s, me).is_some()
    {
        let c = &mut s.players[me].combat;
        c.counter_window = 0;
        c.counter_bonus_step = 1;
        c.counter_payout = 1;
        counter_bonus_taken(events, me);
    }
}

/// The counter window counts down one a tick, and the mark ends with the step: when none of the
/// player's steps is in progress (a run, a charge or a hold), at the step's end, at an evade and
/// at a break.
pub(super) fn counter_tick(s: &mut State, me: PlayerIdx) {
    let p = &mut s.players[me];
    p.combat.counter_window = p.combat.counter_window.saturating_sub(1);
    let c = &p.combo;
    if !(c.running || c.charge.is_some() || c.hold.is_some()) {
        p.combat.counter_bonus_step = 0;
        p.combat.counter_payout = 0;
    }
}
