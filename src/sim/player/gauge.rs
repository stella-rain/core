//! The weapon gauge, spirit and the giant trigger of the player's tick.

use super::{TRIGGER_RING_SLOT, last_press_smash, strike};
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::sim::state::{PlayerIdx, State};
use crate::sim::{World, attacks, levels};

/// A friendly swing of player `owner`'s combo hit `n` enemies this tick: a normal (A) step fills
/// the weapon gauge its style grants by `per_step` once and `per_hit` for each, to the step's cap
/// and the gauge's `max`. A smash, a downed player and a style without a gauge fill nothing.
pub(in crate::sim) fn gauge_from_hits(
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
    let Some(g) = stage.gauge_of(s, me) else {
        return;
    };
    let (fill, have) = (p.combat.gauge_step_fill, p.combat.weapon_gauge);
    let room = g
        .step_cap
        .saturating_sub(fill)
        .min(g.max.saturating_sub(have));
    if room == 0 {
        return;
    }
    let first = if fill == 0 { g.per_step } else { 0 };
    let want = u64::from(first) + u64::from(n) * u64::from(g.per_hit);
    let added = u32::try_from(want.min(u64::from(room))).unwrap_or(room);
    let c = &mut s.players[me].combat;
    c.weapon_gauge += added;
    c.gauge_step_fill += added;
    if added > 0 {
        gauge_filled(events, me, added, c.weapon_gauge, g.max);
    }
}

/// The weapon gauge of `me` gained `added` and holds `value` of `max`. Cold: the benched packs
/// name no gauge, so the callers gain one call on a branch no bench takes.
#[cold]
#[inline(never)]
pub(super) fn gauge_filled(
    events: &mut Vec<DomainEvent>,
    me: PlayerIdx,
    added: u32,
    value: u32,
    max: u32,
) {
    events.push(DomainEvent::GaugeFilled {
        player: me,
        amount: added,
        value,
        full: value >= max,
    });
}

/// The full-gauge smash of `me` spent `amount`. Cold, as `gauge_filled`.
#[cold]
#[inline(never)]
fn gauge_spent(events: &mut Vec<DomainEvent>, me: PlayerIdx, amount: u32) {
    events.push(DomainEvent::GaugeSpent { player: me, amount });
}

/// Player `me` lost `lost` hit points to a hit: the gauge gains `max * lost * pct / (max_hp *
/// 100)`, to `max` at most, when the player is still up. The step cap does not hold it.
pub(in crate::sim) fn gauge_from_damage(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    lost: u32,
    events: &mut Vec<DomainEvent>,
) {
    let p = &s.players[me];
    if lost == 0 || !p.is_up() {
        return;
    }
    let Some(g) = stage.gauge_of(s, me) else {
        return;
    };
    if g.from_damage_pct == 0 {
        return;
    }
    let share = u64::from(g.max) * u64::from(lost) * u64::from(g.from_damage_pct)
        / (u64::from(p.max_hp.max(1)) * 100);
    let room = g.max.saturating_sub(p.combat.weapon_gauge);
    let added = u32::try_from(share.min(u64::from(room))).unwrap_or(room);
    s.players[me].combat.weapon_gauge += added;
    if added > 0 {
        gauge_filled(events, me, added, s.players[me].combat.weapon_gauge, g.max);
    }
}

/// Whether player `me`'s weapon gauge is full. The kinds that read the gauge go through it.
pub(in crate::sim) fn gauge_full(stage: &World, s: &State, me: PlayerIdx) -> bool {
    stage
        .gauge_of(s, me)
        .is_some_and(|g| s.players[me].combat.weapon_gauge >= g.max)
}

/// The spirit level of `me` rose to `level`. Cold, as `gauge_filled`.
#[cold]
#[inline(never)]
fn spirit_raised(events: &mut Vec<DomainEvent>, me: PlayerIdx, level: u8) {
    events.push(DomainEvent::SpiritRaised { player: me, level });
}

