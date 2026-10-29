//! The `any` spawn draw: every point of the spawn rectangle's perimeter has one draw value.

use super::{any_point, any_total};

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
    let (c, ex, ey) = (512_000, 98_304, 62_464);
    let mut points = reached(c, c, ex, ey, 1_024_000, 1_024_000);
    assert_eq!(points.len(), 643_072);
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
