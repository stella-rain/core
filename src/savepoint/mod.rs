//! The canonical binary encoding of a run's state, the savepoint's bytes (ADR-SIM-3): a hand
//! codec that mirrors `StableHash`, with no dependency. `Encode` writes a value, `Decode` reads
//! it back and fails closed with a `DecodeError` on any bytes it does not accept. Nothing here
//! is public yet; the file format around it (a format constant, a golden guard) is a later task.

mod codec;

// Used by the state codec and the savepoint file, which come next.
#[allow(unused_imports)]
pub(crate) use codec::{Decode, DecodeError, Encode, Reader, Writer};

/// What the tests of the codec share.
#[cfg(test)]
pub(crate) mod testing {
    use std::fmt::Debug;

    use super::{Decode, DecodeError, Encode, Reader, Writer};

    /// The bytes of `v`.
    pub(crate) fn to_bytes<T: Encode>(v: &T) -> Vec<u8> {
        let mut w = Writer::new();
        v.encode(&mut w);
        w.finish()
    }

    /// `T` from all of `bytes`.
    pub(crate) fn from_bytes<T: Decode>(bytes: &[u8]) -> Result<T, DecodeError> {
        let mut r = Reader::new(bytes);
        let v = T::decode(&mut r)?;
        r.finish()?;
        Ok(v)
    }

    /// `v` decodes from its bytes to a value that equals it and encodes to the same bytes.
    pub(crate) fn round_trip<T: Encode + Decode + PartialEq + Debug>(v: &T) {
        let bytes = to_bytes(v);
        let back: T = from_bytes(&bytes).unwrap_or_else(|e| panic!("{v:?}: {e:?}"));
        assert_eq!(&back, v);
        assert_eq!(to_bytes(&back), bytes);
    }
}
