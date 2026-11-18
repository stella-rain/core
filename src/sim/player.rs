//! The main character: movement from the input's stick, the persistent facing,
//! the evade on its charges, and the combos of its Main weapon, made by hand.
//!
//! The perfect evade (when the pack names it): a press that starts an evade opens a window of
//! `window_ticks`; an enemy hit, zone or orb that the evade's own invulnerability alone stops
//! inside it is a near miss (`near_miss`), and with no cooldown running the evade is perfect: for
//! `slow_ticks` the enemies, the boss and their runs within `reach` of this player advance only
//! every `slow_every` ticks (`perfect_slow`, the schedule's step), ranged runs among them free
//! their token, and the slow has its own cooldown. Other players are never slowed (ADR-SIM-8).
//! Attack magnetism: a combo step's phase that names an `advance` slides the player along the
//! facing the step took, toward its current target, no further than touching it (`magnetism`).
//!
//! The weapon gauge (when the style names the `gauge` kind): a normal (A) step whose swing hits
//! adds `per_step` once and `per_hit` for each enemy, to the step's cap and the gauge's `max`
//! (`gauge_from_hits`); a hit taken adds a share of the hit points lost (`gauge_from_damage`); a
//! B step at a full gauge plays the style's full-gauge attack in place of the step's own, empties
//! the gauge and lets no hold begin after it (`spend_gauge`). A new step resets the step's fill.
//!
//! The spirit level (when the style names the `spirit` kind): the full-gauge smash raises
//! `spirit_level` by one at the press (`spirit_raise`, so its own hits carry it), every smash step
//! restarts `spirit_decay` (`spirit_restart`), and `spirit_tick` in `update` counts it down before
//! the press, dropping the level at 0, for a downed player too. A combo step starts with the
//! level's damage and range percents (`strike`), a smash hit that leaves its target standing adds
//! the level's bleed stacks (`spirit_bleed_stacks`, read in `collide::damage`).
//!
//! The counter bonus (when the style names the `counter_bonus` kind): every near miss of the
//! evade, in the perfect evade's cooldown too, sets `counter_window` to the kind's window
//! (`near_miss`); `counter_tick` counts it down one a tick; the first step of the dash branch a
//! player starts by hand inside it closes the window and sets `counter_bonus_step` to 1
//! (`counter_take`, at the press, before `strike`), until no step of the player is in progress.
//! `counter_taken` reads the mark: its damage joins the attack-up percent in `strike`, its
//! stagger factor scales every fill in `stun::fill`, its stun lands on each target hit
//! (`stun::counter_bonus_stun`, merged with a counter's) and its advance slides the first phase
//! in `magnetism`. The take also sets `counter_payout` to 1, the payout the counter owes: the
//! first landed combo hit of the step clears it (`counter_first_hit`, called from
//! `stun::counter_bonus_stun`), and the grant's refund and gauge fill pay then. The mark's clears clear it too. A down
//! empties all three fields.
//!
//! The combo tempo (when the style names the `combo_tempo_pct` kind): a swing of a combo step that
//! hits adds one stack to `combo_tempo`, to the kind's `max_stacks` (`tempo_from_hits`);
//! `strike` reads the stacks when an attack starts, adding their percent to the attack speed under
//! the run's cap (`tempo_speed_pct`) and, at full stacks, the kind's `range_pct_at_full` to the
//! range (`tempo_range_pct`), so a step, each loop of a hold and a release take the stacks held
//! at their start. `tempo_lose` empties them: a combo reset away from the middle of
//! a combo (`combo_reset`; the window running out always ends them, any other reset only when the
//! combo did not end on a smash pressed at full stacks under `carry_at_full`: `tempo_carries`;
//! the carried stacks are timed by the combo window at the root too: `combo_reset` restarts
//! `window_left` at the carry and keeps a running one across a reset at the root, `combo` counts
//! it down at depth 0 and `tempo_lose`s at 0), a smash press after the smash itself started
//! (`tempo_on_smash`;
//! full stacks with `keep_on_smash` stay), a hit taken (`hurt_player`; not under `tempo_armoured`,
//! full stacks in a held hold with `super_armour_at_full`), a down and another style.
//!
//! The giant trigger (when the style names the `giant_trigger` kind): `spend_gauge` starts it
//! after the full-gauge smash's own attack began (`trigger_start`), `trigger_tick` counts it down
//! in `update`, `strike` adds its range percent to every step and its smash percent to a smash
//! step when they start, `super_armour` reads `trigger_armoured`, `held_move_pct` raises a held
//! hold's movement to its `hold_move_pct` (`trigger_hold_pct`), and `stand_down` ends it. With
//! `end_ring`, the tick its count reaches 0 (not a down) plays the ring once (`trigger_ring`, in
//! `TRIGGER_RING_SLOT`): the smash's first active phase, free.
//!
//! The breaker (when the style names the `breaker` kind): it holds no state, so nothing starts,
//! counts down or ends. Its factors are read where the number is computed: `breaker_fill`
//! in `stun::fill` (every stagger fill, after the counter bonus's factor), `breaker_poise` in
//! `hit_boxes::flinch_by` (a combo hit's poise damage) and `breaker_part` in the part loop of
//! `hit_boxes::hit_enemy_side` (a smash step's hit on a boss part); a fourth, on the ticks of the
//! stagger the fill starts, is `breaker_stagger_ticks` in the same call, once the gauge is full.
//! Its burst is `stagger_burst`, called by `stun::fill` right after `stagger` in the same call.
//! The rally (when the rules name it): `rally_from_hit` from `hurt_player`, `rally_win_back` from
//! `collide::damage`, `rally_tick` in `update`, `rally_lapse` on a down, `rally_clamp` where the
//! most hit points fall.
//!
//! The combo: a press of `main` or `smash` is buffered (one, the latest; `main` when both come
//! on one tick). With no step running it takes the next step from where the combo is, or from
//! the root when there is none; a running step is cancelled for it once it reaches the
//! recovery after its last hits, if there is a next step for the press; a leaf runs to its end
//! first. When a step has run out with nothing buffered, the combo keeps its place for the
//! weapon's `window_ticks`. A step reached by `smash` is a combo part, which carries the Sub
//! weapon's pull. A step with a charge charges while the button that reached it is still held
//! as it starts: let go before level 1, it attacks as a tap would; later, at the level reached,
//! with the perfect bonus inside a window; at `max_ticks` it releases by itself. Presses while
//! charging are not buffered. A step with a hold holds where its recovery would
//! begin while its button is still held: its loop runs and starts again each time it ends, until
//! the button is let go or `max_ticks`, when its release plays; presses while holding are
//! buffered. An evade cancels the running step, breaks a charge or a hold and clears the buffer,
//! and a press during its dash or `dash_strike_ticks` after it takes the dash branch.
//! Every step's attack is wider by the range rewards and the expertise range options taken,
//! at most `MAX_RANGE_PCT` in all.
//!
//! Aim assist: while `assist` or `auto` is on, each tick picks a target; with `assist`
//! on, a step faces it as its attack starts. Auto mode: while `auto` is on, there is a
//! target and no press by hand is in the combo, it presses for the player, the next press of
//! the auto combo, taken as a buffered press would be; after the auto combo's last step, or
//! anywhere else, it starts again from the root once the step has run out. Its presses never
//! hold; a step with a charge is charged to the level the player released it at by hand in the
//! combo it came from, with no perfect bonus; they never take the dash branch and always face the target. A combo with a press by hand
//! becomes the auto combo when it resets, unless it began in the dash branch.

