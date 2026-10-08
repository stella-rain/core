//! The state hash: FNV-1a 64 over an explicit byte encoding that this crate writes itself
//! (ADR-019). Never `DefaultHasher` or a derived `Hash`: neither promises the same output
//! across Rust releases or platforms. The algorithm and encoding are part of a
//! `sim_version`'s frozen rules.

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
        }
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state ^= u64::from(b);
            self.state = self.state.wrapping_mul(PRIME);
        }
    }

    pub fn write_bool(&mut self, v: bool) {
        self.write_bytes(&[u8::from(v)]);
    }

    pub fn write_i32(&mut self, v: i32) {
        self.write_bytes(&v.to_le_bytes());
    }

    pub fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }

    pub fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }

    pub fn finish(&self) -> u64 {
        self.state
    }
}