/// The spirit level of `me` fell to `level`. Cold, as `gauge_filled`.
#[cold]
#[inline(never)]
fn spirit_fell(events: &mut Vec<DomainEvent>, me: PlayerIdx, level: u8) {
    events.push(DomainEvent::SpiritFell { player: me, level });
}

/// The full-gauge smash of `me` raises its spirit level by one, to its style's top, and restarts
/// the countdown; the event says it only when the level changed (ADR-SIM-9).
fn spirit_raise(stage: &World, s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    let Some(g) = stage.spirit_of(s, me) else {
        return;
    };
    let (top, decay) = (
        u8::try_from(g.levels.len()).unwrap_or(u8::MAX),
        g.decay_ticks,
    );
    let c = &mut s.players[me].combat;
    let level = c.spirit_level.saturating_add(1).min(top);
    let changed = level != c.spirit_level;
    c.spirit_level = level;
    c.spirit_decay = decay;
    if changed {
        spirit_raised(events, me, level);
    }
}

/// A smash step restarts the countdown of a player holding a level.
pub(super) fn spirit_restart(stage: &World, s: &mut State, me: PlayerIdx) {
    if s.players[me].combat.spirit_level == 0 {
        return;
    }
    if let Some(g) = stage.spirit_of(s, me) {
        s.players[me].combat.spirit_decay = g.decay_ticks;
    }
}

/// `me`'s full-gauge smash started its giant trigger: it runs the grant's ticks from now (a second
/// one while it runs starts the count again) (ADR-SIM-9).
fn trigger_start(stage: &World, s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    let Some(g) = stage.trigger_of(s, me) else {
        return;
    };
    s.players[me].combat.giant_trigger = g.ticks;
    trigger_started(events, me, g.ticks);
}

/// One tick of the giant trigger: it ends on the tick it reaches 0, and a trigger that names
/// `end_ring` then plays its ring.
pub(super) fn trigger_tick(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    events: &mut Vec<DomainEvent>,
) {
    let c = &mut s.players[me].combat;
    if c.giant_trigger == 0 {
        return;
    }
    c.giant_trigger -= 1;
    if c.giant_trigger == 0 {
        trigger_ended(events, me);
        trigger_ring(stage, s, me, events);
    }
}

/// The end ring: the first ring of the style's full-gauge smash, once, in `TRIGGER_RING_SLOT`,
/// scaled by `me`'s range as the smash is. Free: no gauge, no wind-up, no combo slot, no
/// trigger. Only for a player who is up; a down ends the trigger through `trigger_end`, which
/// plays nothing. Nothing happens if the style has no ring or the slot is busy.
fn trigger_ring(stage: &World, s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    if !s.players[me].is_up() {
        return;
    }
    let Some(ring) = stage.ring_of(s, me) else {
        return;
    };
    let strike = attacks::Strike {
        bonus_pct: 0,
        speed_pct: 0,
        range_pct: levels::range_pct(s, me),
        smash: None,
    };
    let facing = s.players[me].facing;
    let damage = levels::attack(s, me, stage.player.atk);
    attacks::start_player(
        stage,
        s,
        me,
        TRIGGER_RING_SLOT,
        facing,
        ring,
        damage,
        strike,
        events,
    );
}

/// A down ends the giant trigger at once.
pub(super) fn trigger_end(s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    let c = &mut s.players[me].combat;
    if c.giant_trigger > 0 {
        c.giant_trigger = 0;
        trigger_ended(events, me);
    }
}

/// Whether `me`'s giant trigger runs and names super armour: the one more condition of
/// `super_armour`.
pub(super) fn trigger_armoured(stage: &World, s: &State, me: PlayerIdx) -> bool {
    stage.has_trigger
        && s.players[me].combat.giant_trigger > 0
        && stage.trigger_of(s, me).is_some_and(|g| g.super_armour)
}

