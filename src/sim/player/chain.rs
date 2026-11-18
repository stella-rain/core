//! The combo of the player's tick.

use super::{
    begin_hold, begins_hold, charging, counter_take, counter_taken, holding, spend_gauge,
    spirit_restart, tempo_carries, tempo_lose, tempo_on_smash, tempo_range_pct, tempo_speed_pct,
};
use crate::attack::AttackRef;
use crate::event::{DomainEvent, Press};
use crate::id::EntityId;
use crate::input::{self, Input};
use crate::sim::entity::Who;
use crate::sim::state::{ComboState, Dir, PlayerIdx, Pull, Smash, State};
use crate::sim::{World, attacks, levels};
use crate::weapon::{SmashEffect, Step, Weapon};

/// The main character's attack slots: the combo; the stage skill and the effects
/// that attack on their own come with their own.
pub(in crate::sim) const COMBO_SLOT: u8 = 0;

/// The attack slot of the giant trigger's end ring: past the runes' slots (34 to 40) and below
/// the homing missiles' (64 up), so `runes::of_run` takes it for neither.
pub(in crate::sim) const TRIGGER_RING_SLOT: u8 = 41;
const _: () = assert!(
    TRIGGER_RING_SLOT as usize
        >= super::super::runes::FIRST_SLOT as usize + crate::rune::Rune::ALL.len()
        && TRIGGER_RING_SLOT < super::super::runes::MISSILE_SLOT
);

fn press_of(smash: bool) -> Press {
    if smash { Press::Smash } else { Press::Main }
}

/// The step the combo is at: `None` at the root.
pub(in crate::sim) fn step_at<'a>(main: &'a Weapon, c: &ComboState) -> Option<&'a Step> {
    let mut branches = if c.dash { &main.dash } else { &main.combo };
    let mut step = None;
    for i in 0..c.depth {
        let next = branches.get(c.path >> i & 1 == 1)?;
        branches = &next.next;
        step = Some(next);
    }
    step
}

/// The factor of the combo step `me` is playing for its hits' on-hit runes: none at once when
/// no style names one (`World::has_proc`), and none for a step that has none.
pub(in crate::sim) fn combo_proc(stage: &World, s: &State, me: PlayerIdx) -> Option<u16> {
    if !stage.has_proc {
        return None;
    }
    let weapon = stage.weapon_of(s, me)?;
    step_at(weapon, &s.players[me].combo)?.proc_pct
}

/// The charge level recorded for the combo's step at `index` in `levels` (4 bits a step).
pub(super) fn level_of(levels: u32, index: u8) -> u8 {
    (levels >> (4 * u32::from(index)) & 0xf) as u8
}

/// `levels` with the step at `index` recorded as released at `level`.
pub(super) fn with_level(levels: u32, index: u8, level: u8) -> u32 {
    let shift = 4 * u32::from(index);
    levels & !(0xf << shift) | u32::from(level & 0xf) << shift
}

/// Whether the press that reached the current step was `smash`.
pub(in crate::sim) fn last_press_smash(c: &ComboState) -> bool {
    c.depth > 0 && c.path >> (c.depth - 1) & 1 == 1
}

/// Back to the root, reported if the combo was anywhere else. A combo with a press by hand
/// becomes the auto combo, unless it began in the dash branch.
fn reset(c: &mut ComboState, events: &mut Vec<DomainEvent>) {
    if c.depth > 0 {
        events.push(DomainEvent::ComboReset);
        if c.hand && !c.dash {
            c.auto_path = c.path;
            c.auto_depth = c.depth;
            c.auto_levels = c.levels;
        }
    }
    c.levels = 0;
    c.charge_auto = false;
    c.hand = false;
    c.path = 0;
    c.depth = 0;
    c.dash = false;
    c.window_left = 0;
    c.released = false;
}

/// Which kind of end a `combo_reset` is: the combo window running out ends the tempo's carry,
/// every other reset may keep it (`tempo_carries`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Reset {
    /// The window after the last step ran out.
    Window,
    /// A press that does not continue the combo, auto's restart, an evade or a broken charge.
    Continue,
}

