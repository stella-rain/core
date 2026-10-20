//! The main character (ADR-071): movement from the input's stick, the persistent facing
//! (ADR-069), the evade on its charges (ADR-074), and the combos of its Main weapon, made by hand (ADR-070, ADR-076).
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
//! charging are not buffered. A step with a hold (ADR-084 point 6) holds where its recovery would
//! begin while its button is still held: its loop runs and starts again each time it ends, until
//! the button is let go or `max_ticks`, when its release plays; presses while holding are
//! buffered. An evade cancels the running step, breaks a charge or a hold and clears the buffer,
//! and a press during its dash or `dash_strike_ticks` after it takes the dash branch.
//! Every step's attack is wider by the range rewards and the expertise range options taken
//! (ADR-078, ADR-099), at most `MAX_RANGE_PCT` in all.
//!
//! Aim assist (ADR-072): while `assist` or `auto` is on, each tick picks a target; with `assist`
//! on, a step faces it as its attack starts. Auto mode (ADR-073): while `auto` is on, there is a
//! target and no press by hand is in the combo, it presses for the player, the next press of
//! the auto combo, taken as a buffered press would be; after the auto combo's last step, or
//! anywhere else, it starts again from the root once the step has run out. Its presses never
//! hold, never take the dash branch and always face the target. A combo with a press by hand
//! becomes the auto combo when it resets, unless it began in the dash branch.

use crate::attack::AttackRef;
use crate::engine::{ComboState, DashState, Dir, PlayerState, Pull, Smash, State};
use crate::event::{DomainEvent, Press};
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::input::{self, Input};
use crate::presets;
use crate::rewards::Rewards;
use crate::stage::Stage;
use crate::trig::SCALE;
use crate::validate::limits::MAX_EVADE_CHARGES;
use crate::weapon::{Charge, Hold, SmashEffect, Step, Weapon};

