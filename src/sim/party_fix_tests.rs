//! A downed or out player in a party (ADR-SIM-7 points 4 and 5, ADR-SIM-8): its offers, its hit
//! points, the boss offer and the outcome.

use super::state::{Due, PlayerIdx, State};
use super::{World, decide_outcome, tick_down_and_revive};
use crate::engine::{Engine, Outcome, Run};
use crate::event::DomainEvent;
use crate::input::Input;
use crate::rewards::Rarity;
use crate::run_setup::RunSetup;
use crate::snapshot::OfferKind;

pub(super) fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

/// A party of `n` at the start: stat offers are picked, nothing has been stepped.
pub(super) fn party(n: u8) -> (World, State) {
    let setup = RunSetup {
        seed: 20_261_104,
        pick_stats: true,
        ..RunSetup::default()
    };
    let e = Engine::start_party(&setup, n).expect("a party");
    let Run::V2 { world, state } = e.run() else {
        panic!("the live rules")
    };
    (world.clone(), state.clone())
}

pub(super) fn down(s: &mut State, i: u8) {
    s.players[slot(i)].hp = 0;
    s.players[slot(i)].down_ticks = 0;
}

pub(super) fn out(s: &mut State, i: u8) {
    s.players[slot(i)].hp = 0;
    s.players[slot(i)].out = true;
}

pub(super) fn star_due() -> Due {
    Due {
        kind: OfferKind::Star,
        rarity: Rarity::Common,
    }
}

fn idle() -> [Input; 3] {
    [Input::default(); 3]
}

/// Out, then down (ADR-SIM-7 point 5): with slot 1 out, a party fails when slot 0 is down too,
/// and runs on while it is up.
#[test]
fn one_out_and_the_other_goes_down_fails_the_run() {
    let (w, mut s) = party(2);
    out(&mut s, 1);
    let mut ev = Vec::new();
    decide_outcome(&w, &mut s, &mut ev);
    assert_eq!(s.outcome, Outcome::Running);
    assert!(ev.is_empty());

    down(&mut s, 0);
    decide_outcome(&w, &mut s, &mut ev);
    assert_eq!(s.outcome, Outcome::Failed);
    assert_eq!(ev, vec![DomainEvent::StageFailed]);
}

/// The queued offers of a player are dropped on the tick it goes out (ADR-SIM-7 point 4).
#[test]
fn queued_offers_are_dropped_when_a_player_goes_out() {
    let (w, mut s) = party(2);
    down(&mut s, 1);
    s.players[slot(1)].down_ticks = 1199;
    let career = s.players.career_mut(slot(1));
    career.progress.as_mut().expect("levels").offers_due = 2;
    career
        .offers
        .as_mut()
        .expect("stars")
        .due
        .extend([star_due(), star_due()]);
    let gauge = career.expertise.as_mut().expect("expertise");
    gauge.level = 3;
    gauge.offered = 1;

    let mut ev = Vec::new();
    tick_down_and_revive(&w, &mut s, &idle(), &mut ev);
    assert!(ev.contains(&DomainEvent::PlayerOut { player: slot(1) }));
    let career = s.players.career(slot(1));
    assert_eq!(career.progress.as_ref().unwrap().offers_due, 0);
    assert!(career.offers.as_ref().unwrap().due.is_empty());
    let gauge = career.expertise.as_ref().unwrap();
    assert_eq!(gauge.offered, gauge.level);
}
