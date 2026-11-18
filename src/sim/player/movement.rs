//! Movement of the player's tick.

use super::{COMBO_SLOT, counter_taken, held_move_pct, stick_move};
use crate::fixed::{Fx, Point};
use crate::limits::{MAX_ADVANCE_DISTANCE, MAX_ADVANCE_PER_TICK};
use crate::sim::entity::{Who, speed_pct};
use crate::sim::state::{MAX_PLAYERS, PlayerIdx, PlayerState, State};
use crate::sim::{World, attacks, levels, select};
use crate::trig::SCALE;

/// Attack magnetism (sim_version >= 2): while the combo step's phase names an `advance`, the
/// player slides along the facing the step took toward its target, by the dash's share of the
/// distance each tick (`dash.rs`), at most `MAX_ADVANCE_PER_TICK` and at most the gap that leaves
/// the two circles touching. No slide without a current target, with the target behind the
/// facing or gone. The evade cancels the run, so it ends the slide.
pub(super) fn magnetism(stage: &World, s: &mut State, me: PlayerIdx) {
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

/// The distance between the farthest two up players past which the soft tether pulls (ADR-SIM-5's
/// view, 840 px wide).
pub(in crate::sim) const TETHER_RANGE: Fx = Fx(2 * super::super::VIEW_HALF_WIDTH.0);

/// The share of the overshoot pulled back each tick: a placeholder the bot playtest tunes.
pub(in crate::sim) const TETHER_PULL_DIV: i64 = 8;

/// The soft tether (ADR-SIM-7), after `schedule::steps::tether` checks it can act.
///
/// Over the up players (at most `MAX_PLAYERS`): when the farthest pair is more than
/// `TETHER_RANGE` apart, every up player at the greatest distance from the bounding box's centre
/// that is not dashing moves toward it by `TETHER_PULL_DIV`ths of the overshoot. A dash is never
/// blocked (it is skipped), a downed or out player is neither measured nor pulled. Doubled
/// coordinates keep the centre exact; `i128` holds the squares of a wide map.
pub(in crate::sim) fn tether(stage: &World, s: &mut State) {
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

pub(in crate::sim) fn move_by(stage: &World, p: &mut PlayerState, dx: i64, dy: i64) {
    // Both are within a speed cap or an evade's distance, which the validator bounds.
    p.at = stage.map().clamp(Point {
        x: p.at.x.saturating_add(Fx::from_wide(dx)),
        y: p.at.y.saturating_add(Fx::from_wide(dy)),
    });
}

pub(super) fn move_player(stage: &World, s: &mut State, me: PlayerIdx, x: i8, y: i8) {
    let keep = speed_pct(&s.players[me].statuses)
        * attacks::move_pct(stage, s, s.id_of(Who::Player(me)))
        / 100
        * held_move_pct(stage, s, me)
        / 100;
    let cap = levels::speed(s, me, i64::from(stage.tuning.player_speed) * keep / 100);
    let (dx, dy) = stick_move(x, y, cap);
    move_by(stage, &mut s.players[me], dx, dy);
}
