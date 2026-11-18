//! Holds and charges of the player's tick.

use super::{
    COMBO_SLOT, Reset, combo_reset, last_press_smash, level_of, press_of_pressed, rally_lapse,
    step_at, strike, tempo_armoured, tempo_lose, trigger_armoured, trigger_end, trigger_hold_pct,
    with_level,
};
use crate::event::DomainEvent;
use crate::input::Input;
use crate::sim::entity::Who;
use crate::sim::state::{ComboState, PlayerIdx, State};
use crate::sim::{World, attacks};
use crate::weapon::{Charge, Hold, Step, Weapon};

/// Whether the step whose recovery begins now holds: it has a hold, its press
/// was by hand (auto mode never holds), the button that reached it is still held, and
/// no buffered press takes a next step, which cancels the recovery instead (point 3).
pub(super) fn begins_hold(main: &Weapon, c: &ComboState, input: Input) -> bool {
    let Some(step) = step_at(main, c) else {
        return false;
    };
    if step.hold.is_none() || !c.hand || c.released {
        return false;
    }
    let still = if last_press_smash(c) {
        input.smash
    } else {
        input.main
    };
    let cancels = c
        .buffered
        .is_some_and(|smash| step.next.get(smash).is_some());
    still && !cancels
}

/// The hold begins on the tick the recovery would: the step's own recovery does not play, and
/// the loop starts on this tick.
pub(super) fn begin_hold(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    main: &Weapon,
    input: Input,
    events: &mut Vec<DomainEvent>,
) {
    let me_id = s.id_of(Who::Player(me));
    attacks::end_run(stage, s, me_id, COMBO_SLOT);
    s.players[me].combo.hold = Some(0);
    spin(stage, s, me, main, input, events);
}

/// The loop starts again, as a new attack: its facing is the main character's as it starts,
/// or the target's with aim assist on.
fn spin(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    main: &Weapon,
    input: Input,
    events: &mut Vec<DomainEvent>,
) {
    let Some(hold) = step_at(main, &s.players[me].combo).and_then(|st| st.hold.as_ref()) else {
        return;
    };
    strike(stage, s, me, &hold.loop_attack, 0, input.assist, events);
}

/// One tick of a hold: a press is buffered, as during any step. Letting go,
/// or reaching `max_ticks`, releases with the ticks held; otherwise the hold goes on, and the
/// loop starts again once the last one has ended.
#[allow(clippy::too_many_arguments)]
pub(super) fn holding(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    main: &Weapon,
    input: Input,
    pressed: u8,
    held: u32,
    events: &mut Vec<DomainEvent>,
) {
    if let Some(press) = press_of_pressed(pressed) {
        s.players[me].combo.buffered = Some(press);
    }
    let c = s.players[me].combo;
    let Some(hold) = step_at(main, &c).and_then(|st| st.hold.as_ref()) else {
        s.players[me].combo.hold = None;
        return;
    };
    let still = if last_press_smash(&c) {
        input.smash
    } else {
        input.main
    };
    if !still {
        release_hold(stage, s, me, hold, held, input, events);
        return;
    }
    let held = held.saturating_add(1);
    if held >= hold.max_ticks {
        release_hold(stage, s, me, hold, held, input, events);
        return;
    }
    s.players[me].combo.hold = Some(held);
    if !attacks::busy(s, s.id_of(Who::Player(me)), COMBO_SLOT) {
        spin(stage, s, me, main, input, events);
    }
}

/// The release: the loop ends with the hold, and the release attack starts with its facing
/// locked. Its recovery is the end lag, as any step's (point 3).
fn release_hold(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    hold: &Hold,
    ticks: u32,
    input: Input,
    events: &mut Vec<DomainEvent>,
) {
    let me_id = s.id_of(Who::Player(me));
    attacks::end_run(stage, s, me_id, COMBO_SLOT);
    s.players[me].combo.hold = None;
    s.players[me].combo.released = true;
    strike(stage, s, me, &hold.release, 0, input.assist, events);
    events.push(DomainEvent::HoldReleased { ticks });
}

/// The level `held` ticks of charge reach: 0 before the first of its levels.
fn level_at(charge: &Charge, held: u32) -> usize {
    charge
        .levels
        .iter()
        .take_while(|l| l.at_ticks <= held)
        .count()
}

/// Whether `held` ticks of charge are inside the perfect window of the level they reach.
fn in_perfect_window(charge: &Charge, held: u32) -> bool {
    match level_at(charge, held) {
        0 => false,
        n => {
            let l = &charge.levels[n - 1];
            held < l.at_ticks.saturating_add(l.perfect_ticks)
        }
    }
}

