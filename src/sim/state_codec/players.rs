//! The savepoint encoding of a career.

use crate::savepoint::codec::{Decode, DecodeError, Encode, Reader, Writer};
use crate::sim::state::*;

impl Encode for Career {
    fn encode(&self, w: &mut Writer) {
        // Slot 0's offer generator and offers are written by `State`, at the places the run's
        // generator and star field were (`encode_streams`, `encode_star_field`).
        let Career {
            progress,
            offer: _,
            offers: _,
            // not hashed/saved here: written by `State`.
            expertise: _,
            // not hashed/saved here: written by `State`.
            stardust: _,
            // not hashed/saved here: written by `State`.
            regen: _,
            // not hashed/saved here: written by `State`.
            equipment: _,
        } = self;
        progress.encode(w);
    }
}

impl Decode for Career {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        // The generator and the offers come from `State`'s own places; `decode_fields` sets them.
        Ok(Career {
            progress: Decode::decode(r)?,
            ..Career::default()
        })
    }
}
