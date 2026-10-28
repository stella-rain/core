//! The state hash: FNV-1a 64 over an explicit byte encoding that this crate writes itself
//! (ADR-019). Never `DefaultHasher` or a derived `Hash`: neither promises the same output
//! across Rust releases or platforms. The algorithm and encoding are part of a
//! `sim_version`'s frozen rules.
//!
//! What goes into the hash is written by `StableHash` impls (ADR-SIM-3 point 2). The encoding is
//! prefix-free: every `Option` and enum variant starts with a tag byte, every list with its
//! length, every other value is a fixed width, so two different values never write the same
//! bytes. A struct writes its fields in declaration order with nothing between them.

const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a 64 of `bytes`.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h = StateHasher::new();
    h.write_bytes(bytes);
    h.finish()
}

/// Incremental FNV-1a 64. Integers are written little-endian at their declared width.
#[derive(Clone, Debug)]
pub struct StateHasher {
    state: u64,
    /// The bytes written, for the tests of the encoding.
    #[cfg(test)]
    written: Vec<u8>,
}

impl Default for StateHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl StateHasher {
    pub fn new() -> Self {
        Self {
            state: OFFSET_BASIS,
            #[cfg(test)]
            written: Vec::new(),
        }
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        #[cfg(test)]
        self.written.extend_from_slice(bytes);
        for &b in bytes {
            self.state ^= u64::from(b);
            self.state = self.state.wrapping_mul(PRIME);
        }
    }

    pub fn write_bool(&mut self, v: bool) {
        self.write_bytes(&[u8::from(v)]);
    }

    pub fn write_u8(&mut self, v: u8) {
        self.write_bytes(&[v]);
    }

    pub fn write_u16(&mut self, v: u16) {
        self.write_bytes(&v.to_le_bytes());
    }

    pub fn write_i32(&mut self, v: i32) {
        self.write_bytes(&v.to_le_bytes());
    }

    pub fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }

    pub fn write_i64(&mut self, v: i64) {
        self.write_bytes(&v.to_le_bytes());
    }

    pub fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }

    pub fn finish(&self) -> u64 {
        self.state
    }
}

/// A value that writes itself into the state hash. An impl for a struct destructures it without
/// `..`, so that a field added without a decision does not compile; a field left out is written
/// `field: _` with the reason (`// not hashed: <why>`).
pub trait StableHash {
    fn stable_hash(&self, h: &mut StateHasher);
}

/// The tag byte of an enum variant, a literal in the impl so that a reorder shows in a diff. The
/// `match` it sits in has no wildcard arm, so a new variant does not compile.
macro_rules! hash_enum {
    ($h:expr, $tag:expr) => {
        $h.write_u8($tag)
    };
}

/// The length of a list, as a `u32`.
fn list_len(len: usize) -> u32 {
    u32::try_from(len).expect("a list is shorter than u32::MAX")
}

impl StableHash for bool {
    fn stable_hash(&self, h: &mut StateHasher) {
        h.write_bool(*self);
    }
}

impl StableHash for u8 {
    fn stable_hash(&self, h: &mut StateHasher) {
        h.write_u8(*self);
    }
}

impl StableHash for u16 {
    fn stable_hash(&self, h: &mut StateHasher) {
        h.write_u16(*self);
    }
}

impl StableHash for u32 {
    fn stable_hash(&self, h: &mut StateHasher) {
        h.write_u32(*self);
    }
}

impl StableHash for i32 {
    fn stable_hash(&self, h: &mut StateHasher) {
        h.write_i32(*self);
    }
}

impl StableHash for u64 {
    fn stable_hash(&self, h: &mut StateHasher) {
        h.write_u64(*self);
    }
}

impl StableHash for i64 {
    fn stable_hash(&self, h: &mut StateHasher) {
        h.write_i64(*self);
    }
}

