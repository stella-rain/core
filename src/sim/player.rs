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
use crate::event::DomainEvent;
use crate::fixed::Fx;
use crate::input::{self, Input};
use crate::rewards::Rewards;
use crate::sim::state::PlayerIdx;
use crate::sim::state::{Dir, State};
use crate::trig::SCALE;

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
    tempo_armoured, tempo_from_hits, tempo_hit, tempo_lose, tempo_range_pct, tempo_speed_pct,
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
mod chain;
pub(crate) use chain::combo_view;
pub(super) use chain::{
    COMBO_SLOT, TRIGGER_RING_SLOT, combo_proc, initial_combo, last_press_smash, step_at, toward,
};
use chain::{Reset, combo, combo_reset, level_of, press_of_pressed, strike, with_level};

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
