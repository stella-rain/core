//! The public API names players (ADR-SIM-7): the snapshot and the places hold one entry per
//! player, and `choose` takes the player. At one player the bytes are the pins'.

use crate::bench_fixtures::{hit_case, time_limit_case, worst_case};
use crate::input::Input;

use super::*;

fn orb_cage() -> crate::stage::Stage {
    let bytes = include_bytes!("../../tests/corpus/v1/stages/orb_cage/stage.json");
    crate::validate::parse_and_validate(bytes).expect("a valid stage")
}

#[test]
fn the_snapshot_has_one_player_per_slot() {
    let mut engines = vec![
        worst_case::warmed_up(),
        hit_case::warmed_up(),
        time_limit_case::warmed_up(),
        Engine::new(&orb_cage()).expect("a version 1 run"),
    ];
    for e in &mut engines {
        let mut places = Places::default();
        for round in 0..2 {
            let (snap, tick) = (e.snapshot(), e.tick());
            assert_eq!(e.players(), 1, "tick {tick}");
            assert_eq!(snap.players.len(), 1, "tick {tick}");
            e.places(&mut places);
            assert_eq!(places.players.len(), 1, "tick {tick}");
            assert_eq!(places.players[0], snap.players[0].at, "tick {tick}");
            if round == 0 {
                for _ in 0..60 {
                    e.step(&[Input::default(), Default::default(), Default::default()]);
                }
            }
        }
    }
}

#[test]
fn players_json_is_an_array_of_the_old_player() {
    let e = worst_case::warmed_up();
    let snap = e.snapshot();
    let json = serde_json::to_value(&snap).expect("a snapshot serializes");
    assert!(json.get("player").is_none());
    let players = json["players"].as_array().expect("an array");
    assert_eq!(players.len(), 1);
    assert_eq!(
        players[0],
        serde_json::to_value(&snap.players[0]).expect("a view serializes")
    );
}
