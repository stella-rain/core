//! Minisign trust verification for `packs.json` manifests (SG1a, DS49, D-SIGN).
//!
//! Detached Minisign signature verification using Ed25519 public keys. The manifest is signed
//! offline with Minisign (`minisign -Sm packs.json`) and verified by core before parsing channels.

use std::fmt;

use crate::packs::{Packs, PacksError};

/// Two 32-byte Ed25519 public keys for rotation (DS49, D-SIGN).
///
/// Slot 0 is the active public key (placeholder test key until production keys are populated).
/// Slot 1 is reserved for key rotation.
pub const PUBLIC_KEYS: [[u8; 32]; 2] = [
    [
        23, 56, 28, 152, 98, 105, 33, 97, 209, 110, 106, 198, 68, 166, 87, 110, 159, 114, 144, 255,
        72, 135, 166, 167, 4, 21, 106, 174, 117, 135, 18, 94,
    ],
    [0u8; 32],
];

/// 8-byte Minisign key IDs corresponding to `PUBLIC_KEYS`.
pub const KEY_IDS: [[u8; 8]; 2] = [*b"STELLA01", [0u8; 8]];

/// Why trust verification was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrustError {
    /// The signature string is not a valid minisign signature format.
    Format,
    /// The signature was created by a key not in `PUBLIC_KEYS`.
    UnknownKey,
    /// The signature is cryptographically invalid for the provided bytes.
    Signature,
    /// The signature is authentic, but the manifest bytes failed to parse.
    Pack(PacksError),
}

impl fmt::Display for TrustError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrustError::Format => write!(f, "invalid signature format"),
            TrustError::UnknownKey => write!(f, "signature from an unknown public key"),
            TrustError::Signature => write!(f, "signature verification failed"),
            TrustError::Pack(err) => write!(f, "manifest parse error: {err}"),
        }
    }
}

impl std::error::Error for TrustError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TrustError::Pack(err) => Some(err),
            _ => None,
        }
    }
}

/// Verifies `bytes` against detached Minisign signature `sig` using `PUBLIC_KEYS`.
pub fn verify_packs(bytes: &[u8], sig: &[u8]) -> Result<Packs, TrustError> {
    verify_packs_with_keys(bytes, sig, &KEY_IDS, &PUBLIC_KEYS)
}

/// Verifies `bytes` against detached Minisign signature `sig` using specified key IDs and public keys.
pub fn verify_packs_with_keys(
    _bytes: &[u8],
    _sig: &[u8],
    _key_ids: &[[u8; 8]],
    _public_keys: &[[u8; 32]],
) -> Result<Packs, TrustError> {
    todo!("implement trust verification")
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_MANIFEST: &[u8] = br#"{"schema":1,"seq":1,"channels":{"0.1":{"tag":"v0.1.0","pack_sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","url":"https://content.stella-rain.com/pack.json"}}}"#;

    const VALID_SIG: &[u8] = b"untrusted comment: signature from minisign secret key\n\
RWRTVEVMTEEwMa8N4UsS3HvJAN2N9Zl4kDmI2Qh5RNGkT4DujF+P4m2ftL0XuquAqYOna2yJuTHsIqrz6tDQg9o0EW+ttwPnMQ8=\n\
trusted comment: timestamp:1700000000\tfile:packs.json\n\
VdvQLUtkDQx1XIsVO20qLa/lxv0+Uq4ocstxuLuShOgVMQ1UM8AwDzn9kEjXOkbxHx7NS6W5plxlT6tsLHDrCQ==";

    const UNTRUSTED_KEY_SIG: &[u8] = b"untrusted comment: signature from minisign secret key\n\
RWRVTlRSVVNUMW74St1mTY8ZAJZRm+CvhjWHIqAvnaOXd1GHkwYSZmPWYDvrfRuWQP9Sj8c5EsafPjOTsVKOxTCShofhJnLSJgk=\n\
trusted comment: timestamp:1700000000\tfile:packs.json\n\
g70xZERB5V4bN7Ywk4h2CZm/bL10nMApdfyquxLhGnZ+4RyCHlDc5nUUF8/U6ca8YXYBaQAKptuUe2/9SzqMBA==";

    const TAMPERED_SIG: &[u8] = b"untrusted comment: signature from minisign secret key\n\
RWRTVEVMTEEwMQFNRiwIqQUNQGjVbzOwhz/IXlD3lkYL4Yv9vIN86rYmP+LPfYxCaKsFcjTxmVugdTLIzHDcEfCVI3x0qpUEBwg=\n\
trusted comment: timestamp:1700000000\tfile:packs.json\n\
AA6zysyIBRylA8T+AmJ1Zl+4vGW5QFhh2lR5bQkHOAE19DJL9Jx+sye5S/9CdO87gHfCuWxcph4FAHih1mTHCg==";

    #[test]
    fn test_valid_signature_returns_ok_packs() {
        let packs = verify_packs(VALID_MANIFEST, VALID_SIG).expect("valid signature should verify");
        assert_eq!(packs.seq(), 1);
        let ch = packs.channel("0.1").expect("0.1 channel present");
        assert_eq!(ch.tag(), "v0.1.0");
    }

    #[test]
    fn test_invalid_signature_bytes_returns_signature_error() {
        let err = verify_packs(VALID_MANIFEST, TAMPERED_SIG).unwrap_err();
        assert_eq!(err, TrustError::Signature);
    }

    #[test]
    fn test_untrusted_key_signature_returns_unknown_key() {
        let err = verify_packs(VALID_MANIFEST, UNTRUSTED_KEY_SIG).unwrap_err();
        assert_eq!(err, TrustError::UnknownKey);
    }
}
