//! The public API names players (ADR-SIM-12): the snapshot and the places hold one entry per
//! player, and `choose` takes the player. At one player the bytes are the pins'.

use crate::bench_fixtures::{hit_case, time_limit_case, worst_case, worst_minute_case};
use crate::id::PlayerIdx;
use crate::input::Input;

use super::*;

fn orb_cage() -> crate::stage::Stage {
    let bytes = include_bytes!("../../tests/corpus/v1/stages/orb_cage/stage.json");
    crate::validate::parse_and_validate(bytes).expect("a valid stage")
}

fn p1() -> PlayerIdx {
    PlayerIdx::new(1).expect("player 1")
}

/// An engine and the input that plays it.
type Case = (Engine, fn(u32) -> Input);

/// An engine with an offer open, from the first of the walks that reaches one.
fn with_offer() -> Engine {
    let cases: [Case; 5] = [
        (
            worst_minute_case::worst_minute().engine,
            worst_minute_case::input_at,
        ),
        (
            Engine::new(&orb_cage()).expect("the stage runs"),
            worst_minute_case::input_at,
        ),
        (worst_case::warmed_up(), worst_case::input),
        (hit_case::warmed_up(), hit_case::input),
        (time_limit_case::warmed_up(), time_limit_case::input),
    ];
    for (mut e, input) in cases {
        for _ in 0..600 {
            if e.offer_open() {
                return e;
            }
            let next = e.tick() + 1;
            e.step(input(next));
        }
    }
    panic!("no walk reached an offer");
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
                    e.step(Input::default());
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

#[test]
fn choose_names_the_player() {
    let mut e = with_offer();
    let (hash, events) = (e.state_hash(), e.events().to_vec());
    let err = e.choose(p1(), 0);
    assert_eq!(err, Err(ChoiceError::NoPlayer { player: p1() }));
    assert_eq!((e.state_hash(), e.events()), (hash, events.as_slice()));
    let options = e
        .snapshot()
        .progress
        .and_then(|p| p.offer)
        .map(|o| o.options.len());
    if let Some(n) = options {
        let past = u32::try_from(n).unwrap();
        assert_eq!(
            e.choose(PlayerIdx::SOLO, past),
            Err(ChoiceError::OutOfRange {
                index: past,
                options: past
            })
        );
    }
    assert_eq!(e.state_hash(), hash);
    assert_eq!(e.choose(PlayerIdx::SOLO, 0), Ok(()));
    assert_ne!(e.state_hash(), hash);

    let mut idle = worst_case::warmed_up();
    assert!(!idle.offer_open());
    assert_eq!(
        idle.choose(p1(), 0),
        Err(ChoiceError::NoPlayer { player: p1() })
    );
    assert_eq!(idle.choose(PlayerIdx::SOLO, 0), Err(ChoiceError::NoOffer));
}
