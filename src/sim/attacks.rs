//! Attacks: an attacker runs one phased attack at a time per slot (`AttackRun`). A
//! run starts with its first phase on the tick it starts; at the start of each later tick it
//! goes on to its next phase once the current one has lasted its ticks, and ends after the last.
//! Leaving its attacker (death, or a boss leaving the phase that started it) ends it too. A
//! wind-up announces itself with a `Telegraph` event. Each phase spawns its nodes on its first
//! tick as a task of their own, which runs to its end whatever the run does next.
//!
//! The nodes (ADR-SIM-4 tier 3) are a tree carried out over time. A task keeps a stack
//! of frames, one for each list of nodes it is inside, so that it can stop at a `wait` or between
//! the rounds of a `repeat` and go on in a later tick. Presets are expanded to inline nodes
//! before a task starts (see `presets`), so a preset and its inline copy are the same task.
//!
//! What the nodes mean:
//! - `zone` places an area that does not move (`zones`): at the attacker's spot or where the
//!   target stood when the attack began, moved by its `offset` along its direction. A direction
//!   is a unit vector: `aimed` points at the target (along the facing for a combo step's) and
//!   turns by `offset`, `absolute` is an angle from straight down, `relative` turns the direction
//!   the attack's parent was going (straight down for an attack a rule started; the facing for a
//!   combo step's; the orb's own direction for its burst), and `sequential` turns the last
//!   direction by `step`. The first three also turn by every `rotate`, `ring` and `spread` round
//!   them. Positive angles turn from straight down toward the right.
//! - `orb` fires a slow projectile that lives `ticks` ticks (core Z2), the one thing that flies.
//!   When they are up its `burst` is carried out from where it ended, as an attack of its own;
//!   the main character's hit boxes break a `breakable` one with no burst. At most
//!   `MAX_ORBS_PER_OWNER` per attacker and `MAX_ORBS_ALIVE` in all; one over a cap is dropped, in
//!   node order.
//! - `repeat` runs its body `count` times; the next round starts `interval_ticks` ticks after
//!   the body of the last one ends. `ring` runs its body `count` times at once, round after
//!   round turned a full turn / `count` further; `spread` the same, fanned evenly across
//!   `angle` and centred on the direction. A `wait` in their bodies delays the next round.
//! - `rotate` turns everything in its body by `angle`; `wait` pauses the attack.
//!
//! Expressions are evaluated left to right when the node they belong to is carried out, with
//! `rand` drawing from the run's SplitMix64 in that order. Arithmetic saturates, and dividing by
//! zero gives 0. Counts, ticks and speeds that an expression computes are clamped.
//!
//! Dynamic budgets (ADR-SVC-1): at most `MAX_ATTACK_TASKS` attacks at once, and every tick at most
//! `MAX_EMITTER_STEPS_PER_TICK` node steps across all of them, after which the rest wait for
//! the next tick, in order. Orbs and zones are capped as everywhere else.

use super::{hit_boxes, player, zones};
use std::ops::Deref;

use crate::attack::{AttackPhase, Dash, HitBox, PhaseKind};
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::rewards::MAX_RANGE_PCT;
use crate::sim::state::{AttackRun, State};

use super::agents::condition_holds;
use super::entity::Who;
use super::select::select;
use super::world::RangedPool;
use super::{MAX_RANGED_ATTACKERS, VIEW_HALF_HEIGHT, VIEW_HALF_WIDTH, World};

/// Whether `owner` is running an attack in `slot`.
pub(super) fn busy(s: &State, owner: EntityId, slot: u8) -> bool {
    s.runs.iter().any(|r| r.owner == owner && r.slot == slot)
}

/// The tokens of `pool` (RA1, DS83).
fn pool_tokens(world: &World, pool: RangedPool) -> u32 {
    match pool {
        RangedPool::Zone => world.tuning.ranged_zone_tokens,
        RangedPool::Line => world.tuning.ranged_line_tokens,
    }
}

/// The tokens of `pool` held now (core Z2): its `ranged` runs in their wind-up or active phase.
fn ranged_held(world: &World, s: &State, pool: RangedPool) -> u32 {
    let held = s
        .runs
        .iter()
        .filter(|r| world.attacks[usize::from(r.attack.0)].pool == pool && holds_token(world, r));
    u32::try_from(held.count()).unwrap_or(u32::MAX)
}