impl StableHash for crate::fixed::Fx {
    fn stable_hash(&self, h: &mut StateHasher) {
        h.write_i32(self.0);
    }
}

impl StableHash for crate::fixed::Point {
    fn stable_hash(&self, h: &mut StateHasher) {
        let crate::fixed::Point { x, y } = self;
        x.stable_hash(h);
        y.stable_hash(h);
    }
}

impl StableHash for crate::id::EntityId {
    fn stable_hash(&self, h: &mut StateHasher) {
        h.write_u32(self.0);
    }
}

impl<T: StableHash> StableHash for Option<T> {
    fn stable_hash(&self, h: &mut StateHasher) {
        match self {
            None => hash_enum!(h, 0),
            Some(v) => {
                hash_enum!(h, 1);
                v.stable_hash(h);
            }
        }
    }
}

impl<T: StableHash + ?Sized> StableHash for Box<T> {
    fn stable_hash(&self, h: &mut StateHasher) {
        (**self).stable_hash(h);
    }
}

impl<T: StableHash> StableHash for [T] {
    fn stable_hash(&self, h: &mut StateHasher) {
        h.write_u32(list_len(self.len()));
        for item in self {
            item.stable_hash(h);
        }
    }
}

impl<T: StableHash> StableHash for Vec<T> {
    fn stable_hash(&self, h: &mut StateHasher) {
        self.as_slice().stable_hash(h);
    }
}

impl<T: StableHash, const N: usize> StableHash for [T; N] {
    /// The length is part of the type, so only the items are written.
    fn stable_hash(&self, h: &mut StateHasher) {
        for item in self {
            item.stable_hash(h);
        }
    }
}

impl<T: StableHash, const N: usize> StableHash for crate::inline::InlineVec<T, N> {
    fn stable_hash(&self, h: &mut StateHasher) {
        let items: &[T] = self;
        items.stable_hash(h);
    }
}

impl<A: StableHash, B: StableHash> StableHash for (A, B) {
    fn stable_hash(&self, h: &mut StateHasher) {
        self.0.stable_hash(h);
        self.1.stable_hash(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes_of(v: &(impl StableHash + ?Sized)) -> Vec<u8> {
        let mut h = StateHasher::new();
        v.stable_hash(&mut h);
        h.written
    }

    #[test]
    fn a_hasher_that_captures_agrees_with_the_hash() {
        let mut h = StateHasher::new();
        Some(5u32).stable_hash(&mut h);
        assert_eq!(h.finish(), fnv1a64(&h.written));
    }

    /// No sample's bytes are a proper prefix of another's.
    fn assert_prefix_free(samples: &[Vec<u8>]) {
        for (i, a) in samples.iter().enumerate() {
            for (j, b) in samples.iter().enumerate() {
                if i != j && a != b {
                    assert!(
                        !(b.len() > a.len() && b.starts_with(a)),
                        "{a:?} is a prefix of {b:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn no_encoding_of_a_sample_is_a_proper_prefix_of_another() {
        let options: Vec<Option<u8>> = std::iter::once(None).chain((0..4).map(Some)).collect();
        let mut of_options: Vec<Vec<u8>> = vec![bytes_of(&Vec::<Option<u8>>::new())];
        for a in &options {
            of_options.push(bytes_of(&vec![*a]));
            for b in &options {
                of_options.push(bytes_of(&vec![*a, *b]));
            }
        }
        assert_prefix_free(&of_options);

        let lists: Vec<Vec<u8>> = (0..3u8).map(|n| (0..n).collect()).collect();
        let mut of_lists: Vec<Vec<u8>> = vec![bytes_of(&Vec::<Vec<u8>>::new())];
        for a in &lists {
            of_lists.push(bytes_of(&vec![a.clone()]));
            for b in &lists {
                of_lists.push(bytes_of(&vec![a.clone(), b.clone()]));
            }
        }
        assert_prefix_free(&of_lists);
    }
}
