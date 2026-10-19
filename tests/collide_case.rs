//! The collision benchmark's tick (`benches/collide_case`) has the load it claims: every enemy
//! alive and unhurt, nearly `MAX_BULLETS_ALIVE` friendly bullets in the air, and four friendly
//! hit boxes active over the enemies (ADR-055).

// The benchmark's own module, which brings its own copy of `tests/common`; in the one test
// binary that copy sits beside `crate::common`, as it did when this file was a binary of its own.
#[path = "../benches/collide_case/mod.rs"]
#[allow(clippy::duplicate_mod, clippy::module_inception)]
mod collide_case;

use stella_rain_core::attack::PhaseKind;
use stella_rain_core::snapshot::{BulletView, EntityKind, EntityView};

/// The side of a cell of the enemy grid: the 32-pixel least, since a friendly bullet's radius and
/// the bat's (2 and 8 pixels) add up to less.
const CELL: i64 = 8_192;

/// How many friendly bullets are within one cell of the enemy grid, so that `Grid::first_hit`
/// reads its cells rather than returning at the bounds check, and how many enemies those bullets
/// are checked against: those of the 9 cells round each. The grid's corner is the smallest
/// enemy centre, as in `Grid::new`.
fn grid_load(enemies: &[&EntityView], bullets: &[BulletView]) -> (usize, usize) {
    let xs = enemies.iter().map(|e| i64::from(e.at.x.0));
    let ys = enemies.iter().map(|e| i64::from(e.at.y.0));
    let (min_x, min_y) = (xs.clone().min().unwrap(), ys.clone().min().unwrap());
    let cols = (xs.max().unwrap() - min_x) / CELL + 1;
    let rows = (ys.max().unwrap() - min_y) / CELL + 1;
    let cell_of = |x: i32, y: i32| {
        (
            (i64::from(x) - min_x).div_euclid(CELL),
            (i64::from(y) - min_y).div_euclid(CELL),
        )
    };
    let mut members = vec![0usize; (cols * rows) as usize];
    for e in enemies {
        let (c, r) = cell_of(e.at.x.0, e.at.y.0);
        members[(r * cols + c) as usize] += 1;
    }
    let (mut in_grid, mut checks) = (0, 0);
    for b in bullets.iter().filter(|b| b.friendly) {
        let (col, row) = cell_of(b.at.x.0, b.at.y.0);
        if !(-1..=cols).contains(&col) || !(-1..=rows).contains(&row) {
            continue;
        }
        in_grid += 1;
        for r in (row - 1).max(0)..=(row + 1).min(rows - 1) {
            for c in (col - 1).max(0)..=(col + 1).min(cols - 1) {
                checks += members[(r * cols + c) as usize];
            }
        }
    }
    (in_grid, checks)
}

#[test]
fn the_measured_tick_has_its_load() {
    let mut engine = collide_case::warmed_up();
    engine.step(collide_case::input(collide_case::WARMUP_TICKS + 1));
    let load = engine.load();
    assert_eq!(load.enemies, 500);
    assert!((3_900..4_000).contains(&load.bullets), "{load:?}");
    let snapshot = engine.snapshot();
    let enemies: Vec<_> = snapshot
        .entities
        .iter()
        .filter(|e| e.kind == EntityKind::Enemy)
        .collect();
    assert_eq!(enemies.len(), 500);
    assert!(
        enemies.iter().all(|e| e.hp == 100_000),
        "neither a friendly bullet nor a hit box hits an enemy"
    );
    // The crowd is at rest (ADR-058): a band round its line, no 32-pixel cell of it crowded.
    let mut cells: Vec<(i32, i32)> = enemies
        .iter()
        .map(|e| (e.at.x.0.div_euclid(8_192), e.at.y.0.div_euclid(8_192)))
        .collect();
    cells.sort_unstable();
    let fullest = cells.chunk_by(|a, b| a == b).map(<[_]>::len).max();
    let farthest = enemies
        .iter()
        .map(|e| (e.at.x.0 - collide_case::BAND_X).abs())
        .max();
    assert!(fullest <= Some(12), "{fullest:?} enemies in one cell");
    assert!(farthest <= Some(28_672), "{farthest:?} units from the line");
    // The bullets reach the enemy grid's cells (#100): within one cell of the grid, and checked
    // against the enemies of the 9 cells round them, none of which they hit.
    let (in_grid, checks) = grid_load(&enemies, &snapshot.bullets);
    assert!(
        in_grid >= 3_500,
        "{in_grid} friendly bullets reach the grid"
    );
    assert!(checks >= 12_000, "{checks} enemies checked");
    let guards: Vec<_> = snapshot
        .entities
        .iter()
        .filter(|e| e.kind == EntityKind::Companion)
        .collect();
    assert_eq!(guards.len(), 4);
    for g in guards {
        let a = g.attack.as_ref().expect("an attack runs");
        assert_eq!(a.kind, PhaseKind::Active);
        assert!(a.hit_box.is_some());
    }
}
