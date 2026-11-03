//! The layout of the live rules' state hash (ADR-SIM-17): the shared fields, then one presence
//! byte and the block of each present player in slot order. The tests read the bytes the hasher
//! was fed (`StateHasher::trace`), not only the digest.

use super::state::{Career, MAX_PLAYERS, PlayerIdx, Players, State};
use crate::bench_fixtures::worst_case;
use crate::engine::{Engine, Run};
use crate::hash::{StableHash, StateHasher};
use crate::id::EntityId;

fn live(e: &Engine) -> &State {
    match e.run() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn trace(s: &State) -> Vec<u8> {
    let mut h = StateHasher::new();
    s.stable_hash(&mut h);
    h.trace().to_vec()
}

fn hash_of(s: &State) -> u64 {
    let mut h = StateHasher::new();
    s.stable_hash(&mut h);
    h.finish()
}

fn idx(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

/// `s` with the party `mask`: every slot a copy of slot 0's player and career, player `i` moved
/// `10 * i` units right so the blocks differ.
fn party(s: &State, mask: u8) -> State {
    let mut out = s.clone();
    let p0 = s.players[PlayerIdx::SOLO].clone();
    let career = s.players.career(PlayerIdx::SOLO).clone();
    out.players = Players::with_mask(mask, p0.clone());
    for i in 0..MAX_PLAYERS {
        out.players.careers_mut_for_test()[usize::from(i)] = career.clone();
    }
    for i in out.players.indices().collect::<Vec<_>>() {
        out.players.both_mut(i).0.at.x.0 += 10 * i32::try_from(i.get()).expect("a slot");
    }
    out
}

/// The bytes of slot `i`'s block: the player, the career's progress, the offer generator's
/// state, the offers, the expertise gauge, the Stardust and the regeneration.
fn block(s: &State, i: PlayerIdx) -> Vec<u8> {
    let c: &Career = s.players.career(i);
    let mut h = StateHasher::new();
    s.players[i].stable_hash(&mut h);
    c.progress.stable_hash(&mut h);
    c.offer.state().stable_hash(&mut h);
    c.offers.stable_hash(&mut h);
    c.expertise.stable_hash(&mut h);
    c.stardust.stable_hash(&mut h);
    c.regen.stable_hash(&mut h);
    h.trace().to_vec()
}

fn tail(s: &State) -> Vec<u8> {
    let mut out = vec![s.players.mask()];
    // The generation of every slot follows the mask, present or not.
    out.extend(s.players.generations());
    for i in s.players.indices() {
        out.extend(block(s, i));
    }
    out
}

fn warm() -> State {
    live(&worst_case::warmed_up()).clone()
}

#[test]
fn a_party_of_one_ends_with_the_presence_byte_and_its_block() {
    let s = warm();
    let t = trace(&s);
    let want = tail(&s);
    assert_eq!(want[0], 0b001);
    assert!(
        t.ends_with(&want),
        "the hash ends with [1] and slot 0's block"
    );
}

#[test]
fn a_party_ends_with_the_mask_and_the_blocks_of_the_present_slots_in_order() {
    let base = warm();
    let mut prefix: Option<Vec<u8>> = None;
    for mask in [0b001u8, 0b011, 0b101, 0b111] {
        let s = party(&base, mask);
        let t = trace(&s);
        let want = tail(&s);
        assert!(
            t.ends_with(&want),
            "mask {mask:#b}: the mask, then the blocks"
        );
        let shared = t[..t.len() - want.len()].to_vec();
        match &prefix {
            None => prefix = Some(shared),
            Some(p) => assert_eq!(&shared, p, "mask {mask:#b}: the shared part is the same"),
        }
    }
}

#[test]
fn the_shared_part_starts_with_the_tick_and_four_generator_words() {
    let s = warm();
    let t = trace(&s);
    let mut h = StateHasher::new();
    s.tick.stable_hash(&mut h);
    s.rng.spawn.state().stable_hash(&mut h);
    s.rng.attack.state().stable_hash(&mut h);
    s.rng.loot.state().stable_hash(&mut h);
    s.rng.horde.state().stable_hash(&mut h);
    assert!(
        t.starts_with(h.trace()),
        "tick, then spawn, attack, loot and horde: the offer stream is in the blocks"
    );
}

#[test]
fn entity_numbers_are_written_as_they_are() {
    let mut a = warm();
    let mut b = a.clone();
    a.next_id = 5;
    b.next_id = 6;
    let (ta, tb) = (trace(&a), trace(&b));
    assert_eq!(ta.len(), tb.len());
    let at: Vec<usize> = (0..ta.len()).filter(|&i| ta[i] != tb[i]).collect();
    assert_eq!(at.len(), 1, "one byte differs");
    assert_eq!((ta[at[0]], tb[at[0]]), (5, 6), "5 and 6, not 3 and 4");
    let mut h = StateHasher::new();
    EntityId(5).stable_hash(&mut h);
    assert_eq!(h.trace(), &[5, 0, 0, 0]);
}

#[test]
fn an_absent_slot_is_not_hashed_and_the_order_and_the_holes_are() {
    let base = warm();
    let s = party(&base, 0b011);
    let mut edited = s.clone();
    edited.players.careers_mut_for_test()[2].regen += 1;
    assert_eq!(hash_of(&edited), hash_of(&s), "slot 2 is not in the party");

    let all = party(&base, 0b111);
    let mut swapped = all.clone();
    let (p1, p2) = (all.players[idx(1)].clone(), all.players[idx(2)].clone());
    *swapped.players.both_mut(idx(1)).0 = p2;
    *swapped.players.both_mut(idx(2)).0 = p1;
    assert_ne!(
        hash_of(&swapped),
        hash_of(&all),
        "swapping slots 1 and 2 moves it"
    );

    // The same player in slot 1 or in slot 2 is a different party.
    let mut hole = party(&base, 0b101);
    *hole.players.both_mut(idx(2)).0 = party(&base, 0b011).players[idx(1)].clone();
    assert_ne!(hash_of(&hole), hash_of(&party(&base, 0b011)));
}

// ---- Per-field coverage (hash and encoding), for every present slot ----

use super::state::{Due, OfferState, Offers};
use crate::rewards::Rarity;
use crate::rng::derive;
use crate::savepoint::codec::Encode;
use crate::savepoint::codec::Writer;
use crate::snapshot::OfferKind;

fn encoded(s: &State) -> Vec<u8> {
    let mut w = Writer::new();
    s.encode(&mut w);
    w.finish()
}

type Edit = fn(&mut State, PlayerIdx);

/// Every field of a `Career` and of `Offers` is named here without `..`: a field added later does
/// not compile until it is listed as moving the hash and the encoding (or as content).
#[allow(dead_code)]
fn every_field_is_listed(c: &Career, o: &Offers) {
    let Career {
        progress: _,  // moves: `progress` (covered by `players_tests::careers`)
        offer: _,     // moves: `offer stream`
        offers: _,    // moves: the `offers` edits below
        expertise: _, // moves: `players_tests::career_rest`
        stardust: _,  // moves: `stardust`
        regen: _,     // moves: `regen`
    } = c;
    let Offers {
        due: _,              // moves: `due`
        offer: _,            // moves: `offer`
        rerolls_left: _,     // moves: `rerolls_left`
        banishes_left: _,    // moves: `banishes_left`
        extra_candidates: _, // moves: `extra_candidates`
        taken: _,            // moves: `taken`
        banished: _,         // moves: `banished`
        set_counts: _,       // moves: `set_counts`
    } = o;
}

fn offers_of(s: &mut State, me: PlayerIdx) -> &mut Offers {
    s.players
        .career_mut(me)
        .offers
        .as_mut()
        .expect("the worst case has stars")
}

#[test]
fn every_field_of_every_present_slot_moves_the_hash_and_the_encoding() {
    let base = party(&warm(), 0b111);
    let edits: [(&str, Edit); 14] = [
        ("player.hp", |s, me| s.players.both_mut(me).0.hp += 1),
        ("player.at", |s, me| s.players.both_mut(me).0.at.x.0 += 1),
        ("offer stream", |s, me| {
            s.players.career_mut(me).offer = derive(99, 99);
        }),
        ("stardust", |s, me| {
            let c = s.players.career_mut(me);
            c.stardust = Some(c.stardust.map_or(5, |v| v + 1));
        }),
        ("regen", |s, me| s.players.career_mut(me).regen += 1),
        ("due", |s, me| {
            offers_of(s, me).due.push(Due {
                kind: OfferKind::Star,
                rarity: Rarity::Rare,
            });
        }),
        ("offer", |s, me| {
            let o = offers_of(s, me);
            o.offer = match o.offer.take() {
                Some(_) => None,
                None => Some(OfferState {
                    kind: OfferKind::Boss,
                    rarity: Rarity::Epic,
                    candidates: vec![0],
                }),
            };
        }),
        ("rerolls_left", |s, me| offers_of(s, me).rerolls_left += 1),
        ("banishes_left", |s, me| offers_of(s, me).banishes_left += 1),
        ("extra_candidates", |s, me| {
            offers_of(s, me).extra_candidates += 1;
        }),
        ("taken", |s, me| {
            let o = offers_of(s, me);
            match o.taken.first_mut() {
                Some(n) => *n += 1,
                None => o.taken.push(1),
            }
        }),
        ("banished", |s, me| {
            let o = offers_of(s, me);
            match o.banished.first_mut() {
                Some(b) => *b = !*b,
                None => o.banished.push(true),
            }
        }),
        ("set_counts", |s, me| {
            let o = offers_of(s, me);
            match o.set_counts.first_mut() {
                Some(n) => *n += 1,
                None => o.set_counts.push(1),
            }
        }),
        ("expertise", |s, me| {
            let c = s.players.career_mut(me);
            if let Some(g) = c.expertise.as_mut() {
                g.points += 1;
            } else {
                c.regen += 2;
            }
        }),
    ];
    let (h0, b0) = (hash_of(&base), encoded(&base));
    for me in base.players.indices() {
        for (name, edit) in edits {
            let mut s = base.clone();
            edit(&mut s, me);
            assert_ne!(hash_of(&s), h0, "{name} of slot {}: the hash", me.get());
            assert_ne!(encoded(&s), b0, "{name} of slot {}: the encoding", me.get());
        }
    }
}

#[test]
fn the_shared_star_field_moves_the_hash_and_the_encoding() {
    let base = party(&warm(), 0b011);
    let (h0, b0) = (hash_of(&base), encoded(&base));
    let mut s = base.clone();
    s.stars.as_mut().expect("the worst case has stars").misses += 1;
    assert_ne!(hash_of(&s), h0);
    assert_ne!(encoded(&s), b0);
}