/// The tokens of `pool` cooling now.
fn ranged_cooling(s: &State, pool: RangedPool) -> u32 {
    let cooling = s.ranged_cooling.iter().filter(|(_, p)| *p == pool);
    u32::try_from(cooling.count()).unwrap_or(u32::MAX)
}

/// Whether `pool` has a token to give: the ones held and cooling leave one of its tokens.
fn ranged_room(world: &World, s: &State, pool: RangedPool) -> bool {
    ranged_held(world, s, pool).saturating_add(ranged_cooling(s, pool)) < pool_tokens(world, pool)
}

/// Whether the ranged run `r` holds its token now: in a wind-up or active phase.
fn holds_token(world: &World, r: &AttackRun) -> bool {
    r.ranged && phase(world, r, r.phase as usize).kind != PhaseKind::Recovery
}

/// Starts the cooling of a token of `pool` that was freed on the current tick.
fn cool_token(world: &World, s: &mut State, pool: RangedPool) {
    let cooldown = world.tuning.ranged_token_cooldown_ticks;
    let entry = (s.tick.saturating_add(cooldown), pool);
    let at = s.ranged_cooling.partition_point(|e| *e <= entry);
    s.ranged_cooling.insert(at, entry);
}

/// Whether the regular enemy `id` may start a ranged attack of `pool` now (RA1): it holds this
/// tick's grant for the pool, and the pool still has room.
pub(super) fn ranged_may_start(world: &World, s: &State, id: EntityId, pool: RangedPool) -> bool {
    s.scratch.ranged_grants[pool.index()].contains(&id) && ranged_room(world, s, pool)
}

/// Whether `e` stands inside the view rectangle around `centre` (the camera's, from
/// `view_centre`), edges included.
pub(super) fn in_view(e: Point, area: super::Area) -> bool {
    let centre = area.centre;
    let (dx, dy) = (
        i64::from(e.x.0) - i64::from(centre.x.0),
        i64::from(e.y.0) - i64::from(centre.y.0),
    );
    dx.abs() <= i64::from(VIEW_HALF_WIDTH.0) + area.half_x
        && dy.abs() <= i64::from(VIEW_HALF_HEIGHT.0) + area.half_y
}

/// Gives each pool's free tokens to the nearest ready regular enemies in view (RA1, DS82): at
/// the start of the act step, before any rule fires. The candidates are alive, in view, not
/// stunned and have a ranged rule of the pool that is ready (its cooldown ends by this tick,
/// charges are left, its condition and selector hold now) and no attack of theirs running (RA-STUN);
/// the nearest come first, by squared distance, ties by entity ID. The grants are scratch,
/// worked out from the state each tick. No random number is drawn.
pub(super) fn grant_ranged(world: &World, s: &mut State, up: &super::select::Up) {
    for g in &mut s.scratch.ranged_grants {
        g.clear();
    }
    let now = s.tick;
    s.ranged_cooling.retain(|(until, _)| *until > now);
    let mut free = [0u32; 2];
    for pool in RangedPool::ALL {
        let taken = ranged_held(world, s, pool).saturating_add(ranged_cooling(s, pool));
        free[pool.index()] = pool_tokens(world, pool).saturating_sub(taken);
    }
    if free == [0, 0] {
        return;
    }
    // The best few of each pool so far, nearest first; enemies come in ID order, so a later one
    // at the same distance stays behind.
    const MOST: usize = MAX_RANGED_ATTACKERS as usize;
    let mut best = [[(i64::MAX, EntityId(0)); MOST]; 2];
    let mut len = [0usize; 2];
    let area = super::Area::of_view(&world.map(), &s.players);
    for (i, e) in s.enemies.iter().enumerate() {
        let def = world.defs.get(e.def);
        // A stunned enemy fires nothing (`act_one`), so it could not use a grant.
        if def.ranged_pools.is_empty()
            || e.hp == 0
            || !in_view(e.at, area)
            || super::stun::stunned(&e.statuses)
        {
            continue;
        }
        let d = i64::try_from(up.least_d2(e.at)).unwrap_or(i64::MAX);
        for (rule, pool) in def.ranged_pools.iter().enumerate() {
            let Some(pool) = *pool else { continue };
            let p = pool.index();
            let st = e.rules[rule];
            // A cooldown of 1 ends in this tick's `act_one`, before the rule is tried.
            if st.cooldown_left > 1 || st.charges_left == 0 {
                continue;
            }
            if def.agent.rules[rule].once && st.fired {
                continue;
            }
            let room = (free[p] as usize).min(MOST);
            if room == 0 || len[p] == room && d >= best[p][room - 1].0 {
                continue;
            }
            if best[p][..len[p]].iter().any(|(_, id)| *id == e.id) {
                continue;
            }
            // Not ready while its last attack runs: it could not use the grant.
            if busy(s, e.id, 0) {
                continue;
            }
            // Nor could one whose rule's condition or selector fails: `fire_first_rule` skips it.
            let rule = &def.agent.rules[rule];
            if !condition_holds(s, Who::Enemy(i), &rule.when)
                || select(s, Who::Enemy(i), &rule.target).is_none()
            {
                continue;
            }
            let at = best[p][..len[p]].partition_point(|(bd, _)| *bd <= d);
            if len[p] < room {
                len[p] += 1;
            }
            best[p][at..len[p]].rotate_right(1);
            best[p][at] = (d, e.id);
        }
    }
    for pool in RangedPool::ALL {
        let p = pool.index();
        s.scratch.ranged_grants[p].extend(best[p][..len[p]].iter().map(|(_, id)| *id));
    }
}

