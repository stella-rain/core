//! The writer and reader of the savepoint bytes, the `Encode` and `Decode` traits, and the
//! impls for the leaf and foreign types. The state types' impls are in `sim::state_codec`.
//!
//! Integers are little-endian at their declared width; no `usize` is ever encoded. A list is a
//! `u32` length, checked against the collection's cap and against the bytes left before
//! anything is allocated. The reader never panics on any input.

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
        unimplemented!()
    }
    pub(crate) fn with_capacity(_capacity: usize) -> Self {
        unimplemented!()
    }
    pub(crate) fn u8(&mut self, _v: u8) {
        unimplemented!()
    }
    pub(crate) fn u16(&mut self, _v: u16) {
        unimplemented!()
    }
    pub(crate) fn u32(&mut self, _v: u32) {
        unimplemented!()
    }
    pub(crate) fn i32(&mut self, _v: i32) {
        unimplemented!()
    }
    pub(crate) fn u64(&mut self, _v: u64) {
        unimplemented!()
    }
    pub(crate) fn i64(&mut self, _v: i64) {
        unimplemented!()
    }
    pub(crate) fn bool(&mut self, _v: bool) {
        unimplemented!()
    }
    pub(crate) fn bytes(&mut self, _v: &[u8]) {
        unimplemented!()
    }
    /// The length of a list, as a `u32`.
    pub(crate) fn len(&mut self, _len: usize) {
        unimplemented!()
    }
    pub(crate) fn finish(self) -> Vec<u8> {
        unimplemented!()
    }
}

/// Bytes being read. Every read is bounds-checked.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(_data: &'a [u8]) -> Self {
        unimplemented!()
    }
    pub(crate) fn remaining(&self) -> usize {
        unimplemented!()
    }
    pub(crate) fn u8(&mut self) -> Result<u8, DecodeError> {
        unimplemented!()
    }
    pub(crate) fn u16(&mut self) -> Result<u16, DecodeError> {
        unimplemented!()
    }
    pub(crate) fn u32(&mut self) -> Result<u32, DecodeError> {
        unimplemented!()
    }
    pub(crate) fn i32(&mut self) -> Result<i32, DecodeError> {
        unimplemented!()
    }
    pub(crate) fn u64(&mut self) -> Result<u64, DecodeError> {
        unimplemented!()
    }
    pub(crate) fn i64(&mut self) -> Result<i64, DecodeError> {
        unimplemented!()
    }
    pub(crate) fn bool(&mut self) -> Result<bool, DecodeError> {
        unimplemented!()
    }
    pub(crate) fn bytes(&mut self, _n: usize) -> Result<&'a [u8], DecodeError> {
        unimplemented!()
    }
    /// The length of a list of items of at least one byte each: at most `cap`, and no more than
    /// the bytes left.
    pub(crate) fn list_len(&mut self, _cap: u32) -> Result<usize, DecodeError> {
        unimplemented!()
    }
    /// Ok if every byte was read.
    pub(crate) fn finish(self) -> Result<(), DecodeError> {
        unimplemented!()
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
pub(crate) fn decode_vec<T: Decode>(_r: &mut Reader<'_>, _cap: u32) -> Result<Vec<T>, DecodeError> {
    unimplemented!()
}

impl Encode for u32 {
    fn encode(&self, _w: &mut Writer) {
        unimplemented!()
    }
}

impl Decode for u32 {
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let reads: [(&str, usize, fn(&mut Reader<'_>) -> Result<(), DecodeError>); 8] = [
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
}