/// The current step's charge, if it is charging.
fn current_charge<'a>(stage: &'a World, s: &State, me: PlayerIdx) -> Option<&'a Charge> {
    s.players[me].combo.charge?;
    let main = stage.weapon_of(s, me)?;
    step_at(main, &s.players[me].combo)?.charge.as_ref()
}

/// The charge's view for the snapshot: its level (0 before level 1), ticks held and whether
/// this tick is inside a perfect window.
pub(crate) fn charge_view(stage: &World, s: &State, me: PlayerIdx) -> Option<(u8, u32, bool)> {
    let held = s.players[me].combo.charge?;
    if s.players[me].combo.charge_auto {
        return None;
    }
    let charge = current_charge(stage, s, me)?;
    Some((
        u8::try_from(level_at(charge, held)).unwrap_or(u8::MAX),
        held,
        in_perfect_window(charge, held),
    ))
}

/// The level auto mode is charging the current step to, if it is charging: the
/// snapshot's `auto_charge`, which leaves `charge` for hand charges only.
pub(crate) fn auto_charge_view(s: &State, me: PlayerIdx) -> Option<u8> {
    let c = &s.players[me].combo;
    (c.charge.is_some() && c.charge_auto).then(|| level_of(c.levels, c.depth.saturating_sub(1)))
}

/// A charge held rises a tick; let go, judged on the last held tick (the one whose snapshot
/// the player saw, ADR-BRDG-4), or at `max_ticks` without the bonus, it releases.
#[allow(clippy::too_many_arguments)]
pub(super) fn charging(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    main: &Weapon,
    input: Input,
    pressed: u8,
    held: u32,
    events: &mut Vec<DomainEvent>,
) {
    let c = s.players[me].combo;
    let Some(step) = step_at(main, &c) else {
        s.players[me].combo.charge = None;
        s.players[me].combo.charge_auto = false;
        return;
    };
    let Some(charge) = &step.charge else {
        s.players[me].combo.charge = None;
        s.players[me].combo.charge_auto = false;
        return;
    };
    if c.charge_auto {
        // Auto mode's charge: a press by hand is buffered, as during any auto step;
        // the button does not matter; it lets go on the tick the recorded level is reached,
        // never judged for the perfect bonus.
        if let Some(press) = press_of_pressed(pressed) {
            s.players[me].combo.buffered = Some(press);
        }
        let held = held.saturating_add(1);
        let target = usize::from(level_of(c.levels, c.depth - 1)).min(charge.levels.len());
        if level_at(charge, held) >= target || held >= charge.max_ticks {
            release(stage, s, me, step, charge, held, false, true, events);
        } else {
            s.players[me].combo.charge = Some(held);
        }
        return;
    }
    let still = if last_press_smash(&c) {
        input.smash
    } else {
        input.main
    };
    if !still {
        release(stage, s, me, step, charge, held, true, input.assist, events);
        return;
    }
    let held = held.saturating_add(1);
    if held >= charge.max_ticks {
        release(
            stage,
            s,
            me,
            step,
            charge,
            held,
            false,
            input.assist,
            events,
        );
    } else {
        s.players[me].combo.charge = Some(held);
    }
}

/// The release: level 0 is the step's own attack; a later level attacks with its attack, or
/// with its perfect attack and bonus when `judged` inside its window. It faces the target of
/// this tick when it `aim`s.
#[allow(clippy::too_many_arguments)]
fn release(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    step: &Step,
    charge: &Charge,
    held: u32,
    judged: bool,
    aim: bool,
    events: &mut Vec<DomainEvent>,
) {
    s.players[me].combo.charge = None;
    s.players[me].combo.charge_auto = false;
    let level = level_at(charge, held);
    let c = &mut s.players[me].combo;
    c.levels = with_level(
        c.levels,
        c.depth.saturating_sub(1),
        u8::try_from(level).expect("a charge level is a few steps"),
    );
    if level == 0 {
        strike(stage, s, me, &step.attack, 0, aim, events);
        return;
    }
    let l = &charge.levels[level - 1];
    let perfect = judged && in_perfect_window(charge, held);
    if perfect {
        let attack = l.perfect_attack.as_ref().unwrap_or(&l.attack);
        strike(stage, s, me, attack, u32::from(l.perfect_pct), aim, events);
    } else {
        strike(stage, s, me, &l.attack, 0, aim, events);
    }
    events.push(DomainEvent::ChargeReleased {
        level: u8::try_from(level).unwrap_or(u8::MAX),
        perfect,
    });
}

