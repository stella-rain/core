//! Enemies push apart (ADR-058, decision 3), so a horde spreads into a crowd and a grid cell
//! holds a bounded stack. One pass a tick, after every agent has moved and acted and before
//! attacks run: each enemy adds up a push of half the overlap with each neighbour it counts,
//! along the line from the neighbour's centre to its own, and then every enemy moves by its sum
//! at once. A sum is read from the places the pass started with, so it does not depend on the
//! order the enemies are visited in.
//!
//! An enemy whose counted overlaps add up to more than a pixel is crowded, and yields on the
//! next tick: it does not take its own step to its place (`Hold`, `Follow`, `Orbit`). Without
//! that every enemy of a crowd keeps walking inward while the pushes on one in the middle cancel,
//! and the crowd sinks into a stack whatever the push.
//!
//! The main character, companions, the boss and its parts are neither pushed nor push. Every
//! enemy is, one held still by an attack phase included.

use crate::engine::State;
use crate::fixed::{Fx, Point};
use crate::stage::Stage;
use crate::trig;

use super::agent_def;
use super::grid::Grid;

/// A sum of pushes is clamped to this length: 4 pixels a tick, the speed of `Follow`, so a
/// crowd pulled at that speed holds apart.
const STEP: i64 = Fx::px(4).0 as i64;
/// For pushing, a hurt radius counts as at least 8 pixels. The hurt radius does not change.
const LEAST_RADIUS: i32 = Fx::px(8).0;
/// At most this many other enemies are read from the grid for one enemy.
const CANDIDATES: usize = 32;
/// Of the candidates that overlap it, an enemy counts this many, the nearest.
const NEIGHBOURS: usize = 6;
/// An enemy whose counted overlaps add up to more than this, a pixel, is crowded: it yields on the
/// next tick, not taking its own step to its place.
const CROWDED_OVER: i64 = Fx::px(1).0 as i64;
/// The golden angle, 137.508 degrees, in 1/256 degree: enemies on one point part along their
/// ID's multiple of it, so successive IDs are spread round the turn.
const GOLDEN_ANGLE: i64 = 35_202;

/// What the pass knows of an enemy: its entity ID, its place and its radius for pushing.
#[derive(Clone, Copy, Debug)]
pub(super) struct Body {
    pub id: u32,
    pub at: Point,
    pub radius: i32,
}

/// An enemy's radius for pushing.
pub(super) fn push_radius(hurt: Fx) -> i32 {
    hurt.0.max(LEAST_RADIUS)
}

/// The push of `a` away from `b`, which overlaps it: half the overlap along the line from `b`'s
/// centre to `a`'s. The push of `b` away from `a` is exactly its opposite, because integer
/// division rounds toward zero.
///
/// On the same point there is no line: the pair parts along an angle derived from the higher
/// ID, the higher ID along it and the lower against it. No RNG is read.
fn push(a: &Body, b: &Body, dist: i64) -> (i64, i64) {
    let overlap = i64::from(a.radius) + i64::from(b.radius) - dist;
    if dist > 0 {
        let dx = i64::from(a.at.x.0) - i64::from(b.at.x.0);
        let dy = i64::from(a.at.y.0) - i64::from(b.at.y.0);
        return (dx * overlap / (2 * dist), dy * overlap / (2 * dist));
    }
    let angle = i64::from(a.id.max(b.id)) * GOLDEN_ANGLE % i64::from(trig::PER_TURN);
    let (sin, cos) = trig::sin_cos(angle as i32);
    let along = |component: i32| i64::from(component) * overlap / (2 * i64::from(trig::SCALE));
    if a.id > b.id {
        (along(cos), along(sin))
    } else {
        (-along(cos), -along(sin))
    }
}

