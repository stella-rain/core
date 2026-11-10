//! An engine's immutable run data (`sim::World` and `RunContent`) is shared by a clone, not copied.

use std::sync::Arc;

use super::*;
use crate::bench_fixtures::{equip_case, party_case};

fn world_of(engine: &Engine) -> &Arc<crate::sim::World> {
    match engine.run() {
        Run::V2 { world, .. } => world,
        Run::V1 { .. } => panic!("the live rules run version 2"),
    }
}

fn engines() -> Vec<Engine> {
    let setup = RunSetup {
        seed: 1,
        ..RunSetup::default()
    };
    vec![
        Engine::start_party(&setup, 3).expect("the party starts"),
        equip_case::saved(),
    ]
}

#[test]
fn an_engine_clone_shares_its_world_and_content() {
    for mut e in engines() {
        let mut c = e.clone();
        let same = |e: &Engine, c: &Engine| {
            assert!(std::ptr::eq(e.content(), c.content()), "the content");
            assert!(Arc::ptr_eq(world_of(e), world_of(c)), "the world");
            assert_eq!(Arc::strong_count(world_of(e)), 2);
        };
        same(&e, &c);
        for _ in 0..5 {
            let inputs = party_case::minute_inputs(e.tick());
            e.step(&inputs);
            c.step(&inputs);
        }
        same(&e, &c);
        drop(c);
        assert_eq!(Arc::strong_count(world_of(&e)), 1);
    }
}

#[test]
fn a_clone_steps_exactly_as_the_original() {
    for mut e in engines() {
        let mut c = e.clone();
        for _ in 0..20 {
            let inputs = party_case::minute_inputs(e.tick());
            e.step(&inputs);
            c.step(&inputs);
            assert_eq!(e.state_hash(), c.state_hash());
        }
        assert_eq!(format!("{:?}", e.snapshot()), format!("{:?}", c.snapshot()));
    }
}

#[test]
fn an_engine_is_send_and_sync() {
    fn is<T: Send + Sync>() {}
    is::<Engine>();
}