/// The movement percent `me`'s running giant trigger gives a held hold: 0 while no trigger runs
/// or it names no `hold_move_pct`.
pub(super) fn trigger_hold_pct(stage: &World, s: &State, me: PlayerIdx) -> i64 {
    if stage.has_trigger && s.players[me].combat.giant_trigger > 0 {
        stage
            .trigger_of(s, me)
            .map_or(0, |g| i64::from(g.hold_move_pct))
    } else {
        0
    }
}

/// `me`'s giant trigger started, running `ticks`. Cold: a run with no trigger never calls it.
#[cold]
#[inline(never)]
fn trigger_started(events: &mut Vec<DomainEvent>, me: PlayerIdx, ticks: u16) {
    events.push(DomainEvent::TriggerStarted {
        player: me,
        ticks: u32::from(ticks),
    });
}

/// `me`'s giant trigger ended. Cold, as `trigger_started`.
#[cold]
#[inline(never)]
fn trigger_ended(events: &mut Vec<DomainEvent>, me: PlayerIdx) {
    events.push(DomainEvent::TriggerEnded { player: me });
}

/// One tick of the countdown: at 0 the level drops by one and, while above 0, counts again.
pub(super) fn spirit_tick(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    events: &mut Vec<DomainEvent>,
) {
    if s.players[me].combat.spirit_decay == 0 {
        return;
    }
    let again = stage.spirit_of(s, me).map_or(0, |g| g.decay_ticks);
    let c = &mut s.players[me].combat;
    c.spirit_decay -= 1;
    if c.spirit_decay > 0 {
        return;
    }
    c.spirit_level = c.spirit_level.saturating_sub(1);
    let level = c.spirit_level;
    if level > 0 {
        c.spirit_decay = again;
    }
    spirit_fell(events, me, level);
}

/// The spirit level of `me`, 0 when its style has none: the kinds that read the level go
/// through it.
#[allow(dead_code)] // read by the kinds that build on the level, not yet built
pub(in crate::sim) fn spirit_level_of(stage: &World, s: &State, me: PlayerIdx) -> u8 {
    if stage.spirit_of(s, me).is_some() {
        s.players[me].combat.spirit_level
    } else {
        0
    }
}

/// The extra bleed stacks a smash hit of `me` adds at its level.
pub(in crate::sim) fn spirit_bleed_stacks(stage: &World, s: &State, me: PlayerIdx) -> u32 {
    if !stage.has_spirit {
        return 0;
    }
    let level = s.players[me].combat.spirit_level;
    stage
        .spirit_of(s, me)
        .and_then(|g| g.fx(level))
        .map_or(0, |fx| fx.bleed_stacks)
}

/// A B step taken at a full gauge: spends it and plays the style's full-gauge attack in place of
/// the step's own, which starts no charge and no hold. True when it played. The one place a kind
/// that reacts to a spent gauge adds its line.
pub(super) fn spend_gauge(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    aim: bool,
    events: &mut Vec<DomainEvent>,
) -> bool {
    if !gauge_full(stage, s, me) {
        return false;
    }
    let Some(crate::expertise::Gauge {
        spend: crate::expertise::GaugeSpend::FullSmash { attack },
        ..
    }) = stage.gauge_of(s, me)
    else {
        return false;
    };
    let c = &mut s.players[me];
    let spent = c.combat.weapon_gauge;
    c.combat.weapon_gauge = 0;
    c.combat.gauge_step_fill = 0;
    gauge_spent(events, me, spent);
    // No hold begins after it (`begins_hold`).
    c.combo.released = true;
    if stage.has_spirit {
        // The raise is at the press, so the smash's own hits carry the new level.
        spirit_raise(stage, s, me, events);
    }
    strike(stage, s, me, attack, 0, aim, events);
    if stage.has_trigger {
        // After the slam's own attack began: the first slam carries none of the state's range or
        // damage (a restart's, made while it runs, does: `strike` reads the field).
        trigger_start(stage, s, me, events);
    }
    true
}
