//! The main character (ADR-071): movement from the input's stick, the persistent facing
//! (ADR-069), the evade, and the combos of its Main weapon, made by hand (ADR-070, ADR-076).
//!
//! The combo: a press of `main` or `sub` is buffered (one, the latest; `main` when both come
//! on one tick). With no step running it takes the next step from where the combo is, or from
//! the root when there is none; a running step is cancelled for it once it reaches the
//! recovery after its last hits, if there is a next step for the press; a leaf runs to its end
//! first. When a step has run out with nothing buffered, the combo keeps its place for the
//! weapon's `window_ticks`. A step reached by `sub` is a combo part, which carries the Sub
//! weapon's pull. A step with a charge charges while the button that reached it is still held
//! as it starts: let go before level 1, it attacks as a tap would; later, at the level reached,
//! with the perfect bonus inside a window; at `max_ticks` it releases by itself. Presses while
//! charging are not buffered. An evade cancels the running step, breaks a charge and clears the
//! buffer, and a press during its dash or `dash_strike_ticks` after it takes the dash branch.

use crate::attack::AttackRef;
use crate::engine::{ComboState, DashState, Dir, PlayerState, Pull, State};
use crate::event::{DomainEvent, Press};
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::input::{self, Input};
use crate::presets;
use crate::stage::Stage;
use crate::trig::SCALE;
use crate::weapon::{Charge, MainWeapon, Step, SubBonus};

use super::PLAYER_SPEED;
use super::attacks;
use super::entity::{Who, speed_pct};
use super::levels;
use super::select;

/// The stick's full deflection on each axis (ADR-071); −128 counts as −127.
const STICK_MAX: i8 = 127;

/// `⌈√n⌉`.
fn ceil_sqrt(n: u64) -> u64 {
    let r = n.isqrt();
    if r * r < n { r + 1 } else { r }
}

/// The move of a stick at `(x, y)` under a speed cap of `cap` units: the vector clamped to
/// length 127, then scaled so that 127 is the cap. Its length is rounded up, so a diagonal is
/// never faster than a straight line (ADR-071).
fn stick_move(x: i8, y: i8, cap: i64) -> (i64, i64) {
    let (x, y) = (i64::from(x), i64::from(y));
    let len = ceil_sqrt((x * x + y * y).unsigned_abs()).max(STICK_MAX as u64);
    // `len >= 127 > 0`, and truncation toward zero is the same on every CPU.
    let len = len as i64;
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
    let len = ((x * x + y * y).unsigned_abs() * (scale * scale).unsigned_abs()).isqrt() as i64;
    Some(Dir {
        x: (x * scale * scale / len) as i32,
        y: (y * scale * scale / len) as i32,
    })
}

/// One tick of the main character: timers run down, it turns to its stick, evades on a ready
/// press (cancelling its combo step), walks its combo, then dashes or moves.
pub(super) fn update(stage: &Stage, s: &mut State, input: Input, events: &mut Vec<DomainEvent>) {
    let held = input::held_buttons(input);
    let pressed = held & !s.player.held;
    s.player.held = held;
    let p = &mut s.player;
    p.invulnerable = p.invulnerable.saturating_sub(1);
    p.evade_cooldown = p.evade_cooldown.saturating_sub(1);
    p.combo.dash_strike_left = p.combo.dash_strike_left.saturating_sub(1);
    let (x, y) = (input.move_x.max(-STICK_MAX), input.move_y.max(-STICK_MAX));
    if let Some(dir) = stick_direction(x, y) {
        p.facing = dir;
    }
    // The validator lets only defined presets through.
    let main = presets::main_weapon(&stage.player.main);
    s.player.target = match main {
        Some(m) if input.assist || input.auto => select::assist_target(stage, s, m.assist_range.0),
        _ => None,
    };
    if pressed & input::EVADE != 0 && s.player.evade_ready() {
        let cooldown = levels::evade_cooldown(s, stage.player.evade.cooldown_ticks);
        start_evade(stage, &mut s.player, cooldown, events);
        cancel(s, events);
        let strike = main.map_or(0, |m| m.dash_strike_ticks);
        s.player.combo.dash_strike_left = stage.player.evade.ticks.saturating_add(strike);
    }
    if let Some(main) = main {
        combo(stage, s, main, input, pressed, events);
    }
    if s.player.dash.is_some() {
        dash(stage, &mut s.player);
    } else {
        move_player(stage, s, x, y);
    }
}

