//! The dash of an attack phase (DS11, core Z3a-2): an enemy or the boss in an `Active` phase
//! with `dash: { distance }` moves along its attack's locked facing, so that the phase covers
//! exactly `distance`, and is held to the map. Its hit box is read from where the attacker is
//! when `collide` runs, so it travels with it; a boss's parts follow the boss. The state holds
//! nothing new: the progress is the run's phase, tick and facing.

use crate::attack::PhaseKind;
use crate::fixed::Fx;
use crate::id::EntityId;
use crate::trig::SCALE;

use super::World;
use super::attacks;
use super::entity::Who;
use super::state::AttackRun;
use crate::engine::State;

/// The `k`th of `ticks` shares of `distance`: from `distance × (k − 1) / ticks` to
/// `distance × k / ticks`, so that all of them add up to the distance, as the main character's
/// dash does (`player`).
fn share(distance: i64, k: i64, ticks: i64) -> i64 {
    distance * k / ticks - distance * (k - 1) / ticks
}

/// Every attacker whose run is in a dashing phase moves one share along its facing, in run
/// order.
pub(super) fn act(stage: &World, s: &mut State) {
    let map = stage.map();
    let scale = i64::from(SCALE);
    for i in 0..s.runs.len() {
        let r = &s.runs[i];
        let p = attacks::phase(stage, r, r.phase as usize);
        let (PhaseKind::Active, Some(d)) = (p.kind, p.dash) else {
            continue;
        };
        if r.tick >= p.ticks {
            continue;
        }
        let step = share(
            i64::from(d.distance.0),
            i64::from(r.tick) + 1,
            i64::from(p.ticks),
        );
        let (dx, dy) = (
            i64::from(r.facing.x) * step / scale,
            i64::from(r.facing.y) * step / scale,
        );
        let at = match s.find(r.owner) {
            Some(Who::Enemy(e)) => &mut s.enemies[e].at,
            Some(Who::Boss) => match s.boss.as_mut() {
                Some(b) => &mut b.at,
                None => continue,
            },
            _ => continue,
        };
        // A share is at most the 960 pixel bound of the validator, so these fit.
        let to = crate::fixed::Point {
            x: at.x.saturating_add(Fx(dx as i32)),
            y: at.y.saturating_add(Fx(dy as i32)),
        };
        *at = map.clamp(to);
    }
}

/// Whether any run is in a dashing phase: the push-apart looks for a dasher only then.
pub(super) fn any(stage: &World, runs: &[AttackRun]) -> bool {
    runs.iter().any(|r| is_dashing(stage, r))
}

/// Whether `owner` has a run in a dashing phase.
pub(super) fn dashing(stage: &World, runs: &[AttackRun], owner: EntityId) -> bool {
    runs.iter()
        .any(|r| r.owner == owner && is_dashing(stage, r))
}

fn is_dashing(stage: &World, r: &AttackRun) -> bool {
    let p = attacks::phase(stage, r, r.phase as usize);
    p.kind == PhaseKind::Active && p.dash.is_some()
}
