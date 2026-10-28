//! The simulation PRNG: SplitMix64 (ADR-033). The algorithm, `below(n)` and the order of draws
//! are frozen rules (ADR-017); `tests/rng.rs` pins the outputs.

/// The stream tags: each name padded with zeros to 8 bytes, read as a little-endian `u64`.
pub const TAG_SPAWN: u64 = u64::from_le_bytes(*b"spawn\0\0\0");
pub const TAG_ATTACK: u64 = u64::from_le_bytes(*b"attack\0\0");
pub const TAG_LOOT: u64 = u64::from_le_bytes(*b"loot\0\0\0\0");
pub const TAG_OFFER: u64 = u64::from_le_bytes(*b"offer\0\0\0");
pub const TAG_HORDE: u64 = u64::from_le_bytes(*b"horde\0\0\0");

/// The stream of one purpose: one SplitMix64 step of `seed ^ tag` seeds it (ADR-SIM-2), so a
/// draw of one purpose never moves another's.
pub fn derive(seed: u64, tag: u64) -> SplitMix64 {
    SplitMix64::new(SplitMix64::new(seed ^ tag).next_u64())
}

#[derive(Clone, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A value in `0..n` (n > 0). The tiny modulo bias is deterministic, so it is harmless here.
    pub fn below(&mut self, n: u32) -> u32 {
        assert!(n > 0, "below(0)");
        u32::try_from(self.next_u64() % u64::from(n)).expect("below n, a u32")
    }

    pub fn state(&self) -> u64 {
        self.state
    }
}
