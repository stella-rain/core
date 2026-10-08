//! FNV-1a 64 and the explicit byte encoding behind the state hash (ADR-019).

use stella_rain_core::hash::{StateHasher, fnv1a64};

#[test]
fn fnv1a64_matches_the_reference_vectors() {
    assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
}

#[test]
fn integers_are_encoded_little_endian_at_their_declared_width() {
    let mut h = StateHasher::new();
    h.write_i32(-2);
    h.write_u32(0x0102_0304);
    h.write_u64(1);
    h.write_bool(true);
    let bytes = [
        0xfe, 0xff, 0xff, 0xff, // -2i32
        0x04, 0x03, 0x02, 0x01, // 0x01020304u32
        1, 0, 0, 0, 0, 0, 0, 0, // 1u64
        1, // true
    ];
    assert_eq!(h.finish(), fnv1a64(&bytes));
}

#[test]
fn a_new_hasher_is_the_offset_basis() {
    assert_eq!(StateHasher::new().finish(), fnv1a64(b""));
}
