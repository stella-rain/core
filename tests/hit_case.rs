//! The hit benchmark's tick (`bench_fixtures::hit_case`, core BR1b) has the load it claims: no
//! orb and no orb, 500 enemies alive and the crowd under the main character's melee hit
//! boxes (ADR-055) and 128 active zones (core Z1), clustered and spread over the largest map.
//! Enemy zones hurt only the main character and the companions, so the zones that load the
//! crowd are the friendly ones: 112 of the 128, the other 16 an enemy's.

use stella_rain_core::bench_fixtures::hit_case;

use stella_rain_core::attack::PhaseKind;
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::snapshot::{EntityKind, EntityView};
use stella_rain_core::validate::limits::MAX_MAP_SIDE;

/// The largest map (core#87), and the spread case's two outer crowds: 440 pixels right of the
/// start and 440 above and below it, inside the leash (EC2, DS84).
const SIDE: i32 = MAX_MAP_SIDE;
const TOP_RIGHT: (i32, i32) = (hit_case::START.0 + 112_640, hit_case::START.1 - 112_640);
const BOTTOM_RIGHT: (i32, i32) = (hit_case::START.0 + 112_640, hit_case::START.1 + 112_640);

/// The main character is entity 0.
const PLAYER: EntityId = EntityId(0);

fn measured(mut engine: Engine) -> Engine {
    engine.step(hit_case::input(hit_case::WARMUP_TICKS + 1));
    engine
}

fn enemies(engine: &Engine) -> Vec<EntityView> {
    engine
        .snapshot()
        .entities
        .into_iter()
        .filter(|e| e.kind == EntityKind::Enemy)
        .collect()
}

/// Hits of this tick by `source`.
fn hits_by(engine: &Engine, source: impl Fn(EntityId) -> bool) -> usize {
    engine
        .events()
        .iter()
        .filter(|ev| matches!(ev, DomainEvent::Hit { source: s, .. } if source(*s)))
        .count()
}

