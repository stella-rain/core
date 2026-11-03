//! The deadlines of a timed run (ADR-SIM-7 point 4): where the state holds them, what the hash
//! and the savepoint do with them (ADR-SIM-3 point 2, ADR-SAVE-4 point 1).

use crate::run_setup::{OfferMode, RunSetup};
use crate::savepoint::{
    DecodeError,
    state::{decode_state, encode_state},
};
use crate::sim::state::State;

use super::*;

fn setup(mode: OfferMode) -> RunSetup {
    RunSetup {
        seed: 20_261_010,
        pick_stats: true,
        offer_mode: mode,
        ..RunSetup::default()
    }
}

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).unwrap()
}

fn state(e: &Engine) -> &State {
    match e.run() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn state_mut(e: &mut Engine) -> &mut State {
    match e.run_mut() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn due(e: &Engine) -> [Option<u32>; 3] {
    state(e).offer_timer.as_ref().expect("a timed run").due
}

/// A timed party of `party` with every present slot's first offer open.
fn open_all(party: u8) -> Engine {
    let mut e = Engine::start_party(&setup(OfferMode::Timed), party).unwrap();
    // A stat offer needs a level: make every present slot due one, then open them as the
    // tick's last step does.
    let present: Vec<_> = state(&e).players.indices().collect();
    for me in present {
        let s = state_mut(&mut e);
        let p = s.players.career_mut(me).progress.as_mut().unwrap();
        p.offers_due += 1;
    }
    e.open_offers_for_test();
    e
}

#[test]
fn a_stop_run_holds_no_deadlines_and_a_timed_one_starts_empty() {
    let stop = Engine::start(&setup(OfferMode::Stop)).unwrap();
    assert_eq!(stop.offer_mode(), OfferMode::Stop);
    assert!(state(&stop).offer_timer.is_none());
    let timed = Engine::start(&setup(OfferMode::Timed)).unwrap();
    assert_eq!(timed.offer_mode(), OfferMode::Timed);
    assert_eq!(due(&timed), [None; 3]);
}

#[test]
fn a_timed_run_needs_the_live_rules() {
    let r = Engine::forced(1, || Engine::start(&setup(OfferMode::Timed)));
    assert!(matches!(r, Err(crate::validate::Error::Invalid(i)) if i[0].path == "$.offer_mode"));
}

#[test]
fn offers_get_a_deadline_in_slot_order_and_a_pick_or_a_leave_clears_it() {
    let mut e = open_all(3);
    let ticks = state(&e).tick + 300;
    assert_eq!(due(&e), [Some(ticks); 3]);
    e.leave(slot(2)).unwrap();
    assert_eq!(due(&e), [Some(ticks), Some(ticks), None]);
    e.step(&[Input::default(), Input::picking(0), Input::default()]);
    assert_eq!(due(&e)[1], None);
    assert!(due(&e)[0].is_some());
}

#[test]
fn the_hash_covers_every_deadline() {
    let e = open_all(3);
    let base = e.state_hash();
    for i in 0..3 {
        let mut c = e.clone();
        let t = &mut state_mut(&mut c).offer_timer.as_mut().unwrap().due[i];
        *t = t.map(|d| d + 1);
        assert_ne!(c.state_hash(), base, "slot {i}");
    }
}

fn fresh(e: &Engine, party: u8, mode: OfferMode) -> Engine {
    let _ = e;
    Engine::start_party(&setup(mode), party).unwrap()
}

#[test]
fn the_deadlines_round_trip_and_the_decoder_refuses_what_disagrees() {
    let e = open_all(2);
    let bytes = encode_state(&e);
    let (saved, ..) = decode_state(&bytes, &fresh(&e, 2, OfferMode::Timed)).unwrap();
    match saved {
        Saved::V2(s) => assert_eq!(s.offer_timer, state(&e).offer_timer),
        Saved::V1(_) => panic!("live"),
    }
    let bad = |edit: &dyn Fn(&mut State)| {
        let mut c = e.clone();
        edit(state_mut(&mut c));
        decode_state(&encode_state(&c), &fresh(&e, 2, OfferMode::Timed)).map(|_| ())
    };
    let invalid = |r: Result<(), DecodeError>, why: &str| {
        assert_eq!(
            r,
            Err(DecodeError::Invalid(why.to_string().leak())),
            "{why}"
        )
    };
    invalid(
        bad(&|s| s.offer_timer.as_mut().unwrap().due[1] = None),
        "offer deadline and offer disagree",
    );
    invalid(
        bad(&|s| s.offer_timer.as_mut().unwrap().due[0] = Some(s.tick + 3601)),
        "offer deadline out of range",
    );
}

#[test]
fn a_savepoint_of_the_other_mode_is_refused() {
    let timed = open_all(1);
    let stop = Engine::start(&setup(OfferMode::Stop)).unwrap();
    assert!(decode_state(&encode_state(&timed), &stop).is_err());
    assert!(decode_state(&encode_state(&stop), &fresh(&stop, 1, OfferMode::Timed)).is_err());
}
