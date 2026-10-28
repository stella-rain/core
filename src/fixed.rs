//! Fixed-point positions and velocities: `i32` in 1/256 pixel units (ADR-004).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Sub-pixel units per pixel.
pub const UNITS_PER_PX: i32 = 256;

/// A length in 1/256 pixel units. In files it is the plain integer.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
#[schemars(inline)]
pub struct Fx(pub i32);

impl Fx {
    /// Whole pixels; panics on overflow, like every unmarked operation here.
    pub const fn px(px: i32) -> Self {
        Fx(px * UNITS_PER_PX)
    }

    /// Narrows a widened product back to a length; panics when it does not fit, like
    /// the overflow it replaces (a wrapping or saturating cast would hide the defect).
    pub fn from_wide(v: i64) -> Fx {
        Fx(i32::try_from(v).expect("fixed-point value out of range"))
    }

    pub fn saturating_add(self, other: Fx) -> Fx {
        Fx(self.0.saturating_add(other.0))
    }

    pub fn clamp(self, lo: Fx, hi: Fx) -> Fx {
        Fx(self.0.clamp(lo.0, hi.0))
    }
}

/// A point on the playfield, from its top-left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: Fx,
    pub y: Fx,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_wide_keeps_an_in_range_value() {
        assert_eq!(Fx::from_wide(1_000_000_000_i64).0, 1_000_000_000);
        assert_eq!(Fx::from_wide(-5).0, -5);
        assert_eq!(Fx::from_wide(i64::from(i32::MAX)).0, i32::MAX);
        assert_eq!(Fx::from_wide(i64::from(i32::MIN)).0, i32::MIN);
    }

    #[test]
    #[should_panic(expected = "fixed-point value out of range")]
    fn from_wide_panics_above_the_range() {
        let _ = Fx::from_wide(i64::from(i32::MAX) + 1);
    }

    #[test]
    #[should_panic(expected = "fixed-point value out of range")]
    fn from_wide_panics_below_the_range() {
        let _ = Fx::from_wide(i64::from(i32::MIN) - 1);
    }
}
