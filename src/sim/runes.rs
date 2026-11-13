//! Runes: the Main weapon's set, each rune at most once, the random rune's draw, and
//! the on-hit runes' fire.
//!
//! When a combo step's hit lands (by hit box or orb, as the expertise gauge counts it), each
//! on-hit rune held whose cooldown is 0 fires its attack around the point hit, in the order
//! held, and its cooldown is set to the runes file's; each cooldown counts down by 1 at the
//! start of every tick. The attack runs in the rune's own slot, as the main character's but no
//! combo step's: its hits give no expertise and fire no rune, so a rune never feeds itself or
//! another (point 3). It starts in the collision step, after that tick's hit boxes have
//! landed, so its first phase counts that tick and its hit boxes land from the next. Nothing
//! here draws from the generator.
//!
//! Damage-over-time runes (`apply_dots`): a combo step's hit that leaves its target standing
//! puts each one's status on it, bleed's `bleed`, poison's `poison`, burn's `burn`, or a stack
//! more of it up to the runes file's
//! maximum, starting its duration again (`State::add_dot`). Its damage is the main
//! character's without a combo step (point 3): the runes file's damage a stack, raised by the
//! main character's atk-up and its type bonus for the rune's type as they are when it is
//! applied, as an attack's are when it starts, and by the target's multipliers at
//! each tick (`entity::tick_dots`). The step's own bonuses (a perfect release's) are not its.
//! A smash's hit that leaves its target standing adds one stack of bleed the same way, with the
//! same numbers, whether the rune is held or not (`apply_smash_bleed`): each
//! source adds its stack to the one bleed.
//!
//! Boomerang is thrown (`fly`): its attack starts on the main character, and its hit boxes fly
//! out over its first phase to the point hit and back over its second toward where the main
//! character is now. Each tick, after the main character moves, the centre goes a straight
//! step, the distance left over the ticks left in the phase, so that it is on the point hit on
//! the first phase's last tick and on the main character on the second's; no steering, no
//! orb. Each phase is active and keeps its own hit list, so an enemy on the way is hit once
//! out and once back. The path is a placeholder.
//!
//! Homing missiles launch `missiles` hit boxes from the main character, missile k in slot
//! `MISSILE_SLOT + k`. Missile k's target is the k-th of aim assist's order within the
//! rune's range of the main character, starting over when there are fewer, or the enemy the
//! step hit when there are none: nothing drawn from the generator. Each tick, after everyone
//! has moved (`steer`), its goal is where its target is now, or was last once it is gone, and
//! its centre goes the distance left over the flight's ticks left, as a thrown rune's does, so
//! that it is on its target on the flight's last tick. It hits the first enemy it covers, once,
//! and is gone (`hit_boxes`). The flight is a placeholder.
//!
//! Bounds: one attack at a time per rune, in its slot, the file holding each
//! attack to its cooldown; a rune whose slot is still busy does not fire and keeps its cooldown
//! at 0. A cooldown is held to `MAX_RUNE_COOLDOWN_TICKS`. No rune fires an orb, a thrown one
//! included: the orbs alive from runes stay at 0. Homing missiles launch at most
//! `MAX_HOMING_MISSILES`, all in their slots and the file holding each flight to the cooldown,
//! so no more fly at once. A damage-over-time status is one per kind on an entity, its stacks,
//! interval, duration and damage held to `MAX_DOT_STACKS`, `MAX_DOT_INTERVAL_TICKS`,
//! `MAX_DOT_DURATION_TICKS` and `MAX_DAMAGE`; so an entity carries at most one status of each
//! of `StatusKind`'s kinds, and the enemies at most `World::enemy_cap` of each. A reap smash
//! (`reap_smash`) cashes the bleed's stacks at its count: the status goes and its due ticks land at once.

use super::World;
use crate::sim::state::PlayerIdx;

use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::rewards::{MAX_TYPE_BONUS_PCT, rune_haste_ticks};
use crate::rune::{Rune, RuneKind};
use crate::runes::{
    MAX_DOT_DURATION_TICKS, MAX_DOT_INTERVAL_TICKS, MAX_DOT_STACKS, MAX_HOMING_MISSILES,
    MAX_RUNE_COOLDOWN_TICKS,
};
use crate::sim::state::{DotState, HeldRune, State, StatusKind};
use crate::validate::limits::{MAX_ASSIST_RANGE, MAX_DAMAGE};

use super::entity::{Who, scale_damage, status_pct};
use super::{AttackIndex, Shot, attacks, select};

