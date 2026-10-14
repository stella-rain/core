//! The main character (ADR-050): movement from the input's stick, the persistent facing
//! (ADR-055), the Main and Sub weapons that attack on their own, the evade, the charged Main
//! skill and the Sub skill (ADR-050, core#83).

use crate::attack::AttackRef;
use crate::engine::{Aim, DashState, Dir, PlayerState, State};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::input::Input;
use crate::presets;
use crate::stage::Stage;
use crate::trig::SCALE;
use crate::weapon::{Charge, ChargeLevel, Targeting, WeaponAttack};

use super::PLAYER_SPEED;
use super::attacks;
use super::entity::{Who, speed_pct};
use super::levels;
use super::select::select;

/// The stick's full deflection on each axis (ADR-050); −128 counts as −127.
const STICK_MAX: i8 = 127;

/// `⌈√n⌉`.
fn ceil_sqrt(n: u64) -> u64 {
    let r = n.isqrt();
    if r * r < n { r + 1 } else { r }
}

/// The move of a stick at `(x, y)` under a speed cap of `cap` units: the vector clamped to
/// length 127, then scaled so that 127 is the cap. Its length is rounded up, so a diagonal is
/// never faster than a straight line (ADR-050).
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

/// One tick of the main character: cooldowns run down, it turns to its stick, evades on a
/// ready press (which breaks a charge), charges or releases (skill 2), dashes or moves, then its
/// weapons attack on their own and a ready press of skill 3 starts the Sub skill.
pub(super) fn update(stage: &Stage, s: &mut State, input: Input, events: &mut Vec<DomainEvent>) {
    let p = &mut s.player;
    p.invulnerable = p.invulnerable.saturating_sub(1);
    p.evade_cooldown = p.evade_cooldown.saturating_sub(1);
    p.charge_cooldown = p.charge_cooldown.saturating_sub(1);
    p.skill_cooldown = p.skill_cooldown.saturating_sub(1);
    let (x, y) = (input.move_x.max(-STICK_MAX), input.move_y.max(-STICK_MAX));
    if let Some(dir) = stick_direction(x, y) {
        p.facing = dir;
    }
    if input.evade && p.evade_ready() {
        let cooldown = levels::evade_cooldown(s, stage.player.evade.cooldown_ticks);
        start_evade(stage, &mut s.player, cooldown, events);
        break_charge(stage, s, events);
    }
    charge(stage, s, input, events);
    if s.player.dash.is_some() {
        dash(stage, &mut s.player);
    } else {
        move_player(stage, s, x, y);
    }
    shoot(stage, s, events);
    sub_skill(stage, s, input, events);
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
/// centred (ADR-050). Invulnerability and the cooldown count from the press.
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

/// The attack slots of the main character (core#83): each runs one attack at a time (ADR-055),
/// beside the others, so a skill never waits behind an automatic attack.
const MAIN_SLOT: u8 = 0;
const SUB_SLOT: u8 = 1;
const CHARGE_SLOT: u8 = 2;
const SKILL_SLOT: u8 = 3;

/// What a weapon attacks with, and how, right now. This is the one place that says so, so that
/// what the main character acquires during a run (a different attack, homing, a faster rhythm,
/// more damage) changes what this returns and nothing else: the attack runs as any other, with
/// the aim and the damage given here.
struct Shot<'a> {
    attack: &'a AttackRef,
    /// Where `aimed` directions point: along the facing, or at the target.
    aim: Aim,
    /// Hit points a bullet takes before the attack-up status.
    damage: u32,
    /// Ticks from this attack's start to the next one's.
    cooldown: u32,
}

/// Where a weapon's attack aims, as its targeting says: `None` when its selector finds nothing.
fn aim_for(stage: &Stage, s: &State, targeting: &Targeting) -> Option<Aim> {
    Some(match targeting.selector() {
        None => Aim::Fixed(s.player.facing),
        Some(selector) => {
            let target = select(stage, s, Who::Player, &selector)?;
            // Both are set to the direction to the target as the attack starts.
            Aim::Target {
                id: s.id_of(target),
                heading: s.player.facing,
                last: s.player.facing,
            }
        }
    })
}