/// Back to the root through `reset`, for every caller that has the whole world: the one place the
/// combo's end is seen from outside the combo (ADR-SIM-9). A combo that was mid-combo ends the
/// tempo's stacks, unless it ended on a smash pressed at full stacks and the tempo names
/// `carry_at_full` and the reset is not the window running out: then they stand for the next combo,
/// for one combo window. The reset restarts `window_left` at the weapon's `window_ticks` (a reset
/// at the root keeps the running one, so a second evade does not restart it), and `combo` ends
/// them when it runs out at the root. The carry of a broken charge or hold runs one tick longer
/// than the evade path: the `running` re-set in `combo` gives a fresh window on the next tick.
pub(super) fn combo_reset(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    kind: Reset,
    events: &mut Vec<DomainEvent>,
) {
    // A reset at the root changes nothing: only a combo that was somewhere else ends the stacks.
    let mid_combo = s.players[me].combo.depth > 0;
    // Read before `reset`, which clears the path and depth the smash test needs.
    let carry = mid_combo && kind == Reset::Continue && tempo_carries(stage, s, me);
    // At the root a running `window_left` is the carry's timer: `reset` zeroes it, so it is kept.
    let carried = if mid_combo {
        0
    } else {
        s.players[me].combo.window_left
    };
    reset(&mut s.players[me].combo, events);
    s.players[me].combo.window_left = carried;
    if carry {
        // The carry's timer is the combo window, counted from this reset.
        s.players[me].combo.window_left =
            stage.weapon_of(s, me).map_or(0, |main| main.window_ticks);
    }
    if mid_combo && !carry {
        tempo_lose(stage, s, me, events);
    }
}

/// One tick of the combo: see the module's description.
pub(super) fn combo(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    main: &Weapon,
    input: Input,
    pressed: u8,
    events: &mut Vec<DomainEvent>,
) {
    if let Some(ticks) = s.players[me].combo.charge {
        charging(stage, s, me, main, input, pressed, ticks, events);
        return;
    }
    if let Some(ticks) = s.players[me].combo.hold {
        holding(stage, s, me, main, input, pressed, ticks, events);
        return;
    }
    if let Some(press) = press_of_pressed(pressed) {
        s.players[me].combo.buffered = Some(press);
    }
    let me_id = s.id_of(Who::Player(me));
    let busy = attacks::busy(s, me_id, COMBO_SLOT);
    // A held step holds where its recovery would begin, unless a buffered press takes its next
    // step first.
    if busy
        && attacks::hold_ready(stage, s, me_id, COMBO_SLOT)
        && begins_hold(main, &s.players[me].combo, input)
    {
        begin_hold(stage, s, me, main, input, events);
        return;
    }
    if s.players[me].combo.running && !busy {
        s.players[me].combo.running = false;
        s.players[me].combo.window_left = main.window_ticks;
    }
    // The window counts the ticks after the step on which a press continues the combo; once
    // they are used up, the combo is back at the root before this tick's press.
    if !busy && s.players[me].combo.depth > 0 && s.players[me].combo.window_left == 0 {
        combo_reset(stage, s, me, Reset::Window, events);
    }
    let c = s.players[me].combo;
    let (smash, by_hand) = match (
        c.buffered,
        auto_press(&c, input, s.players[me].target.is_some()),
    ) {
        (Some(smash), _) => (smash, true),
        (None, Some(Auto::Next(smash))) => (smash, false),
        (None, Some(Auto::Again)) if !busy => {
            combo_reset(stage, s, me, Reset::Continue, events);
            (c.auto_path & 1 == 1, false)
        }
        _ => {
            if !busy && c.depth > 0 {
                s.players[me].combo.window_left -= 1;
            } else if !busy && c.window_left > 0 {
                // At the root a running window is a carry's timer (`combo_reset`): it ends the
                // stacks when it runs out, and a window with no stacks left is dropped.
                let left = if s.players[me].combat.combo_tempo == 0 {
                    0
                } else {
                    c.window_left - 1
                };
                s.players[me].combo.window_left = left;
                if left == 0 {
                    tempo_lose(stage, s, me, events);
                }
            }
            return;
        }
    };
    if !busy {
        take_step(stage, s, me, main, smash, by_hand, input, events);
        return;
    }
    let next = step_at(main, &c).is_some_and(|st| st.next.get(smash).is_some());
    if next && attacks::in_end_recovery(stage, s, me_id, COMBO_SLOT) {
        attacks::end_run(stage, s, me_id, COMBO_SLOT);
        take_step(stage, s, me, main, smash, by_hand, input, events);
    }
}

/// The press `pressed` holds, `main` first: `Some(true)` for `smash`.
pub(super) fn press_of_pressed(pressed: u8) -> Option<bool> {
    if pressed & input::MAIN != 0 {
        Some(false)
    } else if pressed & input::SMASH != 0 {
        Some(true)
    } else {
        None
    }
}

