//! The canonical binary encoding of a run's state, the savepoint's bytes (ADR-SIM-3): a hand
//! codec that mirrors `StableHash`, with no dependency. `Encode` writes a value, `Decode` reads
//! it back and fails closed with a `DecodeError` on any bytes it does not accept. Nothing here
//! is public yet; the file format around it (a format constant, a golden guard) is a later task.

mod codec;

pub(crate) use codec::{Decode, DecodeError, Encode, Reader, Writer};