fn move_by(stage: &Stage, p: &mut PlayerState, dx: i64, dy: i64) {
    // Both are within a speed cap or an evade's distance, which the validator bounds.
    p.at = stage.map().clamp(Point {
        x: p.at.x.saturating_add(Fx(dx as i32)),
        y: p.at.y.saturating_add(Fx(dy as i32)),
    });
}

fn move_player(stage: &Stage, s: &mut State, x: i8, y: i8) {
    let keep = speed_pct(&s.player.statuses) * attacks::move_pct(s, s.id_of(Who::Player)) / 100
        * charge_move_pct(stage, s)
        / 100;
    let cap = levels::speed(s, i64::from(PLAYER_SPEED) * keep / 100);
    let (dx, dy) = stick_move(x, y, cap);
    move_by(stage, &mut s.player, dx, dy);
}

/// A ready press dashes along the facing, which is the stick's direction when it is not
/// centred (ADR-071). Invulnerability and the cooldown count from the press.
fn start_evade(stage: &Stage, p: &mut PlayerState, cooldown: u32, events: &mut Vec<DomainEvent>) {
    let evade = &stage.player.evade;
    p.dash = Some(DashState {
        dir: p.facing,
        done: 0,
        left: evade.ticks,
    });
    p.evade_cooldown = cooldown;
    p.invulnerable = p.invulnerable.max(evade.invulnerable_ticks);
    events.push(DomainEvent::Evaded);
}

/// One tick of the dash: the `k`th tick moves from `distance × (k − 1) / ticks` to
/// `distance × k / ticks` along its direction, so the whole dash covers exactly its distance.
/// Slowing and attacks do not slow it, and the stick does not steer it.
fn dash(stage: &Stage, p: &mut PlayerState) {
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

/// The main character's attack slots (ADR-069): the combo; the stage skill and the effects
/// that attack on their own come with their own.
const COMBO_SLOT: u8 = 0;

fn press_of(sub: bool) -> Press {
    if sub { Press::Sub } else { Press::Main }
}

/// The step the combo is at: `None` at the root.
fn step_at<'a>(main: &'a MainWeapon, c: &ComboState) -> Option<&'a Step> {
    let mut branches = if c.dash { &main.dash } else { &main.combo };
    let mut step = None;
    for i in 0..c.depth {
        let next = branches.get(c.path >> i & 1 == 1)?;
        branches = &next.next;
        step = Some(next);
    }
    step
}

/// Whether the press that reached the current step was `sub`.
fn last_press_sub(c: &ComboState) -> bool {
    c.depth > 0 && c.path >> (c.depth - 1) & 1 == 1
}

/// Back to the root, reported if the combo was anywhere else.
fn reset(c: &mut ComboState, events: &mut Vec<DomainEvent>) {
    if c.depth > 0 {
        events.push(DomainEvent::ComboReset);
    }
    c.path = 0;
    c.depth = 0;
    c.dash = false;
    c.window_left = 0;
}

/// One tick of the combo (ADR-070, ADR-076): see the module's description.
fn combo(
    stage: &Stage,
    s: &mut State,
    main: &MainWeapon,
    input: Input,
    pressed: u8,
    events: &mut Vec<DomainEvent>,
) {
    if let Some(ticks) = s.player.combo.charge {
        charging(stage, s, main, input, ticks, events);
        return;
    }
    let press = if pressed & input::MAIN != 0 {
        Some(false)
    } else if pressed & input::SUB != 0 {
        Some(true)
    } else {
        None
    };
    if press.is_some() {
        s.player.combo.buffered = press;
    }
    let me = s.id_of(Who::Player);
    let busy = attacks::busy(s, me, COMBO_SLOT);
    if s.player.combo.running && !busy {
        s.player.combo.running = false;
        s.player.combo.window_left = main.window_ticks;
    }
    // The window counts the ticks after the step on which a press continues the combo; once
    // they are used up, the combo is back at the root before this tick's press.
    if !busy && s.player.combo.depth > 0 && s.player.combo.window_left == 0 {
        reset(&mut s.player.combo, events);
    }
    let c = s.player.combo;
    match c.buffered {
        Some(sub) if busy => {
            let next = step_at(main, &c).and_then(|st| st.next.get(sub)).is_some();
            if next && attacks::in_end_recovery(s, me, COMBO_SLOT) {
                attacks::end_run(s, me, COMBO_SLOT);
                take_step(stage, s, main, sub, input, events);
            }
        }
        Some(sub) => take_step(stage, s, main, sub, input, events),
        None if !busy && c.depth > 0 => s.player.combo.window_left -= 1,
        None => {}
    }
}

