//! The hit benchmark's tick (`bench_fixtures::hit_case`, core BR1b) has the load it claims: no
//! orb, the enemies of the cap alive and the crowd under the main character's melee hit
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
/// start and 440 above and below it, inside the earlier leash's 800 pixels (EC2, DS84).
const SIDE: i32 = MAX_MAP_SIDE;
const SPREAD_OFFSET: i32 = 112_640;
const TOP_RIGHT: (i32, i32) = (
    hit_case::START.0 + SPREAD_OFFSET,
    hit_case::START.1 - SPREAD_OFFSET,
);
const BOTTOM_RIGHT: (i32, i32) = (
    hit_case::START.0 + SPREAD_OFFSET,
    hit_case::START.1 + SPREAD_OFFSET,
);

/// The earlier leash's reach (800 pixels, EC2, DS84; the leash is now `rules.json`'s view
/// rectangle), and the least extent of the spread crowd after the warm-up, in units: 700 pixels on
/// y. The start is 238 pixels from the map's left edge, so x cannot span 700 pixels; it measures
/// 160,883 (about 630 pixels), and the least is EC2's 140,000.
const LEASH_REACH: i64 = 204_800;
const LEAST_EXTENT_X: i32 = 140_000;
const LEAST_EXTENT_Y: i32 = 179_200;

/// The main character is entity 0.
const PLAYER: EntityId = EntityId(0);

/// The enemies of the run's cap (DIR1): 260 for the bundled pack.
fn cap(engine: &Engine) -> usize {
    engine.caps().enemies as usize
}

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
    assert_eq!(load.enemies, engine.caps().enemies, "{load:?}");
    assert_eq!(load.orbs, 0, "{load:?}");
    assert_eq!(load.orbs, 0, "{load:?}");
    assert_eq!(load.zones, 128, "{load:?}");
    let enemies = enemies(engine);
    assert_eq!(enemies.len(), cap(engine));
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
    assert!(
        by_player >= cap(&engine) * 9 / 10,
        "{by_player} hits by the main character"
    );
    let by_guards = hits_by(&engine, |s| s != PLAYER && s.0 < 6);
    assert!(
        by_guards >= 4 * (cap(&engine) * 18 / 100),
        "{by_guards} hits by the companions"
    );
    // The friendly zones cover the crowd: each tests nearly all the enemies every tick.
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
        assert!(
            near >= cap(&engine) * 9 / 10,
            "{near} enemies inside a friendly zone"
        );
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

/// The spread case (`hit_case::spread_stage`): three waves of the cap's 260 (100, 100 and 60), the
/// first at the start point, one at the upper right and one at the lower right, inside the leash,
/// on a map of the largest side that the validator accepts.
#[test]
fn the_spread_waves_stand_at_the_start_and_two_outer_crowds() {
    let stage = hit_case::spread_stage();
    let map = stage.map.as_ref().expect("the spread case has a map");
    assert_eq!((map.width.0, map.height.0), (SIDE, SIDE));
    assert_eq!(map.start.map(|p| (p.x.0, p.y.0)), Some(hit_case::START));
    let waves: Vec<(i32, i32, u16)> = stage
        .waves
        .iter()
        .map(|w| {
            (
                w.spawn.expect("a point").x.0,
                w.spawn.expect("a point").y.0,
                w.count,
            )
        })
        .collect();
    assert_eq!(waves.len(), 3, "{waves:?}");
    let (x, y, _) = waves[0];
    assert_eq!((x, y), hit_case::CROWD_AT);
    assert_eq!((waves[1].0, waves[1].1), TOP_RIGHT);
    assert_eq!((waves[2].0, waves[2].1), BOTTOM_RIGHT);
    let sizes: Vec<u16> = waves.iter().map(|w| w.2).collect();
    assert_eq!(sizes, [100, 100, 60]);
    let total: u32 = waves.iter().map(|w| u32::from(w.2)).sum();
    assert_eq!(total, Engine::new(&stage).unwrap().caps().enemies);
    Engine::new(&stage).expect("the spread stage passes the validator");
}

/// The spread tick has the clustered tick's load, with the main character's step and the zones
/// on the first crowd only and the other crowds spread inside the leash's reach.
#[test]
fn the_spread_tick_has_its_load() {
    let engine = measured(hit_case::spread_warmed_up());
    assert_load(&engine);
    assert_eq!(
        engine.load().relocated,
        0,
        "the leash moves none of the spread crowd"
    );
    let by_player = hits_by(&engine, |s| s == PLAYER);
    assert!(by_player >= 90, "{by_player} hits by the main character");
    let crowd = enemies(&engine);
    let player = engine.snapshot().player.at;
    let farthest = crowd
        .iter()
        .map(|e| {
            i64::try_from(
                (i64::from(e.at.x.0).abs_diff(i64::from(player.x.0)).pow(2)
                    + i64::from(e.at.y.0).abs_diff(i64::from(player.y.0)).pow(2))
                .isqrt(),
            )
            .unwrap()
        })
        .max();
    assert!(
        farthest <= Some(LEASH_REACH),
        "{farthest:?} units from the main character"
    );
    let extent = |axis: fn(&EntityView) -> i32| {
        let lo = crowd.iter().map(axis).min().unwrap_or(0);
        let hi = crowd.iter().map(axis).max().unwrap_or(0);
        hi - lo
    };
    let (ex, ey) = (extent(|e| e.at.x.0), extent(|e| e.at.y.0));
    assert!(ex >= LEAST_EXTENT_X, "x extent {ex}");
    assert!(ey >= LEAST_EXTENT_Y, "y extent {ey}");
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

const PINNED_CLUSTERED: [u64; 3] = [0xfc6d79094e9ba2ec, 0x4140aefa5bd33b0b, 0x41c58750ce40b74d];
const PINNED_SPREAD: [u64; 3] = [0x234c6506597903e7, 0xbbb9a1d90af63429, 0xe50028091d835cbd];