/// How far each body moves and whether it is crowded, in the order of `bodies`.
///
/// For each body, at most `CANDIDATES` others are read from the grid: its own cell first, then
/// the cells around it, each cell in the order of `bodies` (the engine's enemies are in entity-ID
/// order). Of those that overlap it, the `NEIGHBOURS` nearest are counted, ties to the lower ID.
pub(super) fn offsets(bodies: &[Body]) -> Vec<(i32, i32, bool)> {
    let largest = bodies.iter().map(|b| b.radius).max().unwrap_or(0);
    // Cells as wide as two of the largest radius: whatever overlaps a body is in the 9 cells
    // round it.
    let grid = Grid::new(bodies.iter().map(|b| (b.at, b.radius)).collect(), largest);
    let mut near = Vec::with_capacity(CANDIDATES);
    let mut out = Vec::with_capacity(bodies.len());
    for (i, a) in bodies.iter().enumerate() {
        grid.neighbours(i, CANDIDATES, &mut near);
        // The nearest overlapping candidates so far, as (distance squared, ID, index), ascending.
        let mut counted = [(0i64, 0u32, 0u32); NEIGHBOURS];
        let mut n = 0;
        for &j in &near {
            let b = &bodies[j as usize];
            let dx = i64::from(a.at.x.0) - i64::from(b.at.x.0);
            let dy = i64::from(a.at.y.0) - i64::from(b.at.y.0);
            let reach = i64::from(a.radius) + i64::from(b.radius);
            let key = (dx * dx + dy * dy, b.id, j);
            // Touching is not overlapping.
            if key.0 >= reach * reach || (n == NEIGHBOURS && key >= counted[n - 1]) {
                continue;
            }
            let mut at = if n < NEIGHBOURS { n } else { n - 1 };
            while at > 0 && counted[at - 1] > key {
                counted[at] = counted[at - 1];
                at -= 1;
            }
            counted[at] = key;
            n = (n + 1).min(NEIGHBOURS);
        }
        let (mut sx, mut sy, mut overlap) = (0i64, 0i64, 0i64);
        for &(d2, _, j) in &counted[..n] {
            let b = &bodies[j as usize];
            let dist = i64::try_from(d2.unsigned_abs().isqrt()).unwrap_or(i64::MAX);
            overlap += i64::from(a.radius) + i64::from(b.radius) - dist;
            let (px, py) = push(a, b, dist);
            sx += px;
            sy += py;
        }
        let len2 = sx * sx + sy * sy;
        if len2 > STEP * STEP {
            // The length rounded up, so that the clamped sum is never longer than the step.
            let floor = i64::try_from(len2.unsigned_abs().isqrt()).unwrap_or(i64::MAX);
            let len = floor + i64::from(floor * floor < len2);
            (sx, sy) = (sx * STEP / len, sy * STEP / len);
        }
        out.push((sx as i32, sy as i32, overlap > CROWDED_OVER));
    }
    out
}

