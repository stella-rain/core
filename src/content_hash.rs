//! The stage content hash (ADR-034).
//!
//! `SHA-256( SHA-256(stage_bytes) ‖ SHA-256(replay_bytes) )` over the exact bytes of the stage
//! and replay files at the tag. Players' caches, the catalog and the moderation blocklist store
//! the result, so the definition is frozen: a change needs a new ADR and a new field name.

use sha2::{Digest, Sha256};

/// The content hash of a stage file and its replay file, from their exact bytes.
pub fn content_hash(stage_bytes: &[u8], replay_bytes: &[u8]) -> [u8; 32] {
    let stage: [u8; 32] = Sha256::digest(stage_bytes).into();
    let replay: [u8; 32] = Sha256::digest(replay_bytes).into();
    let mut both = Sha256::new();
    both.update(stage);
    both.update(replay);
    both.finalize().into()
}

/// Lowercase hexadecimal, two characters per byte: the form stored in `catalog.json` and
/// `blocklist.json`.
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    out
}
