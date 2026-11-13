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
//! in `magnetism`. A down empties both fields.
//!
//! The combo tempo (when the style names the `combo_tempo_pct` kind): a swing of a combo step that
//! hits adds one stack to `combo_tempo`, to the kind's `max_stacks` (`tempo_from_hits`);
//! `strike` reads the stacks when an attack starts, adding their percent to the attack speed under
//! the run's cap (`tempo_speed_pct`), so a step, each loop of a hold and a release take the
//! stacks held at their start. `tempo_lose` empties them: a combo reset away from the middle of
//! a combo (`combo_reset`), a smash press after the smash itself started (`tempo_on_smash`;
//! full stacks with `keep_on_smash` stay), a hit taken (`hurt_player`; not under `tempo_armoured`,
//! full stacks in a held hold with `super_armour_at_full`), a down and another style.
//!
//! The giant trigger (when the style names the `giant_trigger` kind): `spend_gauge` starts it
//! after the full-gauge smash's own attack began (`trigger_start`), `trigger_tick` counts it down
//! in `update`, `strike` adds its range percent to every step and its smash percent to a smash
//! step when they start, `super_armour` reads `trigger_armoured`, and `stand_down` ends it.
//!
//! The breaker (when the style names the `breaker` kind): it holds no state, so nothing starts,
//! counts down or ends. Its three factors are read where the number is computed: `breaker_fill`
//! in `stun::fill` (every stagger fill, after the counter bonus's factor), `breaker_poise` in
//! `hit_boxes::flinch_by` (a combo hit's poise damage) and `breaker_part` in the part loop of
//! `hit_boxes::hit_enemy_side` (a smash step's hit on a boss part). Its burst is
//! `stagger_burst`, called by `stun::fill` right after `stagger` in the same call.
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
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::input::{self, Input};
use crate::limits::{MAX_ADVANCE_DISTANCE, MAX_ADVANCE_PER_TICK, MAX_COUNTER_WINDOW_TICKS};
use crate::rewards::Rewards;
use crate::sim::state::{ComboState, DashState, Dir, PlayerState, Pull, Smash, State};
use crate::sim::state::{MAX_PLAYERS, PlayerIdx};
use crate::trig::SCALE;
use crate::validate::limits::MAX_EVADE_CHARGES;
use crate::weapon::{Charge, Hold, SmashEffect, Step, Weapon};

use super::attacks;
use super::entity::{Who, speed_pct};
use super::levels;
use super::select;
use super::skill;
use super::world::{BurstGrant, CounterGrant};

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
        trigger_tick(s, me, events);
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

/// Attack magnetism (sim_version >= 2): while the combo step's phase names an `advance`, the
/// player slides along the facing the step took toward its target, by the dash's share of the
/// distance each tick (`dash.rs`), at most `MAX_ADVANCE_PER_TICK` and at most the gap that leaves
/// the two circles touching. No slide without a current target, with the target behind the
/// facing or gone. The evade cancels the run, so it ends the slide.
fn magnetism(stage: &World, s: &mut State, me: PlayerIdx) {
    let Some(target) = s.players[me].target else {
        return;
    };
    let id = s.players.id(me);
    let Some(r) = s
        .runs
        .iter()
        .find(|r| r.owner == id && r.slot == COMBO_SLOT)
    else {
        return;
    };
    // A phase's own `advance` wins; else a taken counter bonus slides over the first phase.
    let Some(advance) = attacks::advance_of(stage, r).or_else(|| {
        if r.phase != 0 {
            return None;
        }
        counter_taken(stage, s, me)
            .map(|g| g.advance)
            .filter(|a| a.0 > 0)
    }) else {
        return;
    };
    let phase_ticks = attacks::phase(stage, r, r.phase as usize).ticks;
    if r.tick >= phase_ticks {
        return;
    }
    let ticks = i64::from(phase_ticks);
    let Some((at, radius)) = select::target_circle(stage, s, target) else {
        return;
    };
    let distance = i64::from(advance.0).clamp(0, i64::from(Fx::px(MAX_ADVANCE_DISTANCE).0));
    let (k, facing) = (i64::from(r.tick) + 1, r.facing);
    let share = distance * k / ticks - distance * (k - 1) / ticks;
    let p = &s.players[me];
    let dx = i64::from(at.x.0) - i64::from(p.at.x.0);
    let dy = i64::from(at.y.0) - i64::from(p.at.y.0);
    if i64::from(facing.x) * dx + i64::from(facing.y) * dy <= 0 {
        return;
    }
    let reach = i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i64::MAX);
    let gap = (reach - i64::from(stage.tuning.player_radius) - i64::from(radius)).max(0);
    let step = share
        .min(i64::from(Fx::px(MAX_ADVANCE_PER_TICK).0))
        .min(gap);
    let scale = i64::from(SCALE);
    move_by(
        stage,
        &mut s.players[me],
        i64::from(facing.x) * step / scale,
        i64::from(facing.y) * step / scale,
    );
}