/// The pass: every enemy moves by its sum. A push never carries an enemy from the map off it;
/// one already off the map is pushed freely, and is gone once it is past the map's margin, as
/// an enemy that walks there is.
// Kept out of `rules_v0::step`, so the tick-cost gate measures the rules, not the inliner
// (ADR-058).
#[inline(never)]
pub(super) fn apply(stage: &Stage, s: &mut State) {
    if s.enemies.len() < 2 {
        for e in &mut s.enemies {
            e.crowded = false;
        }
        return;
    }
    let bodies: Vec<Body> = s
        .enemies
        .iter()
        .map(|e| Body {
            id: e.id.0,
            at: e.at,
            radius: push_radius(agent_def(stage, e.key).radius),
        })
        .collect();
    let map = stage.map();
    let mut left = false;
    for (e, (dx, dy, crowded)) in s.enemies.iter_mut().zip(offsets(&bodies)) {
        e.crowded = crowded;
        // A shrine, or a mimic in disguise, is fixed; the others are pushed off it.
        if (dx, dy) == (0, 0) || e.shrine.is_some() || e.hidden() {
            continue;
        }
        let to = Point {
            x: e.at.x.saturating_add(Fx(dx)),
            y: e.at.y.saturating_add(Fx(dy)),
        };
        if map.clamp(e.at) == e.at {
            e.at = map.clamp(to);
        } else {
            e.at = to;
            left = true;
        }
    }
    if left {
        let holds = map.holds();
        s.enemies.retain(|e| holds(e.at));
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{Body, CANDIDATES, LEAST_RADIUS, NEIGHBOURS, STEP};
    use crate::fixed::{Fx, Point};

    /// How far each body moves.
    fn offsets(bodies: &[Body]) -> Vec<(i32, i32)> {
        let all = super::offsets(bodies);
        all.into_iter().map(|(dx, dy, _)| (dx, dy)).collect()
    }

    #[test]
    fn a_body_is_crowded_when_its_overlaps_add_up_to_more_than_a_pixel() {
        let crowded = |gap: i32| {
            let pair = [body(1, 0, 0, 2_048), body(2, gap, 0, 2_048)];
            super::offsets(&pair)
                .iter()
                .map(|m| m.2)
                .collect::<Vec<_>>()
        };
        // 8-pixel radii: 4,096 units apart they touch; 256 nearer they overlap by one pixel.
        assert_eq!(crowded(4_096), [false, false]);
        assert_eq!(crowded(3_840), [false, false]);
        assert_eq!(crowded(3_839), [true, true]);
    }

    fn body(id: u32, x: i32, y: i32, radius: i32) -> Body {
        Body {
            id,
            at: Point { x: Fx(x), y: Fx(y) },
            radius,
        }
    }

    #[test]
    fn only_the_six_nearest_overlapping_neighbours_are_counted() {
        // Eight neighbours round the first body, each overlapping it: two at 2,900 units (IDs 2
        // and 3), two at 3,000 (IDs 4 and 5) and four at 2,999 on the diagonals (IDs 6 to 9).
        let ring = [
            (2_900, 0),
            (-2_900, 0),
            (0, 3_000),
            (0, -3_000),
            (2_121, 2_121),
            (-2_121, -2_121),
            (2_121, -2_121),
            (-2_121, 2_121),
        ];
        let mut bodies = vec![body(1, 0, 0, 2_048)];
        bodies.extend(
            ring.iter()
                .zip(2..)
                .map(|(&(x, y), id)| body(id, x, y, 2_048)),
        );
        // The nearest six are IDs 2, 3 and 6 to 9. They mirror each other in pairs, so the sum
        // is no push at all.
        assert_eq!(offsets(&bodies)[0], (0, 0));
        // Without ID 3 one more is counted: IDs 4 and 5 are equally near, and the tie goes to
        // the lower. ID 2 pushes by -2,900 * 1,196 / 5,800 = -598 along x, and ID 4 by
        // -3,000 * 1,096 / 6,000 = -548 along y; counting ID 5 as well would cancel that.
        bodies.remove(2);
        assert_eq!(offsets(&bodies)[0], (-598, -548));
    }

    #[test]
    fn at_most_thirty_two_candidates_are_read() {
        // 40 bodies on one point: each reads 32 others, the lowest IDs, and counts the 6 lowest.
        // Body 40 is the higher ID of each of its 6 pairs, so it goes its own way at full step.
        let bodies: Vec<Body> = (1..=40).map(|id| body(id, 0, 0, 2_048)).collect();
        let moves = offsets(&bodies);
        assert!(CANDIDATES < bodies.len() - 1 && NEIGHBOURS < CANDIDATES);
        for (dx, dy) in &moves[6..] {
            let len = (i64::from(*dx).pow(2) + i64::from(*dy).pow(2)).isqrt();
            assert!((STEP - 2..=STEP).contains(&len), "{dx}, {dy}");
        }
        assert_ne!(moves[38], moves[39]);
    }

    fn crowd(most: usize) -> impl Strategy<Value = Vec<Body>> {
        prop::collection::vec(
            (
                -6_000..=6_000i32,
                -6_000..=6_000i32,
                prop_oneof![Just(LEAST_RADIUS), LEAST_RADIUS..=20_000],
            ),
            2..=most,
        )
        .prop_map(|v| {
            v.into_iter()
                .zip(1..)
                .map(|((x, y, r), id)| body(id, x, y, r))
                .collect()
        })
    }

    proptest! {
        /// No sum is longer than 4 pixels, however deep the overlaps and however many.
        #[test]
        fn a_push_is_never_longer_than_the_step(bodies in crowd(120)) {
            for (dx, dy) in offsets(&bodies) {
                let len2 = i64::from(dx).pow(2) + i64::from(dy).pow(2);
                prop_assert!(len2 <= STEP * STEP, "{}, {}", dx, dy);
            }
        }

        /// Two bodies are pushed exactly opposite ways, on one point or not: no ID is favoured.
        #[test]
        fn a_pair_is_pushed_exactly_opposite_ways(
            bodies in crowd(2),
            same_point in any::<bool>(),
        ) {
            let mut bodies = bodies;
            if same_point {
                bodies[1].at = bodies[0].at;
            }
            let moves = offsets(&bodies);
            prop_assert_eq!(moves[0], (-moves[1].0, -moves[1].1));
        }

        /// The result does not depend on the order of the enemies: while every other body is
        /// read (no more than 32 of them), each ID gets the same push wherever it comes in the
        /// list. Some bodies share a point, where the IDs decide the direction.
        #[test]
        fn the_result_does_not_depend_on_the_order(
            bodies in crowd(CANDIDATES + 1),
            order in any::<prop::sample::Index>(),
            stack in 0..4usize,
        ) {
            let mut bodies = bodies;
            for k in 1..=stack.min(bodies.len() - 1) {
                bodies[k].at = bodies[0].at;
            }
            let by_id = |list: &[Body]| {
                let mut moves: Vec<(u32, (i32, i32))> =
                    list.iter().map(|b| b.id).zip(offsets(list)).collect();
                moves.sort_unstable();
                moves
            };
            let mut shuffled = bodies.clone();
            shuffled.reverse();
            let cut = order.index(shuffled.len());
            shuffled.rotate_left(cut);
            prop_assert_eq!(by_id(&bodies), by_id(&shuffled));
        }
    }
}
