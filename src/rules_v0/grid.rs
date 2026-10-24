//! A uniform grid over the enemies, so that a friendly hit box is checked only against the
//! enemies near it rather than all of them (core#62). It is rebuilt in each tick's `collide`,
//! where the enemies do not move. Hit boxes (ADR-055) ask it for every enemy within their reach
//! (`within`), and the push-apart (ADR-058) for the enemies around one (`neighbours`).

use crate::fixed::Point;

/// The smallest cell side: 32 pixels, so small radii do not make thousands of cells.
const MIN_CELL: i64 = 8192;

/// Circles (centre, radius) bucketed into square cells, each cell listing its circles in index
/// order. A cell is at least as wide as the probe radius plus the largest circle radius, so a
/// probe can only overlap circles in its own cell or the 8 around it.
#[derive(Debug, Default)]
pub(super) struct Grid {
    circles: Vec<(Point, i32)>,
    /// The corner of cell (0, 0): the smallest x and y of the circles' centres.
    min_x: i64,
    min_y: i64,
    /// The largest circle radius.
    largest: i64,
    cell: i64,
    cols: i64,
    rows: i64,
    /// Cell `c` lists `members[start[c]..start[c + 1]]`, in ascending index order.
    start: Vec<u32>,
    members: Vec<u32>,
    /// The cell of each circle and the next free place in each cell, kept between builds only so
    /// that `rebuild` reuses their memory.
    cells: Vec<usize>,
    next: Vec<u32>,
}

impl Grid {
    /// The grid of `circles` for probes of at most `probe_radius`.
    #[cfg(test)]
    pub(super) fn new(circles: Vec<(Point, i32)>, probe_radius: i32) -> Grid {
        let mut grid = Grid::default();
        grid.rebuild(circles.iter().copied(), probe_radius);
        grid
    }

    /// Builds the grid of `circles` for probes of at most `probe_radius` in place, reusing the
    /// memory of the grid it replaces: once the lists have grown to the largest crowd, a build
    /// allocates nothing.
    pub(super) fn rebuild(
        &mut self,
        circles: impl Iterator<Item = (Point, i32)>,
        probe_radius: i32,
    ) {
        self.circles.clear();
        self.circles.extend(circles);
        let circles = &self.circles;
        let xs = circles.iter().map(|c| i64::from(c.0.x.0));
        let ys = circles.iter().map(|c| i64::from(c.0.y.0));
        let (min_x, max_x) = (xs.clone().min().unwrap_or(0), xs.max().unwrap_or(0));
        let (min_y, max_y) = (ys.clone().min().unwrap_or(0), ys.max().unwrap_or(0));
        let largest = circles.iter().map(|c| i64::from(c.1)).max().unwrap_or(0);
        let cell = (i64::from(probe_radius) + largest).max(MIN_CELL);
        self.min_x = min_x;
        self.min_y = min_y;
        self.largest = largest;
        self.cell = cell;
        self.cols = (max_x - min_x) / cell + 1;
        self.rows = (max_y - min_y) / cell + 1;
        // A counting sort by cell: counts, then where each cell starts, then the members in
        // index order, so each cell's list ascends.
        let mut cells = std::mem::take(&mut self.cells);
        cells.clear();
        cells.extend(self.circles.iter().map(|c| self.cell_of(c.0)));
        self.start.clear();
        self.start.resize((self.cols * self.rows) as usize + 1, 0);
        for &c in &cells {
            self.start[c + 1] += 1;
        }
        for c in 1..self.start.len() {
            self.start[c] += self.start[c - 1];
        }
        self.next.clear();
        self.next.extend_from_slice(&self.start);
        self.members.clear();
        self.members.resize(cells.len(), 0);
        for (i, &c) in cells.iter().enumerate() {
            self.members[self.next[c] as usize] = i as u32;
            self.next[c] += 1;
        }
        self.cells = cells;
    }

    /// The column and row of `at`, which may lie outside the grid.
    fn col_row(&self, at: Point) -> (i64, i64) {
        let col = (i64::from(at.x.0) - self.min_x).div_euclid(self.cell);
        let row = (i64::from(at.y.0) - self.min_y).div_euclid(self.cell);
        (col, row)
    }

    /// The index of the cell of a circle's centre, which is always inside the grid.
    fn cell_of(&self, at: Point) -> usize {
        let (col, row) = self.col_row(at);
        (row * self.cols + col) as usize
    }

    /// Up to `most` circles other than `me`, into `out` (cleared first): those of `me`'s own cell,
    /// then of the 8 cells around it (west, east, north, south, north-west, south-east,
    /// north-east, south-west), each cell in ascending index order. Reading the own cell first
    /// and then opposite sides in turn keeps a full list from leaning to one side.
    ///
    /// In a grid built for probes as large as its largest circle, every circle that overlaps
    /// `me`'s has its centre in one of those cells.
    pub(super) fn neighbours(&self, me: usize, most: usize, out: &mut Vec<u32>) {
        const AROUND: [(i64, i64); 9] = [
            (0, 0),
            (-1, 0),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
        ];
        out.clear();
        let (col, row) = self.col_row(self.circles[me].0);
        for (dc, dr) in AROUND {
            let (c, r) = (col + dc, row + dr);
            if !(0..self.cols).contains(&c) || !(0..self.rows).contains(&r) {
                continue;
            }
            let cell = (r * self.cols + c) as usize;
            let (from, to) = (self.start[cell] as usize, self.start[cell + 1] as usize);
            for &i in &self.members[from..to] {
                if i as usize == me {
                    continue;
                }
                if out.len() == most {
                    return;
                }
                out.push(i);
            }
        }
    }