/// The buffered press `sub` takes its step: the next one from where the combo is, or else the
/// first from the root, in the dash branch while a dash-strike may start and it has one.
fn take_step(
    stage: &Stage,
    s: &mut State,
    main: &MainWeapon,
    sub: bool,
    input: Input,
    events: &mut Vec<DomainEvent>,
) {
    let c = &mut s.player.combo;
    c.buffered = None;
    let onward = step_at(main, c).is_some_and(|st| st.next.get(sub).is_some());
    if !onward {
        reset(c, events);
        c.dash = c.dash_strike_left > 0 && main.dash.get(sub).is_some();
        if !c.dash && main.combo.get(sub).is_none() {
            return;
        }
    }
    c.path |= u32::from(sub) << c.depth;
    c.depth += 1;
    c.window_left = 0;
    events.push(DomainEvent::ComboStep {
        press: press_of(sub),
        depth: c.depth,
    });
    let Some(step) = step_at(main, c) else {
        return;
    };
    let held = if sub { input.sub } else { input.main };
    if step.charge.is_some() && held {
        s.player.combo.charge = Some(0);
    } else {
        strike(stage, s, &step.attack, 0, input.assist, events);
    }
}

/// The current step's attack starts in the combo slot along the facing (ADR-069), at the
/// stage's attack stat, faster by the attack speed, with `bonus_pct` and, for a combo part, the
/// Sub weapon's pull. When it `aim`s and there is a target, the facing first turns to it
/// (ADR-072).
fn strike(
    stage: &Stage,
    s: &mut State,
    attack: &AttackRef,
    bonus_pct: u32,
    aim: bool,
    events: &mut Vec<DomainEvent>,
) {
    if aim {
        if let Some(dir) = s.player.target.and_then(|t| toward(s, t)) {
            s.player.facing = dir;
        }
    }
    let pull = if last_press_sub(&s.player.combo) {
        presets::sub_weapon(&stage.player.sub).map(|w| match w.bonus {
            SubBonus::Pull { distance, ticks } => Pull {
                distance: distance.0,
                ticks,
            },
        })
    } else {
        None
    };
    let strike = attacks::Strike {
        bonus_pct,
        speed_pct: levels::attack_speed(s),
        pull,
    };
    let facing = s.player.facing;
    let damage = levels::attack(s, stage.player.atk);
    s.player.combo.running =
        attacks::start_player(s, COMBO_SLOT, facing, attack, damage, strike, events);
}

/// The direction from the main character to the entity `target`, if it is there and not on
/// the main character's centre.
fn toward(s: &State, target: EntityId) -> Option<Dir> {
    let to = s.at(s.find(target)?);
    let from = s.player.at;
    let (dx, dy) = (
        i64::from(to.x.0) - i64::from(from.x.0),
        i64::from(to.y.0) - i64::from(from.y.0),
    );
    (dx != 0 || dy != 0).then(|| attacks::normalize(dx, dy))
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
fn current_charge<'a>(stage: &'a Stage, s: &State) -> Option<&'a Charge> {
    s.player.combo.charge?;
    let main = presets::main_weapon(&stage.player.main)?;
    step_at(main, &s.player.combo)?.charge.as_ref()
}

