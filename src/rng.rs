//! The simulation PRNG: SplitMix64 (ADR-033). The algorithm, `below(n)` and the order of draws
//! are frozen rules (ADR-017); `tests/rng.rs` pins the outputs.

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
        (self.next_u64() % u64::from(n)) as u32
    }

    pub fn state(&self) -> u64 {
        self.state
    }
}
