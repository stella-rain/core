//! A uniform grid over the enemies, so that a friendly hit box is checked only against the
//! enemies near it rather than all of them (core#62). It is rebuilt in each tick's `collide`,
//! where the enemies do not move. Hit boxes (ADR-055) ask it for every enemy within their reach
//! (`within`), and the push-apart (ADR-058) walks the enemies around one (`each_neighbour`).

use crate::fixed::Point;

/// The smallest cell side: 32 pixels, so small radii do not make thousands of cells.
const MIN_CELL: i64 = 8192;

/// A cell number as a table index; the grid is `cols * rows` cells, bounded by the horde caps.
fn cell_index(cell: i64) -> usize {
    usize::try_from(cell).expect("cell index is in range")
}

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
    /// Reserves the memory of a grid for up to `circles` circles on a map `width` by `height`
    /// (1/256 pixels), so that no `rebuild` allocates: the cells are at least `MIN_CELL` wide and
    /// the circles lie on the map, with a cell to spare on each side.
    pub(super) fn reserve(&mut self, circles: usize, width: i32, height: i32) {
        let cells = |side: i32| usize::try_from(i64::from(side) / MIN_CELL).unwrap_or(0) + 2;
        let cells = cells(width) * cells(height) + 1;
        super::scratch::reserve_to(&mut self.circles, circles);
        super::scratch::reserve_to(&mut self.cells, circles);
        super::scratch::reserve_to(&mut self.members, circles);
        super::scratch::reserve_to(&mut self.start, cells);
        super::scratch::reserve_to(&mut self.next, cells);
    }

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
        self.start.resize(cell_index(self.cols * self.rows) + 1, 0);
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
            self.members[self.next[c] as usize] = u32::try_from(i).expect("member index fits u32");
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
        cell_index(row * self.cols + col)
    }

    /// Calls `f` with up to `most` circles other than `me`: those of `me`'s own cell, then of the
    /// 8 cells around it (west, east, north, south, north-west, south-east, north-east,
    /// south-west), each cell in ascending index order. Reading the own cell first and then
    /// opposite sides in turn keeps a full list from leaning to one side. It stops at the
    /// first circle past `most`. `f` is called for every circle read, and each one counts toward
    /// `most` whatever `f` does with it, so skipping one inside `f` does not read another.
    ///
    /// In a grid built for probes as large as its largest circle, every circle that overlaps
    /// `me`'s has its centre in one of those cells.
    pub(super) fn each_neighbour(&self, me: usize, most: usize, mut f: impl FnMut(u32)) {
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
        let mut read = 0;
        let (col, row) = self.col_row(self.circles[me].0);
        for &(dc, dr) in &AROUND {
            let (c, r) = (col + dc, row + dr);
            if !(0..self.cols).contains(&c) || !(0..self.rows).contains(&r) {
                continue;
            }
            // The range check above puts `r * cols + c` in `0..cols * rows`, the length of
            // `start` less one: it is the hottest conversion in the tick (ADR-SIM-8).
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "0 <= c < cols and 0 <= r < rows, so the cell number is a valid index"
            )]
            let cell = (r * self.cols + c) as usize;
            let (from, to) = (self.start[cell] as usize, self.start[cell + 1] as usize);
            for &i in &self.members[from..to] {
                if i as usize == me {
                    continue;
                }
                if read == most {
                    return;
                }
                read += 1;
                f(i);
            }
        }
    }

    /// Every circle that overlaps the probe circle at `at` with `reach`, in ascending index
    /// order, into `out` (cleared first). Any reach works: the cells searched are those a
    /// circle overlapping the probe can have its centre in, at most `reach` plus the largest
    /// radius away on each axis.
    ///
    /// The cells are walked in any order: each circle found sets its bit in `bits` (scratch
    /// memory, one bit per circle), and the bits read back in ascending order. That dedups and
    /// sorts in one pass over the circles' count divided by 64, where sorting the list would cost
    /// a comparison per step of its log, so the order can never depend on the cell walk.
    pub(super) fn within(&self, at: Point, reach: i32, bits: &mut Vec<u64>, out: &mut Vec<usize>) {
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
        bits.clear();
        bits.resize(self.circles.len().div_ceil(64), 0);
        for r in r0..=r1 {
            // The cells of a row are one run of the member list: they are laid out cell by cell.
            let first = cell_index(r * self.cols + c0);
            let last = cell_index(r * self.cols + c1);
            let (from, to) = (self.start[first] as usize, self.start[last + 1] as usize);
            for &i in &self.members[from..to] {
                let (centre, radius) = self.circles[i as usize];
                if super::overlaps(at, reach, centre, radius) {
                    bits[(i / 64) as usize] |= 1 << (i % 64);
                }
            }
        }
        for (w, &word) in bits.iter().enumerate() {
            let mut word = word;
            while word != 0 {
                out.push(w * 64 + word.trailing_zeros() as usize);
                word &= word - 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{Grid, cell_index};
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
            i32::try_from((z ^ (z >> 31)) % u64::from(n.unsigned_abs())).expect("below n")
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
                let reach = [1_000, 8_192, 8_193, 65_536, 300_000]
                    [usize::try_from(rng.below(5)).expect("below 5")];
                probes.push((
                    Point {
                        x: Fx(edge),
                        y: Fx(rng.below(span)),
                    },
                    reach,
                ));
            }
            for (at, reach) in probes {
                let mut found = Vec::new();
                grid.within(at, reach, &mut bits, &mut found);
                assert_eq!(found, scan_all(&shapes, at, reach), "seed {seed}");
            }
        }
    }

    /// The walk as it was when it filled a list: the reference `each_neighbour` must match.
    fn neighbours(grid: &Grid, me: usize, most: usize) -> Vec<u32> {
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
        let mut out = Vec::new();
        let (col, row) = grid.col_row(grid.circles[me].0);
        for (dc, dr) in AROUND {
            let (c, r) = (col + dc, row + dr);
            if !(0..grid.cols).contains(&c) || !(0..grid.rows).contains(&r) {
                continue;
            }
            let cell = cell_index(r * grid.cols + c);
            let (from, to) = (grid.start[cell] as usize, grid.start[cell + 1] as usize);
            for &i in &grid.members[from..to] {
                if i as usize == me {
                    continue;
                }
                if out.len() == most {
                    return out;
                }
                out.push(i);
            }
        }
        out
    }

    /// `each_neighbour` names the same indices in the same order as the list walk, on the
    /// layouts of the query test, for caps from none to unlimited.
    #[test]
    fn each_neighbour_names_what_the_list_walk_named() {
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
            let grid = Grid::new(shapes.clone(), 4_000);
            for me in 0..shapes.len() {
                for most in [0, 1, 32, usize::MAX] {
                    let mut seen = Vec::new();
                    grid.each_neighbour(me, most, |i| seen.push(i));
                    assert_eq!(seen, neighbours(&grid, me, most), "seed {seed} me {me}");
                }
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
