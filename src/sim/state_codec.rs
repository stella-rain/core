//! The savepoint encoding of every `State` type, beside the hash in `state_hash.rs` and in the
//! same order: each impl destructures its type without `..`, so a field added without a decision
//! does not compile. Every field is saved, the content indices the hash skips included, because
//! they are what a restored run reads; the pack hash guards that they mean the same content.
//!
//! The layout is the fields in declaration order, an enum a `u8` tag (the number its
//! `StableHash` writes), an `Option` a `0`/`1` tag, a list a `u32` length (see `savepoint`).

mod attack_run;
mod equipment;
mod foes;
mod orbs_zones;
mod player_state;
mod players;
mod progress;
mod run;
mod shared;
mod status;

#[cfg(test)]
mod enum_tests {
    use crate::engine::Outcome;
    use crate::rewards::Rarity;
    use crate::rune::Rune;
    use crate::savepoint::codec::{Decode, DecodeError, Encode};
    use crate::savepoint::testing::{from_bytes, to_bytes};
    use crate::sim::world::RangedPool;

    /// Every variant, in order, is its number; the number after the last is refused.
    fn tags<T: Encode + Decode + PartialEq + std::fmt::Debug>(all: &[T]) {
        for (i, v) in all.iter().enumerate() {
            assert_eq!(to_bytes(v), [u8::try_from(i).unwrap()], "{v:?}");
            assert_eq!(from_bytes::<T>(&[u8::try_from(i).unwrap()]).as_ref(), Ok(v));
        }
        let past = u8::try_from(all.len()).unwrap();
        assert!(matches!(
            from_bytes::<T>(&[past]),
            Err(DecodeError::Invalid(_))
        ));
        assert_eq!(from_bytes::<T>(&[]), Err(DecodeError::Truncated));
    }

    #[test]
    fn the_enums_of_the_state_are_their_hash_numbers() {
        tags(&[Outcome::Running, Outcome::Cleared, Outcome::Failed]);
        tags(&[
            Rarity::Common,
            Rarity::Uncommon,
            Rarity::Rare,
            Rarity::Epic,
            Rarity::Legendary,
            Rarity::Supernova,
        ]);
        tags(&[
            Rune::Explosion,
            Rune::Thunder,
            Rune::Boomerang,
            Rune::HomingMissiles,
            Rune::Bleed,
            Rune::Poison,
            Rune::Burn,
        ]);
        tags(&[RangedPool::Zone, RangedPool::Line]);
    }
}

#[cfg(test)]
mod career_tests {
    use crate::savepoint::testing::{from_bytes, to_bytes};
    use crate::sim::state::Career;

    #[test]
    fn a_career_is_its_progress_and_nothing_else() {
        let e = crate::bench_fixtures::worst_case::warmed_up();
        let crate::engine::Run::V2 { state, .. } = e.run() else {
            panic!("the live rules")
        };
        assert_eq!(to_bytes(&Career::default()), [0]);
        let career = state.players.career(crate::id::PlayerIdx::SOLO).clone();
        assert_eq!(to_bytes(&career), to_bytes(&career.progress));
        let back: Career = from_bytes(&to_bytes(&career)).expect("decodes");
        assert_eq!(back.progress.is_some(), career.progress.is_some());
    }
}