/// Marks the run that just started as a `ranged` one: it is the last in `runs`.
pub(super) fn mark_ranged(s: &mut State) {
    if let Some(r) = s.runs.last_mut() {
        r.ranged = true;
    }
}

/// The movement speed, in percent, that `owner`'s running attacks leave it: the lowest
/// `move_pct` of their current phases, 100 with none.
pub(super) fn move_pct(world: &World, s: &State, owner: EntityId) -> i64 {
    let pct = s.move_index.get(owner);
    debug_assert_eq!(
        pct,
        scan_move_pct(world, s, owner),
        "the move index is stale"
    );
    pct
}

/// What `move_pct` was before `State::move_index`: every run scanned. The index is checked
/// against it in debug builds.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn scan_move_pct(world: &World, s: &State, owner: EntityId) -> i64 {
    s.runs
        .iter()
        .filter(|r| r.owner == owner)
        .map(|r| i64::from(phase(world, r, r.phase as usize).move_pct))
        .min()
        .unwrap_or(100)
}

/// A phase of a run as it is carried out: the compiled phase with attack speed and range
/// applied, which is read when needed instead of kept. It derefs to the compiled
/// phase for everything they leave alone.
pub(crate) struct PhaseView<'a> {
    base: &'a AttackPhase,
    /// The phase's length in ticks, shortened by attack speed.
    pub ticks: u32,
    widened: Option<HitBox>,
}

impl PhaseView<'_> {
    /// The phase's hit box, widened by the range.
    pub fn hit_box(&self) -> Option<&HitBox> {
        self.widened.as_ref().or(self.base.hit_box.as_ref())
    }
}

impl Deref for PhaseView<'_> {
    type Target = AttackPhase;

    fn deref(&self) -> &AttackPhase {
        self.base
    }
}

/// The compiled phases of run `r`'s attack, before attack speed and range.
fn compiled<'a>(world: &'a World, r: &AttackRun) -> &'a [AttackPhase] {
    &world.attacks[usize::from(r.attack.0)].phases
}

/// The dash of run `r`'s current phase, if it is an active phase with one. Read from the compiled
/// phase, so that a tick that has no dash to move pays for a lookup, not for a `PhaseView`.
pub(super) fn dash_of(world: &World, r: &AttackRun) -> Option<Dash> {
    let p = &compiled(world, r)[r.phase as usize];
    p.dash.filter(|_| p.kind == PhaseKind::Active)
}

/// How many phases run `r` has.
pub(super) fn phase_count(world: &World, r: &AttackRun) -> usize {
    compiled(world, r).len()
}

