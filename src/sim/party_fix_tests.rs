//! A downed or out player in a party (ADR-SIM-7 points 4 and 5, ADR-SIM-8): its offers, its hit
//! points, the boss offer and the outcome.

use super::state::{Due, PlayerIdx, State};
use super::{
    Blame, Culprit, Shot, World, decide_outcome, hurt_player, offer_open_for, tick_down_and_revive,
};
use crate::engine::{Engine, Outcome, Run};
use crate::event::{DomainEvent, HitKind};
use crate::id::EntityId;
use crate::input::Input;
use crate::rewards::Rarity;
use crate::run_setup::{OfferMode, RunSetup};
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

// --- offers of a downed and an out player (ADR-SIM-7 point 4) ----------------------------------

/// A party engine, `timed` or not, to edit and to open offers on.
fn engine(n: u8, timed: bool) -> Engine {
    let setup = RunSetup {
        seed: 20_261_104,
        pick_stats: true,
        offer_mode: if timed {
            OfferMode::Timed
        } else {
            OfferMode::Stop
        },
        ..RunSetup::default()
    };
    Engine::start_party(&setup, n).expect("a party")
}

fn st(e: &mut Engine) -> &mut State {
    match e.run_mut() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn stat_offers_due(e: &mut Engine, i: u8, n: u32) {
    let p = st(e).players.career_mut(slot(i)).progress.as_mut().unwrap();
    p.offers_due = n;
}

fn count<F: Fn(&DomainEvent) -> bool>(e: &Engine, f: F) -> usize {
    e.events().iter().filter(|ev| f(ev)).count()
}

fn lethal(e: &mut Engine, i: u8) {
    let Run::V2 { world, state: s } = e.run_mut() else {
        panic!("the live rules")
    };
    let shot = Shot {
        at: s.players[slot(i)].at,
        owner: EntityId(99),
        friendly: false,
        damage: 1_000_000,
        damage_type: None,
        breaks_charge: false,
        smash: None,
        combo: false,
        rune: None,
        chain: false,
    };
    let by = Blame {
        kind: HitKind::Attack,
        by: Culprit::Run(super::world::AttackIndex(0)),
    };
    let mut events = Vec::new();
    hurt_player(world, s, slot(i), shot, by, &mut events);
}

#[test]
fn a_downed_player_takes_a_due_stat_offer_on_the_tick_it_opens() {
    let mut e = engine(2, true);
    down(st(&mut e), 1);
    stat_offers_due(&mut e, 1, 1);
    e.open_offers_for_test();
    let s = st(&mut e);
    assert!(!offer_open_for(s, slot(1)));
    assert_eq!(
        s.players
            .career(slot(1))
            .progress
            .as_ref()
            .unwrap()
            .offers_due,
        0
    );
    assert_eq!(s.offer_timer.as_ref().unwrap().due[1], None);
    assert_eq!(
        count(&e, |ev| matches!(ev, DomainEvent::StatRaised { .. })),
        1
    );
}

#[test]
fn a_downed_player_takes_its_whole_queue_on_one_tick() {
    let mut e = engine(2, true);
    down(st(&mut e), 1);
    st(&mut e)
        .players
        .career_mut(slot(1))
        .offers
        .as_mut()
        .unwrap()
        .due
        .extend([star_due(), star_due(), star_due()]);
    e.open_offers_for_test();
    let s = st(&mut e);
    let offers = s.players.career(slot(1)).offers.as_ref().unwrap();
    assert!(offers.due.is_empty() && offers.offer.is_none());
    assert!(!offer_open_for(s, slot(1)));
    assert_eq!(
        count(&e, |ev| matches!(ev, DomainEvent::RewardTaken { .. })),
        3
    );
}

#[test]
fn going_down_takes_the_open_offer_and_the_one_queued_behind_it() {
    let mut e = engine(2, true);
    stat_offers_due(&mut e, 1, 2);
    e.open_offers_for_test();
    assert!(offer_open_for(st(&mut e), slot(1)));
    lethal(&mut e, 1);
    assert!(st(&mut e).players[slot(1)].is_downed());
    e.open_offers_for_test();
    let s = st(&mut e);
    assert!(!offer_open_for(s, slot(1)));
    assert_eq!(
        s.players
            .career(slot(1))
            .progress
            .as_ref()
            .unwrap()
            .offers_due,
        0
    );
    assert_eq!(s.offer_timer.as_ref().unwrap().due[1], None);
    assert_eq!(
        count(&e, |ev| matches!(ev, DomainEvent::StatRaised { .. })),
        2
    );
}

#[test]
fn a_player_that_is_up_opens_one_offer_with_the_full_timer() {
    let mut e = engine(2, true);
    stat_offers_due(&mut e, 0, 2);
    e.open_offers_for_test();
    let s = st(&mut e);
    assert!(offer_open_for(s, slot(0)));
    assert_eq!(
        s.players
            .career(slot(0))
            .progress
            .as_ref()
            .unwrap()
            .offers_due,
        1
    );
    let want = s.tick + 300;
    assert_eq!(s.offer_timer.as_ref().unwrap().due[0], Some(want));
}

#[test]
fn an_out_player_opens_nothing_and_loses_what_is_queued() {
    let mut e = engine(2, true);
    out(st(&mut e), 1);
    stat_offers_due(&mut e, 1, 2);
    st(&mut e)
        .players
        .career_mut(slot(1))
        .offers
        .as_mut()
        .unwrap()
        .due
        .push(star_due());
    e.open_offers_for_test();
    let s = st(&mut e);
    assert!(!offer_open_for(s, slot(1)));
    let career = s.players.career(slot(1));
    assert_eq!(career.progress.as_ref().unwrap().offers_due, 0);
    assert!(career.offers.as_ref().unwrap().due.is_empty());
    assert_eq!(s.offer_timer.as_ref().unwrap().due[1], None);
    assert_eq!(
        count(&e, |ev| matches!(
            ev,
            DomainEvent::StatOfferOpened { .. } | DomainEvent::OfferOpened { .. }
        )),
        0
    );
}

/// A timed engine: the offer open for slot 1 is taken on the tick it goes down.
#[test]
fn an_offer_open_when_a_player_goes_down_closes_on_that_tick() {
    let mut e = engine(2, true);
    st(&mut e)
        .players
        .career_mut(slot(1))
        .offers
        .as_mut()
        .unwrap()
        .due
        .push(star_due());
    e.step(&idle());
    assert!(offer_open_for(st(&mut e), slot(1)));
    assert!(!e.offer_ratings(slot(1)).is_empty());
    lethal(&mut e, 1);
    e.step(&idle());
    assert!(!offer_open_for(st(&mut e), slot(1)));
    assert!(e.offer_ratings(slot(1)).is_empty());
    assert_eq!(
        count(&e, |ev| matches!(ev, DomainEvent::RewardTaken { .. })),
        1
    );
}