use super::World;
use crate::attack::AttackRef;
use crate::event::{DomainEvent, Press};
use crate::fixed::Fx;
use crate::id::EntityId;
use crate::input::{self, Input};
use crate::rewards::Rewards;
use crate::sim::state::PlayerIdx;
use crate::sim::state::{ComboState, Dir, Pull, Smash, State};
use crate::trig::SCALE;
use crate::weapon::{SmashEffect, Step, Weapon};

use super::attacks;
use super::entity::Who;
use super::levels;
use super::select;
use super::skill;

mod movement;
#[cfg(test)]
pub(super) use movement::{TETHER_PULL_DIV, TETHER_RANGE};
use movement::{magnetism, move_player};
pub(super) use movement::{move_by, tether};
mod evade;
pub(super) use evade::{add_evade_charges, near_miss, perfect_slow, slowed};
use evade::{dash, start_evade};
mod tempo;
pub(super) use tempo::{
    tempo_armoured, tempo_from_hits, tempo_lose, tempo_range_pct, tempo_speed_pct,
};
use tempo::{tempo_carries, tempo_on_smash};
mod counter;
pub(super) use counter::{
    breaker_fill, breaker_part, breaker_poise, breaker_stagger_ticks, counter_first_hit,
    counter_pay, counter_taken, stagger_burst,
};
use counter::{counter_take, counter_tick, counter_window_opened};
mod rally;
pub(super) use rally::{rally_clamp, rally_from_hit, rally_win_back};
use rally::{rally_lapse, rally_tick};
mod gauge;
use gauge::{
    gauge_filled, spend_gauge, spirit_restart, spirit_tick, trigger_armoured, trigger_end,
    trigger_hold_pct, trigger_tick,
};
pub(super) use gauge::{gauge_from_damage, gauge_from_hits, spirit_bleed_stacks};
mod hold;
pub(crate) use hold::{auto_charge_view, charge_view, hold_view};
use hold::{begin_hold, begins_hold, cancel, charging, held_move_pct, holding};
pub(super) use hold::{break_charge, stand_down, super_armour};

