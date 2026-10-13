//! The collision benchmark's tick (`benches/collide_case`) has the load it claims: every enemy
//! alive and unhurt, nearly `MAX_BULLETS_ALIVE` friendly bullets in the air, and four friendly
//! hit boxes active over the enemies (ADR-055).

#[path = "../benches/collide_case/mod.rs"]
mod collide_case;

use stella_rain_core::attack::PhaseKind;
use stella_rain_core::snapshot::EntityKind;

#[test]
fn the_measured_tick_has_its_load() {
    let mut engine = collide_case::warmed_up();
    engine.step(collide_case::input(collide_case::WARMUP_TICKS + 1));
    let load = engine.load();
    assert_eq!(load.enemies, 500);
    assert!(load.bullets >= 3_900, "{load:?}");
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
