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
pub struct Fx(pub i32);

impl Fx {
    /// Whole pixels; panics on overflow, like every unmarked operation here.
    pub const fn px(px: i32) -> Self {
        Fx(px * UNITS_PER_PX)
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
