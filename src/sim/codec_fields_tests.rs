//! The savepoint bytes of the live `State` mirror the hash layout (ADR-SAVE-14, ADR-SIM-17): the
//! shared fields, one presence byte, the block of each present player in slot order. A party
//! round-trips at `State` level here; an engine starts solo until a party can be set up.

use super::state::{MAX_PLAYERS, Offers, PlayerIdx, Players, State};
use crate::bench_fixtures::worst_case;
use crate::engine::{Engine, Run};
use crate::hash::{StableHash, StateHasher};
use crate::savepoint::codec::{DecodeError, Reader};
use crate::savepoint::testing::{from_bytes, round_trip, to_bytes};

fn warm() -> State {
    match worst_case::warmed_up().run() {
        Run::V2 { state, .. } => state.clone(),
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn fresh_engine_state() -> State {
    let e = Engine::new_with(&worst_case::stage(), &worst_case::content()).expect("starts");
    match e.run() {
        Run::V2 { state, .. } => state.clone(),
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn hash_of(s: &State) -> u64 {
    let mut h = StateHasher::new();
    s.stable_hash(&mut h);
    h.finish()
}

fn idx(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

/// `s` with the party `mask`: every slot a copy of slot 0's career, player `i` moved right.
fn party(s: &State, mask: u8) -> State {
    let mut out = s.clone();
    let career = s.players.career(PlayerIdx::SOLO).clone();
    out.players = Players::with_mask(mask, s.players[PlayerIdx::SOLO].clone());
    for i in 0..MAX_PLAYERS {
        out.players.careers_mut_for_test()[usize::from(i)] = career.clone();
    }
    for i in out.players.indices().collect::<Vec<_>>() {
        out.players.both_mut(i).0.at.x.0 += 10 * i32::try_from(i.get()).expect("a slot");
    }
    out
}

/// The bytes of slot `i`'s block, piece by piece.
fn block(s: &State, i: PlayerIdx) -> Vec<u8> {
    let c = s.players.career(i);
    let mut out = to_bytes(&s.players[i]);
    out.extend(to_bytes(&c.progress));
    out.extend(to_bytes(&c.offer));
    out.extend(to_bytes(&c.offers));
    out.extend(to_bytes(&c.expertise));
    out.extend(to_bytes(&c.stardust));
    out.extend(to_bytes(&c.regen));
    out
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

fn decoded(bytes: &[u8], fresh: &State) -> Result<State, DecodeError> {
    State::decode_over(&mut Reader::new(bytes), fresh)
}

#[test]
fn offers_round_trip_on_their_own() {
    let s = warm();
    let offers: Offers = s
        .players
        .career(PlayerIdx::SOLO)
        .offers
        .clone()
        .expect("the stage has stars");
    round_trip(&offers);
}

#[test]
fn a_party_of_one_ends_with_the_presence_byte_and_its_block() {
    let s = warm();
    let bytes = to_bytes(&s);
    let want = tail(&s);
    assert_eq!(want[0], 1);
    assert!(bytes.ends_with(&want));
}

#[test]
fn a_party_ends_with_the_mask_and_the_blocks_in_slot_order() {
    let base = warm();
    for mask in [0b011u8, 0b101, 0b111] {
        let s = party(&base, mask);
        let bytes = to_bytes(&s);
        let want = tail(&s);
        assert!(bytes.ends_with(&want), "mask {mask:#b}");
        assert_eq!(want[0], mask);
    }
}

#[test]
fn entity_numbers_are_written_as_they_are() {
    let mut a = warm();
    let mut b = a.clone();
    a.next_id = 40;
    b.next_id = 41;
    let (ta, tb) = (to_bytes(&a), to_bytes(&b));
    let at: Vec<usize> = (0..ta.len()).filter(|&i| ta[i] != tb[i]).collect();
    assert_eq!(at.len(), 1);
    assert_eq!(
        (ta[at[0]], tb[at[0]]),
        (40, 41),
        "the base writes 38 and 39"
    );
}

#[test]
fn a_party_round_trips_with_the_same_bytes_and_hash() {
    let base = warm();
    for mask in [0b001u8, 0b011, 0b101, 0b111] {
        let s = party(&base, mask);
        let bytes = to_bytes(&s);
        let back = decoded(&bytes, &s).unwrap_or_else(|e| panic!("mask {mask:#b}: {e:?}"));
        assert_eq!(to_bytes(&back), bytes, "mask {mask:#b}");
        assert_eq!(hash_of(&back), hash_of(&s), "mask {mask:#b}");
        for cut in 0..bytes.len() {
            match decoded(&bytes[..cut], &s) {
                Err(DecodeError::Truncated | DecodeError::Invalid(_)) => {}
                other => panic!("mask {mask:#b}, cut {cut}: {:?}", other.map(|_| ())),
            }
        }
    }
}

#[test]
fn a_bad_presence_byte_is_refused() {
    let base = warm();
    let s = party(&base, 0b011);
    let bytes = to_bytes(&s);
    let at = bytes.len() - tail(&s).len();
    assert_eq!(bytes[at], 0b011);
    for (bad, what) in [
        (0u8, "player mask"),
        (0b010, "player mask"),
        (0b1001, "player mask"),
        (0b101, "player mask differs from the setup"),
    ] {
        let mut b = bytes.clone();
        b[at] = bad;
        assert_eq!(
            decoded(&b, &s).err(),
            Some(DecodeError::Invalid(what)),
            "{bad:#b}"
        );
    }
}

#[test]
fn a_mask_unequal_to_the_fresh_engines_is_refused() {
    let solo = party(&warm(), 0b001);
    let bytes = to_bytes(&party(&warm(), 0b011));
    assert_eq!(
        decoded(&bytes, &solo).err(),
        Some(DecodeError::Invalid("player mask differs from the setup"))
    );
}

#[test]
fn a_block_of_an_absent_slot_is_trailing_bytes() {
    let base = warm();
    let s = party(&base, 0b001);
    let mut bytes = to_bytes(&s);
    bytes.extend(block(&party(&base, 0b111), idx(1)));
    let mut r = Reader::new(&bytes);
    State::decode_over(&mut r, &s).expect("the state reads");
    assert!(
        r.finish().is_err(),
        "the bytes of an absent slot are not read"
    );
    assert!(from_bytes::<crate::sim::state::Career>(&[]).is_err());
}

#[test]
fn a_next_id_inside_the_player_slots_is_refused_at_every_mask() {
    let mut s = party(&warm(), 0b011);
    s.next_id = 2;
    assert_eq!(
        decoded(&to_bytes(&s), &s).err(),
        Some(DecodeError::Invalid("next entity id in the player slots"))
    );
}

#[test]
fn the_fresh_state_is_a_party_of_one() {
    assert_eq!(fresh_engine_state().players.mask(), 0b001);
}
