//! FNV-1a 64 and the explicit byte encoding behind the state hash (ADR-019).

use stella_rain_core::fixed::{Fx, Point};
use stella_rain_core::hash::{StableHash, StateHasher, fnv1a64};
use stella_rain_core::id::EntityId;

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

fn hash_of(v: &(impl StableHash + ?Sized)) -> u64 {
    let mut h = StateHasher::new();
    v.stable_hash(&mut h);
    h.finish()
}

#[test]
fn an_option_is_a_tag_byte_and_then_its_value() {
    assert_eq!(hash_of(&None::<u32>), fnv1a64(&[0]));
    assert_eq!(hash_of(&Some(5u32)), fnv1a64(&[1, 5, 0, 0, 0]));
}

#[test]
fn a_list_is_its_length_and_then_its_items() {
    assert_eq!(hash_of(&[1u8, 2][..]), fnv1a64(&[2, 0, 0, 0, 1, 2]));
    assert_eq!(hash_of(&vec![1u8, 2]), fnv1a64(&[2, 0, 0, 0, 1, 2]));
    // A fixed array writes only its items; a pair writes its two.
    assert_eq!(hash_of(&[1u8, 2]), fnv1a64(&[1, 2]));
    assert_eq!(hash_of(&(1u8, 2u16)), fnv1a64(&[1, 2, 0]));
}

#[test]
fn small_and_signed_integers_have_their_declared_width() {
    assert_eq!(hash_of(&7u16), fnv1a64(&[7, 0]));
    assert_eq!(hash_of(&-1i64), fnv1a64(&[0xff; 8]));
    assert_eq!(hash_of(&Fx(-2)), fnv1a64(&[0xfe, 0xff, 0xff, 0xff]));
    assert_eq!(hash_of(&EntityId(3)), fnv1a64(&[3, 0, 0, 0]));
    assert_eq!(
        hash_of(&Point { x: Fx(1), y: Fx(2) }),
        fnv1a64(&[1, 0, 0, 0, 2, 0, 0, 0])
    );
}

#[test]
fn moving_an_item_across_a_list_boundary_changes_the_hash() {
    assert_ne!(
        hash_of(&vec![vec![1u8], vec![2, 3]]),
        hash_of(&vec![vec![1u8, 2], vec![3]])
    );
    assert_ne!(hash_of(&[None, Some(0u32)]), hash_of(&[Some(0u32), None]));
}
