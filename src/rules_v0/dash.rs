//! The dash of an attack phase (DS11, core Z3a-2): an enemy or the boss in an `Active` phase
//! with `dash: { distance }` moves along its attack's locked facing, so that the phase covers
//! exactly `distance`, and is held to the map. Its hit box is read from where the attacker is
//! when `collide` runs, so it travels with it; a boss's parts follow the boss. The state holds
//! nothing new: the progress is the run's phase, tick and facing.

use crate::fixed::Fx;
use crate::id::EntityId;
use crate::trig::SCALE;
use crate::validate::limits::MAX_DASH_DISTANCE;

use super::World;
use super::attacks;
use super::entity::Who;
use super::state::AttackRun;
use crate::engine::State;

/// How far along one axis of the facing (`axis`, scaled by `SCALE`) the dash has gone after `k`
/// of `ticks` ticks: `distance × k / ticks` along the facing, rounded once. A tick moves from
/// the value of `k − 1` to the value of `k`, so the shares add up to the whole dash with no
/// rounding lost on a diagonal.
fn gone(distance: i64, axis: i32, k: i64, ticks: i64) -> i64 {
    i64::from(axis) * (distance * k / ticks) / i64::from(SCALE)
}

/// Every attacker whose run is in a dashing phase moves one share along its facing, in run
/// order.
// Kept out of `rules_v0::step`, so the tick-cost gate measures the rules, not the inliner
// (ADR-058).
#[inline(never)]
pub(super) fn act(stage: &World, s: &mut State) {
    if !stage.has_dash {
        return;
    }
    let map = stage.map();
    let longest = i64::from(Fx::px(MAX_DASH_DISTANCE).0);
    for i in 0..s.runs.len() {
        let r = &s.runs[i];
        let Some(d) = attacks::dash_of(stage, r) else {
            continue;
        };
        let p = attacks::phase(stage, r, r.phase as usize);
        if r.tick >= p.ticks {
            continue;
        }
        // The validator holds a dash to `MAX_DASH_DISTANCE`; the run holds it again.
        let distance = i64::from(d.distance.0).clamp(0, longest);
        let (k, ticks) = (i64::from(r.tick) + 1, i64::from(p.ticks));
        let (dx, dy) = (
            gone(distance, r.facing.x, k, ticks) - gone(distance, r.facing.x, k - 1, ticks),
            gone(distance, r.facing.y, k, ticks) - gone(distance, r.facing.y, k - 1, ticks),
        );
        let at = match s.find(r.owner) {
            Some(Who::Enemy(e)) => &mut s.enemies[e].at,
            Some(Who::Boss) => match s.boss.as_mut() {
                Some(b) => &mut b.at,
                None => continue,
            },
            _ => continue,
        };
        // A share is at most the bound above, so these fit.
        let to = crate::fixed::Point {
            x: at.x.saturating_add(Fx(dx as i32)),
            y: at.y.saturating_add(Fx(dy as i32)),
        };
        *at = map.clamp(to);
    }
}

/// Whether any run is in a dashing phase: the push-apart looks for a dasher only then.
pub(super) fn any(stage: &World, runs: &[AttackRun]) -> bool {
    stage.has_dash && runs.iter().any(|r| is_dashing(stage, r))
}

/// Whether `owner` has a run in a dashing phase.
pub(super) fn dashing(stage: &World, runs: &[AttackRun], owner: EntityId) -> bool {
    runs.iter()
        .any(|r| r.owner == owner && is_dashing(stage, r))
}

fn is_dashing(stage: &World, r: &AttackRun) -> bool {
    attacks::dash_of(stage, r).is_some()
}
