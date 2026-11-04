//! The state holds its players as `players`, addressed by a `PlayerIdx` (ADR-SIM-7), and a system
//! acts on the player it is given. The hash and the savepoint write one block per present player (`hash_fields_tests`).

use super::state::{Career, MAX_PLAYERS, PlayerIdx, Players, State};
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
    s2.players = Players::of(vec![s.players[PlayerIdx::SOLO].clone(), p1])
        .expect("two players")
        .keeping_career_of(&s.players);
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
fn a_second_player_is_a_block_of_the_hash() {
    let e = worst_case::warmed_up();
    let (_, s) = live(&e);
    let s2 = two_players(s);
    assert_ne!(hash_of(&s2), hash_of(s));
    let mut swapped = s2.clone();
    let v: Vec<_> = s2.players.iter().rev().cloned().collect();
    swapped.players = Players::of(v).unwrap().keeping_career_of(&s2.players);
    assert_ne!(hash_of(&swapped), hash_of(&s2));
    let mut hurt = s2.clone();
    hurt.players[p1()].hp -= 1;
    assert_ne!(hash_of(&hurt), hash_of(&s2));
    assert_eq!(hash_of(s), e.state_hash());
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

// ---- Slots 0 to 2 are reserved (ADR-SIM-7) ----

fn ps(e: &Engine) -> super::state::PlayerState {
    live(e).1.players[PlayerIdx::SOLO].clone()
}

#[test]
fn a_solo_party_holds_slot_zero_in_a_mask() {
    let e = worst_case::warmed_up();
    let p = ps(&e);
    let solo = Players::solo(p.clone(), Career::default());
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
fn a_party_with_a_hole_hashes_and_round_trips() {
    use crate::savepoint::codec::Reader;
    let e = worst_case::warmed_up();
    let mut hole = live(&e).1.clone();
    hole.players = Players::with_mask(0b101, ps(&e)).keeping_career_of(&live(&e).1.players);
    let mut two = hole.clone();
    two.players = Players::with_mask(0b011, ps(&e)).keeping_career_of(&live(&e).1.players);
    assert_ne!(hash_of(&hole), hash_of(&two), "slot 2 is not slot 1");
    let mut w = Writer::new();
    hole.encode(&mut w);
    let bytes = w.finish();
    let back = State::decode_over(&mut Reader::new(&bytes), &hole).expect("a hole is a party");
    assert_eq!(hash_of(&back), hash_of(&hole));
    assert_eq!(back.players.mask(), 0b101);
}

#[test]
fn the_hash_of_a_party_of_one_ends_with_its_presence_byte_and_two_players_differ() {
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

// The level progress is per slot: `Players.careers` (ADR-SIM-7).

mod careers {
    use super::super::state::{Progress, StatRule};
    use super::*;
    use crate::id::ContentId;
    use crate::run_stats::StatEffect;

    type Edit = fn(&mut Progress);

    fn progress() -> Progress {
        Progress {
            level: 3,
            xp: 7,
            stats: vec![StatRule {
                id: ContentId::from("attack"),
                effect: StatEffect::AttackPct,
                step: 5,
                limit: 4,
            }],
            priority: vec![0],
            pick_stats: true,
            offers_due: 1,
            offer_open: false,
            spent: vec![1],
            totals: [2; StatEffect::ALL.len()],
        }
    }

    fn party(mask: u8) -> State {
        let e = worst_case::warmed_up();
        let mut s = live(&e).1.clone();
        s.players = Players::with_mask(mask, ps(&e)).keeping_career_of(&live(&e).1.players);
        // The levels' progress starts empty; the offers stay with the stage's stars.
        s.players.career_mut(PlayerIdx::SOLO).progress = None;
        s
    }

    fn with_career(mut s: State, slot: u8, p: Option<Progress>) -> State {
        s.players.career_mut(PlayerIdx::new(slot).unwrap()).progress = p;
        s
    }

    #[test]
    fn a_solo_career_starts_empty_and_is_beside_the_player() {
        let e = worst_case::warmed_up();
        let p = ps(&e);
        let mut ps_ = Players::solo(p.clone(), Career::default());
        assert!(ps_.career(PlayerIdx::SOLO).progress.is_none());
        ps_.career_mut(PlayerIdx::SOLO).progress = Some(progress());
        assert_eq!(ps_[PlayerIdx::SOLO], p);
        let (player, career) = ps_.both_mut(PlayerIdx::SOLO);
        player.hp = 1;
        career.progress.as_mut().unwrap().level = 9;
        assert_eq!(ps_[PlayerIdx::SOLO].hp, 1);
        assert_eq!(
            ps_.career(PlayerIdx::SOLO).progress.as_ref().unwrap().level,
            9
        );
    }

    #[test]
    fn a_career_belongs_to_its_slot() {
        let mut s = with_career(party(0b111), 1, Some(progress()));
        assert!(s.players.career(PlayerIdx::SOLO).progress.is_none());
        assert!(
            s.players
                .career(PlayerIdx::new(2).unwrap())
                .progress
                .is_none()
        );
        assert!(s.players.career(p1()).progress.is_some());
        assert_eq!(s.players.other_careers().count(), 2);
        s.players = party(0b101).players;
        assert_eq!(s.players.other_careers().count(), 1);
        assert_eq!(party(0b001).players.other_careers().count(), 0);
    }

    #[test]
    fn every_hashed_field_of_slot_0s_progress_moves_the_hash() {
        let base = with_career(party(0b001), 0, Some(progress()));
        let h0 = hash_of(&base);
        let Progress {
            level: _,
            xp: _,
            stats: _,
            priority: _,
            pick_stats: _,
            offers_due: _,
            offer_open: _,
            spent: _,
            totals: _,
        } = progress();
        let edits: [(&str, Edit); 8] = [
            ("level", |p| p.level += 1),
            ("xp", |p| p.xp += 1),
            ("priority", |p| p.priority.push(1)),
            ("pick_stats", |p| p.pick_stats = !p.pick_stats),
            ("offers_due", |p| p.offers_due += 1),
            ("offer_open", |p| p.offer_open = !p.offer_open),
            ("spent", |p| p.spent[0] += 1),
            ("totals", |p| p.totals[0] += 1),
        ];
        for (name, edit) in edits {
            let mut s = base.clone();
            edit(
                s.players
                    .career_mut(PlayerIdx::SOLO)
                    .progress
                    .as_mut()
                    .unwrap(),
            );
            assert_ne!(hash_of(&s), h0, "{name}");
        }
        let mut s = base.clone();
        s.players
            .career_mut(PlayerIdx::SOLO)
            .progress
            .as_mut()
            .unwrap()
            .stats[0]
            .step += 1;
        assert_eq!(hash_of(&s), h0, "stats are content");
    }

    #[test]
    fn slots_1_and_2_are_in_the_hash_in_slot_order() {
        for (mask, top) in [(0b011u8, 2u8), (0b111, 3)] {
            let none = party(mask);
            for slot in 1..top {
                let some = with_career(none.clone(), slot, Some(progress()));
                assert_ne!(hash_of(&some), hash_of(&none), "{mask:b} slot {slot}");
                let mut moved = progress();
                moved.xp += 1;
                let more = with_career(none.clone(), slot, Some(moved));
                assert_ne!(hash_of(&more), hash_of(&some), "{mask:b} slot {slot}");
            }
        }
        let mut other = progress();
        other.level = 4;
        let a = with_career(
            with_career(party(0b111), 1, Some(progress())),
            2,
            Some(other.clone()),
        );
        let b = with_career(
            with_career(party(0b111), 1, Some(other)),
            2,
            Some(progress()),
        );
        assert_ne!(hash_of(&a), hash_of(&b));
    }

    #[test]
    fn an_absent_slot_and_slot_0_keep_their_places() {
        let solo = party(0b001);
        let mut s = solo.clone();
        s.players.careers_mut_for_test()[1].progress = Some(progress());
        assert_eq!(hash_of(&s), hash_of(&solo), "an absent slot adds nothing");
        let at0 = with_career(solo, 0, Some(progress()));
        assert_ne!(hash_of(&at0), hash_of(&party(0b001)));
        assert_ne!(
            hash_of(&with_career(party(0b011), 0, Some(progress()))),
            hash_of(&party(0b011))
        );
    }
}

// The star offers and the offer generator are per slot: `Career.offers`, `Career.offer`
// (ADR-SIM-7, ADR-SIM-2).

mod offers {
    use super::super::state::{Due, OfferState, Offers, StarField, StarState};
    use super::*;
    use crate::rewards::Rarity;
    use crate::rng::{TAG_OFFER, TAG_OFFER_1, TAG_OFFER_2, derive};
    use crate::snapshot::OfferKind;

    fn slot(i: u8) -> PlayerIdx {
        PlayerIdx::new(i).unwrap()
    }

    /// A party of `mask` over the worst case, whose slot 0 holds offers with every field set.
    fn party(mask: u8) -> State {
        let e = worst_case::warmed_up();
        let mut s = live(&e).1.clone();
        s.players = Players::with_mask(mask, ps(&e)).keeping_career_of(&live(&e).1.players);
        let c = s.players.career_mut(PlayerIdx::SOLO);
        let o = c.offers.as_mut().expect("the worst case has stars");
        o.due.push(Due {
            kind: OfferKind::Star,
            rarity: Rarity::Rare,
        });
        o.offer = Some(OfferState {
            kind: OfferKind::Boss,
            rarity: Rarity::Epic,
            candidates: vec![0, 1],
        });
        o.rerolls_left = 3;
        o.banishes_left = 2;
        o.extra_candidates = 1;
        o.taken.push(1);
        o.banished.push(true);
        o.set_counts.push(2);
        s
    }

    /// Slot `i` of `s` given a copy of slot 0's offers, and its own generator.
    fn with_offers(mut s: State, i: u8) -> State {
        let c0 = s.players.career(PlayerIdx::SOLO).clone();
        let c = s.players.careers_mut_for_test();
        c[usize::from(i)].offers = c0.offers;
        c[usize::from(i)].offer = derive(u64::from(i), TAG_OFFER);
        s
    }

    fn offers_of(s: &mut State, i: u8) -> &mut Offers {
        s.players.careers_mut_for_test()[usize::from(i)]
            .offers
            .as_mut()
            .expect("offers")
    }

    #[test]
    fn a_career_draws_its_offers_from_its_slots_stream() {
        for seed in [0, 42, u64::MAX] {
            let tags = [TAG_OFFER, TAG_OFFER_1, TAG_OFFER_2];
            let states: Vec<u64> = (0..MAX_PLAYERS)
                .map(|i| {
                    Career::new(seed, slot(i), None, None, None, None)
                        .offer
                        .state()
                })
                .collect();
            for (i, tag) in tags.iter().enumerate() {
                assert_eq!(states[i], derive(seed, *tag).state());
            }
            assert_ne!(states[0], states[1]);
            assert_ne!(states[0], states[2]);
            assert_ne!(states[1], states[2]);
        }
    }

    #[test]
    fn a_slot_0_offer_stream_is_the_one_the_run_started_with() {
        let e = Engine::new_with(&worst_case::stage(), &worst_case::content()).expect("starts");
        let (_, s) = live(&e);
        assert!(s.players.career(PlayerIdx::SOLO).offers.is_some());
        let again = Engine::new_with(&worst_case::stage(), &worst_case::content()).expect("starts");
        assert_eq!(
            s.players.career(PlayerIdx::SOLO).offer,
            live(&again).1.players.career(PlayerIdx::SOLO).offer
        );
    }

    #[test]
    fn every_field_of_slot_0s_offers_and_stream_moves_the_hash() {
        let base = party(0b001);
        let h0 = hash_of(&base);
        let Offers {
            due: _,
            offer: _,
            rerolls_left: _,
            banishes_left: _,
            extra_candidates: _,
            taken: _,
            banished: _,
            set_counts: _,
        } = base.players.career(PlayerIdx::SOLO).offers.clone().unwrap();
        type Edit = fn(&mut Offers);
        let edits: [(&str, Edit); 8] = [
            ("due", |o| {
                o.due.push(Due {
                    kind: OfferKind::Boss,
                    rarity: Rarity::Common,
                })
            }),
            ("offer", |o| o.offer.as_mut().unwrap().candidates.push(2)),
            ("rerolls_left", |o| o.rerolls_left += 1),
            ("banishes_left", |o| o.banishes_left += 1),
            ("extra_candidates", |o| o.extra_candidates += 1),
            ("taken", |o| o.taken[0] += 1),
            ("banished", |o| o.banished[0] = !o.banished[0]),
            ("set_counts", |o| o.set_counts[0] += 1),
        ];
        for (name, edit) in edits {
            let mut s = base.clone();
            edit(offers_of(&mut s, 0));
            assert_ne!(hash_of(&s), h0, "{name}");
        }
        let mut s = base.clone();
        s.players.career_mut(PlayerIdx::SOLO).offer.next_u64();
        assert_ne!(hash_of(&s), h0, "the offer stream");
    }

    #[test]
    fn every_field_of_the_run_part_moves_the_hash() {
        let mut base = party(0b001);
        base.stars.as_mut().unwrap().on_map.push(StarState {
            id: EntityId(9),
            at: crate::fixed::Point::default(),
            rarity: Rarity::Common,
        });
        let h0 = hash_of(&base);
        let StarField {
            misses: _,
            on_map: _,
            rarity: _,
            rune_pool: _,
            pool: _,
        } = base.stars.clone().unwrap();
        type Edit = fn(&mut StarField);
        let edits: [(&str, Edit); 5] = [
            ("misses", |f| f.misses += 1),
            ("on_map", |f| f.on_map[0].rarity = Rarity::Legendary),
            ("rarity", |f| f.rarity.1 = Rarity::Common),
            ("rune_pool", |f| f.rune_pool ^= 1),
            ("pool", |f| f.pool.push(0)),
        ];
        for (name, edit) in edits {
            let mut s = base.clone();
            edit(s.stars.as_mut().unwrap());
            assert_ne!(hash_of(&s), h0, "{name}");
        }
    }

    #[test]
    fn slots_1_and_2_offers_and_streams_are_in_the_hash_in_slot_order() {
        for (mask, top) in [(0b011u8, 2u8), (0b111, 3)] {
            let none = party(mask);
            for i in 1..top {
                let some = with_offers(none.clone(), i);
                assert_ne!(hash_of(&some), hash_of(&none), "{mask:b} slot {i}");
                let mut moved = some.clone();
                offers_of(&mut moved, i).rerolls_left += 1;
                assert_ne!(hash_of(&moved), hash_of(&some), "{mask:b} slot {i} offers");
                let mut drawn = some.clone();
                drawn.players.careers_mut_for_test()[usize::from(i)]
                    .offer
                    .next_u64();
                assert_ne!(hash_of(&drawn), hash_of(&some), "{mask:b} slot {i} stream");
            }
        }
        let both = with_offers(with_offers(party(0b111), 1), 2);
        let mut a = both.clone();
        offers_of(&mut a, 1).rerolls_left += 1;
        let mut b = both.clone();
        offers_of(&mut b, 2).rerolls_left += 1;
        assert_ne!(hash_of(&a), hash_of(&b), "slot 2 is written after slot 1");
        let mut swapped = both.clone();
        let c = swapped.players.careers_mut_for_test();
        c.swap(1, 2);
        assert_ne!(hash_of(&swapped), hash_of(&both), "the careers swapped");
        let mut streams = both.clone();
        let c = streams.players.careers_mut_for_test();
        let (x, y) = (c[1].offer.clone(), c[2].offer.clone());
        (c[1].offer, c[2].offer) = (y, x);
        assert_ne!(hash_of(&streams), hash_of(&both), "the streams swapped");
    }

    #[test]
    fn absent_slots_offers_are_not_hashed_and_slot_0s_sit_at_the_same_place() {
        let solo = party(0b001);
        let touched = with_offers(solo.clone(), 1);
        assert_eq!(hash_of(&touched), hash_of(&solo));
        let mut stream = solo.clone();
        stream.players.careers_mut_for_test()[2].offer.next_u64();
        assert_eq!(hash_of(&stream), hash_of(&solo));
        // Slot 0's offers move the hash alike at one player and at two.
        for mask in [0b001u8, 0b011] {
            let base = party(mask);
            let mut s = base.clone();
            offers_of(&mut s, 0).banishes_left += 1;
            assert_ne!(hash_of(&s), hash_of(&base), "{mask:b}");
            let mut s = base.clone();
            s.players.career_mut(PlayerIdx::SOLO).offer.next_u64();
            assert_ne!(hash_of(&s), hash_of(&base), "{mask:b} stream");
        }
    }

    #[test]
    fn reserve_makes_room_in_each_present_slots_due() {
        let e = worst_case::warmed_up();
        let (world, s) = live(&e);
        let cap = usize::from(
            world
                .stars
                .as_ref()
                .expect("stars")
                .cap
                .min(crate::validate::limits::MAX_STARS_ON_MAP),
        );
        for (mask, present) in [(0b111u8, [true, true, true]), (0b001, [true, false, false])] {
            let mut s = s.clone();
            s.players = Players::with_mask(mask, ps(&e)).keeping_career_of(&s.players);
            for i in 0..MAX_PLAYERS {
                let c = &mut s.players.careers_mut_for_test()[usize::from(i)];
                c.offers = Some(offers_of_empty());
            }
            super::super::reserve(world, &mut s);
            for i in 0..MAX_PLAYERS {
                let c = &s.players.careers_mut_for_test()[usize::from(i)];
                let have = c.offers.as_ref().unwrap().due.capacity();
                match present[usize::from(i)] {
                    true => assert!(have > cap, "{mask:b} slot {i}: {have} <= {cap}"),
                    false => assert_eq!(have, 0, "{mask:b} slot {i}"),
                }
            }
        }
    }

    fn offers_of_empty() -> Offers {
        Offers {
            due: Vec::new(),
            offer: None,
            rerolls_left: 0,
            banishes_left: 0,
            extra_candidates: 0,
            taken: Vec::new(),
            banished: Vec::new(),
            set_counts: Vec::new(),
        }
    }
}

// The expertise gauge, the run's Stardust and the regeneration are per slot: `Career.expertise`,
// `Career.stardust`, `Career.regen` (ADR-SIM-7).

mod career_rest {
    use super::super::state::{ExpertiseGauge, ExpertiseOffer};
    use super::super::{expertise, loadout, stardust};
    use super::*;
    use crate::perk_sets::{PerkEffect, PerkEntry};
    use crate::snapshot::ExpertiseOfferKind;
    use crate::stage::{ExpertisePoints, StardustFactor, StardustTier};

    fn slot(i: u8) -> PlayerIdx {
        PlayerIdx::new(i).unwrap()
    }

    type Edit = Box<dyn Fn(&mut ExpertiseGauge)>;

    fn gauge(k: u32) -> ExpertiseGauge {
        ExpertiseGauge {
            level: 1 + k,
            points: 2 + k,
            next: 30 + k,
            offered: 1 + k,
            offer: Some(ExpertiseOffer {
                level: 1 + k,
                kind: ExpertiseOfferKind::Common,
                candidates: vec![0, 1],
            }),
            taken: vec![1, 2],
            range_pct: 5 + k,
            style: Some(1),
            path: vec![0, 1],
        }
    }

    /// A party of `mask` over the worst case, slot 0 holding a full gauge, Stardust and regen.
    fn party(mask: u8) -> (World, State) {
        let e = worst_case::warmed_up();
        let (w, st) = live(&e);
        let mut s = st.clone();
        s.players = Players::with_mask(mask, ps(&e)).keeping_career_of(&st.players);
        let c = s.players.career_mut(PlayerIdx::SOLO);
        c.expertise = Some(gauge(0));
        c.stardust = Some(7);
        c.regen = 3;
        (w.clone(), s)
    }

    fn career(s: &mut State, i: u8) -> &mut Career {
        &mut s.players.careers_mut_for_test()[usize::from(i)]
    }

    #[test]
    fn every_field_of_slot_0s_gauge_stardust_and_regen_moves_the_hash() {
        for mask in [0b001u8, 0b011] {
            let (_, base) = party(mask);
            let mut edits: Vec<Edit> = vec![
                Box::new(|g| g.level += 1),
                Box::new(|g| g.points += 1),
                Box::new(|g| g.next += 1),
                Box::new(|g| g.offered += 1),
                Box::new(|g| g.offer = None),
                Box::new(|g| g.offer.as_mut().unwrap().level += 1),
                Box::new(|g| g.offer.as_mut().unwrap().candidates.push(2)),
                Box::new(|g| g.taken[0] += 1),
                Box::new(|g| g.range_pct += 1),
                Box::new(|g| g.style = None),
                Box::new(|g| g.path.push(1)),
            ];
            for (n, f) in edits.drain(..).enumerate() {
                let mut s = base.clone();
                f(career(&mut s, 0).expertise.as_mut().unwrap());
                assert_ne!(hash_of(&s), hash_of(&base), "{mask:b} gauge edit {n}");
            }
            let ExpertiseGauge {
                level: _,
                points: _,
                next: _,
                offered: _,
                offer: _,
                taken: _,
                range_pct: _,
                style: _,
                path: _,
            } = gauge(0);
            let ExpertiseOffer {
                level: _,
                kind: _,
                candidates: _,
            } = gauge(0).offer.unwrap();
            let mut s = base.clone();
            career(&mut s, 0).expertise = None;
            assert_ne!(hash_of(&s), hash_of(&base), "{mask:b} gauge none");
            for v in [None, Some(8)] {
                let mut s = base.clone();
                career(&mut s, 0).stardust = v;
                assert_ne!(hash_of(&s), hash_of(&base), "{mask:b} stardust {v:?}");
            }
            let mut s = base.clone();
            career(&mut s, 0).regen += 1;
            assert_ne!(hash_of(&s), hash_of(&base), "{mask:b} regen");
        }
    }

    #[test]
    fn slots_1_and_2_gauge_stardust_and_regen_are_in_the_hash_in_slot_order() {
        for (mask, top) in [(0b011u8, 2u8), (0b111, 3)] {
            let (_, none) = party(mask);
            for i in 1..top {
                let mut g = none.clone();
                career(&mut g, i).expertise = Some(gauge(0));
                assert_ne!(hash_of(&g), hash_of(&none), "{mask:b} slot {i} gauge");
                let mut g2 = g.clone();
                career(&mut g2, i).expertise.as_mut().unwrap().points += 1;
                assert_ne!(hash_of(&g2), hash_of(&g), "{mask:b} slot {i} gauge field");
                let mut d = none.clone();
                career(&mut d, i).stardust = Some(4);
                assert_ne!(hash_of(&d), hash_of(&none), "{mask:b} slot {i} stardust");
                let mut r = none.clone();
                career(&mut r, i).regen = 4;
                assert_ne!(hash_of(&r), hash_of(&none), "{mask:b} slot {i} regen");
            }
        }
        let (_, base) = party(0b111);
        let (mut x, mut y) = (base.clone(), base.clone());
        career(&mut x, 1).expertise = Some(gauge(0));
        career(&mut y, 2).expertise = Some(gauge(0));
        assert_ne!(hash_of(&x), hash_of(&y), "gauge: slot 2 after slot 1");
        let (mut x, mut y) = (base.clone(), base.clone());
        career(&mut x, 1).stardust = Some(4);
        career(&mut y, 2).stardust = Some(4);
        assert_ne!(hash_of(&x), hash_of(&y), "stardust: slot 2 after slot 1");
        let (mut x, mut y) = (base.clone(), base.clone());
        career(&mut x, 1).regen = 4;
        career(&mut y, 2).regen = 4;
        assert_ne!(hash_of(&x), hash_of(&y), "regen: slot 2 after slot 1");
        // The order inside a slot: Stardust then regen.
        let (mut x, mut y) = (base.clone(), base.clone());
        (career(&mut x, 1).stardust, career(&mut x, 1).regen) = (Some(5), 0);
        (career(&mut y, 1).stardust, career(&mut y, 1).regen) = (None, 5);
        assert_ne!(
            hash_of(&x),
            hash_of(&y),
            "stardust and regen differ in place"
        );
    }

    #[test]
    fn absent_slots_gauge_stardust_and_regen_are_not_hashed() {
        let (_, solo) = party(0b001);
        let mut s = solo.clone();
        for i in 1..MAX_PLAYERS {
            let c = career(&mut s, i);
            c.expertise = Some(gauge(1));
            c.stardust = Some(9);
            c.regen = 9;
        }
        assert_eq!(hash_of(&s), hash_of(&solo));
    }

    /// The first enemy of `s` made a wave enemy that gives expertise points and Stardust.
    fn with_rewarding_enemy(w: &mut World, s: &mut State) {
        s.enemies[0].key.source = super::super::state::AgentSource::Wave;
        s.enemies[0].key.index = 0;
        w.stage.waves[0].expertise = Some(ExpertisePoints { hit: 3, kill: 5 });
        w.stage.waves[0].stardust = Some(100);
        w.stage.stardust = Some(StardustFactor {
            tiers: vec![StardustTier {
                start_tick: 0,
                pct: 50,
            }],
            summon_base: None,
        });
    }

    #[test]
    fn credit_goes_to_the_slot_it_is_given() {
        for (me, other) in [(1u8, 0u8), (0, 1)] {
            let (mut w, mut s) = party(0b011);
            with_rewarding_enemy(&mut w, &mut s);
            career(&mut s, 0).expertise = Some(gauge(0));
            career(&mut s, 1).expertise = Some(gauge(0));
            let before = career(&mut s, other).expertise.clone();
            let points = career(&mut s, me).expertise.as_ref().unwrap().points;
            expertise::credit(&w, &mut s, slot(me), Who::Enemy(0), true);
            assert_eq!(
                career(&mut s, me).expertise.as_ref().unwrap().points,
                points + 8
            );
            assert_eq!(career(&mut s, other).expertise, before);
            expertise::credit(&w, &mut s, slot(me), Who::Enemy(0), false);
            assert_eq!(
                career(&mut s, me).expertise.as_ref().unwrap().points,
                points + 11
            );
        }
        // A slot with no gauge stays without one.
        let (mut w, mut s) = party(0b011);
        with_rewarding_enemy(&mut w, &mut s);
        career(&mut s, 1).expertise = None;
        expertise::credit(&w, &mut s, slot(1), Who::Enemy(0), true);
        assert_eq!(career(&mut s, 1).expertise, None);
    }

    #[test]
    fn stardust_is_earned_by_the_slot_it_is_given() {
        let (mut w, mut s) = party(0b011);
        with_rewarding_enemy(&mut w, &mut s);
        s.enemies[0].hp = 0;
        s.enemies[0].burst = false;
        career(&mut s, 0).stardust = Some(0);
        career(&mut s, 1).stardust = Some(0);
        stardust::credit_defeated(&w, &mut s, slot(1));
        assert_eq!(career(&mut s, 1).stardust, Some(50));
        assert_eq!(career(&mut s, 0).stardust, Some(0));
        career(&mut s, 1).stardust = None;
        stardust::credit_defeated(&w, &mut s, slot(1));
        assert_eq!(career(&mut s, 1).stardust, None);
        career(&mut s, 1).stardust = Some(crate::validate::limits::MAX_RUN_STARDUST - 1);
        stardust::credit_defeated(&w, &mut s, slot(1));
        assert_eq!(
            career(&mut s, 1).stardust,
            Some(crate::validate::limits::MAX_RUN_STARDUST)
        );
    }

    #[test]
    fn regen_and_its_perk_act_on_the_slot_they_are_given() {
        let (_, mut s) = party(0b011);
        s.tick = loadout::REGEN_TICKS * 2;
        career(&mut s, 0).regen = 0;
        career(&mut s, 1).regen = 5;
        for i in 0..2 {
            let max = s.players[slot(i)].max_hp;
            s.players[slot(i)].hp = max.saturating_sub(100).max(1);
        }
        let (h0, h1) = (s.players[slot(0)].hp, s.players[slot(1)].hp);
        loadout::regen(&mut s, slot(1));
        assert_eq!(s.players[slot(0)].hp, h0);
        assert_eq!(
            s.players[slot(1)].hp,
            (h1 + 5).min(s.players[slot(1)].max_hp)
        );
        loadout::perk(
            &mut s,
            slot(1),
            PerkEntry {
                effect: PerkEffect::Regen,
                amount: 2,
            },
        );
        assert_eq!((career(&mut s, 0).regen, career(&mut s, 1).regen), (0, 7));
    }
}

/// Routing to the acting player: the slot of an entity, the offers of each slot, and the picks
/// of a party (ADR-SIM-7, ADR-SIM-2).
mod route {
    use super::super::state::{Due, OfferState};
    use super::*;
    use crate::engine::ChoiceError;
    use crate::rewards::Rarity;
    use crate::snapshot::OfferKind;

    fn slot(i: u8) -> PlayerIdx {
        PlayerIdx::new(i).expect("a slot")
    }

    /// The worst case as a party of `n`.
    fn party(n: u8) -> Engine {
        worst_case::warmed_up().into_party(n)
    }

    fn state_mut(e: &mut Engine) -> &mut State {
        match e.run_mut() {
            Run::V2 { state, .. } => state,
            Run::V1 { .. } => panic!("the live rules"),
        }
    }

    /// Puts a due star offer on slot `i` and opens what is due.
    fn due_on(e: &mut Engine, i: u8) {
        let s = state_mut(e);
        s.players
            .career_mut(slot(i))
            .offers
            .as_mut()
            .expect("the worst case has stars")
            .due
            .push(Due {
                kind: OfferKind::Star,
                rarity: Rarity::Common,
            });
        e.open_offers_for_test();
    }

    fn offer_of(e: &Engine, i: u8) -> Option<&OfferState> {
        live(e)
            .1
            .players
            .career(slot(i))
            .offers
            .as_ref()?
            .offer
            .as_ref()
    }

    fn taken_of(e: &Engine, i: u8) -> u32 {
        let o = live(e)
            .1
            .players
            .career(slot(i))
            .offers
            .as_ref()
            .expect("offers");
        o.taken.iter().map(|t| u32::from(*t)).sum()
    }

    fn picks(of: &[(usize, u32)]) -> [Input; 3] {
        let mut v = [Input::default(); 3];
        for (i, k) in of {
            v[*i] = Input::picking(*k);
        }
        v
    }

    #[test]
    fn a_party_is_built_from_a_stage_for_one_to_three_players() {
        let bytes = include_bytes!("../../tests/corpus/v1/stages/orb_cage/stage.json");
        let stage = crate::validate::parse_and_validate(bytes).expect("a valid stage");
        for n in 1..=3u8 {
            let e = Engine::party(&stage, n);
            assert_eq!(e.players(), usize::from(n));
            assert_eq!(live(&e).1.players.len(), usize::from(n));
        }
        let s = live(&Engine::party(&stage, 3)).1.clone();
        assert_ne!(
            s.players.career(slot(1)).offer,
            s.players.career(slot(2)).offer
        );
    }

    #[test]
    fn of_id_names_the_present_slot_of_an_entity_number() {
        let e = worst_case::warmed_up();
        let ps = ps(&e);
        let p = Players::with_mask(0b011, ps.clone());
        assert_eq!(p.of_id(EntityId(0)), Some(PlayerIdx::SOLO));
        assert_eq!(p.of_id(EntityId(1)), Some(slot(1)));
        assert_eq!(p.of_id(EntityId(2)), None);
        let hole = Players::with_mask(0b101, ps);
        assert_eq!(hole.of_id(EntityId(1)), None);
        assert_eq!(hole.of_id(EntityId(2)), Some(slot(2)));
        assert_eq!(hole.of_id(EntityId(3)), None);
        assert_eq!(hole.of_id(EntityId(900)), None);
    }

    #[test]
    fn an_offer_is_open_for_the_slot_that_holds_it() {
        let mut e = party(3);
        due_on(&mut e, 1);
        let s = live(&e).1;
        assert!(super::super::offer_open(s));
        assert!(super::super::offer_open_for(s, slot(1)));
        assert!(!super::super::offer_open_for(s, PlayerIdx::SOLO));
        assert!(!super::super::offer_open_for(s, slot(2)));
        assert!(e.offer_open());
    }

    #[test]
    fn a_pick_is_checked_against_its_own_slots_offer() {
        let mut e = party(2);
        due_on(&mut e, 1);
        assert!(offer_of(&e, 1).is_some() && offer_of(&e, 0).is_none());
        assert_eq!(e.check_pick(PlayerIdx::SOLO, 0), Err(ChoiceError::NoOffer));
        assert_eq!(e.check_pick(slot(1), 0), Ok(()));
        assert_eq!(
            e.check_pick(slot(2), 0),
            Err(ChoiceError::NoPlayer { player: slot(2) })
        );
    }

    #[test]
    fn a_step_applies_the_pick_of_the_slot_with_the_offer() {
        let mut e = party(2);
        due_on(&mut e, 1);
        let (hash, tick) = (e.state_hash(), e.tick());
        let b = [0, 1].map(|i| taken_of(&e, i));
        e.step(&picks(&[(0, 0)]));
        assert_eq!(
            (e.state_hash(), e.tick()),
            (hash, tick),
            "slot 0 has no offer"
        );
        e.step(&picks(&[(1, 0)]));
        assert_eq!(e.tick(), tick, "the tick does not advance");
        assert_eq!((taken_of(&e, 0), taken_of(&e, 1)), (b[0], b[1] + 1));
    }

    #[test]
    fn every_accepted_pick_applies_and_a_refused_one_does_not_hold_back_the_rest() {
        let mut e = party(3);
        due_on(&mut e, 0);
        due_on(&mut e, 2);
        let (tick, b) = (e.tick(), [0, 1, 2].map(|i| taken_of(&e, i)));
        e.step(&picks(&[(0, 0), (2, 0)]));
        assert_eq!(
            [0, 1, 2].map(|i| taken_of(&e, i)),
            [b[0] + 1, b[1], b[2] + 1]
        );
        assert_eq!(e.tick(), tick);

        let mut e = party(3);
        due_on(&mut e, 0);
        due_on(&mut e, 2);
        let b = [0, 1, 2].map(|i| taken_of(&e, i));
        e.step(&picks(&[(0, 99), (2, 0)]));
        assert_eq!((taken_of(&e, 0), taken_of(&e, 2)), (b[0], b[2] + 1));
        assert!(offer_of(&e, 0).is_some(), "slot 0's offer is still open");
        assert!(e.offer_open());
    }

    #[test]
    fn each_slot_draws_from_its_own_stream() {
        let mut both = party(3);
        due_on(&mut both, 0);
        due_on(&mut both, 1);
        let mut only1 = party(3);
        due_on(&mut only1, 1);
        assert_eq!(
            offer_of(&both, 1).map(|o| o.candidates.clone()),
            offer_of(&only1, 1).map(|o| o.candidates.clone()),
            "slot 0's draws do not move slot 1's stream"
        );
        let s = live(&both).1;
        assert_ne!(
            s.players.career(slot(1)).offer,
            s.players.career(slot(2)).offer
        );
        assert_ne!(
            s.players.career(PlayerIdx::SOLO).offer,
            s.players.career(slot(1)).offer
        );
    }

    /// A party's world and state with the kill rewards on slots 0 and 1 and the tick's other
    /// work taken away, so that a test sees only the hits it makes.
    fn kill_state() -> (World, State) {
        let e = party(3);
        let (w, s) = live(&e);
        let mut s = s.clone();
        s.runs.clear();
        s.orbs.clear();
        s.zones.clear();
        s.echoes.clear();
        for i in 0..2 {
            let p = &mut s.players[slot(i)];
            p.burst_pct = 50;
            p.burst_radius = 400;
            p.burst_depth = 1;
            p.kill_heal_every = 2;
            p.kill_heal = 1;
            p.kills_counted = 0;
            p.hp = p.max_hp - 1;
        }
        (w.clone(), s)
    }

    fn shot_of(s: &State, by: u8, at: usize) -> Shot {
        Shot {
            at: s.enemies[at].at,
            owner: slot(by).id(),
            friendly: true,
            damage: 1_000_000,
            damage_type: None,
            breaks_charge: false,
            smash: None,
            combo: false,
            rune: None,
            chain: false,
        }
    }

    /// Player `by` hits enemy `at` for far more than it has.
    fn hit(w: &World, s: &mut State, by: u8, at: usize, ev: &mut Vec<crate::event::DomainEvent>) {
        let shot = shot_of(s, by, at);
        super::super::damage(w, s, slot(by), shot, Who::Enemy(at), ev);
    }

    /// Enemy `i` alone at its place, at `hp`: the others are far away and strong.
    fn lone(s: &mut State, i: usize, hp: u32) {
        s.enemies[i].hp = hp;
        let at = s.enemies[i].at;
        for j in 0..s.enemies.len() {
            if j != i {
                s.enemies[j].at.x.0 = at.x.0.saturating_add(900_000);
                s.enemies[j].hp = s.enemies[j].max_hp.max(1_000);
            }
        }
    }

    fn counted(s: &State, i: u8) -> u32 {
        s.players[slot(i)].kills_counted
    }

    fn bursts_in(ev: &[crate::event::DomainEvent]) -> usize {
        ev.iter()
            .filter(|e| matches!(e, crate::event::DomainEvent::KillBurst { .. }))
            .count()
    }

    #[test]
    fn a_kill_goes_to_the_player_whose_hit_brought_it_to_zero() {
        let (w, mut s) = kill_state();
        lone(&mut s, 0, 1);
        let mut ev = Vec::new();
        hit(&w, &mut s, 1, 0, &mut ev);
        ev.clear();
        super::super::collide(&w, &mut s, false, &mut ev);
        assert_eq!(bursts_in(&ev), 1, "slot 1's burst only: {ev:?}");
        assert_eq!((counted(&s, 0), counted(&s, 1)), (0, 1));

        let (w, mut s) = kill_state();
        lone(&mut s, 0, 1);
        for i in 0..2 {
            s.players[slot(i)].kill_heal_every = 1;
        }
        let mut ev = Vec::new();
        hit(&w, &mut s, 1, 0, &mut ev);
        ev.clear();
        super::super::collide(&w, &mut s, false, &mut ev);
        let healed: Vec<_> = ev
            .iter()
            .filter_map(|e| match e {
                crate::event::DomainEvent::Healed { target, .. } => Some(*target),
                _ => None,
            })
            .collect();
        assert_eq!(healed, vec![slot(1).id()]);

        let (w, mut s) = kill_state();
        lone(&mut s, 0, 1);
        let mut ev = Vec::new();
        hit(&w, &mut s, 0, 0, &mut ev);
        super::super::collide(&w, &mut s, false, &mut ev);
        assert_eq!((counted(&s, 0), counted(&s, 1)), (1, 0));
    }

    #[test]
    fn a_burst_chain_kill_is_the_bursters() {
        let (w, mut s) = kill_state();
        lone(&mut s, 0, 1);
        for i in 0..2 {
            s.players[slot(i)].kill_heal_every = 3;
        }
        let at = s.enemies[0].at;
        s.enemies[1].at = at;
        s.enemies[1].hp = 1;
        let mut ev = Vec::new();
        hit(&w, &mut s, 1, 0, &mut ev);
        super::super::collide(&w, &mut s, false, &mut ev);
        assert_eq!(
            (counted(&s, 0), counted(&s, 1)),
            (0, 2),
            "A and the one it burst"
        );
    }

    #[test]
    fn the_earlier_hit_wins_and_a_defeat_with_no_hit_goes_to_the_lowest_slot() {
        for (first, second) in [(1u8, 0u8), (0, 1)] {
            let (w, mut s) = kill_state();
            lone(&mut s, 0, 1);
            let mut ev = Vec::new();
            hit(&w, &mut s, first, 0, &mut ev);
            hit(&w, &mut s, second, 0, &mut ev);
            super::super::collide(&w, &mut s, false, &mut ev);
            assert_eq!(counted(&s, first), 1, "first {first}");
            assert_eq!(counted(&s, second), 0, "second {second}");
        }
        let (w, mut s) = kill_state();
        lone(&mut s, 0, 0);
        let mut ev = Vec::new();
        super::super::collide(&w, &mut s, false, &mut ev);
        assert_eq!((counted(&s, 0), counted(&s, 1)), (1, 0), "a shrine's win");
    }

    #[test]
    fn a_dot_kills_for_the_player_who_applied_it_last() {
        for by in [1u8, 0] {
            let (w, mut s) = kill_state();
            lone(&mut s, 0, 100);
            s.enemies[0].statuses = Default::default();
            let mut ev = Vec::new();
            super::super::runes::apply_smash_bleed(&mut s, slot(1), Who::Enemy(0), &mut ev);
            super::super::runes::apply_smash_bleed(&mut s, slot(by), Who::Enemy(0), &mut ev);
            let st = s.enemies[0]
                .statuses
                .iter()
                .find(|st| st.dot.is_some())
                .unwrap_or_else(|| panic!("a bleed: {:?}", s.enemies[0].statuses));
            assert_eq!(st.source, slot(by).id());
            for st in s.enemies[0].statuses.iter_mut() {
                if let Some(d) = st.dot.as_mut() {
                    d.next_in = 1;
                    d.damage = 1_000_000;
                }
            }
            super::super::collide(&w, &mut s, true, &mut ev);
            assert_eq!(counted(&s, by), 1, "bleed by {by}");
            assert_eq!(counted(&s, 1 - by), 0);
        }
    }

    #[test]
    fn the_kill_record_is_scratch_and_empty_after_the_tick() {
        let (w, mut s) = kill_state();
        lone(&mut s, 0, 1);
        let mut ev = Vec::new();
        hit(&w, &mut s, 1, 0, &mut ev);
        assert!(!s.scratch.killers.is_empty());
        assert!(s.clone().scratch.killers.is_empty(), "a clone starts empty");
        super::super::collide(&w, &mut s, false, &mut ev);
        assert!(s.scratch.killers.is_empty());
    }

    #[test]
    fn xp_and_stardust_are_shared_by_every_present_player() {
        let e = worst_case::warmed_up().into_party(3);
        let (w, s) = live(&e);
        let (mut w, mut s) = (w.clone(), s.clone());
        w.stage.stardust = Some(crate::stage::StardustFactor {
            tiers: vec![crate::stage::StardustTier {
                start_tick: 0,
                pct: 50,
            }],
            summon_base: None,
        });
        for i in 0..3 {
            let c = s.players.career_mut(slot(i));
            c.stardust = Some(0);
            assert!(c.progress.is_some(), "the worst case has levels");
        }
        s.enemies[0].key.source = super::super::state::AgentSource::Wave;
        s.enemies[0].key.index = 0;
        w.stage.waves[0].stardust = Some(100);
        s.enemies[0].hp = 0;
        s.enemies[0].burst = false;
        let before = [0, 1, 2].map(|i| {
            let p = s.players.career(slot(i)).progress.as_ref().unwrap();
            (p.level, p.xp)
        });
        let mut ev = Vec::new();
        s.runs.clear();
        s.orbs.clear();
        super::super::collide(&w, &mut s, false, &mut ev);
        let after = [0, 1, 2].map(|i| {
            let p = s.players.career(slot(i)).progress.as_ref().unwrap();
            (p.level, p.xp)
        });
        assert!(
            after.iter().zip(before).all(|(a, b)| *a != b),
            "{before:?} {after:?}"
        );
        assert_eq!(after[0], after[1]);
        assert_eq!(after[1], after[2]);
        let dust = [0, 1, 2].map(|i| s.players.career(slot(i)).stardust);
        assert!(
            dust[0] > Some(0) && dust[0] == dust[1] && dust[1] == dust[2],
            "{dust:?}"
        );
    }

    #[test]
    fn an_absent_slot_earns_nothing_and_nothing_panics() {
        let e = worst_case::warmed_up();
        let (w, s) = live(&e);
        let (w, mut s) = (w.clone(), s.clone());
        s.players = Players::with_mask(0b101, ps(&e)).keeping_career_of(&s.players);
        s.enemies[0].hp = 0;
        s.enemies[0].burst = false;
        s.runs.clear();
        s.orbs.clear();
        let mut ev = Vec::new();
        super::super::collide(&w, &mut s, false, &mut ev);
        assert!(s.players.career(PlayerIdx::SOLO).progress.is_some());
    }

    #[test]
    fn the_lowest_slot_takes_the_star_both_circles_cover() {
        use super::super::state::StarState;
        let e = worst_case::warmed_up().into_party(2);
        let (w, s) = live(&e);
        let (w, mut s) = (w.clone(), s.clone());
        let due = |s: &State, i: u8| s.players.career(slot(i)).offers.as_ref().unwrap().due.len();
        let at = s.players[PlayerIdx::SOLO].at;
        s.players[slot(1)].at = at;
        s.players[slot(1)].hp = s.players[slot(1)].max_hp;
        s.stars.as_mut().expect("stars").on_map = vec![StarState {
            id: EntityId(900),
            at,
            rarity: crate::rewards::Rarity::Common,
        }];
        let before = [due(&s, 0), due(&s, 1)];
        let mut ev = Vec::new();
        for me in s.players.indices() {
            super::super::stars::pick_up(&w, &mut s, me, &mut ev);
        }
        assert_eq!((due(&s, 0), due(&s, 1)), (before[0] + 1, before[1]));

        // A star only slot 1 covers goes to slot 1.
        let far = crate::fixed::Point {
            x: at.x,
            y: crate::fixed::Fx(at.y.0.saturating_add(3_000_000)),
        };
        s.players[slot(1)].at = far;
        s.stars.as_mut().unwrap().on_map = vec![StarState {
            id: EntityId(901),
            at: far,
            rarity: crate::rewards::Rarity::Common,
        }];
        let before = [due(&s, 0), due(&s, 1)];
        for me in s.players.indices() {
            super::super::stars::pick_up(&w, &mut s, me, &mut ev);
        }
        assert_eq!((due(&s, 0), due(&s, 1)), (before[0], before[1] + 1));
    }

    #[test]
    fn a_boss_kill_makes_a_boss_offer_due_to_each_present_player() {
        let e = worst_case::warmed_up().into_party(3);
        let (w, s) = live(&e);
        let (w, mut s) = (w.clone(), s.clone());
        for me in s.players.indices() {
            super::super::stars::boss_killed(&w, &mut s, me);
        }
        for i in 0..3 {
            let due = &s.players.career(slot(i)).offers.as_ref().unwrap().due;
            assert_eq!(
                due.first().map(|d| d.kind),
                Some(OfferKind::Boss),
                "slot {i}"
            );
        }
    }
}

/// The events name the player behind a hit, a heal, a buff and a kill (ADR-SIM-7).
mod stats_events {
    use super::super::state::{StatusKind, StatusState};
    use super::*;
    use crate::event::DomainEvent;
    use crate::id::EntityId;

    fn slot(i: u8) -> PlayerIdx {
        PlayerIdx::new(i).expect("a slot")
    }

    fn some(i: u8) -> Option<EntityId> {
        Some(slot(i).id())
    }

    /// A party's world and state with the tick's other work taken away.
    fn bare(n: u8) -> (World, State) {
        let e = worst_case::warmed_up().into_party(n);
        let (w, s) = live(&e);
        let mut s = s.clone();
        s.runs.clear();
        s.orbs.clear();
        s.zones.clear();
        s.echoes.clear();
        (w.clone(), s)
    }

    fn shot(s: &State, owner: EntityId, at: usize, damage: u32) -> Shot {
        Shot {
            at: s.enemies[at].at,
            owner,
            friendly: true,
            damage,
            damage_type: None,
            breaks_charge: false,
            smash: None,
            combo: false,
            rune: None,
            chain: false,
        }
    }

    fn hit_by(
        w: &World,
        s: &mut State,
        by: EntityId,
        me: u8,
        at: usize,
        damage: u32,
    ) -> Vec<DomainEvent> {
        let mut ev = Vec::new();
        let b = shot(s, by, at, damage);
        super::super::damage(w, s, slot(me), b, Who::Enemy(at), &mut ev);
        ev
    }

    fn hits(ev: &[DomainEvent]) -> Vec<(EntityId, Option<EntityId>, Option<EntityId>, bool)> {
        ev.iter()
            .filter_map(|e| match e {
                DomainEvent::Hit {
                    source,
                    player,
                    buffed_by,
                    echo,
                    ..
                } => Some((*source, *player, *buffed_by, *echo)),
                _ => None,
            })
            .collect()
    }

    fn killers(ev: &[DomainEvent]) -> Vec<Option<EntityId>> {
        ev.iter()
            .filter_map(|e| match e {
                DomainEvent::Died { killer, .. } => Some(*killer),
                _ => None,
            })
            .collect()
    }

    fn atk_up(s: &mut State, on: u8, source: EntityId, remaining: u32) {
        s.players[slot(on)].statuses.push(StatusState {
            kind: StatusKind::AtkUp,
            source,
            remaining,
            total: remaining.max(1),
            stacks: 1,
            value_pct: 10,
            dot: None,
        });
    }

    #[test]
    fn a_hit_names_the_player_that_made_it() {
        let (w, mut s) = bare(3);
        s.enemies[0].hp = 1_000_000;
        for i in 0..3 {
            let ev = hit_by(&w, &mut s, slot(i).id(), i, 0, 1);
            assert_eq!(hits(&ev), vec![(slot(i).id(), some(i), None, false)]);
        }
        // A companion's (or any non-player's) hit is credited to slot 0, as its kills are.
        let ev = hit_by(&w, &mut s, EntityId(999), 0, 0, 1);
        assert_eq!(hits(&ev), vec![(EntityId(999), some(0), None, false)]);
    }

    #[test]
    fn an_echo_names_the_player_it_was_queued_by() {
        let (w, mut s) = bare(3);
        s.enemies[0].hp = 1_000_000;
        let id = s.enemies[0].id;
        s.echoes.push(super::super::state::Echo {
            source: slot(1).id(),
            target: id,
            damage: 5,
            damage_type: None,
        });
        let mut ev = Vec::new();
        super::super::combat::deal_echoes(&w, &mut s, &mut ev);
        assert_eq!(hits(&ev), vec![(slot(1).id(), some(1), None, true)]);
    }

    #[test]
    fn the_buffer_is_named_only_for_another_present_player_with_time_left() {
        let (w, mut s) = bare(2);
        s.enemies[0].hp = 1_000_000;
        let cases: [(EntityId, u32, Option<EntityId>); 5] = [
            (slot(0).id(), 10, some(0)),
            (slot(1).id(), 10, None),
            (EntityId(999), 10, None),
            (slot(0).id(), 0, None),
            (slot(2).id(), 10, None),
        ];
        for (source, remaining, want) in cases {
            while !s.players[slot(1)].statuses.is_empty() {
                s.players[slot(1)].statuses.pop();
            }
            atk_up(&mut s, 1, source, remaining);
            let ev = hit_by(&w, &mut s, slot(1).id(), 1, 0, 1);
            assert_eq!(hits(&ev)[0].2, want, "source {source:?}, {remaining} left");
        }
    }

    #[test]
    fn a_solo_hit_never_names_a_buffer() {
        let (w, mut s) = bare(1);
        s.enemies[0].hp = 1_000_000;
        atk_up(&mut s, 0, slot(0).id(), 10);
        let ev = hit_by(&w, &mut s, slot(0).id(), 0, 0, 1);
        assert_eq!(hits(&ev), vec![(slot(0).id(), some(0), None, false)]);
    }

    #[test]
    fn a_kill_names_the_first_hitter_and_a_repeat_names_nobody() {
        let (w, mut s) = bare(3);
        s.enemies[0].hp = 1;
        let first = hit_by(&w, &mut s, slot(1).id(), 1, 0, 1_000_000);
        assert_eq!(killers(&first), vec![some(1)]);
        assert_eq!(
            super::super::killer_of(&s, 0),
            slot(1),
            "the event says what the credit says"
        );
        let again = hit_by(&w, &mut s, slot(0).id(), 0, 0, 1_000_000);
        assert!(killers(&again).iter().all(Option::is_none), "{again:?}");
    }

    #[test]
    fn a_shrine_cleared_has_no_killer() {
        let (_, s) = bare(2);
        let ev = super::super::died_event(&s, Who::Enemy(0), s.enemies[0].id, None);
        assert_eq!(killers(&[ev]), vec![None]);
    }

    #[test]
    fn a_heal_and_a_status_name_who_gave_them() {
        let (w, mut s) = bare(2);
        let mut ev = Vec::new();
        s.players[slot(1)].hp = 1;
        s.players[slot(1)].kill_heal = 1;
        s.players[slot(1)].kill_heal_every = 1;
        s.players[slot(1)].kills_counted = 0;
        s.enemies[0].hp = 1;
        ev.extend(hit_by(&w, &mut s, slot(1).id(), 1, 0, 1_000_000));
        super::super::collide(&w, &mut s, false, &mut ev);
        let healed: Vec<_> = ev
            .iter()
            .filter_map(|e| match e {
                DomainEvent::Healed { target, source, .. } => Some((*target, *source)),
                _ => None,
            })
            .collect();
        assert_eq!(healed, vec![(slot(1).id(), some(1))]);

        let mut ev = Vec::new();
        s.add_status(
            Who::Player(slot(0)),
            StatusKind::AtkUp,
            slot(1).id(),
            10,
            60,
            &mut ev,
        );
        s.add_status(
            Who::Player(slot(0)),
            StatusKind::Slow,
            EntityId(999),
            10,
            60,
            &mut ev,
        );
        s.add_stun(Who::Enemy(1), slot(1).id(), 30, &mut ev);
        let sources: Vec<_> = ev
            .iter()
            .filter_map(|e| match e {
                DomainEvent::StatusApplied { source, .. } => Some(*source),
                _ => None,
            })
            .collect();
        assert_eq!(sources, vec![some(1), None, some(1)]);
    }
}

#[test]
fn lookups_follow_the_mask_in_a_hole() {
    use crate::id::PlayerIdx as P;
    let mut e = Engine::start(&crate::run_setup::RunSetup::default()).unwrap();
    assert!(live(&e).1.find(EntityId(1)).is_none());
    e.join(P::Third).unwrap();
    let s = live(&e).1;
    assert!(s.find(EntityId(1)).is_none());
    assert!(matches!(s.find(EntityId(2)), Some(Who::Player(P::Third))));
    assert!(matches!(
        s.find_ally(EntityId(2)),
        Some(Who::Player(P::Third))
    ));
    assert!(s.find_ally(EntityId(1)).is_none());
}

/// Up, downed and out helper state coverage (ADR-SIM-7).
#[test]
fn player_state_up_down_out_helpers() {
    use crate::id::PlayerIdx as P;
    let mut e = Engine::start(&crate::run_setup::RunSetup::default()).unwrap();
    e.join(P::Second).unwrap();
    let mut s = live(&e).1.clone();
    assert_eq!(s.players.up_count(), 2);
    assert!(s.players.any_up());
    assert!(s.players[P::First].is_up());
    assert!(!s.players[P::First].is_downed());
    assert!(!s.players[P::First].is_out());

    s.players[P::First].hp = 0;
    assert_eq!(s.players.up_count(), 1);
    assert!(s.players.any_up());
    assert!(!s.players[P::First].is_up());
    assert!(s.players[P::First].is_downed());
    assert!(!s.players[P::First].is_out());

    s.players[P::First].out = true;
    assert_eq!(s.players.up_count(), 1);
    assert!(!s.players[P::First].is_up());
    assert!(!s.players[P::First].is_downed());
    assert!(s.players[P::First].is_out());

    s.players[P::Second].out = true;
    assert_eq!(s.players.up_count(), 0);
    assert!(!s.players.any_up());
}
