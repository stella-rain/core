//! A party changes at a tick (ADR-SIM-13 point 1, ADR-SIM-14 point 5, ADR-REPL-10): the engine
//! joins and leaves a slot between steps, the step log carries the change, and record, resume,
//! `replay_at`, save and verify agree on a run whose presence mask moves.

use crate::common::run::trial_setup;
use stella_rain_core::engine::{ChoiceError, Engine, Outcome, PresenceError};
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).unwrap()
}

fn a(t: u32) -> Input {
    Input {
        move_x: if t / 40 % 2 == 0 { 100 } else { -100 },
        main: true,
        ..Input::default()
    }
}

fn d() -> Input {
    Input::default()
}

fn advance(engine: &mut Engine, from: u32, to: u32) {
    for t in from..to {
        // An offer stops the world; a pick by whoever holds it closes it.
        while engine.offer_open() {
            let holder = (0..3u8)
                .find(|i| engine.check_pick(slot(*i), 0).is_ok())
                .expect("an open offer belongs to a slot");
            let mut inputs = [d(); 3];
            inputs[usize::from(holder)] = Input::picking(0);
            engine.step(&inputs);
        }
        engine.step(&[a(t), d(), d()]);
    }
}

#[test]
fn a_join_and_a_leave_move_the_mask_and_leave_the_hash_where_it_was() {
    let mut engine = Engine::start(&trial_setup()).unwrap();
    advance(&mut engine, 0, 120);
    let h0 = engine.state_hash();
    assert_eq!(engine.present(), 0b001);
    engine.join(slot(1)).unwrap();
    assert_eq!(engine.players(), 2);
    assert_eq!(engine.present(), 0b011);
    assert!(engine.events().is_empty());
    engine.leave(slot(1)).unwrap();
    assert_eq!(engine.present(), 0b001);
    assert_eq!(engine.state_hash(), h0);
}

#[test]
fn a_join_at_tick_zero_is_the_party_start() {
    let setup = trial_setup();
    let mut two = Engine::start(&setup).unwrap();
    two.join(slot(1)).unwrap();
    assert_eq!(
        two.state_hash(),
        Engine::start_party(&setup, 2).unwrap().state_hash()
    );
    two.join(slot(2)).unwrap();
    let three = Engine::start_party(&setup, 3).unwrap().state_hash();
    assert_eq!(two.state_hash(), three);
    let mut other = Engine::start(&setup).unwrap();
    other.join(slot(2)).unwrap();
    other.join(slot(1)).unwrap();
    assert_eq!(other.state_hash(), three);
}

#[test]
fn a_hole_steps_and_ignores_the_absent_slot() {
    let setup = trial_setup();
    let run = |junk: Input| {
        let mut e = Engine::start(&setup).unwrap();
        e.join(slot(2)).unwrap();
        assert_eq!(e.present(), 0b101);
        assert_eq!(e.players(), 2);
        for t in 0..120 {
            while e.offer_open() {
                let holder = (0..3u8)
                    .find(|i| e.check_pick(slot(*i), 0).is_ok())
                    .unwrap();
                let mut inputs = [d(); 3];
                inputs[usize::from(holder)] = Input::picking(0);
                e.step(&inputs);
            }
            e.step(&[a(t), junk, d()]);
        }
        assert_eq!(
            e.check_pick(slot(1), 0),
            Err(ChoiceError::NoPlayer { player: slot(1) })
        );
        e.state_hash()
    };
    assert_eq!(
        run(d()),
        run(Input {
            move_x: 90,
            main: true,
            ..d()
        })
    );
}

#[test]
fn a_refusal_changes_nothing() {
    let mut e = Engine::start(&trial_setup()).unwrap();
    advance(&mut e, 0, 30);
    let h = e.state_hash();
    assert_eq!(
        e.join(PlayerIdx::SOLO),
        Err(PresenceError::Occupied {
            slot: PlayerIdx::SOLO
        })
    );
    assert_eq!(
        e.leave(slot(1)),
        Err(PresenceError::Absent { slot: slot(1) })
    );
    assert_eq!(e.leave(PlayerIdx::SOLO), Err(PresenceError::FirstSlot));
    e.join(slot(1)).unwrap();
    assert_eq!(
        e.join(slot(1)),
        Err(PresenceError::Occupied { slot: slot(1) })
    );
    e.leave(slot(1)).unwrap();
    assert_eq!(e.state_hash(), h);
}

#[test]
fn a_call_on_an_ended_run_is_refused() {
    let mut e = Engine::start(&trial_setup()).unwrap();
    for t in 0..200_000 {
        if e.outcome() != Outcome::Running {
            break;
        }
        advance(&mut e, t, t + 1);
    }
    assert_ne!(e.outcome(), Outcome::Running, "the trial ends");
    assert_eq!(e.join(slot(1)), Err(PresenceError::Ended));
    assert_eq!(e.leave(slot(1)), Err(PresenceError::Ended));
}

#[test]
fn a_joiner_is_a_fresh_player_at_the_first_players_place() {
    let mut e = Engine::start(&trial_setup()).unwrap();
    let mut t = 0;
    while e.snapshot().progress.as_ref().is_some_and(|p| p.level < 2) {
        advance(&mut e, t, t + 1);
        t += 1;
        assert!(t < 3_000, "level 2 is reached");
    }
    let lvl0 = e.snapshot().progress.as_ref().unwrap().level;
    e.join(slot(1)).unwrap();
    let snap = e.snapshot();
    assert_eq!(snap.players.len(), 2);
    assert_eq!(
        snap.progress.as_ref().unwrap().level,
        lvl0,
        "slot 0 keeps its level"
    );
    assert_eq!(snap.players[1].at, snap.players[0].at);
    assert!(!e.offer_open() || e.check_pick(slot(1), 0).is_err());
}
