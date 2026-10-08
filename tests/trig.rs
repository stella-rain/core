//! The sine table (ADR-004): integer sine and cosine, in 1/256 degree, scaled by 65536, with
//! linear interpolation between whole degrees.

use stella_rain_core::trig::{PER_DEGREE, PER_TURN, SCALE, sin_cos};

fn at(degree: i32) -> (i32, i32) {
    sin_cos(degree * PER_DEGREE)
}

#[test]
fn the_cardinal_angles_are_exact() {
    assert_eq!(at(0), (0, SCALE));
    assert_eq!(at(90), (SCALE, 0));
    assert_eq!(at(180), (0, -SCALE));
    assert_eq!(at(270), (-SCALE, 0));
}

#[test]
fn known_angles_match_real_sines() {
    // sin 30 = 0.5, sin 45 = 0.70711, sin 60 = 0.86603, times 65536; and sin 1 = 0.0174524.
    assert_eq!(at(30), (32_768, 56_756));
    assert_eq!(at(45), (46_341, 46_341));
    assert_eq!(at(60), (56_756, 32_768));
    assert_eq!(at(1), (1_144, 65_526));
}

#[test]
fn angles_between_degrees_are_interpolated() {
    // Halfway between 0 and 1 degree: half of 1144.
    assert_eq!(sin_cos(128).0, 572);
    // 1.5 degrees: sin = 0.026177 (1715.5 in table units), cos = 0.999657 (65513.5). A straight
    // line between table entries lies inside a sine by up to (1 degree in radians)^2 / 8, which
    // is 2.5 table units, 4e-5 of the whole range; that is the error to expect.
    let (s, c) = sin_cos(384);
    assert!((1_713..=1_717).contains(&s), "{s}");
    assert!((65_510..=65_515).contains(&c), "{c}");
    // A quarter of the way from 89 to 90 degrees: sin = 0.999914 (65530.4), cos = 0.013090 (857.8).
    let (s, c) = sin_cos(89 * PER_DEGREE + 64);
    assert!((65_527..=65_531).contains(&s), "{s}");
    assert!((855..=860).contains(&c), "{c}");
}

#[test]
fn the_curve_is_smooth_with_no_steps_between_neighbouring_angles() {
    // One 256th of a degree moves a sine by at most 1144 / 256, about 4.5 table units.
    let mut previous = sin_cos(-PER_TURN);
    for angle in -PER_TURN + 1..=PER_TURN {
        let now = sin_cos(angle);
        assert!((now.0 - previous.0).abs() <= 5, "sin jumps at {angle}");
        assert!((now.1 - previous.1).abs() <= 5, "cos jumps at {angle}");
        previous = now;
    }
}

#[test]
fn every_angle_is_on_the_unit_circle_within_rounding() {
    let one = i64::from(SCALE) * i64::from(SCALE);
    for angle in (0..PER_TURN).step_by(7) {
        let (s, c) = sin_cos(angle);
        let r2 = i64::from(s).pow(2) + i64::from(c).pow(2);
        // Linear interpolation lies inside the circle by up to 2.5 units on each axis, so r^2
        // falls short of the unit by at most about 2 * SCALE * 4.
        assert!(
            (r2 - one).abs() <= 8 * i64::from(SCALE),
            "{angle}: {r2} vs {one}"
        );
    }
}

#[test]
fn the_four_quarters_are_mirror_images() {
    for d in 0..=90 {
        let (s, c) = at(d);
        assert_eq!(at(180 - d), (s, -c), "{d}");
        assert_eq!(at(180 + d), (-s, -c), "{d}");
        assert_eq!(at(360 - d), (-s, c), "{d}");
        assert_eq!(at(d + 90).0, c, "{d}");
    }
    // The same between degrees.
    for frac in [1, 64, 128, 200, 255] {
        let c = sin_cos(37 * PER_DEGREE + frac).1;
        assert_eq!(sin_cos(127 * PER_DEGREE + frac).0, c, "{frac}");
    }
}

#[test]
fn any_angle_is_taken_modulo_a_full_turn() {
    for angle in [
        0,
        1,
        255,
        256,
        12_345,
        92_159,
        -1,
        -256,
        -257,
        -92_160,
        2_000_000_000,
        -2_000_000_000,
    ] {
        assert_eq!(sin_cos(angle), sin_cos(angle + PER_TURN), "{angle}");
        assert_eq!(sin_cos(angle), sin_cos(angle - PER_TURN), "{angle}");
    }
    // The extremes are fine too (a turn does not divide 2^32, so only these are not compared).
    for angle in [i32::MIN, i32::MAX] {
        let (s, c) = sin_cos(angle);
        assert!(s.abs() <= SCALE && c.abs() <= SCALE);
    }
    // Negative angles count down from a full turn: -1/256 degree is just below 360.
    let (s, c) = sin_cos(-1);
    assert!((-17..=0).contains(&s) && c > SCALE - 3, "{s} {c}");
    assert_eq!(at(-90), (-SCALE, 0));
}

#[test]
fn the_quarter_rises_without_stepping_back() {
    let sines: Vec<i32> = (0..=90 * PER_DEGREE).map(|a| sin_cos(a).0).collect();
    assert!(sines.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!((sines[0], *sines.last().unwrap()), (0, SCALE));
    // And it really moves: no stretch of a whole degree is flat.
    assert!(
        sines
            .chunks_exact(PER_DEGREE as usize)
            .all(|c| c.first() != c.last())
    );
}
