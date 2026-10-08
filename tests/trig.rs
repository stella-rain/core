//! The sine table (ADR-004): integer sine and cosine, in 1/256 degree, scaled by 1024.

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
fn known_angles_match_real_sines_to_the_table_resolution() {
    // sin 30 = 0.5, sin 45 = 0.7071, sin 60 = 0.8660, times 1024.
    assert_eq!(at(30), (512, 887));
    assert_eq!(at(45), (724, 724));
    assert_eq!(at(60), (887, 512));
    assert_eq!(at(1), (18, 1024));
    assert_eq!(at(6), (107, 1018));
}

#[test]
fn every_degree_is_on_the_unit_circle_within_rounding() {
    let one = i64::from(SCALE) * i64::from(SCALE);
    for degree in 0..360 {
        let (s, c) = at(degree);
        let r2 = i64::from(s).pow(2) + i64::from(c).pow(2);
        // Each value is rounded to the nearest 1/1024, so r^2 is within about 2 * 1024 * 0.5 * 2.
        assert!(
            (r2 - one).abs() <= 2 * i64::from(SCALE),
            "{degree} degrees: {r2} vs {one}"
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
        // Cosine is sine a quarter turn on.
        assert_eq!(at(d + 90).0, c, "{d}");
    }
}

#[test]
fn any_angle_is_taken_modulo_a_full_turn_and_rounded_down_to_a_degree() {
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
    // Part of a degree is dropped, not rounded.
    assert_eq!(sin_cos(255), sin_cos(0));
    assert_eq!(sin_cos(256 + 255), at(1));
    // Negative angles round toward minus infinity: -1/256 degree is -1 degree, i.e. 359.
    assert_eq!(sin_cos(-1), at(359));
    assert_eq!(at(-90), (-SCALE, 0));
}

#[test]
fn the_quarter_rises_without_stepping_back() {
    let sines: Vec<i32> = (0..=90).map(|d| at(d).0).collect();
    assert!(sines.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!((sines[0], sines[90]), (0, SCALE));
}
