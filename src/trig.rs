//! Sine and cosine from a table (ADR-004: no floating point in the simulation, trigonometry
//! through lookup tables). Angles are in 1/256 degree, like the angles in attack patterns;
//! the table has one entry per whole degree and does not interpolate.

/// What a sine or cosine of 1 is: results are multiplied by this.
pub const SCALE: i32 = 1024;

/// 1/256 degree units in one degree and in a full turn.
pub const PER_DEGREE: i32 = 256;
pub const PER_TURN: i32 = 360 * PER_DEGREE;

/// `sin` of 0 to 90 degrees, times `SCALE`, rounded. The other three quarters follow by
/// symmetry. The values were computed once and are written out here, so no platform ever
/// computes them.
const SIN_QUARTER: [i16; 91] = [
    0, 18, 36, 54, 71, 89, 107, 125, 143, 160, 178, 195, 213, 230, 248, 265, 282, 299, 316, 333,
    350, 367, 384, 400, 416, 433, 449, 465, 481, 496, 512, 527, 543, 558, 573, 587, 602, 616, 630,
    644, 658, 672, 685, 698, 711, 724, 737, 749, 761, 773, 784, 796, 807, 818, 828, 839, 849, 859,
    868, 878, 887, 896, 904, 912, 920, 928, 935, 943, 949, 956, 962, 968, 974, 979, 984, 989, 994,
    998, 1002, 1005, 1008, 1011, 1014, 1016, 1018, 1020, 1022, 1023, 1023, 1024, 1024,
];

fn sin_degree(degree: usize) -> i32 {
    let at = |d: usize| i32::from(SIN_QUARTER[d]);
    match degree {
        0..=90 => at(degree),
        91..=180 => at(180 - degree),
        181..=270 => -at(degree - 180),
        _ => -at(360 - degree),
    }
}

/// `(sin, cos)` of `angle`, each times `SCALE`. Any angle is accepted; it is taken modulo a
/// full turn and rounded down to a whole degree.
pub fn sin_cos(angle: i32) -> (i32, i32) {
    // `>>` on a negative number rounds toward minus infinity, which is what a turn needs.
    let degree = (angle >> 8).rem_euclid(360) as usize;
    (sin_degree(degree), sin_degree((degree + 90) % 360))
}
