//! What a leaver owned must not bind to the next occupant of its slot (ADR-SIM-21, ADR-SIM-22).
//! Every per-player rule acts for slot 0 today, so slots 1 and 2 own nothing in a real run and
//! these tests plant the state.

use super::attacks;
use super::entity::Who;
use super::state::{Aim, AttackTask, Dir, Frame, OrbState, Origin, PlayerIdx, State, ZoneState};
use super::world::ProgramIndex;
use crate::attack::ZoneShape;
use crate::engine::{Engine, Run};
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::input::Input;
use crate::run_setup::RunSetup;

fn setup() -> RunSetup {
    RunSetup {
        seed: 20_261_010,
        pick_stats: true,
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
const LIFE: u32 = 100_000;

/// The number the run gives slot `i` now.
fn number(s: &State, i: u8) -> EntityId {
    s.players.id(slot(i))
}

fn input() -> [Input; 3] {
    [
        Input {
            main: true,
            ..Input::default()
        },
        Input::default(),
        Input::default(),
    ]
}

fn live(e: &mut Engine) -> &mut State {
    match e.run_mut() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn orb(owner: EntityId) -> OrbState {
    OrbState {
        at: Point { x: Fx(1), y: Fx(2) },
        vx: Fx(0),
        vy: Fx(0),
        dir: Dir { x: 0, y: 256 },
        style: 0,
        owner,
        damage: 9,
        damage_type: None,
        breaks_charge: false,
        smash: None,
        combo: false,
        hook: None,
        breakable: false,
        life: LIFE,
        stop: None,
        link: None,
        program: ProgramIndex(0),
    }
}

fn zone(owner: EntityId) -> ZoneState {
    ZoneState {
        owner,
        friendly: true,
        shape: ZoneShape::Circle { radius: Fx(256) },
        at: Point { x: Fx(1), y: Fx(2) },
        dir: Dir { x: 0, y: 256 },
        wind_left: LIFE,
        active_left: LIFE,
        wind_ticks: LIFE,
        active_ticks: LIFE,
        damage: 0,
        damage_type: None,
        breaks_charge: false,
        warn_only: true,
        hits: Vec::new(),
        program: ProgramIndex(0),
    }
}

fn task(owner: EntityId) -> AttackTask {
    AttackTask {
        owner,
        program: ProgramIndex(0),
        base: Vec::new(),
        origin: Origin::Owner,
        aim: Aim::Fixed(Dir { x: 0, y: 256 }),
        friendly: true,
        damage: 0,
        damage_type: None,
        breaks_charge: false,
        smash: None,
        combo: false,
        age: 0,
        wait_left: LIFE,
        frames: [Frame::default()].into_iter().collect(),
        parent_dir: Dir { x: 0, y: 256 },
        last_dir: Dir { x: 0, y: 256 },
        spot: None,
    }
}

/// A party of two, past the first attack of slot 0, with things owned by slot 1 planted, and
/// controls owned by slot 0. Returns the engine, slot 1's number and the control counts.
fn planted() -> (Engine, EntityId) {
    let mut e = Engine::start_party(&setup(), 2).unwrap();
    while state(&e).runs.is_empty() {
        e.step(&input());
    }
    let old = slot(1).id();
    let s = live(&mut e);
    let mut run = s.runs[0].clone();
    run.owner = old;
    run.slot = 99;
    run.age = 0;
    s.runs.push(run);
    s.tasks.push(task(old));
    s.orbs.push(orb(old));
    s.orbs.push(orb(slot(0).id()));
    s.zones.push(zone(old));
    s.zones.push(zone(slot(0).id()));
    s.tasks.push(task(slot(0).id()));
    s.move_index.lower(old, 50);
    assert!(s.runs.iter().any(|r| r.owner == old && r.slot == 99));
    (e, old)
}

fn assert_unbound(e: &mut Engine, old: EntityId) {
    let new = number(state(e), 1);
    assert_ne!(new, old, "the slot's new occupant has a number of its own");
    let s = state(e);
    assert_eq!(s.find(old), None);
    assert_eq!(s.players.of_id(old), None);
    assert_eq!(s.find(new), Some(Who::Player(slot(1))));
    e.step(&input());
    let s = state(e);
    assert!(s.runs.iter().all(|r| r.owner != old));
    assert!(s.tasks.iter().all(|t| t.owner != old));
    assert!(!attacks::busy(s, new, 99));
    assert_eq!(s.move_index.get(new), 100);
    assert!(s.orbs.iter().any(|o| o.owner == old));
    assert!(s.zones.iter().any(|z| z.owner == old));
    assert_eq!(s.players.of_id(old), None);
    assert!(s.orbs.iter().any(|o| o.owner == slot(0).id()));
    assert!(s.zones.iter().any(|z| z.owner == slot(0).id()));
    assert!(s.tasks.iter().any(|t| t.owner == slot(0).id()));
}

#[test]
fn a_rejoined_slot_is_not_the_leavers_owner() {
    let (mut e, old) = planted();
    e.leave(slot(1)).unwrap();
    e.join(slot(1)).unwrap();
    assert_unbound(&mut e, old);
}

#[test]
fn the_same_with_a_step_between_the_leave_and_the_join() {
    let (mut e, old) = planted();
    e.leave(slot(1)).unwrap();
    e.step(&input());
    e.join(slot(1)).unwrap();
    assert_unbound(&mut e, old);
}

#[test]
fn a_slot_that_never_left_keeps_its_number() {
    let e = Engine::start_party(&setup(), 3).unwrap();
    for i in 0..3 {
        assert_eq!(number(state(&e), i), EntityId(u32::from(i)));
    }
    let mut e = Engine::start(&setup()).unwrap();
    e.join(slot(2)).unwrap();
    assert_eq!(number(state(&e), 2), EntityId(2));
}

#[test]
fn a_leavers_things_stay_while_the_slot_is_empty() {
    let (mut e, old) = planted();
    e.leave(slot(1)).unwrap();
    for _ in 0..3 {
        e.step(&input());
    }
    let s = state(&e);
    assert!(s.orbs.iter().any(|o| o.owner == old));
    assert!(s.zones.iter().any(|z| z.owner == old));
}

#[test]
fn a_rejoined_slot_has_the_same_roll_key() {
    let (mut e, old) = planted();
    e.leave(slot(1)).unwrap();
    e.join(slot(1)).unwrap();
    let s = state(&e);
    assert_eq!(s.roll_key(number(s, 1)), s.roll_key(old));
}

#[test]
fn the_numbers_of_things_stop_short_of_a_players() {
    // A normal run has spawned enemies by tick 200; one whose numbers are spent spawns none and
    // never takes a number a player could have.
    let play = |next_id: Option<u32>| {
        let mut e = Engine::start(&setup()).unwrap();
        if let Some(n) = next_id {
            live(&mut e).next_id = n;
        }
        for _ in 0..300 {
            e.step(&input());
        }
        let s = state(&e);
        (s.enemies.len(), s.next_id)
    };
    assert!(play(None).0 > 0);
    assert_eq!(
        play(Some(crate::id::MAX_ENTITY_NUMBER)),
        (0, crate::id::MAX_ENTITY_NUMBER)
    );
}

#[test]
fn a_slot_cannot_leave_past_generation_255() {
    let mut e = Engine::start_party(&setup(), 2).unwrap();
    live(&mut e).players.set_generation_for_test(slot(1), 255);
    let h = e.state_hash();
    assert_eq!(
        e.leave(slot(1)),
        Err(crate::engine::PresenceError::Exhausted { slot: slot(1) })
    );
    assert_eq!(e.state_hash(), h);
    assert_eq!(e.present(), 0b011);
}

#[test]
fn the_hash_and_the_savepoint_read_the_generation() {
    let rejoin = |times: u8| {
        let mut e = Engine::start_party(&setup(), 2).unwrap();
        for _ in 0..times {
            e.leave(slot(1)).unwrap();
            e.join(slot(1)).unwrap();
        }
        e
    };
    let (once, twice) = (rejoin(1), rejoin(2));
    assert_eq!(state(&once).players.generation(slot(1)), 1);
    assert_eq!(state(&twice).players.generation(slot(1)), 2);
    assert_ne!(once.state_hash(), twice.state_hash());
}