    /// Every circle that overlaps the probe circle at `at` with `reach`, in ascending index
    /// order, into `out` (cleared first). Any reach works: the cells searched are those a
    /// circle overlapping the probe can have its centre in, at most `reach` plus the largest
    /// radius away on each axis.
    pub(super) fn within(&self, at: Point, reach: i32, out: &mut Vec<usize>) {
        out.clear();
        if self.circles.is_empty() {
            return;
        }
        let span = i64::from(reach) + self.largest;
        let (x, y) = (i64::from(at.x.0), i64::from(at.y.0));
        let col = |v: i64| (v - self.min_x).div_euclid(self.cell);
        let row = |v: i64| (v - self.min_y).div_euclid(self.cell);
        let (c0, c1) = (col(x - span).max(0), col(x + span).min(self.cols - 1));
        let (r0, r1) = (row(y - span).max(0), row(y + span).min(self.rows - 1));
        if c0 > c1 || r0 > r1 {
            return;
        }
        for r in r0..=r1 {
            for c in c0..=c1 {
                let cell = (r * self.cols + c) as usize;
                let (from, to) = (self.start[cell] as usize, self.start[cell + 1] as usize);
                out.extend(
                    self.members[from..to]
                        .iter()
                        .map(|&i| i as usize)
                        .filter(|&i| {
                            let (centre, radius) = self.circles[i];
                            super::overlaps(at, reach, centre, radius)
                        }),
                );
            }
        }
        out.sort_unstable();
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::Grid;
    use crate::fixed::{Fx, Point};

    fn point(range: i32) -> impl Strategy<Value = Point> {
        (-range..=range, -range..=range).prop_map(|(x, y)| Point { x: Fx(x), y: Fx(y) })
    }

    /// Every circle the probe overlaps, in index order, as a plain scan finds them.
    fn scan_all(circles: &[(Point, i32)], at: Point, probe: i32) -> Vec<usize> {
        (0..circles.len())
            .filter(|&i| super::super::overlaps(at, probe, circles[i].0, circles[i].1))
            .collect()
    }

    /// A fixed-seed generator (SplitMix64 steps), so the layouts below are the same every run.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: i32) -> i32 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            ((z ^ (z >> 31)) % n as u64) as i32
        }
    }

    /// Layouts of the kinds the hit case has: a clump, a spread over a map, and circles on
    /// cell boundaries. Probes on boundaries (one unit either side), at the circles themselves
    /// and wider than a cell: the query finds what a plain scan finds, in index order.
    #[test]
    fn the_query_matches_the_scan_on_seeded_layouts() {
        let mut rng = Lcg(7);
        for seed in 0..40 {
            let (span, kind) = [(40_000, 0), (1_000_000, 1), (200_000, 2)][seed % 3];
            let shapes: Vec<(Point, i32)> = (0..150)
                .map(|_| {
                    let (x, y) = match kind {
                        2 => (rng.below(25) * 8_192, rng.below(25) * 8_192),
                        _ => (rng.below(span), rng.below(span)),
                    };
                    (Point { x: Fx(x), y: Fx(y) }, 1_000 + rng.below(3_000))
                })
                .collect();
            let grid = Grid::new(shapes.clone(), 512);
            let mut bits = Vec::new();
            let mut probes: Vec<(Point, i32)> = shapes.iter().map(|c| (c.0, 65_536)).collect();
            for _ in 0..60 {
                let edge = rng.below(25) * 8_192 + rng.below(3) - 1;
                let reach = [1_000, 8_192, 8_193, 65_536, 300_000][rng.below(5) as usize];
                probes.push((Point { x: Fx(edge), y: Fx(rng.below(span)) }, reach));
            }
            for (at, reach) in probes {
                let mut found = Vec::new();
                grid.within(at, reach, &mut bits, &mut found);
                assert_eq!(found, scan_all(&shapes, at, reach), "seed {seed}");
            }
        }
    }

    proptest! {
        // A range query for any reach, whatever probe radius the grid was built for: small
        // reaches, and reaches far wider than a cell.
        #[test]
        fn the_range_query_finds_what_the_scan_finds(
            range in prop_oneof![Just(20_000), Just(200_000)],
            circles in prop::collection::vec((point(200_000), 1..=65_536i32), 0..80),
            probes in prop::collection::vec((point(260_000), 1..=200_000i32), 1..20),
            probe_radius in 1..=2_048i32,
        ) {
            let shapes: Vec<(Point, i32)> = circles
                .into_iter()
                .map(|(p, r)| {
                    let p = Point { x: Fx(p.x.0 % range), y: Fx(p.y.0 % range) };
                    (p, if range == 20_000 { r % 4_096 + 1 } else { r })
                })
                .collect();
            let grid = Grid::new(shapes.clone(), probe_radius);
            let near = shapes.iter().map(|c| (c.0, 1));
            for (at, reach) in probes.into_iter().chain(near) {
                let mut found = Vec::new();
                grid.within(at, reach, &mut Vec::new(), &mut found);
                prop_assert_eq!(found, scan_all(&shapes, at, reach));
            }
        }
    }
}
