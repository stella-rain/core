//! Tests for leash relocation edge cases, neither side fitting, and purity.

use super::World;
use super::leash::land_point;
use crate::fixed::{Fx, Point};
use crate::run_content::RunContent;
use crate::stage::Map;

#[test]
fn neither_side_fits_keeps_mirrored_side_and_clamps_to_border() {
    // Tests first (c), row F:
    // A map 700 px wide, centre x 350: left side 350 - 484 = -134 and right side 834 are both off.
    // An enemy at dx = +900, dy = 0: usable_side keeps the mirrored side (left); the point is
    // x = -134 clamped to 0, y = the centre's.
    let world = World::for_test(RunContent::bundled().trial().clone());
    let map_x = Map {
        width: Fx::px(700),
        height: Fx::px(4000),
        start: None,
        facing: None,
    };
    let centre_x = Point {
        x: Fx::px(350),
        y: Fx::px(2000),
    };
    let p1 = land_point(&world, &map_x, centre_x, 900 * 256, 0);
    let p1_again = land_point(&world, &map_x, centre_x, 900 * 256, 0);
    assert_eq!(p1, p1_again, "land_point is pure and deterministic");
    assert_eq!(
        p1,
        Point {
            x: Fx(0),
            y: Fx::px(2000),
        }
    );

    // The horizontal twin:
    // A map 300 px high with the enemy at dy = +500: centre y 150, sides -94 and 394 against 300;
    // both off. usable_side keeps the mirrored side (top); point is y = -94 clamped to 0.
    let map_y = Map {
        width: Fx::px(4000),
        height: Fx::px(300),
        start: None,
        facing: None,
    };
    let centre_y = Point {
        x: Fx::px(2000),
        y: Fx::px(150),
    };
    let p2 = land_point(&world, &map_y, centre_y, 0, 500 * 256);
    let p2_again = land_point(&world, &map_y, centre_y, 0, 500 * 256);
    assert_eq!(p2, p2_again, "land_point is pure and deterministic");
    assert_eq!(
        p2,
        Point {
            x: Fx::px(2000),
            y: Fx(0),
        }
    );
}