/// A player going down stops what it was doing: the dash ends, the combo run ends, a charge or
/// hold breaks and the buffer clears (ADR-SIM-7). Party only.
pub(in crate::sim) fn stand_down(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    events: &mut Vec<DomainEvent>,
) {
    s.players[me].dash = None;
    cancel(stage, s, me, events);
    // A down holds no window at the root either: `combo` does not run while down.
    s.players[me].combo.window_left = 0;
    // A downed player cannot press, and a revive does not resume an old window.
    s.players[me].combat.counter_window = 0;
    s.players[me].combat.counter_bonus_step = 0;
    s.players[me].combat.counter_payout = 0;
    // A downed player holds no stacks, and a revive does not resume them.
    tempo_lose(stage, s, me, events);
    // Nor a giant trigger, nor a rally.
    trigger_end(s, me, events);
    rally_lapse(s, me, events);
}

/// An evade cancels the running step, breaks a charge and clears the buffer.
pub(super) fn cancel(stage: &World, s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    let me_id = s.id_of(Who::Player(me));
    attacks::end_run(stage, s, me_id, COMBO_SLOT);
    let c = &mut s.players[me].combo;
    if c.charge.take().is_some() {
        events.push(DomainEvent::ChargeBroken);
    }
    if c.hold.take().is_some() {
        events.push(DomainEvent::HoldBroken);
    }
    c.buffered = None;
    c.running = false;
    combo_reset(stage, s, me, Reset::Continue, events);
}

/// A hit from a phase that breaks charges ends a charge or a hold in progress without an
/// attack (a hold's loop ends with it), and the combo starts again from the root.
pub(in crate::sim) fn break_charge(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    events: &mut Vec<DomainEvent>,
) {
    let me_id = s.id_of(Who::Player(me));
    let charging = s.players[me].combo.charge.take().is_some();
    let holding = s.players[me].combo.hold.take().is_some();
    if !charging && !holding {
        return;
    }
    if holding {
        attacks::end_run(stage, s, me_id, COMBO_SLOT);
        events.push(DomainEvent::HoldBroken);
    }
    if charging {
        events.push(DomainEvent::ChargeBroken);
    }
    s.players[me].combo.buffered = None;
    combo_reset(stage, s, me, Reset::Continue, events);
}

/// The movement speed, in percent, a charge or a hold leaves the main character: its
/// `move_pct` (a hold's raised to the running trigger's `hold_move_pct`), 100 when neither is on.
pub(super) fn held_move_pct(stage: &World, s: &State, me: PlayerIdx) -> i64 {
    if let Some(hold) = current_hold(stage, s, me) {
        return i64::from(hold.move_pct).max(trigger_hold_pct(stage, s, me));
    }
    current_charge(stage, s, me).map_or(100, |c| i64::from(c.move_pct))
}

/// The current step's hold, if it is held.
fn current_hold<'a>(stage: &'a World, s: &State, me: PlayerIdx) -> Option<&'a Hold> {
    s.players[me].combo.hold?;
    let main = stage.weapon_of(s, me)?;
    step_at(main, &s.players[me].combo)?.hold.as_ref()
}

/// The hold's view for the snapshot: the ticks held and the most it can last.
pub(crate) fn hold_view(stage: &World, s: &State, me: PlayerIdx) -> Option<(u32, u32)> {
    let held = s.players[me].combo.hold?;
    Some((held, current_hold(stage, s, me)?.max_ticks))
}

/// Whether `me` has super armour: a `breaks_charge` hit does not break its charge or hold. Derived,
/// never state: its charge is at level 2 or more (by hand, or the level auto mode recorded), or its
/// running combo step is a smash step in an active phase (a hold's loop). The one place a later
/// condition is added: the giant trigger's (`trigger_armoured`) and the combo tempo's
/// (`tempo_armoured`) are two.
pub(in crate::sim) fn super_armour(stage: &World, s: &State, me: PlayerIdx) -> bool {
    if trigger_armoured(stage, s, me) || tempo_armoured(stage, s, me) {
        return true;
    }
    let c = &s.players[me].combo;
    if let (Some(held), Some(charge)) = (c.charge, current_charge(stage, s, me)) {
        if level_at(charge, held) >= 2 {
            return true;
        }
    }
    last_press_smash(c)
        && attacks::run_phase_kind(stage, s, s.players.id(me), COMBO_SLOT)
            == Some(crate::attack::PhaseKind::Active)
}