use super::PLAYER_SPEED;
use super::attacks;
use super::entity::{Who, speed_pct};
use super::levels;
use super::select;
use super::skill;

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
pub(super) fn update(
    stage: &Stage,
    rewards: &Rewards,
    s: &mut State,
    input: Input,
    events: &mut Vec<DomainEvent>,
) {
    let held = input::held_buttons(input);
    let pressed = held & !s.player.held;
    s.player.held = held;
    let p = &mut s.player;
    p.invulnerable = p.invulnerable.saturating_sub(1);
    for t in &mut p.evade_recovering {
        *t -= 1;
    }
    p.evade_recovering.retain(|&t| t > 0);
    p.combo.dash_strike_left = p.combo.dash_strike_left.saturating_sub(1);
    skill::tick(s);
    let p = &mut s.player;
    let (x, y) = (input.move_x.max(-STICK_MAX), input.move_y.max(-STICK_MAX));
    if let Some(dir) = stick_direction(x, y) {
        p.facing = dir;
    }
    // The validator lets only defined presets through.
    let main = presets::weapon(&stage.player.weapon);
    s.player.target = match main {
        Some(m) if input.assist || input.auto => select::assist_target(stage, s, m.assist_range.0),
        _ => None,
    };
    if pressed & input::EVADE != 0 && s.player.evade_ready() > 0 {
        let cooldown = levels::evade_cooldown(s, stage.player.evade.cooldown_ticks);
        start_evade(stage, &mut s.player, cooldown, events);
        cancel(s, events);
        let strike = main.map_or(0, |m| m.dash_strike_ticks);
        s.player.combo.dash_strike_left = stage.player.evade.ticks.saturating_add(strike);
    }
    if pressed & input::SKILL != 0 {
        skill::press(stage, rewards, s, input, events);
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
        * held_move_pct(stage, s)
        / 100;
    let cap = levels::speed(s, i64::from(PLAYER_SPEED) * keep / 100);
    let (dx, dy) = stick_move(x, y, cap);
    move_by(stage, &mut s.player, dx, dy);
}

/// A press with a charge ready dashes along the facing, which is the stick's direction when it
/// is not centred (ADR-071), from where the main character is, a dash in progress or not
/// (ADR-074). Invulnerability and the spent charge's `cooldown` count from the press.
fn start_evade(stage: &Stage, p: &mut PlayerState, cooldown: u32, events: &mut Vec<DomainEvent>) {
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

/// A reward's `amount` more evade charges, never beyond the stage's `max_charges` (ADR-074 §5).
/// The new ones are ready: the spent ones keep their timers.
pub(super) fn add_evade_charges(stage: &Stage, p: &mut PlayerState, amount: u32) {
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
pub(super) const COMBO_SLOT: u8 = 0;

fn press_of(smash: bool) -> Press {
    if smash { Press::Smash } else { Press::Main }
}

/// The step the combo is at: `None` at the root.
fn step_at<'a>(main: &'a Weapon, c: &ComboState) -> Option<&'a Step> {
    let mut branches = if c.dash { &main.dash } else { &main.combo };
    let mut step = None;
    for i in 0..c.depth {
        let next = branches.get_open(c.path >> i & 1 == 1, c.upgrades)?;
        branches = &next.next;
        step = Some(next);
    }
    step
}

/// Whether the press that reached the current step was `smash`.
fn last_press_smash(c: &ComboState) -> bool {
    c.depth > 0 && c.path >> (c.depth - 1) & 1 == 1
}

/// Back to the root, reported if the combo was anywhere else. A combo with a press by hand
/// becomes the auto combo, unless it began in the dash branch (ADR-073).
fn reset(c: &mut ComboState, events: &mut Vec<DomainEvent>) {
    if c.depth > 0 {
        events.push(DomainEvent::ComboReset);
        if c.hand && !c.dash {
            c.auto_path = c.path;
            c.auto_depth = c.depth;
        }
    }
    c.hand = false;
    c.path = 0;
    c.depth = 0;
    c.dash = false;
    c.window_left = 0;
    c.released = false;
}

/// One tick of the combo (ADR-070, ADR-076): see the module's description.
fn combo(
    stage: &Stage,
    s: &mut State,
    main: &Weapon,
    input: Input,
    pressed: u8,
    events: &mut Vec<DomainEvent>,
) {
    if let Some(ticks) = s.player.combo.charge {
        charging(stage, s, main, input, ticks, events);
        return;
    }
    if let Some(ticks) = s.player.combo.hold {
        holding(stage, s, main, input, pressed, ticks, events);
        return;
    }
    if let Some(press) = press_of_pressed(pressed) {
        s.player.combo.buffered = Some(press);
    }
    let me = s.id_of(Who::Player);
    let busy = attacks::busy(s, me, COMBO_SLOT);
    // A held step holds where its recovery would begin, unless a buffered press takes its next
    // step first (ADR-084 points 3 and 6).
    if busy && attacks::hold_ready(s, me, COMBO_SLOT) && begins_hold(main, &s.player.combo, input) {
        begin_hold(stage, s, main, input, events);
        return;
    }
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
    let (smash, by_hand) = match (c.buffered, auto_press(&c, input, s.player.target.is_some())) {
        (Some(smash), _) => (smash, true),
        (None, Some(Auto::Next(smash))) => (smash, false),
        (None, Some(Auto::Again)) if !busy => {
            reset(&mut s.player.combo, events);
            (c.auto_path & 1 == 1, false)
        }
        _ => {
            if !busy && c.depth > 0 {
                s.player.combo.window_left -= 1;
            }
            return;
        }
    };
    if !busy {
        take_step(stage, s, main, smash, by_hand, input, events);
        return;
    }
    let next = step_at(main, &c).is_some_and(|st| st.next.get_open(smash, c.upgrades).is_some());
    if next && attacks::in_end_recovery(s, me, COMBO_SLOT) {
        attacks::end_run(s, me, COMBO_SLOT);
        take_step(stage, s, main, smash, by_hand, input, events);
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

/// Whether the step whose recovery begins now holds (ADR-084 point 6): it has a hold, its press
/// was by hand (auto mode never holds, ADR-073), the button that reached it is still held, and
/// no buffered press takes a next step, which cancels the recovery instead (point 3).
fn begins_hold(main: &Weapon, c: &ComboState, input: Input) -> bool {
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
        .is_some_and(|smash| step.next.get_open(smash, c.upgrades).is_some());
    still && !cancels
}

/// The hold begins on the tick the recovery would: the step's own recovery does not play, and
/// the loop starts on this tick.
fn begin_hold(
    stage: &Stage,
    s: &mut State,
    main: &Weapon,
    input: Input,
    events: &mut Vec<DomainEvent>,
) {
    let me = s.id_of(Who::Player);
    attacks::end_run(s, me, COMBO_SLOT);
    s.player.combo.hold = Some(0);
    spin(stage, s, main, input, events);
}

/// The loop starts again, as a new attack: its facing is the main character's as it starts
/// (ADR-069), or the target's with aim assist on (ADR-072).
fn spin(stage: &Stage, s: &mut State, main: &Weapon, input: Input, events: &mut Vec<DomainEvent>) {
    let Some(hold) = step_at(main, &s.player.combo).and_then(|st| st.hold.as_ref()) else {
        return;
    };
    strike(stage, s, &hold.loop_attack, 0, input.assist, events);
}

/// One tick of a hold (ADR-084 point 6): a press is buffered, as during any step. Letting go,
/// or reaching `max_ticks`, releases with the ticks held; otherwise the hold goes on, and the
/// loop starts again once the last one has ended.
fn holding(
    stage: &Stage,
    s: &mut State,
    main: &Weapon,
    input: Input,
    pressed: u8,
    held: u32,
    events: &mut Vec<DomainEvent>,
) {
    if let Some(press) = press_of_pressed(pressed) {
        s.player.combo.buffered = Some(press);
    }
    let c = s.player.combo;
    let Some(hold) = step_at(main, &c).and_then(|st| st.hold.as_ref()) else {
        s.player.combo.hold = None;
        return;
    };
    let still = if last_press_smash(&c) {
        input.smash
    } else {
        input.main
    };
    if !still {
        release_hold(stage, s, hold, held, input, events);
        return;
    }
    let held = held.saturating_add(1);
    if held >= hold.max_ticks {
        release_hold(stage, s, hold, held, input, events);
        return;
    }
    s.player.combo.hold = Some(held);
    if !attacks::busy(s, s.id_of(Who::Player), COMBO_SLOT) {
        spin(stage, s, main, input, events);
    }
}

/// The release: the loop ends with the hold, and the release attack starts with its facing
/// locked (ADR-069). Its recovery is the end lag, as any step's (point 3).
fn release_hold(
    stage: &Stage,
    s: &mut State,
    hold: &Hold,
    ticks: u32,
    input: Input,
    events: &mut Vec<DomainEvent>,
) {
    let me = s.id_of(Who::Player);
    attacks::end_run(s, me, COMBO_SLOT);
    s.player.combo.hold = None;
    s.player.combo.released = true;
    strike(stage, s, &hold.release, 0, input.assist, events);
    events.push(DomainEvent::HoldReleased { ticks });
}

/// What auto mode presses this tick (ADR-073).
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
/// `main` pressed from the root until a step has no next step for it (ADR-073).
pub(super) fn initial_combo(stage: &Stage, upgrades: u32) -> ComboState {
    let mut depth = 0u8;
    if let Some(main) = presets::weapon(&stage.player.weapon) {
        let mut step = main.combo.get_open(false, upgrades);
        while let Some(st) = step {
            depth = depth.saturating_add(1);
            step = st.next.get_open(false, upgrades);
        }
    }
    ComboState {
        auto_depth: depth,
        upgrades,
        ..ComboState::default()
    }
}

/// The press `smash`, buffered `by_hand` or auto mode's, takes its step: the next one from where
/// the combo is, or else the first from the root, in the dash branch while a dash-strike may
/// start and it has one (by hand only). Auto mode's step never charges and faces the target.
#[allow(clippy::too_many_arguments)]
fn take_step(
    stage: &Stage,
    s: &mut State,
    main: &Weapon,
    smash: bool,
    by_hand: bool,
    input: Input,
    events: &mut Vec<DomainEvent>,
) {
    let c = &mut s.player.combo;
    c.buffered = None;
    c.released = false;
    let onward = step_at(main, c).is_some_and(|st| st.next.get_open(smash, c.upgrades).is_some());
    if !onward {
        reset(c, events);
        c.dash = by_hand && c.dash_strike_left > 0 && main.dash.get_open(smash, c.upgrades).is_some();
        if !c.dash && main.combo.get_open(smash, c.upgrades).is_none() {
            return;
        }
    }
    c.hand |= by_hand;
    c.path |= u32::from(smash) << c.depth;
    c.depth += 1;
    c.window_left = 0;
    events.push(DomainEvent::ComboStep {
        press: press_of(smash),
        depth: c.depth,
    });
    let Some(step) = step_at(main, c) else {
        return;
    };
    let held = by_hand && if smash { input.smash } else { input.main };
    if step.charge.is_some() && held {
        s.player.combo.charge = Some(0);
    } else {
        strike(stage, s, &step.attack, 0, input.assist || !by_hand, events);
    }
}

/// The current step's attack starts in the combo slot along the facing (ADR-069), at the
/// stage's attack stat, faster by the attack speed, wider by the range rewards and the
/// expertise range options taken (at most `MAX_RANGE_PCT` in all), with
/// `bonus_pct` and, for a combo part, the Sub weapon's pull. When it `aim`s and there is a target, the facing first turns to it
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
    let smash = if last_press_smash(&s.player.combo) {
        presets::weapon(&stage.player.weapon)
            .and_then(|w| w.smash)
            .map(|effect| match effect {
                SmashEffect::Pull { distance, ticks } => Smash::Pull(Pull {
                    distance: distance.0,
                    ticks,
                }),
                SmashEffect::Bleed {} => Smash::Bleed,
                SmashEffect::Stun { ticks } => Smash::Stun { ticks },
            })
    } else {
        None
    };
    let strike = attacks::Strike {
        bonus_pct,
        speed_pct: levels::attack_speed(s),
        range_pct: s
            .player
            .range_pct
            .saturating_add(s.expertise.as_ref().map_or(0, |g| g.range_pct)),
        smash,
    };
    let facing = s.player.facing;
    let damage = levels::attack(s, stage.player.atk);
    s.player.combo.running =
        attacks::start_player(s, COMBO_SLOT, facing, attack, damage, strike, events);
}

/// The direction from the main character to the entity `target`, if it is there and not on
/// the main character's centre.
pub(super) fn toward(s: &State, target: EntityId) -> Option<Dir> {
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
    let main = presets::weapon(&stage.player.weapon)?;
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
    main: &Weapon,
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
    let still = if last_press_smash(&c) {
        input.smash
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
    if c.hold.take().is_some() {
        events.push(DomainEvent::HoldBroken);
    }
    c.buffered = None;
    c.running = false;
    reset(c, events);
}

/// A hit from a phase that breaks charges ends a charge or a hold in progress without an
/// attack (a hold's loop ends with it), and the combo starts again from the root (ADR-070,
/// ADR-084 point 6).
pub(super) fn break_charge(s: &mut State, events: &mut Vec<DomainEvent>) {
    let me = s.id_of(Who::Player);
    let charging = s.player.combo.charge.take().is_some();
    let holding = s.player.combo.hold.take().is_some();
    if !charging && !holding {
        return;
    }
    if holding {
        attacks::end_run(s, me, COMBO_SLOT);
        events.push(DomainEvent::HoldBroken);
    }
    if charging {
        events.push(DomainEvent::ChargeBroken);
    }
    let c = &mut s.player.combo;
    c.buffered = None;
    reset(c, events);
}

/// The movement speed, in percent, a charge or a hold leaves the main character: its
/// `move_pct`, 100 when neither is on.
fn held_move_pct(stage: &Stage, s: &State) -> i64 {
    if let Some(hold) = current_hold(stage, s) {
        return i64::from(hold.move_pct);
    }
    current_charge(stage, s).map_or(100, |c| i64::from(c.move_pct))
}

/// The current step's hold, if it is held.
fn current_hold<'a>(stage: &'a Stage, s: &State) -> Option<&'a Hold> {
    s.player.combo.hold?;
    let main = presets::weapon(&stage.player.weapon)?;
    step_at(main, &s.player.combo)?.hold.as_ref()
}

/// The hold's view for the snapshot (ADR-084 point 9): the ticks held and the most it can last.
pub(crate) fn hold_view(stage: &Stage, s: &State) -> Option<(u32, u32)> {
    let held = s.player.combo.hold?;
    Some((held, current_hold(stage, s)?.max_ticks))
}

/// The presses of `path` from the root, `depth` of them.
fn presses_of(path: u32, depth: u8) -> Vec<Press> {
    (0..depth).map(|i| press_of(path >> i & 1 == 1)).collect()
}

/// The combo's view for the snapshot: the presses from the root, whether it began in the dash
/// branch, the buffered press, the window's ticks left and the auto combo.
pub(crate) fn combo_view(s: &State) -> (Vec<Press>, bool, Option<Press>, u32, Vec<Press>) {
    let c = &s.player.combo;
    (
        presses_of(c.path, c.depth),
        c.dash,
        c.buffered.map(press_of),
        c.window_left,
        presses_of(c.auto_path, c.auto_depth),
    )
}
