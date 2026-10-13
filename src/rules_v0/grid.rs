//! A uniform grid over the enemies, so that a friendly bullet is checked only against the enemies
//! near it rather than all of them (core#62). It is rebuilt in each tick's `collide`, where the
//! enemies do not move, and it picks exactly the enemy the plain scan in index order picks: the
//! lowest-index live enemy whose circle overlaps the bullet's. Hit boxes (ADR-055) ask it for
//! every enemy within their reach instead (`within`), and the push-apart (ADR-058) for the
//! enemies around one (`neighbours`).

use crate::fixed::Point;

/// The smallest cell side: 32 pixels, so small radii do not make thousands of cells.
const MIN_CELL: i64 = 8192;

/// Circles (centre, radius) bucketed into square cells, each cell listing its circles in index
/// order. A cell is at least as wide as the probe radius plus the largest circle radius, so a
/// probe can only overlap circles in its own cell or the 8 around it.
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
}

impl Grid {
    /// The grid of `circles` for probes of at most `probe_radius`.
    pub(super) fn new(circles: Vec<(Point, i32)>, probe_radius: i32) -> Grid {
        let xs = circles.iter().map(|c| i64::from(c.0.x.0));
        let ys = circles.iter().map(|c| i64::from(c.0.y.0));
        let (min_x, max_x) = (xs.clone().min().unwrap_or(0), xs.max().unwrap_or(0));
        let (min_y, max_y) = (ys.clone().min().unwrap_or(0), ys.max().unwrap_or(0));
        let largest = circles.iter().map(|c| i64::from(c.1)).max().unwrap_or(0);
        let cell = (i64::from(probe_radius) + largest).max(MIN_CELL);
        let cols = (max_x - min_x) / cell + 1;
        let rows = (max_y - min_y) / cell + 1;
        let mut grid = Grid {
            circles,
            min_x,
            min_y,
            largest,
            cell,
            cols,
            rows,
            start: Vec::new(),
            members: Vec::new(),
        };
        // A counting sort by cell: counts, then where each cell starts, then the members in
        // index order, so each cell's list ascends.
        let cells: Vec<usize> = grid.circles.iter().map(|c| grid.cell_of(c.0)).collect();
        let mut start = vec![0u32; (cols * rows) as usize + 1];
        for &c in &cells {
            start[c + 1] += 1;
        }
        for c in 1..start.len() {
            start[c] += start[c - 1];
        }
        let mut next = start.clone();
        let mut members = vec![0u32; cells.len()];
        for (i, &c) in cells.iter().enumerate() {
            members[next[c] as usize] = i as u32;
            next[c] += 1;
        }
        grid.start = start;
        grid.members = members;
        grid
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

    /// The lowest index of a circle that `alive` accepts and that overlaps the probe circle at
    /// `at` with `probe_radius` (at most the radius the grid was built for).
    ///
    /// A circle that overlaps the probe has its centre within one cell side of `at` on each
    /// axis, so its cell is `at`'s cell or a neighbour, and a probe more than one cell outside
    /// the grid overlaps nothing. Each cell lists its circles in ascending index order, so the
    /// first hit in a cell is that cell's lowest, and the lowest over the 9 cells is the scan's
    /// choice.
    ///
    /// Always inlined: it runs once per friendly bullet, and out of line it costs about 10% more
    /// instructions in `collide_tick`.
    #[inline(always)]
    pub(super) fn first_hit(
        &self,
        at: Point,
        probe_radius: i32,
        alive: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        let (col, row) = self.col_row(at);
        if self.circles.is_empty()
            || !(-1..=self.cols).contains(&col)
            || !(-1..=self.rows).contains(&row)
        {
            return None;
        }
        let mut best: Option<usize> = None;
        for r in (row - 1).max(0)..=(row + 1).min(self.rows - 1) {
            for c in (col - 1).max(0)..=(col + 1).min(self.cols - 1) {
                let cell = (r * self.cols + c) as usize;
                let (from, to) = (self.start[cell] as usize, self.start[cell + 1] as usize);
                for &i in &self.members[from..to] {
                    let i = i as usize;
                    if best.is_some_and(|b| i >= b) {
                        break;
                    }
                    let (centre, radius) = self.circles[i];
                    if alive(i) && super::overlaps(at, probe_radius, centre, radius) {
                        best = Some(i);
                        break;
                    }
                }
            }
        }
        best
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

    /// The plain scan the grid replaces.
    fn scan(circles: &[(Point, i32)], alive: &[bool], at: Point, probe: i32) -> Option<usize> {
        (0..circles.len())
            .find(|&i| alive[i] && super::super::overlaps(at, probe, circles[i].0, circles[i].1))
    }

    fn point(range: i32) -> impl Strategy<Value = Point> {
        (-range..=range, -range..=range).prop_map(|(x, y)| Point { x: Fx(x), y: Fx(y) })
    }

    /// Every circle the probe overlaps, in index order, as a plain scan finds them.
    fn scan_all(circles: &[(Point, i32)], at: Point, probe: i32) -> Vec<usize> {
        (0..circles.len())
            .filter(|&i| super::super::overlaps(at, probe, circles[i].0, circles[i].1))
            .collect()
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
                grid.within(at, reach, &mut found);
                prop_assert_eq!(found, scan_all(&shapes, at, reach));
            }
        }

        // Crowded (a few cells) and spread out (many cells), with radii up to the validator's 256
        // pixels, dead circles, and probes inside and far outside the circles' bounds.
        #[test]
        fn the_grid_picks_what_the_scan_picks(
            range in prop_oneof![Just(20_000), Just(200_000)],
            circles in prop::collection::vec((point(200_000), 1..=65_536i32, any::<bool>()), 0..80),
            probes in prop::collection::vec(point(260_000), 1..40),
            probe_radius in 1..=2_048i32,
        ) {
            let circles: Vec<(Point, i32, bool)> = circles
                .into_iter()
                .map(|(p, r, alive)| {
                    let p = Point { x: Fx(p.x.0 % range), y: Fx(p.y.0 % range) };
                    (p, if range == 20_000 { r % 4_096 + 1 } else { r }, alive)
                })
                .collect();
            let alive: Vec<bool> = circles.iter().map(|c| c.2).collect();
            let shapes: Vec<(Point, i32)> = circles.iter().map(|c| (c.0, c.1)).collect();
            let grid = Grid::new(shapes.clone(), probe_radius);
            // Probes at the circles' centres hit something whenever one is alive there.
            let near = shapes.iter().map(|c| c.0);
            for at in probes.into_iter().map(|p| Point { x: Fx(p.x.0 % (range + 60_000)), y: Fx(p.y.0 % (range + 60_000)) }).chain(near) {
                prop_assert_eq!(
                    grid.first_hit(at, probe_radius, |i| alive[i]),
                    scan(&shapes, &alive, at, probe_radius)
                );
            }
        }
    }
}