/// A near miss: the evade's own invulnerability stopped a hit on player `me` (the caller has
/// seen the hit overlap and `invulnerable > 0`). Inside the window, and when the skip is the
/// evade's (`invulnerable` is what the evade has left, a tie goes to the evade; more is a hit's or
/// a revive's residue), the window closes and, with no cooldown running, the evade is perfect: the
/// slow and its own cooldown start (ADR-SIM-8: only this player's reach is slowed). No damage, no
/// record of the hit. The counter window opens beside it, cooldown or not.
pub(super) fn near_miss(
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
pub(super) fn perfect_slow(stage: &World, s: &mut State) {
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
    super::scratch::reserve_to(&mut set, s.enemies.len() + 1 + 16);
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
pub(super) fn slowed(s: &State, id: EntityId) -> Option<bool> {
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

/// The distance between the farthest two up players past which the soft tether pulls (ADR-SIM-5's
/// view, 840 px wide).
pub(super) const TETHER_RANGE: Fx = Fx(2 * super::VIEW_HALF_WIDTH.0);

/// The share of the overshoot pulled back each tick: a placeholder the bot playtest tunes.
pub(super) const TETHER_PULL_DIV: i64 = 8;

/// The soft tether (ADR-SIM-7), after `schedule::steps::tether` checks it can act.
///
/// Over the up players (at most `MAX_PLAYERS`): when the farthest pair is more than
/// `TETHER_RANGE` apart, every up player at the greatest distance from the bounding box's centre
/// that is not dashing moves toward it by `TETHER_PULL_DIV`ths of the overshoot. A dash is never
/// blocked (it is skipped), a downed or out player is neither measured nor pulled. Doubled
/// coordinates keep the centre exact; `i128` holds the squares of a wide map.
pub(super) fn tether(stage: &World, s: &mut State) {
    let mut who = [PlayerIdx::SOLO; MAX_PLAYERS as usize];
    let mut at = [(0_i128, 0_i128); MAX_PLAYERS as usize];
    let mut n = 0;
    for me in s.players.indices() {
        let p = &s.players[me];
        if p.is_up() {
            who[n] = me;
            at[n] = (i128::from(p.at.x.0), i128::from(p.at.y.0));
            n += 1;
        }
    }
    if n < 2 {
        return;
    }
    let (who, at) = (&who[..n], &at[..n]);
    let mut far = 0_i128;
    let (mut lo, mut hi) = (at[0], at[0]);
    for (i, &(ax, ay)) in at.iter().enumerate() {
        lo = (lo.0.min(ax), lo.1.min(ay));
        hi = (hi.0.max(ax), hi.1.max(ay));
        for &(bx, by) in &at[i + 1..] {
            far = far.max((ax - bx) * (ax - bx) + (ay - by) * (ay - by));
        }
    }
    let range = i128::from(TETHER_RANGE.0);
    if far <= range * range {
        return;
    }
    // The pull is a share of the farthest pair's overshoot.
    let overshoot = far.isqrt() - range;
    let pull = overshoot / i128::from(TETHER_PULL_DIV);
    let (cx, cy) = (lo.0 + hi.0, lo.1 + hi.1);
    let offset = |&(x, y): &(i128, i128)| (cx - 2 * x, cy - 2 * y);
    let dist2 = |(x, y): (i128, i128)| x * x + y * y;
    let top = at.iter().map(|a| dist2(offset(a))).max().unwrap_or(0);
    for (me, a) in who.iter().zip(at) {
        let v = offset(a);
        if dist2(v) != top || s.players[*me].dash.is_some() {
            continue;
        }
        let len2 = dist2(v).isqrt();
        if len2 == 0 {
            continue;
        }
        let step = pull.min(len2 / 2);
        let dx = i64::try_from(v.0 * step / len2).unwrap_or(0);
        let dy = i64::try_from(v.1 * step / len2).unwrap_or(0);
        move_by(stage, &mut s.players[*me], dx, dy);
    }
}

pub(super) fn move_by(stage: &World, p: &mut PlayerState, dx: i64, dy: i64) {
    // Both are within a speed cap or an evade's distance, which the validator bounds.
    p.at = stage.map().clamp(Point {
        x: p.at.x.saturating_add(Fx::from_wide(dx)),
        y: p.at.y.saturating_add(Fx::from_wide(dy)),
    });
}

fn move_player(stage: &World, s: &mut State, me: PlayerIdx, x: i8, y: i8) {
    let keep = speed_pct(&s.players[me].statuses)
        * attacks::move_pct(stage, s, s.id_of(Who::Player(me)))
        / 100
        * held_move_pct(stage, s, me)
        / 100;
    let cap = levels::speed(s, me, i64::from(stage.tuning.player_speed) * keep / 100);
    let (dx, dy) = stick_move(x, y, cap);
    move_by(stage, &mut s.players[me], dx, dy);
}

/// A press with a charge ready dashes along the facing, which is the stick's direction when it
/// is not centred, from where the main character is, a dash in progress or not
///. Invulnerability and the spent charge's `cooldown` count from the press.
fn start_evade(stage: &World, p: &mut PlayerState, cooldown: u32, events: &mut Vec<DomainEvent>) {
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
pub(super) fn add_evade_charges(stage: &World, p: &mut PlayerState, amount: u32) {
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
fn dash(stage: &World, p: &mut PlayerState) {
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

/// The main character's attack slots: the combo; the stage skill and the effects
/// that attack on their own come with their own.
pub(super) const COMBO_SLOT: u8 = 0;

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

/// Back to the root through `reset`, for every caller that has the whole world: the one place the
/// combo's end is seen from outside the combo (ADR-SIM-9).
fn combo_reset(stage: &World, s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    // A reset at the root changes nothing: only a combo that was somewhere else ends the stacks.
    let mid_combo = s.players[me].combo.depth > 0;
    reset(&mut s.players[me].combo, events);
    if mid_combo {
        tempo_lose(stage, s, me, events);
    }
}

/// The combo tempo of `me` ends: the stacks go to 0 and `TempoLost` says so, once. None after one
/// bool test for a run that names no combo tempo (ADR-SIM-9).
pub(super) fn tempo_lose(
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
pub(super) fn tempo_armoured(stage: &World, s: &State, me: PlayerIdx) -> bool {
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
pub(super) fn tempo_speed_pct(stage: &World, s: &State, me: PlayerIdx) -> u32 {
    if !stage.has_tempo {
        return 0;
    }
    stage.tempo_of(s, me).map_or(0, |g| {
        u32::from(s.players[me].combat.combo_tempo) * g.speed_pct
    })
}

/// A smash step is pressed: the stacks end, unless the style keeps them and they are full.
/// Called after the smash's own attack started, so that attack took the speed of the stacks
/// held at the press.
fn tempo_on_smash(stage: &World, s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
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
pub(super) fn tempo_from_hits(
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
        combo_reset(stage, s, me, events);
    }
    let c = s.players[me].combo;
    let (smash, by_hand) = match (
        c.buffered,
        auto_press(&c, input, s.players[me].target.is_some()),
    ) {
        (Some(smash), _) => (smash, true),
        (None, Some(Auto::Next(smash))) => (smash, false),
        (None, Some(Auto::Again)) if !busy => {
            combo_reset(stage, s, me, events);
            (c.auto_path & 1 == 1, false)
        }
        _ => {
            if !busy && c.depth > 0 {
                s.players[me].combo.window_left -= 1;
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

/// Whether the step whose recovery begins now holds: it has a hold, its press
/// was by hand (auto mode never holds), the button that reached it is still held, and
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
        .is_some_and(|smash| step.next.get(smash).is_some());
    still && !cancels
}

/// The hold begins on the tick the recovery would: the step's own recovery does not play, and
/// the loop starts on this tick.
fn begin_hold(
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
fn holding(
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
        combo_reset(stage, s, me, events);
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

/// A friendly swing of player `owner`'s combo hit `n` enemies this tick: a normal (A) step fills
/// the weapon gauge its style grants by `per_step` once and `per_hit` for each, to the step's cap
/// and the gauge's `max`. A smash, a downed player and a style without a gauge fill nothing.
pub(super) fn gauge_from_hits(
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
fn gauge_filled(events: &mut Vec<DomainEvent>, me: PlayerIdx, added: u32, value: u32, max: u32) {
    events.push(DomainEvent::GaugeFilled {
        player: me,
        amount: added,
        value,
        full: value >= max,
    });
}

/// The counter window of `me` opened. Cold, as `gauge_filled`.
#[cold]
#[inline(never)]
fn counter_window_opened(events: &mut Vec<DomainEvent>, me: PlayerIdx) {
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
pub(super) fn counter_taken<'a>(
    stage: &'a World,
    s: &State,
    me: PlayerIdx,
) -> Option<&'a CounterGrant> {
    if !stage.has_counter_bonus || s.players[me].combat.counter_bonus_step != 1 {
        return None;
    }
    stage.counter_of(s, me)
}

/// `value` scaled by `pct` percent, one truncation, saturating to `u32`.
pub(super) fn scaled(value: u32, pct: u32) -> u32 {
    u32::try_from(u64::from(value) * u64::from(pct) / 100).unwrap_or(u32::MAX)
}

/// The breaker's factor on a stagger fill credited to `me`: `value` unchanged without a breaker
/// naming `stagger_pct`, after one bool test for a run that names no breaker.
pub(super) fn breaker_fill(stage: &World, s: &State, me: PlayerIdx, value: u32) -> u32 {
    match stage.breaker_of(s, me) {
        Some(g) if g.stagger_pct > 100 => scaled(value, g.stagger_pct),
        _ => value,
    }
}

/// The breaker's factor on the poise damage of a combo hit of `me`.
pub(super) fn breaker_poise(stage: &World, s: &State, me: PlayerIdx, damage: u32) -> u32 {
    match stage.breaker_of(s, me) {
        Some(g) if g.poise_pct > 100 => scaled(damage, g.poise_pct),
        _ => damage,
    }
}

/// The breaker's factor on the damage of a smash step's combo hit of `me` on a boss part: `damage`
/// unchanged for any other hit.
pub(super) fn breaker_part(
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
pub(super) fn stagger_burst(
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
            && super::kills::damageable(s, Who::Enemy(j))
            && super::collide::overlaps(at, burst.radius, e.at, stage.defs.agent(e.def).radius.0)
    };
    if burst.damage_pct > 0 {
        let shot = super::Shot {
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
        };
        for j in 0..s.enemies.len() {
            if reached(s, j) {
                super::damage(stage, s, me, shot, Who::Enemy(j), events);
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
            super::stun::apply(stage, s, Who::Enemy(j), owner, burst.stun_ticks, events);
        }
        if burst.knock_down {
            super::stun::knock_down(stage, s, me, j, events);
        }
    }
}

/// A step of `me` starts: the mark of the step before is gone, and the first step of the dash
/// branch pressed inside an open window takes the bonus and closes the window. A step of the
/// ordinary combo, a later step of the dash branch and an auto step take nothing and leave an
/// open window running.
fn counter_take(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    first_dash: bool,
    events: &mut Vec<DomainEvent>,
) {
    let c = &mut s.players[me].combat;
    c.counter_bonus_step = 0;
    if first_dash
        && (1..=MAX_COUNTER_WINDOW_TICKS).contains(&c.counter_window)
        && stage.counter_of(s, me).is_some()
    {
        let c = &mut s.players[me].combat;
        c.counter_window = 0;
        c.counter_bonus_step = 1;
        counter_bonus_taken(events, me);
    }
}

/// The counter window counts down one a tick, and the mark ends with the step: when none of the
/// player's steps is in progress (a run, a charge or a hold), at the step's end, at an evade and
/// at a break.
fn counter_tick(s: &mut State, me: PlayerIdx) {
    let p = &mut s.players[me];
    p.combat.counter_window = p.combat.counter_window.saturating_sub(1);
    let c = &p.combo;
    if !(c.running || c.charge.is_some() || c.hold.is_some()) {
        p.combat.counter_bonus_step = 0;
    }
}

/// The full-gauge smash of `me` spent `amount`. Cold, as `gauge_filled`.
#[cold]
#[inline(never)]
fn gauge_spent(events: &mut Vec<DomainEvent>, me: PlayerIdx, amount: u32) {
    events.push(DomainEvent::GaugeSpent { player: me, amount });
}

/// `me`'s near miss was a perfect evade. Cold, as `gauge_filled`.
#[cold]
#[inline(never)]
fn perfect_evade_event(events: &mut Vec<DomainEvent>, me: PlayerIdx) {
    events.push(DomainEvent::PerfectEvade { player: me });
}

/// Player `me` lost `lost` hit points to a hit: the gauge gains `max * lost * pct / (max_hp *
/// 100)`, to `max` at most, when the player is still up. The step cap does not hold it.
pub(super) fn gauge_from_damage(
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
pub(super) fn gauge_full(stage: &World, s: &State, me: PlayerIdx) -> bool {
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
fn spirit_restart(stage: &World, s: &mut State, me: PlayerIdx) {
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

/// One tick of the giant trigger: it ends on the tick it reaches 0.
fn trigger_tick(s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    let c = &mut s.players[me].combat;
    if c.giant_trigger == 0 {
        return;
    }
    c.giant_trigger -= 1;
    if c.giant_trigger == 0 {
        trigger_ended(events, me);
    }
}

/// A down ends the giant trigger at once.
fn trigger_end(s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    let c = &mut s.players[me].combat;
    if c.giant_trigger > 0 {
        c.giant_trigger = 0;
        trigger_ended(events, me);
    }
}

/// Whether `me`'s giant trigger runs and names super armour: the one more condition of
/// `super_armour`.
fn trigger_armoured(stage: &World, s: &State, me: PlayerIdx) -> bool {
    stage.has_trigger
        && s.players[me].combat.giant_trigger > 0
        && stage.trigger_of(s, me).is_some_and(|g| g.super_armour)
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

/// A hit took `lost` hit points from `me` (after the absorb and the defence): `pct` percent of them
/// stay as rally, times the giant trigger's factor while it runs, up to the hit points `me` is
/// missing, and the rally's ticks start again (ADR-SIM-9). Nothing for a hit that took none or a
/// player at 0 hit points.
pub(super) fn rally_from_hit(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    lost: u32,
    events: &mut Vec<DomainEvent>,
) {
    let Some(rules) = stage.tuning.rally else {
        return;
    };
    if lost == 0 || s.players[me].hp == 0 {
        return;
    }
    let factor = if s.players[me].combat.giant_trigger > 0 {
        stage.trigger_of(s, me).map_or(100, |g| g.rally_pct)
    } else {
        100
    };
    let added = u64::from(lost) * u64::from(rules.pct) * u64::from(factor) / 10_000;
    let p = &mut s.players[me];
    let was = p.combat.rally;
    let missing = p.max_hp.saturating_sub(p.hp);
    let rally = was
        .saturating_add(u32::try_from(added).unwrap_or(u32::MAX))
        .min(missing);
    if rally > 0 {
        p.combat.rally_ticks = rules.ticks;
    }
    if rally != was {
        // A heal can leave more rally than the missing points a later hit allows: it is held.
        p.combat.rally = rally;
        if rally > was {
            rally_gained(events, me, rally - was, rally);
        }
    }
}

/// A combo hit of `me` dealt `dealt` damage to a foe: `regain_pct` percent of it is won back from
/// the rally as hit points, never more than the rally or the hit points missing (ADR-SIM-9).
pub(super) fn rally_win_back(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    dealt: u32,
    events: &mut Vec<DomainEvent>,
) {
    let Some(rules) = stage.tuning.rally else {
        return;
    };
    let p = &mut s.players[me];
    if dealt == 0 || p.combat.rally == 0 || !p.is_up() {
        return;
    }
    let back = u64::from(dealt) * u64::from(rules.regain_pct) / 100;
    let won = u32::try_from(back)
        .unwrap_or(u32::MAX)
        .min(p.combat.rally)
        .min(p.max_hp.saturating_sub(p.hp));
    if won == 0 {
        return;
    }
    p.hp += won;
    p.combat.rally -= won;
    if p.combat.rally == 0 {
        p.combat.rally_ticks = 0;
    }
    rally_won(events, me, won, p.combat.rally);
}

/// One tick of the rally's countdown: when it reaches 0 the rest is lost.
fn rally_tick(s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    if s.players[me].combat.rally_ticks == 0 {
        return;
    }
    s.players[me].combat.rally_ticks -= 1;
    if s.players[me].combat.rally_ticks == 0 {
        rally_lapse(s, me, events);
    }
}

/// The rest of `me`'s rally is lost (its ticks ran out or it went down).
fn rally_lapse(s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    let c = &mut s.players[me].combat;
    c.rally_ticks = 0;
    if c.rally > 0 {
        c.rally = 0;
        rally_lapsed(events, me);
    }
}

/// Keeps `p`'s rally within its most hit points, the invariant the savepoint's decode checks. A
/// player with no rally is untouched.
pub(super) fn rally_clamp(p: &mut PlayerState) {
    if p.combat.rally > p.max_hp {
        p.combat.rally = p.max_hp;
    }
}

/// `me`'s hit left `amount` more rally, `rally` in all. Cold: a run with no rally never calls it.
#[cold]
#[inline(never)]
fn rally_gained(events: &mut Vec<DomainEvent>, me: PlayerIdx, amount: u32, rally: u32) {
    events.push(DomainEvent::RallyGained {
        player: me,
        amount,
        rally,
    });
}

/// `me`'s combo hit won `amount` hit points back, `rally` left. Cold, as `rally_gained`.
#[cold]
#[inline(never)]
fn rally_won(events: &mut Vec<DomainEvent>, me: PlayerIdx, amount: u32, rally: u32) {
    events.push(DomainEvent::RallyWon {
        player: me,
        amount,
        rally,
    });
}

/// The rest of `me`'s rally was lost. Cold, as `rally_gained`.
#[cold]
#[inline(never)]
fn rally_lapsed(events: &mut Vec<DomainEvent>, me: PlayerIdx) {
    events.push(DomainEvent::RallyLapsed { player: me });
}

/// One tick of the countdown: at 0 the level drops by one and, while above 0, counts again.
fn spirit_tick(stage: &World, s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
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
pub(super) fn spirit_level_of(stage: &World, s: &State, me: PlayerIdx) -> u8 {
    if stage.spirit_of(s, me).is_some() {
        s.players[me].combat.spirit_level
    } else {
        0
    }
}

/// The extra bleed stacks a smash hit of `me` adds at its level.
pub(super) fn spirit_bleed_stacks(stage: &World, s: &State, me: PlayerIdx) -> u32 {
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
fn spend_gauge(
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
fn charging(
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
pub(super) fn stand_down(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    events: &mut Vec<DomainEvent>,
) {
    s.players[me].dash = None;
    cancel(stage, s, me, events);
    // A downed player cannot press, and a revive does not resume an old window.
    s.players[me].combat.counter_window = 0;
    s.players[me].combat.counter_bonus_step = 0;
    // A downed player holds no stacks, and a revive does not resume them.
    tempo_lose(stage, s, me, events);
    // Nor a giant trigger, nor a rally.
    trigger_end(s, me, events);
    rally_lapse(s, me, events);
}

/// An evade cancels the running step, breaks a charge and clears the buffer.
fn cancel(stage: &World, s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
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
    combo_reset(stage, s, me, events);
}

/// A hit from a phase that breaks charges ends a charge or a hold in progress without an
/// attack (a hold's loop ends with it), and the combo starts again from the root.
pub(super) fn break_charge(
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
    combo_reset(stage, s, me, events);
}

/// The movement speed, in percent, a charge or a hold leaves the main character: its
/// `move_pct`, 100 when neither is on.
fn held_move_pct(stage: &World, s: &State, me: PlayerIdx) -> i64 {
    if let Some(hold) = current_hold(stage, s, me) {
        return i64::from(hold.move_pct);
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
pub(super) fn super_armour(stage: &World, s: &State, me: PlayerIdx) -> bool {
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
