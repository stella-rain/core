//! The stage content hash (ADR-034): SHA-256 of the SHA-256 of the stage bytes followed by the
//! SHA-256 of the replay bytes. Players' caches and the moderation blocklist store it, so the
//! vector below is frozen.

use stella_rain_core::content_hash::{content_hash, to_hex};

const STAGE: &[u8] = b"{\"stage\":\"a\"}\n";
const REPLAY: &[u8] = b"{\"replay\":\"b\"}\n";

#[test]
fn matches_the_adr_034_vector() {
    assert_eq!(STAGE.len(), 14);
    assert_eq!(REPLAY.len(), 15);
    assert_eq!(
        to_hex(&content_hash(STAGE, REPLAY)),
        "f2f2dbb0338839e28e2a394831c76abd0c5fee77e2a2ca73f7315e53ba40bf4b"
    );
}

#[test]
fn the_boundary_between_the_files_is_not_ambiguous() {
    // The same concatenated bytes split at a different place must give a different hash.
    let a = content_hash(b"ab", b"c");
    let b = content_hash(b"a", b"bc");
    assert_ne!(a, b);
}

#[test]
fn one_changed_byte_in_either_file_changes_the_hash() {
    let base = content_hash(STAGE, REPLAY);
    assert_ne!(base, content_hash(b"{\"stage\":\"b\"}\n", REPLAY));
    assert_ne!(base, content_hash(STAGE, b"{\"replay\":\"c\"}\n"));
}

#[test]
fn hex_is_64_lowercase_characters() {
    let hex = to_hex(&content_hash(b"", b""));
    assert_eq!(hex.len(), 64);
    assert!(
        hex.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    );
}