/// Today the stage's weapon attack, at the stage's attack stat, aimed as its targeting says:
/// `None` when its selector finds nothing to aim at.
fn shot<'a>(stage: &Stage, s: &State, weapon: &'a WeaponAttack) -> Option<Shot<'a>> {
    Some(Shot {
        attack: &weapon.attack,
        aim: aim_for(stage, s, &weapon.targeting)?,
        damage: levels::attack(s, stage.player.atk),
        cooldown: weapon.cooldown_ticks,
    })
}

/// The Main weapon, then the Sub weapon (ADR-050, core#83). The Main weapon's automatic attack
/// pauses while the main character charges; the Sub weapon's goes on.
fn shoot(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    // The validator lets only defined presets through.
    if let Some(main) = presets::main_weapon(&stage.player.main) {
        if s.player.charge.is_none() {
            attack_with(stage, s, &main.auto, MAIN_SLOT, events);
        }
    }
    if let Some(sub) = presets::sub_weapon(&stage.player.sub) {
        attack_with(stage, s, &sub.auto, SUB_SLOT, events);
    }
}

/// The cooldown counter of an automatic attack's slot.
fn auto_cooldown(p: &mut PlayerState, slot: u8) -> &mut u32 {
    if slot == MAIN_SLOT {
        &mut p.main_cooldown
    } else {
        &mut p.sub_cooldown
    }
}

/// A weapon attacks on its own once its cooldown, counted from its last attack's start, is
/// over and that attack has ended (ADR-055: one attack at a time). With nothing to aim at it
/// does not attack and spends no cooldown, so it attacks on the first tick it finds a target.
/// The attack is a task like any other: once started it runs to its end, follows the main
/// character and counts against the attack and bullet budgets.
fn attack_with(
    stage: &Stage,
    s: &mut State,
    weapon: &WeaponAttack,
    slot: u8,
    events: &mut Vec<DomainEvent>,
) {
    let cooldown = auto_cooldown(&mut s.player, slot);
    *cooldown = cooldown.saturating_sub(1);
    if *cooldown > 0 || attacks::busy(s, s.id_of(Who::Player), slot) {
        return;
    }
    let Some(shot) = shot(stage, s, weapon) else {
        return;
    };
    let started = attacks::start_with(
        s,
        Who::Player,
        slot,
        shot.aim,
        shot.attack,
        shot.damage,
        0,
        0,
        events,
    );
    if started {
        *auto_cooldown(&mut s.player, slot) = levels::auto_cooldown(s, shot.cooldown);
    }
}

/// Skill 3 (ADR-050): a ready press starts the Sub weapon's skill on that tick, at what its
/// targeting picks. With nothing to aim at nothing happens and no cooldown is spent.
fn sub_skill(stage: &Stage, s: &mut State, input: Input, events: &mut Vec<DomainEvent>) {
    if !input.sub
        || s.player.skill_cooldown > 0
        || attacks::busy(s, s.id_of(Who::Player), SKILL_SLOT)
    {
        return;
    }
    let Some(sub) = presets::sub_weapon(&stage.player.sub) else {
        return;
    };
    let Some(shot) = shot(stage, s, &sub.skill) else {
        return;
    };
    let started = attacks::start_with(
        s,
        Who::Player,
        SKILL_SLOT,
        shot.aim,
        shot.attack,
        shot.damage,
        0,
        0,
        events,
    );
    if started {
        s.player.skill_cooldown = shot.cooldown;
    }
}

/// The level `held` ticks of charge reach: the last whose `at_ticks` is at most `held`, with
/// its index. `None` only for a charge without levels, which the validator refuses.
fn level_at(charge: &Charge, held: u32) -> Option<(usize, &ChargeLevel)> {
    charge
        .levels
        .iter()
        .enumerate()
        .take_while(|(_, l)| l.at_ticks <= held)
        .last()
}

/// Whether `held` ticks of charge are inside the perfect window of the level they reach.
pub(crate) fn in_perfect_window(charge: &Charge, held: u32) -> bool {
    level_at(charge, held).is_some_and(|(_, l)| held < l.at_ticks.saturating_add(l.perfect_ticks))
}

