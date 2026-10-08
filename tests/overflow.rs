//! `overflow-checks = true` in every profile (ADR-019): CI runs this in debug and release.

use std::hint::black_box;

#[test]
#[should_panic(expected = "overflow")]
fn integer_overflow_panics_in_this_profile() {
    let x: i32 = black_box(i32::MAX);
    let _ = black_box(x + black_box(1));
}
