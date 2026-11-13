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
    cool_into(&mut s.ranged_cooling, s.tick, world, pool);
}

/// `cool_token` on the cooling list alone, for a caller that holds another part of the state.
pub(in crate::sim) fn cool_into(
    cooling: &mut Vec<(u32, RangedPool)>,
    tick: u32,
    world: &World,
    pool: RangedPool,
) {
    let cooldown = world.tuning.ranged_token_cooldown_ticks;
    let entry = (tick.saturating_add(cooldown), pool);
    let at = cooling.partition_point(|e| *e <= entry);
    cooling.insert(at, entry);
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
    let down_on = world.has_down;
    for (i, e) in s.enemies.iter().enumerate() {
        let def = world.defs.get(e.def);
        // A stunned, flinching or downed enemy fires nothing (`act_one`), so it could not use a grant.
        if def.ranged_pools.is_empty()
            || e.hp == 0
            || !in_view(e.at, area)
            || super::stun::stunned(&e.statuses)
            || e.control.flinch > 0
            || (down_on && e.control.knocked_down > 0)
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

#[cfg(test)]
mod tests {
    //! The grace: a regular enemy whose ranged rule targets a player in grace is not granted a
    //! token, a hit sets the timer and the player's update counts it down. Built on the bench
    //! fixture's worst-case tick, in a party of two.

    use super::*;
    use crate::bench_fixtures::worst_case;
    use crate::engine::{Engine, Run};
    use crate::event::HitKind;
    use crate::sim::PlayerIdx;
    use crate::sim::select::Up;
    use crate::sim::{Blame, Culprit, Shot, hurt_player, player};

    fn party() -> (World, State, crate::rewards::Rewards) {
        let e: Engine = worst_case::warmed_up().into_party(2);
        let rewards = e.content().rewards().clone();
        let (w, s) = match e.run() {
            Run::V2 { world, state, .. } => (world, state),
            Run::V1 { .. } => panic!("the live rules"),
        };
        (World::clone(w), state_of(s), rewards)
    }

    fn state_of(s: &State) -> State {
        let mut s = s.clone();
        s.runs.clear();
        s.ranged_cooling.clear();
        s
    }

    fn p(i: u8) -> PlayerIdx {
        PlayerIdx::new(i).unwrap()
    }

    /// The enemies granted a token now, with the player each one's ranged rule targets.
    fn grants(w: &World, s: &mut State) -> Vec<(EntityId, Option<PlayerIdx>)> {
        let up = Up::of(&s.players);
        grant_ranged(w, s, &up);
        let ids: Vec<EntityId> = s.scratch.ranged_grants.iter().flatten().copied().collect();
        ids.into_iter()
            .map(|id| {
                let i = s.enemies.iter().position(|e| e.id == id).unwrap();
                let def = w.defs.get(s.enemies[i].def);
                let target = def
                    .ranged_pools
                    .iter()
                    .position(Option::is_some)
                    .and_then(|r| select(s, Who::Enemy(i), &def.agent.rules[r].target));
                (
                    id,
                    match target {
                        Some(Who::Player(me)) => Some(me),
                        _ => None,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn g1_a_candidate_whose_target_is_in_grace_is_not_granted() {
        let (mut w, mut s, _) = party();
        let base = grants(&w, &mut s);
        let (_, Some(hit)) = *base.first().expect("a granted enemy in the fixture") else {
            panic!("it targets a player: {base:?}");
        };
        w.tuning.ranged_grace_ticks = 60;
        s.players[hit].combat.ranged_grace = 5;
        let with = grants(&w, &mut s);
        assert!(
            with.iter().all(|(_, t)| *t != Some(hit)),
            "none targets the player in grace: {with:?}"
        );
    }

    #[test]
    fn g2_with_every_target_in_grace_nobody_is_granted_and_a_zero_grace_changes_nothing() {
        let (mut w, mut s, _) = party();
        let base = grants(&w, &mut s);
        assert!(!base.is_empty());
        for i in 0..2 {
            s.players[p(i)].combat.ranged_grace = 5;
        }
        // The row absent: the timers are never read.
        assert_eq!(grants(&w, &mut s), base);
        w.tuning.ranged_grace_ticks = 60;
        assert_eq!(grants(&w, &mut s), []);
    }

    fn shot(s: &State) -> Shot {
        Shot {
            at: s.players[p(0)].at,
            owner: EntityId(99),
            friendly: false,
            damage: 1,
            damage_type: None,
            breaks_charge: false,
            smash: None,
            combo: false,
            rune: None,
            chain: false,
        }
    }

    fn hurt(w: &World, s: &mut State, me: PlayerIdx) {
        let by = Blame {
            kind: HitKind::Attack,
            by: Culprit::Run(crate::sim::world::AttackIndex(0)),
        };
        let shot = shot(s);
        hurt_player(w, s, me, shot, by, &mut Vec::new());
    }

    #[test]
    fn g4_a_hit_sets_the_grace_on_that_player_only() {
        let (mut w, mut s, _) = party();
        s.players[p(0)].invulnerable = 0;
        w.tuning.ranged_grace_ticks = 90;
        hurt(&w, &mut s, p(0));
        assert_eq!(s.players[p(0)].combat.ranged_grace, 90);
        assert_eq!(s.players[p(1)].combat.ranged_grace, 0);
        let (mut w, mut s, _) = party();
        w.tuning.ranged_grace_ticks = 0;
        hurt(&w, &mut s, p(0));
        assert_eq!(s.players[p(0)].combat.ranged_grace, 0);
    }

    #[test]
    fn g5_the_timer_runs_down_once_a_tick_for_an_up_and_a_downed_player() {
        let (w, mut s, rewards) = party();
        s.players[p(1)].hp = 0;
        for me in [p(0), p(1)] {
            s.players[me].combat.ranged_grace = 3;
            player::update(&w, &rewards, &mut s, me, Default::default(), false, &mut Vec::new());
            assert_eq!(s.players[me].combat.ranged_grace, 2, "{me:?}");
            s.players[me].combat.ranged_grace = 0;
            player::update(&w, &rewards, &mut s, me, Default::default(), false, &mut Vec::new());
            assert_eq!(s.players[me].combat.ranged_grace, 0);
        }
    }
}