/// The charge's view for the snapshot: its level (0 before level 1), ticks held and whether
/// this tick is inside a perfect window.
pub(crate) fn charge_view(stage: &Stage, s: &State) -> Option<(u8, u32, bool)> {
    let held = s.player.combo.charge?;
    let charge = current_charge(stage, s)?;
    Some((
        u8::try_from(level_at(charge, held)).unwrap_or(u8::MAX),
        held,
        in_perfect_window(charge, held),
    ))
}

/// A charge held rises a tick; let go, judged on the last held tick (the one whose snapshot
/// the player saw, ADR-005), or at `max_ticks` without the bonus, it releases.
fn charging(
    stage: &Stage,
    s: &mut State,
    main: &MainWeapon,
    input: Input,
    held: u32,
    events: &mut Vec<DomainEvent>,
) {
    let c = s.player.combo;
    let Some(step) = step_at(main, &c) else {
        s.player.combo.charge = None;
        return;
    };
    let Some(charge) = &step.charge else {
        s.player.combo.charge = None;
        return;
    };
    let still = if last_press_sub(&c) {
        input.sub
    } else {
        input.main
    };
    if !still {
        release(stage, s, step, charge, held, true, input.assist, events);
        return;
    }
    let held = held.saturating_add(1);
    if held >= charge.max_ticks {
        release(stage, s, step, charge, held, false, input.assist, events);
    } else {
        s.player.combo.charge = Some(held);
    }
}

/// The release: level 0 is the step's own attack; a later level attacks with its attack, or
/// with its perfect attack and bonus when `judged` inside its window. It faces the target of
/// this tick when it `aim`s.
#[allow(clippy::too_many_arguments)]
fn release(
    stage: &Stage,
    s: &mut State,
    step: &Step,
    charge: &Charge,
    held: u32,
    judged: bool,
    aim: bool,
    events: &mut Vec<DomainEvent>,
) {
    s.player.combo.charge = None;
    let level = level_at(charge, held);
    if level == 0 {
        strike(stage, s, &step.attack, 0, aim, events);
        return;
    }
    let l = &charge.levels[level - 1];
    let perfect = judged && in_perfect_window(charge, held);
    if perfect {
        let attack = l.perfect_attack.as_ref().unwrap_or(&l.attack);
        strike(stage, s, attack, u32::from(l.perfect_pct), aim, events);
    } else {
        strike(stage, s, &l.attack, 0, aim, events);
    }
    events.push(DomainEvent::ChargeReleased {
        level: u8::try_from(level).unwrap_or(u8::MAX),
        perfect,
    });
}

/// An evade cancels the running step, breaks a charge and clears the buffer (ADR-070).
fn cancel(s: &mut State, events: &mut Vec<DomainEvent>) {
    let me = s.id_of(Who::Player);
    attacks::end_run(s, me, COMBO_SLOT);
    let c = &mut s.player.combo;
    if c.charge.take().is_some() {
        events.push(DomainEvent::ChargeBroken);
    }
    c.buffered = None;
    c.running = false;
    reset(c, events);
}

/// A hit from a phase that breaks charges ends a charge in progress without an attack, and the
/// combo starts again from the root (ADR-070).
pub(super) fn break_charge(s: &mut State, events: &mut Vec<DomainEvent>) {
    let c = &mut s.player.combo;
    if c.charge.take().is_none() {
        return;
    }
    events.push(DomainEvent::ChargeBroken);
    c.buffered = None;
    reset(c, events);
}

/// The movement speed, in percent, a charge leaves the main character: its `move_pct`, 100
/// when not charging.
fn charge_move_pct(stage: &Stage, s: &State) -> i64 {
    current_charge(stage, s).map_or(100, |c| i64::from(c.move_pct))
}

/// The combo's view for the snapshot: the presses from the root, whether it began in the dash
/// branch, the buffered press and the window's ticks left.
pub(crate) fn combo_view(s: &State) -> (Vec<Press>, bool, Option<Press>, u32) {
    let c = &s.player.combo;
    let presses = (0..c.depth)
        .map(|i| press_of(c.path >> i & 1 == 1))
        .collect();
    (presses, c.dash, c.buffered.map(press_of), c.window_left)
}