/// What auto mode presses this tick.
enum Auto {
    /// The next press of the auto combo, `true` for `smash`, taken as a buffered press would be.
    Next(bool),
    /// The auto combo again from the root, once the current step has run out.
    Again,
}

/// Auto mode's press: none while `auto` is off, with no target, an empty auto combo or a press
/// by hand in the combo; the next press while the combo is on the auto combo's path short of
/// its end; otherwise the auto combo again from the root.
fn auto_press(c: &ComboState, input: Input, target: bool) -> Option<Auto> {
    if !input.auto || !target || c.hand || c.auto_depth == 0 {
        return None;
    }
    // Both depths are at most the tree's depth (`MAX_COMBO_DEPTH`), well short of 32.
    let mask = (1u32 << c.depth) - 1;
    if !c.dash && c.depth < c.auto_depth && c.path & mask == c.auto_path & mask {
        Some(Auto::Next(c.auto_path >> c.depth & 1 == 1))
    } else {
        Some(Auto::Again)
    }
}

/// The combo a run starts with: at the root, with the weapon's A chain as the auto combo,
/// `main` pressed from the root until a step has no next step for it.
pub(in crate::sim) fn initial_combo(stage: &World) -> ComboState {
    let mut depth = 0u8;
    if let Some(main) = stage.weapon() {
        let mut step = main.combo.get(false);
        while let Some(st) = step {
            depth = depth.saturating_add(1);
            step = st.next.get(false);
        }
    }
    ComboState {
        auto_depth: depth,
        ..ComboState::default()
    }
}

/// The press `smash`, buffered `by_hand` or auto mode's, takes its step: the next one from where
/// the combo is, or else the first from the root, in the dash branch while a dash-strike may
/// start and it has one (by hand only). Auto mode's step charges only to its recorded level and faces the target.
#[allow(clippy::too_many_arguments)]
fn take_step(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    main: &Weapon,
    smash: bool,
    by_hand: bool,
    input: Input,
    events: &mut Vec<DomainEvent>,
) {
    s.players[me].combo.buffered = None;
    s.players[me].combo.released = false;
    let onward = step_at(main, &s.players[me].combo).is_some_and(|st| st.next.get(smash).is_some());
    if !onward {
        combo_reset(stage, s, me, Reset::Continue, events);
        let c = &mut s.players[me].combo;
        c.dash = by_hand && c.dash_strike_left > 0 && main.dash.get(smash).is_some();
        if !c.dash && main.combo.get(smash).is_none() {
            return;
        }
    }
    let c = &mut s.players[me].combo;
    c.hand |= by_hand;
    c.path |= u32::from(smash) << c.depth;
    // A step auto mode plays carries the level the auto combo records for it; a
    // press by hand starts at 0 and records its release.
    let level = if by_hand {
        0
    } else {
        level_of(c.auto_levels, c.depth)
    };
    c.levels = with_level(c.levels, c.depth, level);
    c.depth += 1;
    c.window_left = 0;
    events.push(DomainEvent::ComboStep {
        press: press_of(smash),
        depth: c.depth,
    });
    let first_dash = !onward && c.dash;
    let Some(step) = step_at(main, c) else {
        return;
    };
    if stage.has_counter_bonus {
        counter_take(stage, s, me, first_dash, events);
    }
    if stage.has_gauge {
        // A new step starts its fill again (the step cap, ADR-SIM-5).
        s.players[me].combat.gauge_step_fill = 0;
    }
    if smash && stage.has_spirit {
        // Every smash step restarts a raised level's countdown (the raise below sets it too).
        spirit_restart(stage, s, me);
    }
    if smash && stage.has_gauge && spend_gauge(stage, s, me, input.assist || !by_hand, events) {
        if smash {
            tempo_on_smash(stage, s, me, events);
        }
        return;
    }
    let held = by_hand && if smash { input.smash } else { input.main };
    // Auto mode charges a step to the level recorded for it, with no button.
    let auto_level = step.charge.as_ref().filter(|_| !by_hand).map_or(0, |_| {
        level_of(s.players[me].combo.levels, s.players[me].combo.depth - 1)
    });
    if step.charge.is_some() && (held || auto_level > 0) {
        s.players[me].combo.charge = Some(0);
        s.players[me].combo.charge_auto = !held;
        if smash {
            tempo_on_smash(stage, s, me, events);
        }
    } else {
        strike(
            stage,
            s,
            me,
            &step.attack,
            0,
            input.assist || !by_hand,
            events,
        );
        if smash {
            tempo_on_smash(stage, s, me, events);
        }
    }
}