/// The first on-hit rune's slot: after the combo (0), the stage skill (1) and the orbits' orbs
/// (2 to 33); each rune's is this plus its place in `Rune::ALL`.
pub(super) const FIRST_SLOT: u8 = 34;
/// Homing missiles' first missile's slot; missile k's is this plus k, past every rune's.
pub(super) const MISSILE_SLOT: u8 = 64;

/// The on-hit rune whose attack run `r` is, by its slot (`FIRST_SLOT` plus the rune's place, or
/// a missile's); `None` for every other run.
pub(super) fn of_run(r: &crate::sim::state::AttackRun) -> Option<Rune> {
    // A player's number names a slot, present or not (ADR-SIM-7).
    crate::id::player_of(r.owner)?;
    match r.slot {
        s if s >= MISSILE_SLOT => Some(Rune::HomingMissiles),
        s => Rune::ALL
            .get(usize::from(s.checked_sub(FIRST_SLOT)?))
            .copied()
            .filter(|r| r.kind() == RuneKind::OnHit),
    }
}

/// Every on-hit rune's cooldown counts down by a tick.
pub(super) fn tick(s: &mut State, me: PlayerIdx) {
    for r in &mut s.players[me].runes {
        r.cooldown_left = r.cooldown_left.saturating_sub(1);
    }
}

/// A combo step hit `hit` at `at`: each on-hit rune held and ready fires its attack there, at
/// its base damage, and waits out its cooldown. With a `proc` factor, a rune that would fire
/// takes one draw from the attack stream and fires only if it falls under `proc` percent; one
/// that does not stays ready and the next is tried. No factor: no draw (ADR-SIM-9 point 3).
pub(super) fn on_hit(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    hit: EntityId,
    at: Point,
    events: &mut Vec<DomainEvent>,
    proc: Option<u16>,
) {
    for i in 0..s.players[me].runes.len() {
        let HeldRune {
            rune,
            cooldown_left,
        } = s.players[me].runes[i];
        if cooldown_left > 0 {
            continue;
        }
        let Some(entry) = s.runes.on_hit(rune) else {
            continue;
        };
        // The validator holds the cooldown to the cap; the clamp keeps an unvalidated file
        // there. Rune Haste shortens it as it is set.
        let cooldown = rune_haste_ticks(
            entry.cooldown_ticks.min(MAX_RUNE_COOLDOWN_TICKS),
            s.players[me].rune_haste_pct.min(s.max_rune_haste_pct),
        );
        let damage = entry.damage;
        let Some(attack) = stage.rune_attacks[usize::from(rune.index())] else {
            continue;
        };
        if let Some(p) = proc {
            if s.rng.attack.below(100) >= u32::from(p) {
                continue;
            }
        }
        let slot = FIRST_SLOT.saturating_add(rune.index());
        let facing = s.players[me].facing;
        if rune == Rune::HomingMissiles {
            let (missiles, range) = (entry.missiles.unwrap_or(1), entry.range.unwrap_or(Fx(0)));
            if launch(
                stage, s, me, hit, at, missiles, range, attack, damage, events,
            ) {
                s.players[me].runes[i].cooldown_left = cooldown;
            }
            continue;
        }
        let thrown = rune == Rune::Boomerang;
        let from = if thrown { s.players[me].at } else { at };
        if attacks::start_rune(stage, s, me, slot, from, facing, attack, damage, events) {
            if thrown {
                // It was pushed last; its first step counts the tick it is thrown on.
                if let Some(r) = s.runs.last_mut() {
                    r.throw = Some(at);
                }
                if let Some(r) = s.runs.len().checked_sub(1) {
                    fly_one(stage, s, me, r);
                }
            }
            s.players[me].runes[i].cooldown_left = cooldown;
        }
    }
}

/// A combo step hit `target`, which still stands: each damage-over-time rune held puts its
/// status on it, in the order held.
pub(super) fn apply_dots(s: &mut State, me: PlayerIdx, target: Who, events: &mut Vec<DomainEvent>) {
    for i in 0..s.players[me].runes.len() {
        let rune = s.players[me].runes[i].rune;
        apply_dot(s, me, target, rune, events);
    }
}

/// A smash's hit that leaves `target` standing puts the bleed status on it, a stack more, with
/// the bleed rune's numbers. It adds its own stack when the rune is held too:
/// each source adds its stacks to the one bleed.
pub(super) fn apply_smash_bleed(
    s: &mut State,
    me: PlayerIdx,
    target: Who,
    events: &mut Vec<DomainEvent>,
) {
    apply_dot(s, me, target, Rune::Bleed, events);
}

