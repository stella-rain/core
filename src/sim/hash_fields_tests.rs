//! The layout of the live rules' state hash (ADR-SIM-3): the shared fields, then one presence
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
