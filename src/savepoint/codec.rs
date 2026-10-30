//! The writer and reader of the savepoint bytes, the `Encode` and `Decode` traits, and the
//! impls for the leaf and foreign types. The state types' impls are in `sim::state_codec`.
//!
//! Integers are little-endian at their declared width; no `usize` is ever encoded. A list is a
//! `u32` length, checked against the collection's cap and against the bytes left before
//! anything is allocated. The reader never panics on any input.

use crate::attack::{DamageType, ZoneShape};
use crate::behaviour::BossCall;
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId};
use crate::inline::InlineVec;
use crate::kinds::KindIndex;
use crate::rng::SplitMix64;
use crate::sim::{AttackIndex, BossIndex, DefIndex, ProgramIndex};

/// Why some bytes are not a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The bytes end before the value does.
    Truncated,
    /// The bytes are complete but say something the format does not allow.
    Invalid(&'static str),
}

/// Bytes being written. The writer cannot fail.
pub(crate) struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub(crate) fn new() -> Self {
        Writer { buf: Vec::new() }
    }
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Writer {
            buf: Vec::with_capacity(capacity),
        }
    }
    pub(crate) fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub(crate) fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }
    pub(crate) fn bytes(&mut self, v: &[u8]) {
        self.buf.extend_from_slice(v);
    }
    /// The length of a list, as a `u32`.
    pub(crate) fn len(&mut self, len: usize) {
        self.u32(u32::try_from(len).expect("a list is shorter than u32::MAX"));
    }
    pub(crate) fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// Bytes being read. Every read is bounds-checked, and a read that fails consumes nothing.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Reader { data }
    }
    pub(crate) fn remaining(&self) -> usize {
        self.data.len()
    }
    fn take<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let (head, rest) = self
            .data
            .split_first_chunk::<N>()
            .ok_or(DecodeError::Truncated)?;
        let head = *head;
        self.data = rest;
        Ok(head)
    }
    pub(crate) fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(u8::from_le_bytes(self.take()?))
    }
    pub(crate) fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    pub(crate) fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    pub(crate) fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.take()?))
    }
    pub(crate) fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.take()?))
    }
    pub(crate) fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.take()?))
    }
    pub(crate) fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.data.first() {
            None => Err(DecodeError::Truncated),
            Some(0) => self.u8().map(|_| false),
            Some(1) => self.u8().map(|_| true),
            Some(_) => Err(DecodeError::Invalid("bool")),
        }
    }
    pub(crate) fn bytes(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let (head, rest) = self
            .data
            .split_at_checked(n)
            .ok_or(DecodeError::Truncated)?;
        self.data = rest;
        Ok(head)
    }
    /// The length of a list of items of at least one byte each: at most `cap`, and no more than
    /// the bytes left.
    pub(crate) fn list_len(&mut self, cap: u32) -> Result<usize, DecodeError> {
        let n = self.u32()?;
        if n > cap {
            return Err(DecodeError::Invalid("list too long"));
        }
        let n = usize::try_from(n).map_err(|_| DecodeError::Invalid("list too long"))?;
        if n > self.data.len() {
            return Err(DecodeError::Truncated);
        }
        Ok(n)
    }
    /// Ok if every byte was read.
    pub(crate) fn finish(self) -> Result<(), DecodeError> {
        if self.data.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::Invalid("trailing bytes"))
        }
    }
}

/// A value that writes itself as savepoint bytes. An impl for a struct destructures it without
/// `..`, as its `StableHash` impl does.
pub(crate) trait Encode {
    fn encode(&self, w: &mut Writer);
}

/// A value read back from savepoint bytes.
pub(crate) trait Decode: Sized {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError>;
}

/// A list of at most `cap` items, read with the length checks of `Reader::list_len`.
pub(crate) fn decode_vec<T: Decode>(r: &mut Reader<'_>, cap: u32) -> Result<Vec<T>, DecodeError> {
    let n = r.list_len(cap)?;
    let mut items = Vec::with_capacity(n);
    for _ in 0..n {
        items.push(T::decode(r)?);
    }
    Ok(items)
}

macro_rules! int_codec {
    ($($t:ty => $m:ident),* $(,)?) => {$(
        impl Encode for $t {
            fn encode(&self, w: &mut Writer) {
                w.$m(*self);
            }
        }
        impl Decode for $t {
            fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
                r.$m()
            }
        }
    )*};
}
int_codec!(bool => bool, u8 => u8, u16 => u16, u32 => u32, i32 => i32, u64 => u64, i64 => i64);