/// A reap smash's hit that leaves `target` standing (ADR-SIM-9): on a target holding the bleed
/// at `stacks` stacks or more it cashes them, the status goes and the target takes at once the
/// damage of the bleed ticks still due, each as `tick_dots_of` scales it, a boss and its parts
/// `boss_pct` percent of it (an invulnerable boss absorbs it, as it does a tick); else it adds a
/// bleed stack as `apply_smash_bleed` does. The burst is a bleed hit: no expertise, no on-hit rune.
#[allow(clippy::too_many_arguments)]
pub(super) fn reap_smash(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    target: Who,
    stacks: u32,
    boss_pct: u32,
    events: &mut Vec<DomainEvent>,
) {
    let held = s
        .statuses(target)
        .iter()
        .position(|st| st.kind == StatusKind::Bleed && st.dot.is_some());
    let cash = held.filter(|&i| u32::from(s.statuses(target)[i].stacks) >= stacks.max(1));
    let Some(i) = cash else {
        apply_smash_bleed(s, me, target, events);
        return;
    };
    let st = s.statuses(target)[i];
    let Some(dot) = st.dot else { return };
    s.statuses_mut(target).remove(i);
    let boss = matches!(target, Who::Boss | Who::BossPart(_));
    if boss && s.boss.as_ref().is_some_and(|b| b.invulnerable > 0) {
        return;
    }
    let interval = dot.interval.max(1);
    let due = if st.remaining < dot.next_in {
        0
    } else {
        (st.remaining - dot.next_in) / interval + 1
    };
    let tick = scale_damage(
        dot.damage.saturating_mul(u32::from(st.stacks)),
        dot.atk_up_pct,
        dot.type_bonus_pct,
        0,
        0,
        0,
        stage.tuning.defence_k,
    );
    let mut burst = u64::from(tick) * u64::from(due);
    if boss {
        burst = burst * u64::from(boss_pct.min(100)) / 100;
    }
    let damage = u32::try_from(burst).unwrap_or(u32::MAX);
    if damage == 0 {
        return;
    }
    let shot = Shot {
        at: s.at(target),
        owner: st.source,
        friendly: true,
        damage,
        damage_type: dot.damage_type,
        breaks_charge: false,
        smash: None,
        combo: false,
        rune: Rune::of_dot(st.kind),
        chain: false,
    };
    let me = super::hitter(s, st.source);
    super::damage(stage, s, me, shot, target, events);
}

/// Puts `rune`'s status on `target`, when `rune` is damage over time and the run's content has
/// its entry; a run without one applies none (fail closed).
fn apply_dot(s: &mut State, me: PlayerIdx, target: Who, rune: Rune, events: &mut Vec<DomainEvent>) {
    let kind = match rune {
        Rune::Bleed => StatusKind::Bleed,
        Rune::Poison => StatusKind::Poison,
        Rune::Burn => StatusKind::Burn,
        Rune::Explosion | Rune::Thunder | Rune::Boomerang | Rune::HomingMissiles => return,
    };
    let Some(entry) = s.runes.damage_over_time(rune) else {
        return;
    };
    // The validator holds every number to its cap; the clamps keep an unvalidated file there.
    let interval = rune_haste_ticks(
        entry.interval_ticks.clamp(1, MAX_DOT_INTERVAL_TICKS),
        s.players[me].rune_haste_pct.min(s.max_rune_haste_pct),
    );
    let duration = entry.duration_ticks.clamp(1, MAX_DOT_DURATION_TICKS);
    let max_stacks = u16::try_from(entry.max_stacks.clamp(1, MAX_DOT_STACKS)).unwrap_or(1);
    let limit = s.match_ups.max_type_bonus_pct().min(MAX_TYPE_BONUS_PCT);
    let dot = DotState {
        damage: entry.damage.min(MAX_DAMAGE),
        damage_type: entry.damage_type,
        atk_up_pct: status_pct(&s.players[me].statuses, StatusKind::AtkUp),
        type_bonus_pct: entry
            .damage_type
            .map_or(0, |ty| s.players[me].type_bonus_pct[ty.index()].min(limit)),
        interval,
        next_in: interval,
    };
    let source = s.id_of(Who::Player(me));
    s.add_dot(target, kind, source, dot, duration, max_stacks, events);
}

