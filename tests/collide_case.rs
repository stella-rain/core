//! The collision benchmark's tick (`benches/collide_case`) has the load it claims: every enemy
//! alive and unhurt, nearly `MAX_BULLETS_ALIVE` friendly bullets in the air, and four friendly
//! hit boxes active over the enemies (ADR-055).

// The benchmark's own module, which brings its own copy of `tests/common`; in the one test
// binary that copy sits beside `crate::common`, as it did when this file was a binary of its own.
#[path = "../benches/collide_case/mod.rs"]
#[allow(clippy::duplicate_mod, clippy::module_inception)]
mod collide_case;

use stella_rain_core::attack::PhaseKind;
use stella_rain_core::engine::Engine;
use stella_rain_core::snapshot::{BulletView, EntityKind, EntityView};
use stella_rain_core::validate::limits::MAX_MAP_SIDE;

/// The largest map (core#87), and the spread case's two far corners: 128 pixels in from the right
/// edge, at the top and at the bottom.
const SIDE: i32 = MAX_MAP_SIDE;
const TOP_RIGHT: (i32, i32) = (SIDE - 32_768, 32_768);
const BOTTOM_RIGHT: (i32, i32) = (SIDE - 32_768, SIDE - 32_768);

/// The spread case (`collide_case::spread_stage`): five waves, the first at the start point, two
/// at the top-right corner and two at the bottom-right one, 500 enemies in all, on a map of the
/// largest side that the validator accepts.
#[test]
fn the_spread_waves_stand_at_the_start_and_two_far_corners() {
    let stage = collide_case::spread_stage();
    let map = stage.map.as_ref().expect("the spread case has a map");
    assert_eq!((map.width.0, map.height.0), (SIDE, SIDE));
    assert_eq!(
        map.start.map(|p| (p.x.0, p.y.0)),
        Some(collide_case::START)
    );
    let waves: Vec<(i32, i32, u16)> = stage
        .waves
        .iter()
        .map(|w| (w.spawn.x.0, w.spawn.y.0, w.count))
        .collect();
    assert_eq!(waves.len(), 5, "{waves:?}");
    let (x, y, _) = waves[0];
    assert_eq!((x, y), (collide_case::SPREAD_START_X, collide_case::START.1));
    for wave in &waves[1..3] {
        assert_eq!((wave.0, wave.1), TOP_RIGHT);
    }
    for wave in &waves[3..] {
        assert_eq!((wave.0, wave.1), BOTTOM_RIGHT);
    }
    let total: u32 = waves.iter().map(|w| u32::from(w.2)).sum();
    assert_eq!(total, 500);
    Engine::new(&stage).expect("the spread stage passes the validator");
}

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

/// The spread case's measured tick (`collide_case::spread_stage`) has the clustered tick's load,
/// with its crowds reaching the corners: every enemy alive and unhurt, nearly `MAX_BULLETS_ALIVE`
/// friendly bullets in the air, and the crowds at the map's far edges (core#146).
#[test]
fn the_spread_tick_has_its_load() {
    let mut engine = collide_case::spread_warmed_up();
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
    // The crowds reach the far edges: the top-right and bottom-right corners' crowds are within
    // 256 pixels of the right edge and of the top and bottom edges.
    let max_x = enemies.iter().map(|e| e.at.x.0).max();
    let min_y = enemies.iter().map(|e| e.at.y.0).min();
    let max_y = enemies.iter().map(|e| e.at.y.0).max();
    assert!(max_x >= Some(SIDE - 65_536), "{max_x:?}");
    assert!(min_y <= Some(65_536), "{min_y:?}");
    assert!(max_y >= Some(SIDE - 65_536), "{max_y:?}");
}
