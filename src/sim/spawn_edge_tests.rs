//! The `any` spawn draw: every point of the spawn rectangle's perimeter has one draw value.

use super::{any_point, any_total, usable_side, view_centre};
use crate::fixed::{Fx, Point};
use crate::stage::Map;

/// Every `(x, y)` the draw reaches, in draw order.
fn reached(px: i64, py: i64, ex: i64, ey: i64, w: i64, h: i64) -> Vec<(i64, i64)> {
    (0..any_total(px, py, ex, ey, w, h))
        .map(|t| any_point(px, py, ex, ey, w, h, t))
        .collect()
}

/// The integer points on the border of the rectangle `px +- ex`, `py +- ey`.
fn border(px: i64, py: i64, ex: i64, ey: i64) -> Vec<(i64, i64)> {
    let mut points = Vec::new();
    for x in px - ex..=px + ex {
        for y in py - ey..=py + ey {
            if (x - px).abs() == ex || (y - py).abs() == ey {
                points.push((x, y));
            }
        }
    }
    points.sort_unstable();
    points
}

#[test]
fn every_perimeter_point_has_one_draw_value() {
    let mut points = reached(50, 50, 3, 2, 100, 100);
    assert_eq!(points.len(), 20);
    points.sort_unstable();
    assert_eq!(points, border(50, 50, 3, 2));
}

#[test]
fn each_corner_has_exactly_one_draw_value() {
    let points = reached(50, 50, 3, 2, 100, 100);
    for corner in [(47, 48), (53, 48), (47, 52), (53, 52)] {
        let n = points.iter().filter(|p| **p == corner).count();
        assert_eq!(n, 1, "corner {corner:?}");
    }
}

#[test]
fn the_real_size_rectangle_has_equal_weight_per_point() {
    let (c, ex, ey) = (512_000, 123_904, 62_464);
    let mut points = reached(c, c, ex, ey, 1_024_000, 1_024_000);
    assert_eq!(points.len(), 745_472);
    points.sort_unstable();
    assert!(points.windows(2).all(|w| w[0] != w[1]), "a point twice");
    let xs = points.iter().map(|p| p.0);
    let ys = points.iter().map(|p| p.1);
    assert_eq!((xs.clone().min(), xs.max()), (Some(c - ex), Some(c + ex)));
    assert_eq!((ys.clone().min(), ys.max()), (Some(c - ey), Some(c + ey)));
}

#[test]
fn a_clipped_map_reaches_each_on_map_point_once() {
    // The rectangle `2 +- 3` by `2 +- 2` on a map 4 by 4: the left and right sides stand off
    // the map, the top (y = 0) and bottom (y = 4) run along x in [0, 4).
    let mut points = reached(2, 2, 3, 2, 4, 4);
    points.sort_unstable();
    let want: Vec<(i64, i64)> = (0..4).flat_map(|x| [(x, 0), (x, 4)]).collect();
    let mut want = want;
    want.sort_unstable();
    assert_eq!(points, want);
}

/// A map of `w` by `h` px, and the centre of the view for a main character at (`x`, `y`) px.
fn centre_px(w: i32, h: i32, x: i32, y: i32) -> (i32, i32) {
    let map = Map {
        width: Fx::px(w),
        height: Fx::px(h),
        start: None,
        facing: None,
    };
    let c = view_centre(
        &map,
        Point {
            x: Fx::px(x),
            y: Fx::px(y),
        },
    );
    (c.x.0, c.y.0)
}

#[test]
fn the_view_centre_is_the_cameras_clamp() {
    // Each row is worked from `camera_follow.gd _axis(target, map, view)`: `map <= view` gives
    // `map / 2`, else `clampf(target, view / 2, map - view / 2)`; the view is 840 by 360 px.
    let px = |x: i32, y: i32| (x * 256, y * 256);
    let rows = [
        // Mid-map: the target itself.
        ((4000, 4000, 2000, 2000), px(2000, 2000)),
        // 100 < 420: held at the half width.
        ((4000, 4000, 100, 2000), px(420, 2000)),
        // 3900 > 4000 - 420 and 3950 > 4000 - 180.
        ((4000, 4000, 3900, 3950), px(3580, 3820)),
        // map == view: the middle on both axes.
        ((840, 360, 100, 100), px(420, 180)),
        // x: the map is narrower than the view, so 360 / 2; y: clamp(10, 180, 640 - 180).
        ((360, 640, 10, 10), px(180, 180)),
        // The player outside the map still clamps.
        ((4000, 4000, -50, 5000), px(420, 3820)),
    ];
    for ((w, h, x, y), want) in rows {
        assert_eq!(centre_px(w, h, x, y), want, "{w}x{h} at ({x}, {y})");
    }
    // One unit under the view: `map <= view`, and `map / 2` truncates, 215,039 / 2 = 107,519.
    let map = Map {
        width: Fx(215_039),
        height: Fx::px(4000),
        start: None,
        facing: None,
    };
    let at = Point {
        x: Fx::px(100),
        y: Fx::px(2000),
    };
    assert_eq!(view_centre(&map, at).x, Fx(107_519));
}

#[test]
fn the_view_centre_stays_on_the_map_and_within_the_half_view_of_the_player() {
    let side = 4096 * 256;
    let map = Map {
        width: Fx(side),
        height: Fx(side),
        start: None,
        facing: None,
    };
    for x in (0..=side).step_by(37 * 256 + 5) {
        for y in (0..=side).step_by(41 * 256 + 3) {
            let at = Point { x: Fx(x), y: Fx(y) };
            let c = view_centre(&map, at);
            assert!((0..=side).contains(&c.x.0) && (0..=side).contains(&c.y.0));
            assert!((c.x.0 - x).abs() <= 420 * 256 && (c.y.0 - y).abs() <= 180 * 256);
        }
    }
}

#[test]
fn usable_side_falls_back_to_opposite_only_when_target_is_off_and_opposite_is_on() {
    let some = Some((0, 100));
    for i in 0..4 {
        let opp = (i + 2) % 4;

        // When parts[i] is Some: gives i (regardless of opposite).
        let mut parts = [None; 4];
        parts[i] = some;
        assert_eq!(usable_side(&parts, i), i);
        parts[opp] = some;
        assert_eq!(usable_side(&parts, i), i);

        // When parts[i] is None and opposite is Some: gives opposite.
        let mut parts = [None; 4];
        parts[opp] = some;
        assert_eq!(usable_side(&parts, i), opp);

        // When both parts[i] and opposite are None: gives i.
        let parts = [None; 4];
        assert_eq!(usable_side(&parts, i), i);
    }
}
