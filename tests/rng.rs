//! SplitMix64 is the simulation PRNG (ADR-033). Its outputs are frozen rules: these vectors
//! come from the published algorithm and must never change.

use stella_rain_core::rng::SplitMix64;

#[test]
fn seed_zero_gives_the_published_sequence() {
    let mut rng = SplitMix64::new(0);
    assert_eq!(rng.next_u64(), 0xe220_a839_7b1d_cdaf);
    assert_eq!(rng.next_u64(), 0x6e78_9e6a_a1b9_65f4);
    assert_eq!(rng.next_u64(), 0x06c4_5d18_8009_454f);
}

#[test]
fn the_spike_seed_gives_the_pinned_sequence() {
    let mut rng = SplitMix64::new(0x5354_454c_4c41_5241);
    assert_eq!(rng.next_u64(), 0x424b_882e_8174_a4df);
    assert_eq!(rng.next_u64(), 0x5dc2_2b34_0f92_a76d);
}

#[test]
fn below_is_next_u64_modulo_n() {
    let mut a = SplitMix64::new(7);
    let mut b = SplitMix64::new(7);
    for n in [1_u32, 2, 3, 100, 4_000, u32::MAX] {
        let expected = u32::try_from(b.next_u64() % u64::from(n)).unwrap();
        assert_eq!(a.below(n), expected);
    }
}

#[test]
#[should_panic(expected = "below(0)")]
fn below_zero_panics() {
    SplitMix64::new(1).below(0);
}
