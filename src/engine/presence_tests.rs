//! A slot joins and leaves between steps (ADR-SIM-7, ADR-SIM-8): what the state holds after,
//! the hole-safe lookups, and the `id_gap` rule at the tick the count changes.

use crate::id::EntityId;
use crate::run_setup::RunSetup;

use super::*;

fn setup() -> RunSetup {
    RunSetup {
        seed: 20_261_010,
        ..RunSetup::default()
    }
}

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).unwrap()
}

fn state(e: &Engine) -> &crate::sim::state::State {
    match e.run() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

#[test]
fn a_version_one_run_refuses_a_change() {
    let mut e = Engine::forced(1, || Engine::start(&setup())).unwrap();
    let h = e.state_hash();
    assert_eq!(e.join(slot(1)), Err(PresenceError::SoloRules));
    assert_eq!(e.leave(slot(1)), Err(PresenceError::SoloRules));
    assert_eq!(e.state_hash(), h);
    assert_eq!(e.present(), 0b001);
}

#[test]
fn the_roll_key_follows_the_count_in_force() {
    let mut e = Engine::start(&setup()).unwrap();
    let keys = |e: &Engine| {
        let s = state(e);
        [
            s.roll_key(EntityId(7)),
            s.roll_key(EntityId(3)),
            s.roll_key(EntityId(4)),
            s.roll_key(EntityId(1)),
            s.roll_key(EntityId(2)),
        ]
    };
    assert_eq!(keys(&e), [5, 1, 2, 1, 2]);
    e.join(slot(1)).unwrap();
    assert_eq!(keys(&e), [6, 2, 3, 1, 2]);
    e.join(slot(2)).unwrap();
    assert_eq!(keys(&e), [7, 3, 4, 1, 2]);
    e.leave(slot(1)).unwrap();
    assert_eq!(keys(&e), [6, 2, 3, 1, 2]);
}

#[test]
fn the_snapshot_follows_the_mask_in_a_hole() {
    let mut e = Engine::start(&setup()).unwrap();
    e.join(slot(2)).unwrap();
    let snap = e.snapshot();
    assert_eq!(snap.players.len(), 2);
    assert_eq!(snap.players[1].at, state(&e).players[PlayerIdx::Third].at);
}

#[test]
fn a_joiner_is_the_start_player_at_the_first_slots_place() {
    let mut e = Engine::start(&setup()).unwrap();
    let party = Engine::start_party(&setup(), 2).unwrap();
    for _ in 0..30 {
        e.step(&[
            Input {
                move_x: 100,
                move_y: 40,
                ..Input::default()
            },
            Input::default(),
            Input::default(),
        ]);
    }
    e.join(slot(1)).unwrap();
    let (s, p) = (state(&e), state(&party));
    assert_eq!(s.players[slot(1)].at, s.players[PlayerIdx::SOLO].at);
    assert_eq!(s.players[slot(1)].facing, s.players[PlayerIdx::SOLO].facing);
    assert_eq!(s.players[slot(1)].hp, p.players[slot(1)].max_hp);
    assert_eq!(s.players.career(slot(1)), p.players.career(slot(1)));
    assert_eq!(s.players[slot(1)].invulnerable, 0);
}

#[test]
fn a_leave_closes_the_leavers_offer() {
    let mut e = Engine::start_party(&setup(), 2).unwrap();
    for _ in 0..3_000 {
        if e.offer_open() {
            break;
        }
        e.step(&[Input::default(); 3]);
    }
    if e.offer_open() {
        let holders: Vec<u8> = (0..2)
            .filter(|i| e.check_pick(slot(*i), 0).is_ok())
            .collect();
        if holders == [1] {
            e.leave(slot(1)).unwrap();
            assert!(!e.offer_open());
            let t = e.tick();
            e.step(&[Input::default(); 3]);
            assert_eq!(e.tick(), t + 1);
        }
    }
}

#[test]
fn a_leaver_keeps_its_totals_until_the_slot_is_joined_again() {
    let mut e = Engine::start_party(&setup(), 2).unwrap();
    e.player_stats.hit_for_test(slot(1), 9);
    let before = e.player_stats().slot(slot(1));
    assert_ne!(before, crate::ledger::SlotStats::default());
    e.leave(slot(1)).unwrap();
    assert_eq!(e.player_stats().slot(slot(1)), before);
    e.join(slot(1)).unwrap();
    assert_eq!(
        e.player_stats().slot(slot(1)),
        crate::ledger::SlotStats::default()
    );
}

#[test]
fn what_a_leaver_left_in_flight_does_not_panic_and_a_quick_rejoin_agrees() {
    let run = |rejoin: bool| {
        let mut e = Engine::start_party(&setup(), 2).unwrap();
        for t in 0..60 {
            e.step(&[
                Input {
                    move_x: if t < 30 { 90 } else { -90 },
                    main: true,
                    ..Input::default()
                },
                Input::default(),
                Input::default(),
            ]);
        }
        e.leave(slot(1)).unwrap();
        if rejoin {
            e.join(slot(1)).unwrap();
        }
        for _ in 0..300 {
            if e.offer_open() {
                break;
            }
            e.step(&[
                Input {
                    main: true,
                    ..Input::default()
                },
                Input::default(),
                Input::default(),
            ]);
        }
        e.state_hash()
    };
    assert_eq!(run(true), run(true));
    let _ = run(false);
}
