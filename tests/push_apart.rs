//! Enemies push apart (ADR-058, decision 3): one pass a tick after every agent has moved and
//! acted. Each enemy adds up half of each counted overlap, along the line from the neighbour's
//! centre to its own; the sum is clamped to 4 pixels (1,024 units) and every enemy moves at once.
//! Hurt circles overlap, each radius counted as at least 8 pixels (2,048 units).

use crate::common;

use common::*;
use proptest::prelude::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::Stage;

/// The push's largest step, and the least radius it counts.
const STEP: i64 = 1_024;
const LEAST: i32 = 2_048;

/// An enemy that never moves by itself.
fn still() -> Value {
    json!({ "type": "straight", "vx": 0, "vy": 0 })
}

fn enemy_of(radius: i32, movement: Value) -> Value {
    json!({ "base": "bat", "hp": 50, "radius": radius, "movement": movement, "rules": [] })
}

/// Enemies that stand still, each `(x, y, radius)`, all spawned on tick 1: entities 2, 3, ...
/// after the far boss.
fn standing(enemies: &[(i32, i32, i32)]) -> Stage {
    let waves = enemies
        .iter()
        .map(|&(x, y, r)| wave(1, 1, 0, enemy_of(r, still()), x, y))
        .collect();
    quiet(stage_full(18_000, Some(far_boss()), waves, vec![]))
}

/// Where the enemies are, in entity-ID order.
fn enemies_at(e: &Engine) -> Vec<(i32, i32)> {
    e.snapshot()
        .entities
        .iter()
        .filter(|v| v.kind == EntityKind::Enemy)
        .map(|v| (v.at.x.0, v.at.y.0))
        .collect()
}

fn after(stage: &Stage, ticks: u32) -> Vec<(i32, i32)> {
    let mut e = engine(stage);
    for _ in 0..ticks {
        e.step(idle());
    }
    enemies_at(&e)
}

fn dist(a: (i32, i32), b: (i32, i32)) -> i64 {
    (i64::from(a.0 - b.0).pow(2) + i64::from(a.1 - b.1).pow(2)).isqrt()
}

// --- How far ---------------------------------------------------------------------------------

#[test]
fn two_overlapping_enemies_each_move_half_the_overlap() {
    // 10 pixels apart with 8-pixel radii: 6 pixels (1,536 units) of overlap, 768 each.
    let stage = standing(&[(40_000, 50_000, 2_048), (42_560, 50_000, 2_048)]);
    assert_eq!(after(&stage, 1), [(39_232, 50_000), (43_328, 50_000)]);
    // They touch now, and touching is not overlapping.
    assert_eq!(after(&stage, 5), [(39_232, 50_000), (43_328, 50_000)]);
}

#[test]
fn enemies_that_do_not_overlap_are_not_pushed() {
    let stage = standing(&[(40_000, 50_000, 2_048), (44_096, 50_000, 2_048)]);
    assert_eq!(after(&stage, 3), [(40_000, 50_000), (44_096, 50_000)]);
}

#[test]
fn a_radius_counts_as_at_least_eight_pixels_for_pushing() {
    // One-pixel hurt circles 10 pixels apart do not touch, and are pushed as 8-pixel ones are.
    let stage = standing(&[(40_000, 50_000, 256), (42_560, 50_000, 256)]);
    assert_eq!(after(&stage, 1), [(39_232, 50_000), (43_328, 50_000)]);
    // The hurt radius itself is unchanged.
    let mut e = engine(&stage);
    e.step(idle());
    let s = e.snapshot();
    assert!(
        s.entities
            .iter()
            .filter(|v| v.kind == EntityKind::Enemy)
            .all(|v| v.radius.0 == 256)
    );
}

#[test]
fn a_larger_hurt_circle_pushes_from_farther() {
    // 16 and 8 pixels of radius, 20 pixels apart along y: 4 pixels of overlap, 2 each.
    let stage = standing(&[(40_000, 50_000, 4_096), (40_000, 55_120, 2_048)]);
    assert_eq!(after(&stage, 1), [(40_000, 49_488), (40_000, 55_632)]);
}

#[test]
fn the_push_is_clamped_to_four_pixels_a_tick() {
    // Two 256-pixel enemies a pixel apart overlap by 511 pixels; each moves 4 a tick.
    let stage = standing(&[(40_000, 50_000, 65_536), (40_256, 50_000, 65_536)]);
    assert_eq!(after(&stage, 1), [(38_976, 50_000), (41_280, 50_000)]);
    assert_eq!(after(&stage, 3), [(36_928, 50_000), (43_328, 50_000)]);
}

