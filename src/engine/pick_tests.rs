//! An offer pick travels inside the step's input (ADR-SIM-12, ADR-REPL-3): `step` takes one
//! input per player, `check_pick` says why a pick is refused, and a pick changes nothing it
//! should not.

use super::players_api_tests::{p1, with_offer};
use crate::bench_fixtures::worst_case;
use crate::id::PlayerIdx;
use crate::input::Input;

use super::*;

/// How many options the open offer holds: the first index `check_pick` refuses.
fn options_of(e: &Engine) -> u32 {
    (0..)
        .find(|i| e.check_pick(PlayerIdx::SOLO, *i).is_err())
        .unwrap()
}

#[test]
fn pick_travels_inside_the_input() {
    let mut e = with_offer();
    let (hash, tick) = (e.state_hash(), e.tick());
    assert!(options_of(&e) > 0, "the walk's first offer is a plain one");
    e.step(&[Input::picking(0)]);
    assert_eq!(e.tick(), tick, "a pick does not advance the world");
    assert_ne!(e.state_hash(), hash, "the pick was taken");
    assert!(!e.events().is_empty(), "the events are the pick's");
}

#[test]
fn a_refused_pick_changes_nothing() {
    let mut e = with_offer();
    let n = options_of(&e);
    let (hash, tick) = (e.state_hash(), e.tick());
    e.step(&[Input::picking(n)]);
    assert_eq!((e.state_hash(), e.tick()), (hash, tick));
    assert_eq!(
        e.check_pick(PlayerIdx::SOLO, n),
        Err(ChoiceError::OutOfRange {
            index: n,
            options: n
        })
    );
    assert_eq!(
        e.check_pick(p1(), 0),
        Err(ChoiceError::NoPlayer { player: p1() })
    );
    assert_eq!(e.check_pick(PlayerIdx::SOLO, 0), Ok(()));

    let idle = worst_case::warmed_up();
    assert!(!idle.offer_open());
    assert_eq!(
        idle.check_pick(PlayerIdx::SOLO, 0),
        Err(ChoiceError::NoOffer)
    );
    assert_eq!(
        idle.check_pick(p1(), 0),
        Err(ChoiceError::NoPlayer { player: p1() }),
        "the player is checked before the offer"
    );
}

#[test]
fn a_pick_on_a_running_tick_is_ignored() {
    let mut a = worst_case::warmed_up();
    let mut b = a.clone();
    a.step(&[Input {
        pick: Some(3),
        ..Input::default()
    }]);
    b.step(&[Input::default()]);
    assert_eq!(a.state_hash(), b.state_hash());
    assert_eq!(a.events(), b.events());
}

#[test]
fn an_open_offer_stops_the_world_without_a_pick() {
    let mut e = with_offer();
    let (hash, tick) = (e.state_hash(), e.tick());
    e.step(&[Input::default()]);
    assert_eq!((e.state_hash(), e.tick()), (hash, tick));
}

#[test]
#[should_panic(expected = "one input per player")]
fn step_refuses_no_input() {
    worst_case::warmed_up().step(&[]);
}

#[test]
#[should_panic(expected = "one input per player")]
fn step_refuses_an_input_too_many() {
    worst_case::warmed_up().step(&[Input::default(), Input::default()]);
}
