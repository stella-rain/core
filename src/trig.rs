//! Sine and cosine from a table (ADR-004: no floating point in the simulation, trigonometry
//! through lookup tables). Angles are in 1/256 degree, like the angles in attack patterns.
//! The table has one entry per whole degree; angles in between are interpolated linearly, so
//! slow turns, such as a spiral, move smoothly rather than in one-degree steps.

/// What a sine or cosine of 1 is: results are multiplied by this.
pub const SCALE: i32 = 65_536;

/// 1/256 degree units in one degree and in a full turn.
pub const PER_DEGREE: i32 = 256;
pub const PER_TURN: i32 = 360 * PER_DEGREE;

/// `sin` of 0 to 90 degrees, times `SCALE`, rounded. The other three quarters follow by
/// symmetry. The values were computed once and are written out here, so no platform ever
/// computes them.
const SIN_QUARTER: [i32; 91] = [
    0, 1144, 2287, 3430, 4572, 5712, 6850, 7987, 9121, 10252, 11380, 12505, 13626, 14742, 15855,
    16962, 18064, 19161, 20252, 21336, 22415, 23486, 24550, 25607, 26656, 27697, 28729, 29753,
    30767, 31772, 32768, 33754, 34729, 35693, 36647, 37590, 38521, 39441, 40348, 41243, 42126,
    42995, 43852, 44695, 45525, 46341, 47143, 47930, 48703, 49461, 50203, 50931, 51643, 52339,
    53020, 53684, 54332, 54963, 55578, 56175, 56756, 57319, 57865, 58393, 58903, 59396, 59870,
    60326, 60764, 61183, 61584, 61966, 62328, 62672, 62997, 63303, 63589, 63856, 64104, 64332,
    64540, 64729, 64898, 65048, 65177, 65287, 65376, 65446, 65496, 65526, 65536,
];

fn sin_degree(degree: usize) -> i32 {
    let at = |d: usize| SIN_QUARTER[d];
    match degree % 360 {
        d @ 0..=90 => at(d),
        d @ 91..=180 => at(180 - d),
        d @ 181..=270 => -at(d - 180),
        d => -at(360 - d),
    }
}

/// The sine of `degree` degrees and `frac` 256ths of the way to the next degree.
fn sin_between(degree: usize, frac: i32) -> i32 {
    let (a, b) = (sin_degree(degree), sin_degree(degree + 1));
    // The difference between neighbours is at most 1,144, so this cannot overflow.
    a + (b - a) * frac / PER_DEGREE
}

/// `(sin, cos)` of `angle`, each times `SCALE`. Any angle is accepted; it is taken modulo a
/// full turn. Whole degrees are exact table values; angles in between are interpolated.
pub fn sin_cos(angle: i32) -> (i32, i32) {
    // `>>` on a negative number rounds toward minus infinity, which is what a turn needs, and
    // the low eight bits are then the distance above that degree.
    let degree = (angle >> 8).rem_euclid(360) as usize;
    let frac = angle & 0xff;
    (sin_between(degree, frac), sin_between(degree + 90, frac))
}