#[test]
fn pushes_from_several_neighbours_add_up() {
    // The middle enemy is pushed down by both of its upper neighbours and sideways by neither:
    // they mirror each other. 3-4-5 triangles: each neighbour is 2,560 units away (1,536 across,
    // 2,048 up), an overlap of 1,536, so a push of 768 that is 614 down (768 * 4 / 5, rounded
    // toward zero); two of them make 1,228, clamped to 1,024.
    let stage = standing(&[
        (40_000, 50_000, 2_048),
        (38_464, 47_952, 2_048),
        (41_536, 47_952, 2_048),
    ]);
    assert_eq!(after(&stage, 1)[0], (40_000, 51_024));
}

// --- Enemies on the same point ---------------------------------------------------------------

#[test]
fn two_enemies_on_one_point_part_opposite_ways_along_the_higher_ids_angle() {
    let at = (40_000, 50_000);
    let stage = standing(&[(at.0, at.1, 2_048), (at.0, at.1, 2_048)]);
    let now = after(&stage, 1);
    // Entity 3 is the higher ID: 3 golden-angle steps of 137.508 degrees are 52.52 degrees,
    // whose cosine and sine are 0.6085 and 0.7935. Half the 16-pixel overlap is more than the
    // 4 pixels a tick, so each moves 1,024 units.
    let (dx, dy) = (now[1].0 - at.0, now[1].1 - at.1);
    assert!((dx - 623).abs() <= 2 && (dy - 812).abs() <= 2, "{dx}, {dy}");
    // The lower ID goes exactly the other way.
    assert_eq!((now[0].0 - at.0, now[0].1 - at.1), (-dx, -dy));
    // From then on they part along the line between them, until they touch.
    let later = after(&stage, 4);
    assert!(
        (4_090..=4_100).contains(&dist(later[0], later[1])),
        "{later:?}"
    );
    assert_eq!(after(&stage, 30), later);
}

#[test]
fn a_stack_on_one_point_opens() {
    let stage = standing(&[(40_000, 50_000, 2_048); 40]);
    let now = after(&stage, 120);
    for (i, a) in now.iter().enumerate() {
        for b in &now[i + 1..] {
            assert!(dist(*a, *b) >= 2 * i64::from(LEAST) - 8, "{a:?} and {b:?}");
        }
    }
}

// --- Yielding --------------------------------------------------------------------------------

#[test]
fn an_overlapped_enemy_does_not_walk_back_into_the_crowd_on_the_next_tick() {
    // Two enemies hold the point they spawn on. Tick 1 parts them 4 pixels each; they overlap by
    // more than a pixel, so on tick 2 neither walks back, and the push parts them 4 more: they
    // touch. Walking back each tick, they would never be more than 8 pixels apart.
    let (x, y) = (40_000, 50_000);
    let waves = vec![wave(1, 1, 0, enemy_of(2_048, hold(x, y)), x, y); 2];
    let stage = quiet(stage_full(18_000, Some(far_boss()), waves, vec![]));
    let now = after(&stage, 2);
    assert!((4_088..=4_100).contains(&dist(now[0], now[1])), "{now:?}");
    // Touching, they are free to walk again, and are parted again: never nearer than 8 pixels.
    let mut e = engine(&stage);
    for tick in 1..=60 {
        e.step(idle());
        let now = enemies_at(&e);
        assert!(dist(now[0], now[1]) >= 2_040, "tick {tick}: {now:?}");
    }
}

#[test]
fn an_enemy_alone_at_its_point_is_not_held_back() {
    // Nothing overlaps it, so it walks its 4 pixels a tick.
    let waves = vec![wave(
        1,
        1,
        0,
        enemy_of(2_048, hold(40_000, 60_240)),
        40_000,
        50_000,
    )];
    let stage = quiet(stage_full(18_000, Some(far_boss()), waves, vec![]));
    assert_eq!(after(&stage, 10), [(40_000, 60_240)]);
}

#[test]
fn an_overlapped_straight_enemy_keeps_flying() {
    // Yielding is for enemies that go to a place (Hold, Follow, Orbit). Two that fly down 100 a
    // tick from one point are pushed apart, opposite ways, and both keep their 100 a tick.
    let fly = json!({ "type": "straight", "vx": 0, "vy": 100 });
    let waves = vec![wave(1, 1, 0, enemy_of(2_048, fly), 40_000, 50_000); 2];
    let stage = quiet(stage_full(18_000, Some(far_boss()), waves, vec![]));
    for ticks in 1..=6 {
        let now = after(&stage, ticks);
        assert_eq!(now[0].1 + now[1].1, 2 * (50_000 + 100 * ticks as i32));
    }
}

