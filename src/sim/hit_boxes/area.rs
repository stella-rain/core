//! The ground a hit box covers, in integer geometry.

use crate::fixed::{Fx, Point};
use crate::sim::state::Dir;
use crate::trig;

use super::Swing;
use crate::sim::overlaps;

/// The ground a hit box covers: a circle at `centre`, narrowed to a sector when `arc` is set.
pub(in crate::sim) struct Area {
    pub(super) centre: Point,
    pub(super) facing: Dir,
    /// The circle's radius; a line's reach from its start: its length and half its width, which
    /// holds every point of it.
    pub(super) radius: i32,
    /// `None` for a circle, or an arc of 180 degrees or more to each side, which is one.
    pub(super) arc: Option<Sector>,
    /// A rectangle instead of a circle or an arc.
    pub(super) line: Option<Line>,
}

/// A rectangle from `Area::centre` along `dir`.
pub(super) struct Line {
    dir: Dir,
    length: i32,
    /// Half the width: the reach to each side.
    half_width: i32,
}

impl Line {
    pub(super) fn new(dir: Dir, length: Fx, width: Fx) -> Line {
        Line {
            dir,
            length: length.0,
            half_width: width.0 / 2,
        }
    }

    /// The area of a line starting at `centre`.
    pub(super) fn area(self, centre: Point) -> Area {
        Area {
            centre,
            facing: self.dir,
            radius: self.length.saturating_add(self.half_width),
            arc: None,
            line: Some(self),
        }
    }

    /// Whether the circle at `d` (from the line's start) with `radius` touches the rectangle:
    /// the nearest point of it to the circle's centre is within `radius`. Squared and scaled by
    /// `trig::SCALE`, so no root is taken.
    pub(super) fn touches(&self, d: (i128, i128), radius: i32) -> bool {
        let (ex, ey) = (i128::from(self.dir.x), i128::from(self.dir.y));
        let scale = i128::from(trig::SCALE);
        let along = d.0 * ex + d.1 * ey;
        let across = d.0 * ey - d.1 * ex;
        let reach = i128::from(self.half_width) * scale;
        let past_along = along - along.clamp(0, i128::from(self.length) * scale);
        let past_across = across - across.clamp(-reach, reach);
        let r = i128::from(radius) * scale;
        past_along * past_along + past_across * past_across <= r * r
    }
}

/// What an arc's test needs, worked out once per swing rather than once per target: the cosine
/// of its half angle and its two edges.
pub(super) struct Sector {
    cos: i32,
    edges: [Dir; 2],
}

impl Sector {
    pub(super) fn new(facing: Dir, half_angle: i32) -> Option<Sector> {
        (half_angle < 180 * trig::PER_DEGREE).then(|| Sector {
            cos: trig::sin_cos(half_angle).1,
            edges: [half_angle, -half_angle].map(|turn| super::attacks::rotate(facing, turn)),
        })
    }
}

pub(super) fn circle(centre: Point, facing: Dir, radius: i32, arc: Option<Sector>) -> Area {
    Area {
        centre,
        facing,
        radius,
        arc,
        line: None,
    }
}

/// The hit box's centre: `forward` along `facing` and `right` a quarter turn clockwise from it.
pub(in crate::sim) fn centre(at: Point, facing: Dir, forward: i32, right: i32) -> Point {
    let (fx, fy) = (i64::from(facing.x), i64::from(facing.y));
    let (fw, rt) = (i64::from(forward), i64::from(right));
    let scale = i64::from(trig::SCALE);
    let dx = (fx * fw - fy * rt) / scale;
    let dy = (fy * fw + fx * rt) / scale;
    let move_by = |v: Fx, d: i64| Fx(i32::try_from(i64::from(v.0) + d).unwrap_or(v.0));
    Point {
        x: move_by(at.x, dx),
        y: move_by(at.y, dy),
    }
}

/// Whether the swing's hit box overlaps the circle at `at` with `radius`.
pub(super) fn covers(w: &Swing, at: Point, radius: i32) -> bool {
    w.area.covers(at, radius)
}

impl Area {
    pub(in crate::sim) fn covers(&self, at: Point, radius: i32) -> bool {
        if !overlaps(self.centre, self.radius, at, radius) {
            return false;
        }
        if self.arc.is_none() && self.line.is_none() {
            return true;
        }
        let d = (
            i128::from(at.x.0) - i128::from(self.centre.x.0),
            i128::from(at.y.0) - i128::from(self.centre.y.0),
        );
        let Some(arc) = &self.arc else {
            return self
                .line
                .as_ref()
                .is_some_and(|line| line.touches(d, radius));
        };
        // A centre within the angle and within reach overlaps; one outside the angle overlaps only
        // if its circle crosses one of the two edges, the nearest part of the sector to it.
        within_angle(d, self.facing, arc.cos)
            || arc
                .edges
                .iter()
                .any(|&edge| touches_edge(d, edge, self.radius, radius))
    }
}

/// Whether `d` is at most the half angle whose cosine is `cos` from `facing`:
/// `d · f ≥ |d| |f| cos`, squared so no root is taken. At the centre itself, it is.
fn within_angle(d: (i128, i128), facing: Dir, cos: i32) -> bool {
    let (fx, fy) = (i128::from(facing.x), i128::from(facing.y));
    let dot = d.0 * fx + d.1 * fy;
    let d2 = d.0 * d.0 + d.1 * d.1;
    if d2 == 0 {
        return true;
    }
    let cos = i128::from(cos);
    let scale = i128::from(trig::SCALE);
    let lhs = dot * scale * (dot * scale);
    let rhs = cos * cos * d2 * (fx * fx + fy * fy);
    if cos >= 0 {
        dot >= 0 && lhs >= rhs
    } else {
        dot >= 0 || lhs <= rhs
    }
}

/// Whether the circle at `d` (from the arc's centre) with `radius` reaches the edge running
/// `length` along `edge`, a direction of length `trig::SCALE`.
fn touches_edge(d: (i128, i128), edge: Dir, length: i32, radius: i32) -> bool {
    let (ex, ey) = (i128::from(edge.x), i128::from(edge.y));
    let scale = i128::from(trig::SCALE);
    let r2 = i128::from(radius) * i128::from(radius);
    let along = d.0 * ex + d.1 * ey;
    if along <= 0 {
        return d.0 * d.0 + d.1 * d.1 <= r2;
    }
    let len = i128::from(length);
    if along >= len * scale {
        let (px, py) = (d.0 - len * ex / scale, d.1 - len * ey / scale);
        return px * px + py * py <= r2;
    }
    let across = d.0 * ey - d.1 * ex;
    across * across <= r2 * scale * scale
}