/// The current step's attack starts in the combo slot along the facing, at the
/// stage's attack stat, faster by the attack speed, wider by the range rewards and the
/// expertise range options taken (at most `MAX_RANGE_PCT` in all), with
/// `bonus_pct` and, for a combo part, the Sub weapon's pull. When it `aim`s and there is a target, the facing first turns to it
///.
pub(super) fn strike(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    attack: &AttackRef,
    bonus_pct: u32,
    aim: bool,
    events: &mut Vec<DomainEvent>,
) {
    if aim {
        if let Some(dir) = s.players[me].target.and_then(|t| toward(s, me, t)) {
            s.players[me].facing = dir;
        }
    }
    let smash = if last_press_smash(&s.players[me].combo) {
        stage
            .weapon_of(s, me)
            .and_then(|w| w.smash)
            .map(|effect| match effect {
                SmashEffect::Pull { distance, ticks } => Smash::Pull(Pull {
                    distance: distance.0,
                    ticks,
                }),
                SmashEffect::Knockback { distance, ticks } => Smash::Knockback(Pull {
                    distance: distance.0,
                    ticks,
                }),
                SmashEffect::Bleed {} => Smash::Bleed,
                SmashEffect::Stun { ticks } => Smash::Stun { ticks },
                SmashEffect::Reap { stacks, boss_pct } => Smash::Reap { stacks, boss_pct },
            })
    } else {
        None
    };
    let (mut bonus_pct, mut range_pct) = (bonus_pct, levels::range_pct(s, me));
    if stage.has_spirit {
        let level = s.players[me].combat.spirit_level;
        if let Some(fx) = stage.spirit_of(s, me).and_then(|g| g.fx(level)) {
            bonus_pct = bonus_pct.saturating_add(fx.damage_pct);
            range_pct = range_pct.saturating_add(fx.range_pct);
        }
    }
    if let Some(fx) = counter_taken(stage, s, me) {
        bonus_pct = bonus_pct.saturating_add(fx.damage_pct);
    }
    if stage.has_trigger && s.players[me].combat.giant_trigger > 0 {
        // Fixed when the attack starts, as the spirit's and the counter's are.
        if let Some(g) = stage.trigger_of(s, me) {
            range_pct = range_pct.saturating_add(g.range_pct);
            if last_press_smash(&s.players[me].combo) {
                bonus_pct = bonus_pct.saturating_add(g.smash_pct);
            }
        }
    }
    // Read when the attack starts, as the speed is: full stacks widen this attack.
    range_pct = range_pct.saturating_add(tempo_range_pct(stage, s, me));
    let strike = attacks::Strike {
        bonus_pct,
        speed_pct: if stage.has_tempo {
            // The stacks held when the attack starts, under the run's cap (ADR-SIM-9).
            crate::run_stats::capped_attack_speed_pct(
                levels::attack_speed(s, me).saturating_add(tempo_speed_pct(stage, s, me)),
                s.max_attack_speed_pct,
            )
        } else {
            levels::attack_speed(s, me)
        },
        range_pct,
        smash,
    };
    let facing = s.players[me].facing;
    let damage = levels::attack(s, me, stage.player.atk);
    s.players[me].combo.running = attacks::start_player(
        stage, s, me, COMBO_SLOT, facing, attack, damage, strike, events,
    );
}

/// The direction from the main character to the entity `target`, if it is there and not on
/// the main character's centre.
pub(in crate::sim) fn toward(s: &State, me: PlayerIdx, target: EntityId) -> Option<Dir> {
    let to = s.at(s.find(target)?);
    let from = s.players[me].at;
    let (dx, dy) = (
        i64::from(to.x.0) - i64::from(from.x.0),
        i64::from(to.y.0) - i64::from(from.y.0),
    );
    (dx != 0 || dy != 0).then(|| attacks::normalize(dx, dy))
}

/// The presses of `path` from the root, `depth` of them.
fn presses_of(path: u32, depth: u8) -> Vec<Press> {
    (0..depth).map(|i| press_of(path >> i & 1 == 1)).collect()
}

/// The combo's view for the snapshot: the presses from the root, whether it began in the dash
/// branch, the buffered press, the window's ticks left and the auto combo.
pub(crate) fn combo_view(
    s: &State,
    me: PlayerIdx,
) -> (Vec<Press>, bool, Option<Press>, u32, Vec<Press>) {
    let c = &s.players[me].combo;
    (
        presses_of(c.path, c.depth),
        c.dash,
        c.buffered.map(press_of),
        c.window_left,
        presses_of(c.auto_path, c.auto_depth),
    )
}