// --- Who -------------------------------------------------------------------------------------

#[test]
fn the_main_character_companions_and_the_boss_neither_push_nor_are_pushed() {
    // One enemy stands on the main character, one on a companion and one on the boss.
    let guard = companion(50, hold(20_000, 100_000), json!([]));
    let boss = json!({ "base": "golem", "hp": 1_000_000, "radius": 8_192,
                       "spawn": { "x": 60_000, "y": 30_000 }, "phases": [{}] });
    let waves = vec![
        wave(1, 1, 0, enemy_of(2_048, still()), START_X, START_Y),
        wave(1, 1, 0, enemy_of(2_048, still()), 20_000, 100_000),
        wave(1, 1, 0, enemy_of(2_048, still()), 60_000, 30_000),
    ];
    let mut e = engine(&quiet(stage_full(18_000, Some(boss), waves, vec![guard])));
    for _ in 0..5 {
        e.step(idle());
    }
    assert_eq!(
        enemies_at(&e),
        [(START_X, START_Y), (20_000, 100_000), (60_000, 30_000)]
    );
    assert_eq!(player_at(&e), (START_X, START_Y));
    let s = e.snapshot();
    let at_of = |kind: EntityKind| {
        let v = s.entities.iter().find(|v| v.kind == kind).expect("there");
        (v.at.x.0, v.at.y.0)
    };
    assert_eq!(at_of(EntityKind::Companion), (20_000, 100_000));
    assert_eq!(at_of(EntityKind::Boss), (60_000, 30_000));
}

#[test]
fn a_frozen_enemy_is_pushed_too() {
    // Both walk down 100 a tick and start an attack on tick 1 whose phase stops them
    // (`move_pct` 0) for 10 ticks. They share a point, so the push still parts them.
    let walker = json!({ "base": "bat", "hp": 50, "radius": 2_048,
        "movement": { "type": "straight", "vx": 0, "vy": 100 },
        "rules": [{ "when": always(), "target": "player", "once": true,
                    "do": { "type": "attack", "attack": { "inline": [
                        { "kind": "active", "ticks": 10, "move_pct": 0 }] } } }] });
    let waves = vec![
        wave(1, 1, 0, walker.clone(), 40_000, 50_000),
        wave(1, 1, 0, walker, 40_000, 50_000),
    ];
    let mut e = engine(&quiet(stage_full(18_000, Some(far_boss()), waves, vec![])));
    for _ in 0..6 {
        e.step(idle());
    }
    let now = enemies_at(&e);
    assert!((4_090..=4_100).contains(&dist(now[0], now[1])), "{now:?}");
    // Neither walked: together they are still where they stood after their one step.
    assert_eq!(now[0].1 + now[1].1, 2 * 50_100);
}

// --- The map ---------------------------------------------------------------------------------

#[test]
fn a_push_never_carries_an_enemy_off_the_map() {
    // The left one would be pushed 4 pixels past the left edge; it stays on the edge.
    let stage = standing(&[(0, 50_000, 2_048), (1_000, 50_000, 2_048)]);
    assert_eq!(after(&stage, 1), [(0, 50_000), (2_024, 50_000)]);
}

#[test]
fn an_enemy_already_off_the_map_is_pushed_freely() {
    let stage = standing(&[(-3_000, 50_000, 2_048), (-2_000, 50_000, 2_048)]);
    assert_eq!(after(&stage, 1), [(-4_024, 50_000), (-976, 50_000)]);
}

// --- Crowds ----------------------------------------------------------------------------------

/// The most enemies whose centres share one 32-pixel square of a fixed lattice.
fn fullest_cell(at: &[(i32, i32)]) -> usize {
    let mut cells: Vec<(i32, i32)> = at
        .iter()
        .map(|p| (p.0.div_euclid(8_192), p.1.div_euclid(8_192)))
        .collect();
    cells.sort_unstable();
    cells
        .chunk_by(|a, b| a == b)
        .map(<[_]>::len)
        .max()
        .unwrap_or(0)
}

#[test]
fn a_wave_that_holds_one_point_becomes_a_crowd() {
    let (x, y) = (46_080, 60_000);
    let waves = vec![wave(1, 100, 1, enemy_of(2_048, hold(x, y)), x, y)];
    let mut e = engine(&quiet(stage_full(18_000, Some(far_boss()), waves, vec![])));
    for _ in 0..400 {
        e.step(idle());
    }
    let now = enemies_at(&e);
    assert_eq!(now.len(), 100);
    // Unpushed, all 100 would share the point, and one cell.
    assert!(
        fullest_cell(&now) <= 12,
        "{} in one cell",
        fullest_cell(&now)
    );
    // The crowd stays round its point: 100 circles of 8 pixels fill a disc of about 80.
    let farthest = now.iter().map(|p| dist(*p, (x, y))).max().unwrap_or(0);
    assert!(farthest <= 30_000, "{farthest} units from the point");
}

