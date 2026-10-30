//! The savepoint encoding of every `State` type, beside the hash in `state_hash.rs` and in the
//! same order: each impl destructures its type without `..`, so a field added without a decision
//! does not compile. Every field is saved, the content indices the hash skips included, because
//! they are what a restored run reads; the pack hash guards that they mean the same content.
//!
//! The layout is the fields in declaration order, an enum a `u8` tag (the number its
//! `StableHash` writes), an `Option` a `0`/`1` tag, a list a `u32` length (see `savepoint`).

use crate::savepoint::{Decode, DecodeError, Encode, Reader, Writer};

use super::state::*;

// ---- Attacks, orbs and zones ----

impl Encode for Dir {
    fn encode(&self, _w: &mut Writer) {
        unimplemented!()
    }
}

impl Decode for Dir {
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        unimplemented!()
    }
}

impl Encode for PathStep {
    fn encode(&self, _w: &mut Writer) {
        unimplemented!()
    }
}

impl Decode for PathStep {
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        unimplemented!()
    }
}

impl Encode for Frame {
    fn encode(&self, _w: &mut Writer) {
        unimplemented!()
    }
}

impl Decode for Frame {
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        unimplemented!()
    }
}

impl Encode for Aim {
    fn encode(&self, _w: &mut Writer) {
        unimplemented!()
    }
}

impl Decode for Aim {
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        unimplemented!()
    }
}

impl Encode for Origin {
    fn encode(&self, _w: &mut Writer) {
        unimplemented!()
    }
}

impl Decode for Origin {
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        unimplemented!()
    }
}

impl Encode for Pull {
    fn encode(&self, _w: &mut Writer) {
        unimplemented!()
    }
}

impl Decode for Pull {
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        unimplemented!()
    }
}

impl Encode for Smash {
    fn encode(&self, _w: &mut Writer) {
        unimplemented!()
    }
}

impl Decode for Smash {
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        unimplemented!()
    }
}

impl Encode for AttackTask {
    fn encode(&self, _w: &mut Writer) {
        unimplemented!()
    }
}

impl Decode for AttackTask {
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        unimplemented!()
    }
}

impl Encode for AttackRun {
    fn encode(&self, _w: &mut Writer) {
        unimplemented!()
    }
}

impl Decode for AttackRun {
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        unimplemented!()
    }
}

impl Encode for OrbLink {
    fn encode(&self, _w: &mut Writer) {
        unimplemented!()
    }
}

impl Decode for OrbLink {
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        unimplemented!()
    }
}

impl Encode for Hook {
    fn encode(&self, _w: &mut Writer) {
        unimplemented!()
    }
}

impl Decode for Hook {
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        unimplemented!()
    }
}

impl Encode for OrbState {
    fn encode(&self, _w: &mut Writer) {
        unimplemented!()
    }
}

impl Decode for OrbState {
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        unimplemented!()
    }
}

impl Encode for ZoneState {
    fn encode(&self, _w: &mut Writer) {
        unimplemented!()
    }
}

impl Decode for ZoneState {
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        unimplemented!()
    }
}
