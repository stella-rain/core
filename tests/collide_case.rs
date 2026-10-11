//! The collision benchmark's tick (`benches/collide_case`) has the load it claims: every enemy
//! alive and unhurt, and nearly `MAX_BULLETS_ALIVE` friendly bullets in the air.

#[path = "../benches/collide_case/mod.rs"]
mod collide_case;

use stella_rain_core::snapshot::EntityKind;

#[test]
fn the_measured_tick_has_its_load() {
    let mut engine = collide_case::warmed_up();
    engine.step(collide_case::input(collide_case::WARMUP_TICKS + 1));
    let load = engine.load();
    assert_eq!(load.enemies, 300);
    assert!(load.bullets >= 3_900, "{load:?}");
    let snapshot = engine.snapshot();
    let enemies: Vec<_> = snapshot
        .entities
        .iter()
        .filter(|e| e.kind == EntityKind::Enemy)
        .collect();
    assert_eq!(enemies.len(), 300);
    assert!(
        enemies.iter().all(|e| e.hp == 100_000),
        "no friendly bullet hits an enemy"
    );
}