/// Phase `i` of run `r`, `r.speed_pct` percent faster and `r.width_pct` percent wider.
pub(crate) fn phase<'a>(world: &'a World, r: &AttackRun, i: usize) -> PhaseView<'a> {
    let phases = compiled(world, r);
    let base = &phases[i];
    PhaseView {
        base,
        ticks: phase_ticks(phases, r.speed_pct, i),
        widened: if r.width_pct == 0 {
            None
        } else {
            base.hit_box.as_ref().map(|h| wider_box(h, r.width_pct))
        },
    }
}

/// The length of phase `i` of `phases` at `speed_pct` percent attack speed: its own length at
/// 0, else `faster_ticks`.
#[inline]
pub(super) fn phase_ticks(phases: &[AttackPhase], speed_pct: u32, i: usize) -> u32 {
    if speed_pct == 0 {
        phases[i].ticks
    } else {
        faster_ticks(phases, speed_pct, i)
    }
}

/// The length of phase `i` of `phases` `pct` percent faster: the attack's length divided by
/// `1 + pct / 100`, rounded to the nearest tick, and shared among the phases in proportion,
/// each at least 1 tick. The ticks left over after rounding each phase down go to the phases
/// with the largest remainders, the first of equal ones first.
fn faster_ticks(phases: &[AttackPhase], pct: u32, i: usize) -> u32 {
    let per = 100 + u64::from(pct);
    let total: u64 = phases.iter().map(|p| u64::from(p.ticks)).sum();
    let target = ((total * 100 + per / 2) / per).max(phases.len() as u64);
    let rounded = |p: &AttackPhase| {
        let exact = u64::from(p.ticks) * 100;
        let ticks = (exact / per).max(1);
        (ticks, exact.saturating_sub(ticks * per))
    };
    let (own, own_rest) = rounded(&phases[i]);
    let mut sum = 0u64;
    // Phases that come before this one in the order of the largest remainders.
    let mut before = 0u64;
    for (j, p) in phases.iter().enumerate() {
        let (ticks, rest) = rounded(p);
        sum += ticks;
        if rest > own_rest || (rest == own_rest && j < i) {
            before += 1;
        }
    }
    let ticks = u32::try_from(own).unwrap_or(u32::MAX);
    if before < target.saturating_sub(sum) {
        ticks.saturating_add(1)
    } else {
        ticks
    }
}

/// `hit_box` `pct` percent larger, at most `MAX_RANGE_PCT`: its radius and its offset, each
/// rounded toward zero and held within `i32`; an arc's angle stays.
fn wider_box(hit_box: &HitBox, pct: u32) -> HitBox {
    // The validator holds the amount to `MAX_RANGE_PCT`; the clamp keeps an unvalidated file there.
    let per = 100 + i64::from(pct.min(MAX_RANGE_PCT));
    let grow = |v: &mut Fx| {
        v.0 = i32::try_from(i64::from(v.0) * per / 100).unwrap_or(if v.0 < 0 {
            i32::MIN
        } else {
            i32::MAX
        });
    };
    let mut out = hit_box.clone();
    match &mut out {
        HitBox::Circle { offset, radius, .. } | HitBox::Arc { offset, radius, .. } => {
            grow(radius);
            grow(&mut offset.forward);
            grow(&mut offset.right);
        }
        HitBox::Line {
            offset,
            length,
            width,
            ..
        } => {
            grow(length);
            grow(width);
            grow(&mut offset.forward);
            grow(&mut offset.right);
        }
    }
    out
}

// --- Carrying out ----------------------------------------------------------------------------

#[cfg(test)]
mod attack_table_tests;

mod emit;
use emit::{exec, uses_spot};

mod program;
use program::{
    Above, above, aim, body_of, list_at, root_list, rounds, saturate, step_task, ticks, value,
};
pub(super) use program::{normalize, rotate};

mod runs;
use runs::begin_phase;
pub(super) use runs::{
    advance_runs, capped_type_bonus, end_run, end_runs_of, fire_hooks, hold_ready, in_end_recovery,
    is_combo, phase_damage, retain_runs, run, swing_damage,
};

mod start;
pub(super) use start::{Strike, start, start_aged, start_player, start_rune};