/// The load both cases share: 500 enemies alive and unbeaten, nothing in the air, 128 zones
/// active (112 friendly) and four companions in an active phase with a hit box.
fn assert_load(engine: &Engine) {
    let load = engine.load();
    assert_eq!(load.enemies, 500, "{load:?}");
    assert_eq!(load.orbs, 0, "{load:?}");
    assert_eq!(load.orbs, 0, "{load:?}");
    assert_eq!(load.zones, 128, "{load:?}");
    let enemies = enemies(engine);
    assert_eq!(enemies.len(), 500);
    assert!(enemies.iter().all(|e| e.hp > 0), "the crowd is unbeaten");
    let snapshot = engine.snapshot();
    assert!(snapshot.orbs.is_empty(), "nothing is in the air");
    assert_eq!(snapshot.zones.len(), 128);
    assert!(snapshot.zones.iter().all(|z| z.phase == PhaseKind::Active));
    assert_eq!(snapshot.zones.iter().filter(|z| z.friendly).count(), 112);
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

#[test]
fn the_measured_tick_has_its_load() {
    let engine = measured(hit_case::warmed_up());
    assert_load(&engine);
    // The main character's step lands on nearly the whole crowd, and the companions' arcs on a
    // good part of it, this tick.
    let by_player = hits_by(&engine, |s| s == PLAYER);
    assert!(by_player >= 450, "{by_player} hits by the main character");
    let by_guards = hits_by(&engine, |s| s != PLAYER && s.0 < 6);
    assert!(by_guards >= 4 * 90, "{by_guards} hits by the companions");
    // The friendly zones cover the crowd: each tests nearly all 500 enemies every tick.
    let crowd = enemies(&engine);
    let snapshot = engine.snapshot();
    for z in snapshot.zones.iter().filter(|z| z.friendly) {
        let near = crowd
            .iter()
            .filter(|e| {
                let (dx, dy) = (
                    i64::from(e.at.x.0 - z.at.x.0),
                    i64::from(e.at.y.0 - z.at.y.0),
                );
                dx * dx + dy * dy <= i64::from(hit_case::ZONE_RADIUS).pow(2)
            })
            .count();
        assert!(near >= 450, "{near} enemies inside a friendly zone");
    }
    // The crowd is a clump round its point, not spread over the map.
    let (x, y) = hit_case::CROWD_AT;
    let farthest = crowd
        .iter()
        .map(|e| (i64::from(e.at.x.0 - x).pow(2) + i64::from(e.at.y.0 - y).pow(2)).isqrt())
        .max();
    assert!(
        farthest <= Some(60_000),
        "{farthest:?} units from the point"
    );
}

/// The spread case (`hit_case::spread_stage`): five waves, the first at the start point, two at
/// the upper right and two at the lower right, inside the leash, 500 enemies in all, on a map of the
/// largest side that the validator accepts.
#[test]
fn the_spread_waves_stand_at_the_start_and_two_far_corners() {
    let stage = hit_case::spread_stage();
    let map = stage.map.as_ref().expect("the spread case has a map");
    assert_eq!((map.width.0, map.height.0), (SIDE, SIDE));
    assert_eq!(map.start.map(|p| (p.x.0, p.y.0)), Some(hit_case::START));
    let waves: Vec<(i32, i32, u16)> = stage
        .waves
        .iter()
        .map(|w| (w.spawn.x.0, w.spawn.y.0, w.count))
        .collect();
    assert_eq!(waves.len(), 5, "{waves:?}");
    let (x, y, _) = waves[0];
    assert_eq!((x, y), hit_case::CROWD_AT);
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

/// The least extent of the spread crowd in x and in y after the warm-up, in units.
/// Measured 160,883 and 284,272 (about 630 and 1,110 pixels). DS84 asked for about 1,400 pixels in
/// both, which the map's left edge (the start is 238 pixels from it) and the leash's 800 pixels
/// leave out of reach; these are the extents the case has.
const EXTENT_X: i32 = 140_000;
const EXTENT_Y: i32 = 250_000;

/// The spread tick has the clustered tick's load, with the main character's step and the zones
/// on the first crowd only and the other crowds spread inside the leash, so it relocates none.
#[test]
fn the_spread_tick_has_its_load() {
    let engine = measured(hit_case::spread_warmed_up());
    assert_load(&engine);
    let by_player = hits_by(&engine, |s| s == PLAYER);
    assert!(by_player >= 90, "{by_player} hits by the main character");
    let crowd = enemies(&engine);
    assert_eq!(
        engine.load().relocated,
        0,
        "the leash moves none of the spread crowd"
    );
    let extent = |axis: fn(&EntityView) -> i32| {
        let lo = crowd.iter().map(axis).min().unwrap_or(0);
        let hi = crowd.iter().map(axis).max().unwrap_or(0);
        hi - lo
    };
    // The start is 238 pixels from the left edge, so the crowd spans less in x than in y.
    let (ex, ey) = (extent(|e| e.at.x.0), extent(|e| e.at.y.0));
    assert!(ex >= EXTENT_X, "x extent");
    assert!(extent(|e| e.at.y.0) >= EXTENT_Y, "y extent");
}

/// The state hash of each of the ticks `WARMUP_TICKS + 1` to `+ 3` (the measured tick and two
/// after it), which a change to how hits are found must not move (core HB1).
fn hashes_after_warmup(mut engine: Engine) -> Vec<u64> {
    (1..=3)
        .map(|k| {
            engine.step(hit_case::input(hit_case::WARMUP_TICKS + k));
            engine.state_hash()
        })
        .collect()
}

#[test]
fn the_clustered_ticks_hash_as_they_did_with_every_enemy_tested() {
    let hashes = hashes_after_warmup(hit_case::warmed_up());
    assert_eq!(hashes, PINNED_CLUSTERED, "{hashes:x?}");
}

#[test]
fn the_spread_ticks_hash_as_they_did_with_every_enemy_tested() {
    let hashes = hashes_after_warmup(hit_case::spread_warmed_up());
    assert_eq!(hashes, PINNED_SPREAD, "{hashes:x?}");
}

const PINNED_CLUSTERED: [u64; 3] = [0x95527ebc68a22c23, 0x87e3b4b6e281d125, 0xe528b8947fab1e1d];
const PINNED_SPREAD: [u64; 3] = [0xab1fe8f19a88d366, 0x592a4927fce29a11, 0x58c3019d89b9c2a4];