/// A newtype or struct of numbers, written as its fields in order.
macro_rules! newtype_codec {
    ($($t:ident($inner:ty)),* $(,)?) => {$(
        impl Encode for $t {
            fn encode(&self, w: &mut Writer) {
                let $t(n) = self;
                n.encode(w);
            }
        }
        impl Decode for $t {
            fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
                Ok($t(<$inner>::decode(r)?))
            }
        }
    )*};
}
newtype_codec!(
    Fx(i32),
    EntityId(u32),
    DefIndex(u16),
    BossIndex(u16),
    AttackIndex(u16),
    ProgramIndex(u32),
    KindIndex(u16),
);

impl Encode for Point {
    fn encode(&self, w: &mut Writer) {
        let Point { x, y } = self;
        x.encode(w);
        y.encode(w);
    }
}

impl Decode for Point {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Point {
            x: Fx::decode(r)?,
            y: Fx::decode(r)?,
        })
    }
}

impl Encode for SplitMix64 {
    fn encode(&self, w: &mut Writer) {
        w.u64(self.state());
    }
}

impl Decode for SplitMix64 {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(SplitMix64::new(r.u64()?))
    }
}

impl<T: Encode + ?Sized> Encode for Box<T> {
    fn encode(&self, w: &mut Writer) {
        (**self).encode(w);
    }
}

impl<T: Decode> Decode for Box<T> {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        T::decode(r).map(Box::new)
    }
}

impl<T: Encode> Encode for [T] {
    fn encode(&self, w: &mut Writer) {
        w.len(self.len());
        for item in self {
            item.encode(w);
        }
    }
}

impl<T: Encode> Encode for Vec<T> {
    fn encode(&self, w: &mut Writer) {
        self.as_slice().encode(w);
    }
}

impl<T: Encode, const N: usize> Encode for [T; N] {
    /// The length is part of the type, so only the items are written.
    fn encode(&self, w: &mut Writer) {
        for item in self {
            item.encode(w);
        }
    }
}

impl<T: Decode, const N: usize> Decode for [T; N] {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let mut items = Vec::with_capacity(N);
        for _ in 0..N {
            items.push(T::decode(r)?);
        }
        items
            .try_into()
            .map_err(|_| DecodeError::Invalid("array length"))
    }
}

impl<A: Encode, B: Encode> Encode for (A, B) {
    fn encode(&self, w: &mut Writer) {
        self.0.encode(w);
        self.1.encode(w);
    }
}

impl<A: Decode, B: Decode> Decode for (A, B) {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok((A::decode(r)?, B::decode(r)?))
    }
}

/// The most bytes of a `ContentId` (`registry` holds an ID to 32 characters).
const MAX_CONTENT_ID_BYTES: u32 = 32;

impl<T: Encode> Encode for Option<T> {
    fn encode(&self, w: &mut Writer) {
        match self {
            None => w.u8(0),
            Some(v) => {
                w.u8(1);
                v.encode(w);
            }
        }
    }
}

impl<T: Decode> Decode for Option<T> {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        match r.u8()? {
            0 => Ok(None),
            1 => T::decode(r).map(Some),
            _ => Err(DecodeError::Invalid("option tag")),
        }
    }
}

impl Encode for ContentId {
    fn encode(&self, w: &mut Writer) {
        w.len(self.as_str().len());
        w.bytes(self.as_str().as_bytes());
    }
}

impl Decode for ContentId {
    /// The ID goes through `ContentId::from`, which does not check the character set: the
    /// registry's validator does, and a savepoint is trusted only after its pack hash.
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let n = r.list_len(MAX_CONTENT_ID_BYTES)?;
        let text = std::str::from_utf8(r.bytes(n)?).map_err(|_| DecodeError::Invalid("utf8"))?;
        Ok(ContentId::from(text))
    }
}

impl<T: Encode + Copy, const N: usize> Encode for InlineVec<T, N> {
    /// The live items only: the unused places hold whatever `pop` left there.
    fn encode(&self, w: &mut Writer) {
        let items: &[T] = self;
        items.encode(w);
    }
}

impl<T: Decode + Copy + Default, const N: usize> Decode for InlineVec<T, N> {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let cap = u32::try_from(N).map_err(|_| DecodeError::Invalid("list too long"))?;
        let n = r.list_len(cap)?;
        let mut list = InlineVec::new();
        for _ in 0..n {
            if !list.try_push(T::decode(r)?) {
                return Err(DecodeError::Invalid("list over its capacity"));
            }
        }
        Ok(list)
    }
}

