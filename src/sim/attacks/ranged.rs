//! Ranged attack tokens of the live rules: the pools, who holds one and who may start.

use super::super::agents::condition_holds;
use super::super::entity::Who;
use super::super::select::select;
use super::super::world::RangedPool;
use super::super::{MAX_RANGED_ATTACKERS, VIEW_HALF_HEIGHT, VIEW_HALF_WIDTH, World};
use super::{busy, phase};
use crate::attack::PhaseKind;
use crate::fixed::Point;
use crate::id::EntityId;
use crate::sim::state::{AttackRun, State};

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
pub(super) fn holds_token(world: &World, r: &AttackRun) -> bool {
    r.ranged && phase(world, r, r.phase as usize).kind != PhaseKind::Recovery
}

/// Starts the cooling of a token of `pool` that was freed on the current tick.
pub(super) fn cool_token(world: &World, s: &mut State, pool: RangedPool) {
    let cooldown = world.tuning.ranged_token_cooldown_ticks;
    let entry = (s.tick.saturating_add(cooldown), pool);
    let at = s.ranged_cooling.partition_point(|e| *e <= entry);
    s.ranged_cooling.insert(at, entry);
}

/// Whether the regular enemy `id` may start a ranged attack of `pool` now (RA1): it holds this
/// tick's grant for the pool, and the pool still has room.
pub(in crate::sim) fn ranged_may_start(
    world: &World,
    s: &State,
    id: EntityId,
    pool: RangedPool,
) -> bool {
    s.scratch.ranged_grants[pool.index()].contains(&id) && ranged_room(world, s, pool)
}

/// Whether `e` stands inside the view rectangle around `centre` (the camera's, from
/// `view_centre`), edges included.
pub(in crate::sim) fn in_view(e: Point, area: super::Area) -> bool {
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
pub(in crate::sim) fn grant_ranged(world: &World, s: &mut State, up: &super::select::Up) {
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
pub(in crate::sim) fn mark_ranged(s: &mut State) {
    if let Some(r) = s.runs.last_mut() {
        r.ranged = true;
    }
}
