//! The caps, the foreign enums and the text impls that more than one area of the savepoint encoding uses.

use crate::engine::Outcome;
use crate::rewards::Rarity;
use crate::rune::Rune;
use crate::savepoint::codec::{Decode, DecodeError, Encode, Reader, Writer};
use crate::sim::world::RangedPool;

/// A fieldless enum: a literal tag per variant, the numbers its `StableHash` writes, and no
/// wildcard arm, so a variant added without a number does not compile.
macro_rules! enum_codec {
    ($($ty:ty, $what:literal: { $($variant:path => $tag:literal),* $(,)? });* $(;)?) => {$(
        impl Encode for $ty {
            fn encode(&self, w: &mut Writer) {
                w.u8(match self {
                    $($variant => $tag,)*
                });
            }
        }

        impl Decode for $ty {
            fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
                match r.u8()? {
                    $($tag => Ok($variant),)*
                    _ => Err(DecodeError::Invalid($what)),
                }
            }
        }
    )*};
}

enum_codec! {
    Outcome, "outcome tag": {
        Outcome::Running => 0,
        Outcome::Cleared => 1,
        Outcome::Failed => 2,
    };
    Rarity, "rarity tag": {
        Rarity::Common => 0,
        Rarity::Uncommon => 1,
        Rarity::Rare => 2,
        Rarity::Epic => 3,
        Rarity::Legendary => 4,
        Rarity::Supernova => 5,
    };
    Rune, "rune tag": {
        Rune::Explosion => 0,
        Rune::Thunder => 1,
        Rune::Boomerang => 2,
        Rune::HomingMissiles => 3,
        Rune::Bleed => 4,
        Rune::Poison => 5,
        Rune::Burn => 6,
    };
    RangedPool, "ranged pool tag": {
        RangedPool::Zone => 0,
        RangedPool::Line => 1,
    };
}

/// A cap, from the constant that bounds the list in play.
pub(super) fn cap_u32(n: usize) -> u32 {
    u32::try_from(n).expect("a small cap")
}

/// The cap of a list whose length is the content's (waves, bosses): `decode_state` checks it
/// against the fresh engine's, and the bytes left bound the length before that.
pub(super) const MAX_CONTENT_LEN: u32 = 1 << 16;

/// The most bytes of a name the ledger and the tallies hold (a kill key, a star's name).
const MAX_TEXT_BYTES: u32 = 256;

impl Encode for str {
    fn encode(&self, w: &mut Writer) {
        w.len(self.len());
        w.bytes(self.as_bytes());
    }
}

impl Encode for String {
    fn encode(&self, w: &mut Writer) {
        self.as_str().encode(w);
    }
}

impl Encode for std::sync::Arc<str> {
    fn encode(&self, w: &mut Writer) {
        (**self).encode(w);
    }
}

/// The text of at most `MAX_TEXT_BYTES` bytes.
fn decode_text<'a>(r: &mut Reader<'a>) -> Result<&'a str, DecodeError> {
    let n = r.list_len(MAX_TEXT_BYTES)?;
    std::str::from_utf8(r.bytes(n)?).map_err(|_| DecodeError::Invalid("utf8"))
}

impl Decode for String {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        decode_text(r).map(str::to_owned)
    }
}

impl Decode for std::sync::Arc<str> {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        decode_text(r).map(std::sync::Arc::from)
    }
}