impl Encode for DamageType {
    fn encode(&self, w: &mut Writer) {
        w.u8(match self {
            DamageType::Slash => 0,
            DamageType::Pierce => 1,
            DamageType::Strike => 2,
            DamageType::Pyro => 3,
            DamageType::Hydro => 4,
            DamageType::Electro => 5,
            DamageType::Cryo => 6,
            DamageType::Anemo => 7,
            DamageType::Geo => 8,
            DamageType::Dendro => 9,
        });
    }
}

impl Decode for DamageType {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(match r.u8()? {
            0 => DamageType::Slash,
            1 => DamageType::Pierce,
            2 => DamageType::Strike,
            3 => DamageType::Pyro,
            4 => DamageType::Hydro,
            5 => DamageType::Electro,
            6 => DamageType::Cryo,
            7 => DamageType::Anemo,
            8 => DamageType::Geo,
            9 => DamageType::Dendro,
            _ => return Err(DecodeError::Invalid("damage type tag")),
        })
    }
}

impl Encode for ZoneShape {
    fn encode(&self, w: &mut Writer) {
        match self {
            ZoneShape::Circle { radius } => {
                w.u8(0);
                radius.encode(w);
            }
            ZoneShape::Arc { radius, half_angle } => {
                w.u8(1);
                radius.encode(w);
                half_angle.encode(w);
            }
            ZoneShape::Line { length, width } => {
                w.u8(2);
                length.encode(w);
                width.encode(w);
            }
        }
    }
}

impl Decode for ZoneShape {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        match r.u8()? {
            0 => Ok(ZoneShape::Circle {
                radius: Fx::decode(r)?,
            }),
            1 => Ok(ZoneShape::Arc {
                radius: Fx::decode(r)?,
                half_angle: i32::decode(r)?,
            }),
            2 => Ok(ZoneShape::Line {
                length: Fx::decode(r)?,
                width: Fx::decode(r)?,
            }),
            _ => Err(DecodeError::Invalid("zone shape tag")),
        }
    }
}

impl Encode for BossCall {
    fn encode(&self, w: &mut Writer) {
        w.u8(match self {
            BossCall::Join => 0,
            BossCall::Leave => 1,
            BossCall::Stay => 2,
        });
    }
}