/// The charged Main skill's view for the snapshot: its 1-based level and whether this tick
/// is inside a perfect window, while charging.
pub(crate) fn charge_view(stage: &Stage, held: u32) -> Option<(u8, bool)> {
    let charge = &presets::main_weapon(&stage.player.main)?.charge;
    let (i, _) = level_at(charge, held)?;
    Some((
        u8::try_from(i + 1).unwrap_or(u8::MAX),
        in_perfect_window(charge, held),
    ))
}

/// Whether a press of skill 2 would start a charge.
pub(crate) fn charge_ready(s: &State) -> bool {
    s.player.charge.is_none()
        && s.player.charge_cooldown == 0
        && !attacks::busy(s, s.id_of(Who::Player), CHARGE_SLOT)
}

/// Whether a press of skill 3 would start the Sub skill (if it finds a target).
pub(crate) fn skill_ready(s: &State) -> bool {
    s.player.skill_cooldown == 0 && !attacks::busy(s, s.id_of(Who::Player), SKILL_SLOT)
}

/// Skill 2 (ADR-050). A press while ready starts the charge: `held` is 0 on that tick and
/// rises by 1 every tick it stays held. Letting go releases it, judged on the last held tick,
/// the one whose snapshot the player saw; reaching `max_ticks` releases it without the bonus.
fn charge(stage: &Stage, s: &mut State, input: Input, events: &mut Vec<DomainEvent>) {
    let pressed = input.charge && !s.player.charge_held;
    s.player.charge_held = input.charge;
    let Some(main) = presets::main_weapon(&stage.player.main) else {
        return;
    };
    let c = &main.charge;
    match s.player.charge {
        Some(held) if !input.charge => release(stage, s, c, held, true, events),
        Some(held) => {
            let held = held.saturating_add(1);
            if held >= c.max_ticks {
                release(stage, s, c, held, false, events);
            } else {
                s.player.charge = Some(held);
            }
        }
        None if pressed && charge_ready(s) => s.player.charge = Some(0),
        None => {}
    }
}

/// The release: the level `held` reaches attacks along the facing, with the perfect bonus
/// when `judged` and inside its window. The cooldown counts from here.
fn release(
    stage: &Stage,
    s: &mut State,
    c: &Charge,
    held: u32,
    judged: bool,
    events: &mut Vec<DomainEvent>,
) {
    s.player.charge = None;
    s.player.charge_cooldown = c.cooldown_ticks;
    let Some((i, level)) = level_at(c, held) else {
        return;
    };
    let perfect = judged && in_perfect_window(c, held);
    let (attack, bonus) = if perfect {
        (
            level.perfect_attack.as_ref().unwrap_or(&level.attack),
            u32::from(level.perfect_pct),
        )
    } else {
        (&level.attack, 0)
    };
    attacks::start_with(
        s,
        Who::Player,
        CHARGE_SLOT,
        Aim::Fixed(s.player.facing),
        attack,
        levels::attack(s, stage.player.atk),
        0,
        bonus,
        events,
    );
    events.push(DomainEvent::ChargeReleased {
        level: u8::try_from(i + 1).unwrap_or(u8::MAX),
        perfect,
    });
}

/// An evade, or a hit from an attack that breaks charges, ends a charge in progress without
/// an attack; the charged skill then waits half its cooldown, rounded up.
pub(super) fn break_charge(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    if s.player.charge.take().is_none() {
        return;
    }
    let cooldown = presets::main_weapon(&stage.player.main).map_or(0, |m| m.charge.cooldown_ticks);
    s.player.charge_cooldown = cooldown - cooldown / 2;
    events.push(DomainEvent::ChargeBroken);
}

/// The movement speed, in percent, the charge leaves the main character: its `move_pct`, 100
/// when not charging.
fn charge_move_pct(stage: &Stage, s: &State) -> i64 {
    match (s.player.charge, presets::main_weapon(&stage.player.main)) {
        (Some(_), Some(main)) => i64::from(main.charge.move_pct),
        _ => 100,
    }
}
