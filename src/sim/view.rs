//! The view rectangle of the live rules: its half sizes, its centre and the area type.

use crate::fixed::{Fx, Point};
use crate::sim::state::{PlayerIdx, Players};

/// The view rectangle (RA1, DS82): the widest supported aspect, 21:9, of the 360 px design
/// height, 840 by 360 px, never the device's screen. It is the camera's rectangle: centred on the
/// main character but held inside the map (`view_centre`). An enemy starts a ranged attack
/// only from inside it, edges included. Tuning values.
pub(crate) const VIEW_HALF_WIDTH: Fx = Fx::px(crate::limits::VIEW_HALF_WIDTH);
pub(crate) const VIEW_HALF_HEIGHT: Fx = Fx::px(crate::limits::VIEW_HALF_HEIGHT);

/// The centre of the view rectangle for a main character at `at` on `map`: the camera's
/// centre, which follows the main character but stops where the view would leave the map. It
/// mirrors `camera_follow.gd` (`_axis`) on each axis: a map no larger than the view gives
/// `map / 2` (integer division, so an odd number of units loses half a unit, 1/512 px), else
/// the main character's coordinate held between the half view and `map - half view`.
pub(crate) fn view_centre(map: &crate::stage::Map, at: Point) -> Point {
    let axis = |target: Fx, side: Fx, half: Fx| {
        let (target, side, half) = (i64::from(target.0), i64::from(side.0), i64::from(half.0));
        let centre = if side <= 2 * half {
            side / 2
        } else {
            target.clamp(half, side - half)
        };
        // Within `side`, which fits `i32`.
        Fx(i32::try_from(centre).unwrap_or(i32::MAX))
    };
    Point {
        x: axis(at.x, map.width, VIEW_HALF_WIDTH),
        y: axis(at.y, map.height, VIEW_HALF_HEIGHT),
    }
}

/// The bounding box of the view centres of the players the world reads (`Players::targets`):
/// its centre and half extents. One player gives that player's view centre and half extents 0,
/// so every use equals the single-player rule (ADR-SIM-7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Area {
    pub(crate) centre: Point,
    pub(crate) half_x: i64,
    pub(crate) half_y: i64,
}

impl Area {
    pub(crate) fn of_view(map: &crate::stage::Map, players: &Players) -> Area {
        let mask = players.targets();
        // One player (nearly every run): its view centre, no box.
        if mask & mask.wrapping_sub(1) == 0 {
            let me = Players::slots(mask).next().unwrap_or(PlayerIdx::SOLO);
            return Area::at(view_centre(map, players[me].at));
        }
        let (mut lo, mut hi) = ((i32::MAX, i32::MAX), (i32::MIN, i32::MIN));
        for me in Players::slots(mask) {
            let c = view_centre(map, players[me].at);
            lo = (lo.0.min(c.x.0), lo.1.min(c.y.0));
            hi = (hi.0.max(c.x.0), hi.1.max(c.y.0));
        }
        let mid = |lo: i32, hi: i32| lo + (hi - lo) / 2;
        let centre = Point {
            x: Fx(mid(lo.0, hi.0)),
            y: Fx(mid(lo.1, hi.1)),
        };
        Area {
            centre,
            half_x: i64::from(hi.0 - centre.x.0),
            half_y: i64::from(hi.1 - centre.y.0),
        }
    }

    /// The area of a single point: no extent.
    pub(crate) fn at(centre: Point) -> Area {
        Area {
            centre,
            half_x: 0,
            half_y: 0,
        }
    }
}