/// The stick's full deflection on each axis; −128 counts as −127.
const STICK_MAX: i8 = 127;

/// `⌈√n⌉`.
fn ceil_sqrt(n: u64) -> u64 {
    let r = n.isqrt();
    if r * r < n { r + 1 } else { r }
}

/// The move of a stick at `(x, y)` under a speed cap of `cap` units: the vector clamped to
/// length 127, then scaled so that 127 is the cap. Its length is rounded up, so a diagonal is
/// never faster than a straight line.
fn stick_move(x: i8, y: i8, cap: i64) -> (i64, i64) {
    let (x, y) = (i64::from(x), i64::from(y));
    let len = ceil_sqrt((x * x + y * y).unsigned_abs()).max(STICK_MAX as u64);
    // `len >= 127 > 0`, and truncation toward zero is the same on every CPU.
    let len = i64::try_from(len).expect("len is at most a few hundred");
    (x * cap / len, y * cap / len)
}

/// The direction of a stick at `(x, y)` as a vector of length about `SCALE`, or `None` when the
/// stick is centred.
fn stick_direction(x: i8, y: i8) -> Option<Dir> {
    if x == 0 && y == 0 {
        return None;
    }
    let (x, y) = (i64::from(x), i64::from(y));
    let scale = i64::from(SCALE);
    // Both are at most 127 in size, so every product fits in 64 bits.
    let len = ((x * x + y * y).unsigned_abs() * (scale * scale).unsigned_abs()).isqrt();
    let len = i64::try_from(len).expect("len is at most 127 * SCALE * 2");
    Some(Dir {
        x: Fx::from_wide(x * scale * scale / len).0,
        y: Fx::from_wide(y * scale * scale / len).0,
    })
}

