//! Fixed-point positions and velocities: `i32` in 1/256 pixel units (ADR-004).

/// Sub-pixel units per pixel.
pub const UNITS_PER_PX: i32 = 256;

/// A length in 1/256 pixel units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
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
