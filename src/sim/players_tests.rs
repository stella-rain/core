//! The state holds its players as `players`, addressed by a `PlayerIdx` (ADR-SIM-12), and a system
//! acts on the player it is given. At one player the bytes are the pins' (`players_pin_tests`).

use super::state::{MAX_PLAYERS, PlayerIdx, Players, State};
use super::{Blame, Culprit, Shot, World, hurt_player, player};
use crate::bench_fixtures::{hit_case, time_limit_case, worst_case};
use crate::engine::{Engine, Run};
use crate::event::HitKind;
use crate::hash::{StableHash, StateHasher};
use crate::id::EntityId;
use crate::input::Input;
use crate::savepoint::codec::{Encode, Writer};
use crate::sim::entity::Who;

fn engines() -> [Engine; 3] {
    [
        worst_case::warmed_up(),
        hit_case::warmed_up(),
        time_limit_case::warmed_up(),
    ]
}

fn live(e: &Engine) -> (&World, &State) {
    match e.run() {
        Run::V2 { world, state } => (world, state),
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn hash_of(s: &State) -> u64 {
    let mut h = StateHasher::new();
    s.stable_hash(&mut h);
    h.finish()
}

/// `s` with a second player: a clone of the first, moved 10 units right.
fn two_players(s: &State) -> State {
    let mut s2 = s.clone();
    let mut p1 = s.players[PlayerIdx::SOLO].clone();
    p1.at.x.0 += 10;
    s2.players = Players::of(vec![s.players[PlayerIdx::SOLO].clone(), p1]).expect("two players");
    s2
}

fn p1() -> PlayerIdx {
    PlayerIdx::new(1).expect("player 1")
}

#[test]
fn player_idx_is_bounded_and_names_its_entity() {
    assert_eq!(MAX_PLAYERS, 3);
    assert_eq!(PlayerIdx::new(0), Some(PlayerIdx::SOLO));
    assert!(PlayerIdx::new(2).is_some());
    assert!(PlayerIdx::new(3).is_none());
    assert_eq!(PlayerIdx::SOLO.id(), EntityId(0));
    assert_eq!(PlayerIdx::new(2).unwrap().id(), EntityId(2));
    assert_eq!(PlayerIdx::of_id(EntityId(1), 1), None);
    assert_eq!(PlayerIdx::of_id(EntityId(1), 2), Some(p1()));
}

#[test]
fn a_solo_engine_has_one_player() {
    for e in engines() {
        let (_, s) = live(&e);
        assert_eq!(s.players.len(), 1);
        assert!(s.players[PlayerIdx::SOLO].hp > 0);
        assert!(s.boss.as_ref().is_none_or(|b| b.id.0 >= 1));
        assert!(s.enemies.iter().all(|a| a.id.0 >= 1));
    }
}

#[test]
fn a_second_player_is_a_tail_of_the_hash() {
    let e = worst_case::warmed_up();
    let (_, s) = live(&e);
    let s2 = two_players(s);
    assert_ne!(hash_of(&s2), hash_of(s));
    let mut swapped = s2.clone();
    let v: Vec<_> = s2.players.iter().rev().cloned().collect();
    swapped.players = Players::of(v).unwrap();
    assert_ne!(hash_of(&swapped), hash_of(&s2));
    let mut hurt = s2.clone();
    hurt.players[p1()].hp -= 1;
    assert_ne!(hash_of(&hurt), hash_of(&s2));
    assert_eq!(hash_of(s), e.state_hash());
}

#[test]
#[should_panic(expected = "one player")]
fn a_state_of_two_players_is_not_a_savepoint() {
    let e = worst_case::warmed_up();
    let s2 = two_players(live(&e).1);
    s2.encode(&mut Writer::new());
}

#[test]
fn a_system_acts_on_the_player_it_is_given() {
    let e = worst_case::warmed_up();
    let (world, s) = live(&e);
    let rewards = e.content().rewards();
    let input = Input {
        move_x: 127,
        ..Input::default()
    };
    let mut s2 = two_players(s);
    s2.players[p1()] = s.players[PlayerIdx::SOLO].clone();
    let before0 = s2.players[PlayerIdx::SOLO].clone();
    let mut events = Vec::new();
    let mut solo = s.clone();
    player::update(
        world,
        rewards,
        &mut solo,
        PlayerIdx::SOLO,
        input,
        &mut events,
    );
    player::update(world, rewards, &mut s2, p1(), input, &mut events);
    assert_eq!(s2.players[PlayerIdx::SOLO], before0);
    assert_eq!(s2.players[p1()].at, solo.players[PlayerIdx::SOLO].at);
    assert_ne!(s2.players[p1()].at, before0.at);

    let hp = (s2.players[PlayerIdx::SOLO].hp, s2.players[p1()].hp);
    let shot = Shot {
        at: before0.at,
        owner: EntityId(99),
        friendly: false,
        damage: 5,
        damage_type: None,
        breaks_charge: false,
        smash: None,
        combo: false,
        rune: None,
        chain: false,
    };
    let by = Blame {
        kind: HitKind::Attack,
        by: Culprit::Run(crate::sim::world::AttackIndex(0)),
    };
    hurt_player(world, &mut s2, p1(), shot, by, &mut events);
    assert_eq!(s2.players[PlayerIdx::SOLO].hp, hp.0);
    assert!(s2.players[p1()].hp < hp.1);

    assert_eq!(s2.at(Who::Player(p1())), s2.players[p1()].at);
    assert_eq!(s2.id_of(Who::Player(p1())), EntityId(1));
}

#[test]
fn a_party_of_two_has_two_views() {
    let e = worst_case::warmed_up();
    let (world, s) = live(&e);
    let mut s2 = two_players(s);
    s2.players[p1()].hp = 1;
    let snap = super::snapshot::build(world, e.content(), &s2);
    assert_eq!(snap.players.len(), 2);
    assert_eq!(snap.players[1].hp, 1);
    assert_eq!(snap.players[0].hp, s.players[PlayerIdx::SOLO].hp);
    let mut places = crate::engine::Places::default();
    super::snapshot::places(&s2, &mut places);
    assert_eq!(places.players.len(), 2);
    assert_eq!(places.players[0], s2.players[PlayerIdx::SOLO].at);
    assert_eq!(places.players[1], s2.players[p1()].at);
}

// ---- Slots 0 to 2 are reserved (ADR-SIM-12, ADR-SIM-13) ----

fn ps(e: &Engine) -> super::state::PlayerState {
    live(e).1.players[PlayerIdx::SOLO].clone()
}

#[test]
fn a_solo_party_holds_slot_zero_in_a_mask() {
    let e = worst_case::warmed_up();
    let p = ps(&e);
    let solo = Players::solo(p.clone());
    assert_eq!(solo.mask(), 0b001);
    assert_eq!(solo.len(), 1);
    let three = Players::of(vec![p.clone(), p.clone(), p.clone()]).unwrap();
    assert_eq!(three.mask(), 0b111);
    assert_eq!(three.iter().count(), 3);
    assert_eq!(three.others().count(), 2);
}

#[test]
fn a_mask_with_a_hole_names_the_present_slots() {
    let p = ps(&worst_case::warmed_up());
    let holey = Players::with_mask(0b101, p);
    assert_eq!(holey.len(), 2);
    assert!(holey.is_present(PlayerIdx::First));
    assert!(!holey.is_present(p1()));
    assert!(holey.is_present(PlayerIdx::new(2).unwrap()));
    let order: Vec<_> = holey.indices().collect();
    assert_eq!(order, [PlayerIdx::First, PlayerIdx::new(2).unwrap()]);
}

#[test]
#[should_panic(expected = "slot 0")]
fn a_party_without_slot_zero_is_refused() {
    Players::with_mask(0b110, ps(&worst_case::warmed_up()));
}

#[test]
#[should_panic(expected = "prefix")]
fn the_hash_refuses_a_party_with_a_hole() {
    let e = worst_case::warmed_up();
    let mut s = live(&e).1.clone();
    s.players = Players::with_mask(0b101, ps(&e));
    hash_of(&s);
}

#[test]
fn the_hash_of_a_party_of_one_has_no_tail_and_two_players_differ() {
    let e = worst_case::warmed_up();
    let (_, s) = live(&e);
    assert_eq!(hash_of(s), e.state_hash());
    assert_ne!(hash_of(&two_players(s)), hash_of(s));
}

#[test]
fn the_id_gap_follows_the_party() {
    let p = ps(&worst_case::warmed_up());
    let state_with = |n: usize| {
        let mut s = live(&worst_case::warmed_up()).1.clone();
        s.players = Players::of(vec![p.clone(); n]).unwrap();
        s
    };
    assert_eq!(state_with(1).id_gap(), 2);
    assert_eq!(state_with(2).id_gap(), 1);
    assert_eq!(state_with(3).id_gap(), 0);
    let s = state_with(1);
    assert_eq!(s.roll_key(EntityId(3)), 1);
    assert_eq!(s.roll_key(EntityId(0)), 0);
    assert_eq!(state_with(3).roll_key(EntityId(3)), 3);
}

#[test]
fn entities_are_numbered_from_three() {
    for e in engines() {
        let (_, s) = live(&e);
        assert!(s.boss.as_ref().is_none_or(|b| b.id.0 >= 3));
        assert!(s.companions.iter().all(|c| c.id.0 >= 3));
        assert!(s.enemies.iter().all(|a| a.id.0 >= 3));
        assert!(s.next_id >= 3);
    }
    let fresh = Engine::new_with(&worst_case::stage(), &worst_case::content()).expect("starts");
    let (_, s) = live(&fresh);
    if let Some(boss) = &s.boss {
        assert_eq!(boss.id, EntityId(3), "slots 0 to 2 are reserved");
    }
    let spawned = u32::try_from(s.companions.len()).unwrap();
    assert!(s.next_id >= 3 + spawned);
}

#[test]
fn a_saved_next_id_inside_the_player_slots_is_refused() {
    use crate::savepoint::codec::{DecodeError, Reader};
    let e = worst_case::warmed_up();
    let mut s = live(&e).1.clone();
    let mut w = Writer::new();
    s.encode(&mut w);
    let ok = w.finish();
    assert!(State::decode_over(&mut Reader::new(&ok), &s).is_ok());
    s.next_id = 0;
    let mut w = Writer::new();
    s.encode(&mut w);
    let bad = w.finish();
    assert_eq!(
        State::decode_over(&mut Reader::new(&bad), &s).err(),
        Some(DecodeError::Invalid("next entity id in the player slots"))
    );
}