/// One tick of the main character: timers run down, it turns to its stick, evades on a ready
/// press (cancelling its combo step), walks its combo, then dashes or moves.
pub(super) fn update(
    stage: &World,
    rewards: &Rewards,
    s: &mut State,
    me: PlayerIdx,
    input: Input,
    rooted: bool,
    events: &mut Vec<DomainEvent>,
) {
    // The held buttons follow the input of every present player, so a button held through a
    // down is never a fresh press at the revive.
    let held = input::held_buttons(input);
    let pressed = held & !s.players[me].held;
    s.players[me].held = held;
    // The timers run for a downed or out player too.
    let p = &mut s.players[me];
    p.invulnerable = p.invulnerable.saturating_sub(1);
    p.combat.ranged_grace = p.combat.ranged_grace.saturating_sub(1);
    for t in &mut p.evade_recovering {
        *t -= 1;
    }
    p.evade_recovering.retain(|&t| t > 0);
    p.combo.dash_strike_left = p.combo.dash_strike_left.saturating_sub(1);
    skill::tick(s, me);
    // The spirit's countdown runs for a downed player too, and is read before this tick's press.
    if stage.has_spirit {
        spirit_tick(stage, s, me, events);
    }
    // So is the giant trigger's.
    if stage.has_trigger {
        trigger_tick(stage, s, me, events);
    }
    // The rally's countdown too (ADR-SIM-9).
    if stage.tuning.rally.is_some() {
        rally_tick(s, me, events);
    }
    if !s.players[me].is_up() {
        return;
    }
    // A rooted reviver stands still and does not attack, evade or use a skill; `assist` is a
    // switch and stays. A charge or hold would release an attack when `main` reads as let go.
    let (input, pressed) = if rooted {
        break_charge(stage, s, me, events);
        let acts = input::MAIN | input::SMASH | input::EVADE | input::SKILL;
        let still = Input {
            move_x: 0,
            move_y: 0,
            main: false,
            smash: false,
            evade: false,
            skill: false,
            auto: false,
            ..input
        };
        (still, pressed & !acts)
    } else {
        (input, pressed)
    };
    let p = &mut s.players[me];
    let (x, y) = (input.move_x.max(-STICK_MAX), input.move_y.max(-STICK_MAX));
    if let Some(dir) = stick_direction(x, y) {
        p.facing = dir;
    }
    // The validator lets only defined presets through.
    let main = stage.weapon_of(s, me);
    s.players[me].target = match main {
        Some(m) if input.assist || input.auto => {
            select::assist_target(stage, s, me, m.assist_range.0)
        }
        _ => None,
    };
    if pressed & input::EVADE != 0 && s.players[me].evade_ready() > 0 {
        let cooldown = levels::evade_cooldown(s, me, stage.player.evade.cooldown_ticks);
        start_evade(stage, &mut s.players[me], cooldown, events);
        cancel(stage, s, me, events);
        // The perfect evade (sim_version >= 2, named by the pack): every press reopens the window.
        if let Some(t) = stage.tuning.perfect_evade {
            s.players[me].combat.perfect_window = t.window_ticks;
        }
        let strike = main.map_or(0, |m| m.dash_strike_ticks);
        s.players[me].combo.dash_strike_left = stage.player.evade.ticks.saturating_add(strike);
    }
    if pressed & input::SKILL != 0 {
        skill::press(stage, rewards, s, me, input, events);
    }
    if let Some(main) = main {
        combo(stage, s, me, main, input, pressed, events);
    }
    if stage.has_advance {
        magnetism(stage, s, me);
    }
    if stage.has_counter_bonus {
        counter_tick(s, me);
    }
    if s.players[me].dash.is_some() {
        dash(stage, &mut s.players[me]);
    } else {
        move_player(stage, s, me, x, y);
    }
}

/// The main character's attack slots: the combo; the stage skill and the effects
/// that attack on their own come with their own.
pub(super) const COMBO_SLOT: u8 = 0;

/// The attack slot of the giant trigger's end ring: past the runes' slots (34 to 40) and below
/// the homing missiles' (64 up), so `runes::of_run` takes it for neither.
pub(super) const TRIGGER_RING_SLOT: u8 = 41;
const _: () = assert!(
    TRIGGER_RING_SLOT as usize >= super::runes::FIRST_SLOT as usize + crate::rune::Rune::ALL.len()
        && TRIGGER_RING_SLOT < super::runes::MISSILE_SLOT
);

fn press_of(smash: bool) -> Press {
    if smash { Press::Smash } else { Press::Main }
}

/// The step the combo is at: `None` at the root.
pub(super) fn step_at<'a>(main: &'a Weapon, c: &ComboState) -> Option<&'a Step> {
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
pub(super) fn combo_proc(stage: &World, s: &State, me: PlayerIdx) -> Option<u16> {
    if !stage.has_proc {
        return None;
    }
    let weapon = stage.weapon_of(s, me)?;
    step_at(weapon, &s.players[me].combo)?.proc_pct
}

/// The charge level recorded for the combo's step at `index` in `levels` (4 bits a step).
fn level_of(levels: u32, index: u8) -> u8 {
    (levels >> (4 * u32::from(index)) & 0xf) as u8
}

/// `levels` with the step at `index` recorded as released at `level`.
fn with_level(levels: u32, index: u8, level: u8) -> u32 {
    let shift = 4 * u32::from(index);
    levels & !(0xf << shift) | u32::from(level & 0xf) << shift
}

/// Whether the press that reached the current step was `smash`.
pub(super) fn last_press_smash(c: &ComboState) -> bool {
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
enum Reset {
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
fn combo_reset(
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
fn combo(
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
fn press_of_pressed(pressed: u8) -> Option<bool> {
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
pub(super) fn initial_combo(stage: &World) -> ComboState {
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
fn strike(
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
pub(super) fn toward(s: &State, me: PlayerIdx, target: EntityId) -> Option<Dir> {
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