impl Decode for BossCall {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        match r.u8()? {
            0 => Ok(BossCall::Join),
            1 => Ok(BossCall::Leave),
            2 => Ok(BossCall::Stay),
            _ => Err(DecodeError::Invalid("boss call tag")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::savepoint::testing::{from_bytes, round_trip, to_bytes};

    type Read = fn(&mut Reader<'_>) -> Result<(), DecodeError>;

    fn written(f: impl FnOnce(&mut Writer)) -> Vec<u8> {
        let mut w = Writer::new();
        f(&mut w);
        w.finish()
    }

    #[test]
    fn the_writer_writes_little_endian_at_the_declared_width() {
        assert_eq!(written(|w| w.u8(7)), [7]);
        assert_eq!(written(|w| w.u16(0x0102)), [2, 1]);
        assert_eq!(written(|w| w.u32(1)), [1, 0, 0, 0]);
        assert_eq!(written(|w| w.i32(-2)), [0xfe, 0xff, 0xff, 0xff]);
        assert_eq!(written(|w| w.u64(1)), [1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(written(|w| w.i64(-1)), [0xff; 8]);
        assert_eq!(written(|w| w.bool(true)), [1]);
        assert_eq!(written(|w| w.bool(false)), [0]);
        assert_eq!(written(|w| w.len(3)), [3, 0, 0, 0]);
        assert_eq!(written(|w| w.bytes(&[9, 8])), [9, 8]);
        let mut w = Writer::with_capacity(16);
        w.u8(1);
        assert_eq!(w.finish(), [1]);
    }

    #[test]
    fn the_reader_reads_what_the_writer_wrote() {
        let bytes = written(|w| {
            w.u8(1);
            w.u16(2);
            w.u32(3);
            w.i32(-4);
            w.u64(5);
            w.i64(-6);
            w.bool(true);
        });
        let mut r = Reader::new(&bytes);
        assert_eq!(r.u8(), Ok(1));
        assert_eq!(r.u16(), Ok(2));
        assert_eq!(r.u32(), Ok(3));
        assert_eq!(r.i32(), Ok(-4));
        assert_eq!(r.u64(), Ok(5));
        assert_eq!(r.i64(), Ok(-6));
        assert_eq!(r.bool(), Ok(true));
        assert_eq!(r.remaining(), 0);
        assert_eq!(r.finish(), Ok(()));
    }

    /// A read that fails leaves `remaining()` as it was, so the choice is stated here: a failed
    /// read consumes nothing.
    #[test]
    fn a_read_past_the_end_is_truncated_and_consumes_nothing() {
        let reads: [(&str, usize, Read); 8] = [
            ("u8", 1, |r| r.u8().map(drop)),
            ("u16", 2, |r| r.u16().map(drop)),
            ("u32", 4, |r| r.u32().map(drop)),
            ("i32", 4, |r| r.i32().map(drop)),
            ("u64", 8, |r| r.u64().map(drop)),
            ("i64", 8, |r| r.i64().map(drop)),
            ("bool", 1, |r| r.bool().map(drop)),
            ("bytes", 4, |r| r.bytes(4).map(drop)),
        ];
        for (name, width, read) in reads {
            for short in [0, width - 1] {
                let data = vec![0u8; short];
                let mut r = Reader::new(&data);
                assert_eq!(read(&mut r), Err(DecodeError::Truncated), "{name} {short}");
                assert_eq!(r.remaining(), short, "{name} {short}");
            }
        }
    }

    #[test]
    fn a_bool_is_zero_or_one() {
        assert_eq!(Reader::new(&[2]).bool(), Err(DecodeError::Invalid("bool")));
    }

    #[test]
    fn finish_refuses_trailing_bytes() {
        let mut r = Reader::new(&[1, 2]);
        r.u8().unwrap();
        assert_eq!(r.finish(), Err(DecodeError::Invalid("trailing bytes")));
    }

    #[test]
    fn a_list_longer_than_its_cap_is_refused() {
        let data = [5, 0, 0, 0, 1, 1, 1, 1, 1];
        assert_eq!(
            Reader::new(&data).list_len(4),
            Err(DecodeError::Invalid("list too long"))
        );
        assert_eq!(Reader::new(&data).list_len(5), Ok(5));
    }

    #[test]
    fn a_list_longer_than_the_bytes_left_is_truncated() {
        assert_eq!(
            Reader::new(&[3, 0, 0, 0, 1, 2]).list_len(8),
            Err(DecodeError::Truncated)
        );
        assert_eq!(Reader::new(&[2, 0, 0, 0, 1, 2]).list_len(8), Ok(2));
    }

    #[test]
    fn a_huge_length_is_refused_before_anything_is_allocated() {
        let data = [0xff, 0xff, 0xff, 0xff];
        assert_eq!(
            Reader::new(&data).list_len(4),
            Err(DecodeError::Invalid("list too long"))
        );
        // With no cap in the way the bytes left decide, and no `Vec` is reserved first.
        assert_eq!(
            Reader::new(&data).list_len(u32::MAX),
            Err(DecodeError::Truncated)
        );
        assert_eq!(
            decode_vec::<u32>(&mut Reader::new(&data), u32::MAX),
            Err(DecodeError::Truncated)
        );
    }

    #[test]
    fn a_list_of_u32_round_trips_through_decode_vec() {
        let bytes = written(|w| {
            w.len(2);
            7u32.encode(w);
            8u32.encode(w);
        });
        let mut r = Reader::new(&bytes);
        assert_eq!(decode_vec::<u32>(&mut r, 4), Ok(vec![7, 8]));
        assert_eq!(r.finish(), Ok(()));
    }

    #[test]
    fn an_option_is_a_tag_then_the_value() {
        assert_eq!(to_bytes(&None::<u32>), [0]);
        assert_eq!(to_bytes(&Some(5u32)), [1, 5, 0, 0, 0]);
        assert_eq!(from_bytes::<Option<u32>>(&[0]), Ok(None));
        assert_eq!(from_bytes::<Option<u32>>(&[1, 5, 0, 0, 0]), Ok(Some(5)));
        assert_eq!(
            from_bytes::<Option<u32>>(&[2]),
            Err(DecodeError::Invalid("option tag"))
        );
        assert_eq!(
            from_bytes::<Option<u32>>(&[1, 5]),
            Err(DecodeError::Truncated)
        );
    }

    #[test]
    fn an_enum_tag_past_the_last_variant_is_refused() {
        assert_eq!(from_bytes::<DamageType>(&[9]), Ok(DamageType::Dendro));
        assert!(matches!(
            from_bytes::<DamageType>(&[10]),
            Err(DecodeError::Invalid(_))
        ));
        assert!(matches!(
            from_bytes::<ZoneShape>(&[3, 0, 0, 0, 0]),
            Err(DecodeError::Invalid(_))
        ));
        assert_eq!(from_bytes::<BossCall>(&[2]), Ok(BossCall::Stay));
        assert!(matches!(
            from_bytes::<BossCall>(&[3]),
            Err(DecodeError::Invalid(_))
        ));
    }

    #[test]
    fn every_damage_type_and_zone_shape_round_trips() {
        for t in DamageType::ALL {
            round_trip(&t);
        }
        round_trip(&ZoneShape::Circle { radius: Fx(3) });
        round_trip(&ZoneShape::Arc {
            radius: Fx(3),
            half_angle: -4,
        });
        round_trip(&ZoneShape::Line {
            length: Fx(5),
            width: Fx(6),
        });
        for c in [BossCall::Join, BossCall::Leave, BossCall::Stay] {
            round_trip(&c);
        }
        // The tags are those of the types' `StableHash`, and of the declaration order.
        assert_eq!(to_bytes(&DamageType::Pyro), [3]);
        assert_eq!(to_bytes(&BossCall::Leave), [1]);
    }

    #[test]
    fn a_content_id_is_a_length_then_utf8() {
        assert_eq!(to_bytes(&ContentId::from("ab")), [2, 0, 0, 0, b'a', b'b']);
        round_trip(&ContentId::from("giant_sword"));
        let mut long = vec![33, 0, 0, 0];
        long.extend([b'a'; 33]);
        assert_eq!(
            from_bytes::<ContentId>(&long),
            Err(DecodeError::Invalid("list too long"))
        );
        let mut max = vec![32, 0, 0, 0];
        max.extend([b'a'; 32]);
        round_trip(&ContentId::from("a".repeat(32)));
        assert!(from_bytes::<ContentId>(&max).is_ok());
        assert_eq!(
            from_bytes::<ContentId>(&[1, 0, 0, 0, 0xff]),
            Err(DecodeError::Invalid("utf8"))
        );
        assert_eq!(
            from_bytes::<ContentId>(&[5, 0, 0, 0, b'a']),
            Err(DecodeError::Truncated)
        );
    }

    #[test]
    fn an_inline_list_is_its_live_items_and_no_more_than_its_size() {
        let over = [4, 0, 0, 0, 1, 2, 3, 4];
        assert_eq!(
            from_bytes::<InlineVec<u8, 3>>(&over),
            Err(DecodeError::Invalid("list too long"))
        );
        let mut list: InlineVec<u8, 3> = [1, 2, 3].into_iter().collect();
        list.pop();
        // The stale 3 still sits in the third place; the bytes hold the live items only.
        assert_eq!(to_bytes(&list), [2, 0, 0, 0, 1, 2]);
        round_trip(&list);
        let full: InlineVec<u8, 3> = [1, 2, 3].into_iter().collect();
        round_trip(&full);
        round_trip(&InlineVec::<u8, 3>::new());
    }

    #[test]
    fn the_numbers_and_points_have_their_layout() {
        assert_eq!(to_bytes(&Fx(-2)), [0xfe, 0xff, 0xff, 0xff]);
        assert_eq!(
            to_bytes(&Point { x: Fx(1), y: Fx(2) }),
            [1, 0, 0, 0, 2, 0, 0, 0]
        );
        assert_eq!(to_bytes(&EntityId(7)), [7, 0, 0, 0]);
        assert_eq!(to_bytes(&DefIndex(0x0102)), [2, 1]);
        assert_eq!(to_bytes(&ProgramIndex(1)), [1, 0, 0, 0]);
        round_trip(&AttackIndex(9));
        round_trip(&BossIndex(9));
        round_trip(&KindIndex(9));
        let rng = SplitMix64::new(0x0102_0304_0506_0708);
        assert_eq!(to_bytes(&rng), [8, 7, 6, 5, 4, 3, 2, 1]);
        round_trip(&rng);
        round_trip(&(EntityId(1), 2u32));
        round_trip(&Box::new(3u16));
        round_trip(&[1u32, 2, 3]);
        let hits = vec![(EntityId(1), 2u32), (EntityId(3), 4)];
        let bytes = to_bytes(&hits);
        assert_eq!(bytes.len(), 4 + 2 * 8);
        let mut r = Reader::new(&bytes);
        assert_eq!(decode_vec::<(EntityId, u32)>(&mut r, 2), Ok(hits));
        assert_eq!(
            decode_vec::<(EntityId, u32)>(&mut Reader::new(&bytes), 1),
            Err(DecodeError::Invalid("list too long"))
        );
        assert_eq!(to_bytes(&[1u32, 2]), [1, 0, 0, 0, 2, 0, 0, 0]);
    }
}