/// Launches homing missiles' `missiles` from the main character, each toward its target, at
/// `damage`. False, launching none, if a missile's slot is busy.
#[allow(clippy::too_many_arguments)]
fn launch(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    hit: EntityId,
    at: Point,
    missiles: u32,
    range: Fx,
    attack: AttackIndex,
    damage: u32,
    events: &mut Vec<DomainEvent>,
) -> bool {
    // The validator holds both to their caps; the clamps keep an unvalidated file there.
    let n = missiles.clamp(1, MAX_HOMING_MISSILES) as u8;
    let range = range.0.clamp(0, MAX_ASSIST_RANGE);
    let player = s.id_of(Who::Player(me));
    if (0..n).any(|k| attacks::busy(s, player, MISSILE_SLOT.saturating_add(k))) {
        return false;
    }
    // The buffers wait in the tick's scratch; `start_rune` below needs `s` whole.
    let mut kept = std::mem::take(&mut s.scratch.assist_kept);
    let mut targets = std::mem::take(&mut s.scratch.assist_targets);
    select::assist_targets(stage, s, me, range, usize::from(n), &mut kept, &mut targets);
    let (from, facing) = (s.players[me].at, s.players[me].facing);
    for k in 0..n {
        let target = match targets.len() {
            0 => hit,
            len => targets[usize::from(k) % len],
        };
        let goal = s.find(target).map_or(at, |w| s.at(w));
        let slot = MISSILE_SLOT.saturating_add(k);
        if attacks::start_rune(stage, s, me, slot, from, facing, attack, damage, events) {
            // It was pushed last; its first step counts the tick it launches on.
            if let Some(r) = s.runs.last_mut() {
                r.throw = Some(goal);
                r.homing = Some(target);
            }
            if let Some(r) = s.runs.len().checked_sub(1) {
                fly_one(stage, s, me, r);
            }
        }
    }
    s.scratch.assist_kept = kept;
    s.scratch.assist_targets = targets;
    true
}

/// Every thrown rune's hit boxes go a step: out to its turn point in its first phase, back to
/// the main character in the others. Homing missiles steer later (`steer`).
pub(super) fn fly(stage: &World, s: &mut State) {
    for i in 0..s.runs.len() {
        if s.runs[i].throw.is_some() && s.runs[i].homing.is_none() {
            // A run whose owner is no present player has nothing to fly to.
            if let Some(me) = s.players.of_id(s.runs[i].owner) {
                fly_one(stage, s, me, i);
            }
        }
    }
}

/// Every homing missile goes a step toward where its target is now, or was last.
pub(super) fn steer(stage: &World, s: &mut State) {
    for i in 0..s.runs.len() {
        let Some(target) = s.runs[i].homing else {
            continue;
        };
        if let Some(w) = s.find(target).filter(|w| s.is_alive(*w)) {
            s.runs[i].throw = Some(s.at(w));
        }
        if let Some(me) = s.players.of_id(s.runs[i].owner) {
            fly_one(stage, s, me, i);
        }
    }
}

/// Run `i`'s centre goes the distance left to its goal over the phase's ticks left, this one
/// counted, so that it is there on the phase's last tick.
fn fly_one(stage: &World, s: &mut State, me: PlayerIdx, i: usize) {
    let to_player = s.players[me].at;
    let r = &mut s.runs[i];
    let (Some(at), Some(turn)) = (r.at, r.throw) else {
        return;
    };
    let goal = if r.phase == 0 { turn } else { to_player };
    let ticks = if (r.phase as usize) < attacks::phase_count(stage, r) {
        attacks::phase(stage, r, r.phase as usize).ticks
    } else {
        1
    };
    let left = i64::from(ticks.saturating_sub(r.tick).max(1));
    let step = |from: Fx, to: Fx| {
        let d = (i64::from(to.0) - i64::from(from.0)) / left;
        Fx(i32::try_from(i64::from(from.0) + d).unwrap_or(to.0))
    };
    r.at = Some(Point {
        x: step(at.x, goal.x),
        y: step(at.y, goal.y),
    });
}

/// Whether `rune` is in the run's rune pool, a mask by `Rune::index`.
pub(crate) fn in_pool(pool: u8, rune: Rune) -> bool {
    pool >> rune.index() & 1 == 1
}

/// The mask of the runes `ids` name; the whole list when it is empty.
pub(crate) fn pool_mask(ids: &[crate::id::ContentId]) -> u8 {
    if ids.is_empty() {
        return FULL_POOL;
    }
    ids.iter()
        .filter_map(|id| Rune::from_id(id.as_str()))
        .fold(0, |mask, r| mask | 1 << r.index())
}

/// Every rune in the pool.
pub(crate) const FULL_POOL: u8 = (1 << Rune::ALL.len()) - 1;