#[test]
fn a_horde_that_follows_the_main_character_does_not_close_into_a_point() {
    let follow = json!({ "type": "follow", "distance": 0 });
    let waves = vec![wave(1, 100, 1, enemy_of(2_048, follow), 46_080, 60_000)];
    let mut e = engine(&quiet(stage_full(18_000, Some(far_boss()), waves, vec![])));
    for _ in 0..600 {
        e.step(idle());
    }
    let now = enemies_at(&e);
    assert_eq!(now.len(), 100);
    assert!(
        fullest_cell(&now) <= 12,
        "{} in one cell",
        fullest_cell(&now)
    );
}

// --- Invariants ------------------------------------------------------------------------------

/// A movement and the farthest it can carry an enemy in a tick, in units.
fn movement() -> impl Strategy<Value = (Value, i64)> {
    prop_oneof![
        (30_000..60_000i32, 40_000..80_000i32).prop_map(|(x, y)| (hold(x, y), STEP)),
        (0..20_000i32).prop_map(|d| (json!({ "type": "follow", "distance": d }), STEP)),
        (-600..=600i32, -600..=600i32).prop_map(|(vx, vy)| {
            let far = (i64::from(vx).pow(2) + i64::from(vy).pow(2)).isqrt() + 1;
            (json!({ "type": "straight", "vx": vx, "vy": vy }), far)
        }),
        // The orbit step is the chord of 1.5 degrees: less than 0.0262 of the radius.
        (2_000..30_000i32).prop_map(|r| {
            let chord = i64::from(r) * 262 / 10_000 + 2;
            (json!({ "type": "orbit", "radius": r }), chord + STEP)
        }),
    ]
}

/// Waves of enemies that spawn close together, so they overlap and push from the first tick.
fn horde() -> impl Strategy<Value = Vec<(Value, i64, i32, u16, i32, i32)>> {
    prop::collection::vec(
        (
            movement(),
            prop_oneof![Just(256), Just(2_048), 1_000..20_000i32],
            1..=20u16,
            40_000..48_000i32,
            50_000..58_000i32,
        )
            .prop_map(|((m, far), r, count, x, y)| (m, far, r, count, x, y)),
        1..=6,
    )
}

fn steps() -> impl Strategy<Value = Vec<Input>> {
    prop::collection::vec((-127..=127i8, -127..=127i8, 1usize..=40), 1..=6).prop_map(|runs| {
        runs.into_iter()
            .flat_map(|(x, y, ticks)| std::iter::repeat_n(moving(x, y), ticks))
            .take(120)
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// From one tick to the next, no enemy is farther from where it was than its own movement
    /// carries it plus the push's 4 pixels (and 2 units of rounding).
    #[test]
    fn no_enemy_moves_more_than_its_movement_plus_four_pixels(
        horde in horde(),
        inputs in steps(),
    ) {
        let waves = horde
            .iter()
            .map(|(m, _, r, count, x, y)| wave(1, *count, 1, enemy_of(*r, m.clone()), *x, *y))
            .collect();
        let mut e = engine(&quiet(stage_full(18_000, Some(far_boss()), waves, vec![])));
        // Enemies keep their wave's asset and radius, but not its index: each wave's reach is
        // looked up by the enemy's entity ID, in the order the waves spawn them.
        let mut reach: std::collections::BTreeMap<u32, i64> = std::collections::BTreeMap::new();
        let mut was: std::collections::BTreeMap<u32, (i32, i32)> = std::collections::BTreeMap::new();
        let mut next_id = 2;
        for (tick, input) in (1u32..).zip(&inputs) {
            for (_, far, _, count, _, _) in &horde {
                if tick <= u32::from(*count) {
                    reach.insert(next_id, *far);
                    next_id += 1;
                }
            }
            e.step(*input);
            let s = e.snapshot();
            let mut now = std::collections::BTreeMap::new();
            for v in s.entities.iter().filter(|v| v.kind == EntityKind::Enemy) {
                let at = (v.at.x.0, v.at.y.0);
                if let Some(before) = was.get(&v.id.0) {
                    let most = reach[&v.id.0] + STEP + 2;
                    prop_assert!(
                        dist(*before, at) <= most,
                        "enemy {} went from {:?} to {:?} on tick {}, more than {}",
                        v.id.0, before, at, tick, most
                    );
                }
                now.insert(v.id.0, at);
            }
            was = now;
        }
    }
}
