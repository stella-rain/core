//! Minisign trust verification for `packs.json` manifests (SG1a, DS49, D-SIGN).
//!
//! Detached Minisign signature verification using Ed25519 public keys. The manifest is signed
//! offline with Minisign (`minisign -Sm packs.json`) and verified by core before parsing channels.

use std::fmt;

use crate::packs::{Packs, PacksError};

/// Two 32-byte Ed25519 public keys for rotation (DS49, D-SIGN).
///
/// Slot 0 is the active pack-signing public key (minisign key id 618d4d6ab87367ec).
/// Slot 1 is reserved for key rotation: all zero, never matched.
pub const PUBLIC_KEYS: [[u8; 32]; 2] = [
    [
        242, 178, 76, 159, 41, 113, 19, 194, 215, 104, 0, 53, 218, 1, 224, 172, 253, 14, 35, 188,
        37, 4, 174, 51, 126, 171, 26, 196, 203, 224, 196, 73,
    ],
    [0u8; 32],
];

/// 8-byte Minisign key IDs corresponding to `PUBLIC_KEYS`.
pub const KEY_IDS: [[u8; 8]; 2] = [[0x61, 0x8d, 0x4d, 0x6a, 0xb8, 0x73, 0x67, 0xec], [0u8; 8]];

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
    bytes: &[u8],
    sig: &[u8],
    key_ids: &[[u8; 8]],
    public_keys: &[[u8; 32]],
) -> Result<Packs, TrustError> {
    let sig_str = std::str::from_utf8(sig).map_err(|_| TrustError::Format)?;
    let signature = minisign_verify::Signature::decode(sig_str).map_err(|_| TrustError::Format)?;

    let mut lines = sig_str.lines();
    let _untrusted = lines.next().ok_or(TrustError::Format)?;
    let line2 = lines.next().ok_or(TrustError::Format)?;
    let bin1 = crate::base64::decode(line2).map_err(|_| TrustError::Format)?;
    if bin1.len() != 74 {
        return Err(TrustError::Format);
    }
    let sig_algo = &bin1[0..2];
    let sig_key_id = &bin1[2..10];

    let matched = key_ids.iter().enumerate().find_map(|(i, k)| {
        if k == sig_key_id && i < public_keys.len() && public_keys[i] != [0u8; 32] {
            Some(i)
        } else {
            None
        }
    });

    let index = match matched {
        Some(i) => i,
        None => return Err(TrustError::UnknownKey),
    };

    let pk_bytes = &public_keys[index];
    let mut pub_bin = [0u8; 42];
    pub_bin[0..2].copy_from_slice(sig_algo);
    pub_bin[2..10].copy_from_slice(sig_key_id);
    pub_bin[10..42].copy_from_slice(pk_bytes);

    let pub_b64 = crate::base64::encode(&pub_bin);
    let public_key =
        minisign_verify::PublicKey::from_base64(&pub_b64).map_err(|_| TrustError::Format)?;

    public_key
        .verify(bytes, &signature, true)
        .map_err(|e| match e {
            minisign_verify::Error::UnexpectedKeyId => TrustError::UnknownKey,
            _ => TrustError::Signature,
        })?;

    let packs = Packs::from_bytes(bytes).map_err(TrustError::Pack)?;
    Ok(packs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The placeholder key the fixtures below were signed with; not core's trusted key.
    const TEST_KEY_ID: [u8; 8] = *b"STELLA01";
    const TEST_PUBLIC_KEY: [u8; 32] = [
        23, 56, 28, 152, 98, 105, 33, 97, 209, 110, 106, 198, 68, 166, 87, 110, 159, 114, 144, 255,
        72, 135, 166, 167, 4, 21, 106, 174, 117, 135, 18, 94,
    ];

    fn verify_with_test_key(bytes: &[u8], sig: &[u8]) -> Result<Packs, TrustError> {
        verify_packs_with_keys(bytes, sig, &[TEST_KEY_ID], &[TEST_PUBLIC_KEY])
    }

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

    const BAD_SCHEMA_MANIFEST: &[u8] = b"{\"schema\":2}";
    const BAD_SCHEMA_SIG: &[u8] = b"untrusted comment: signature from minisign secret key\n\
RWRTVEVMTEEwMayStsgGIkZNHQ5yD1RTvANGoRlxy2Vrbe9PLzUrxZdhjXJ4lW0iQgeUCRTd4rM9pPf8UnShUoDmA7wkLvRoQgg=\n\
trusted comment: timestamp:1700000000\tfile:packs.json\n\
TOkgu21okl/ci5q4+rWnkEXgVRSkNAUBhYZ4ecntIUaZiHpJWZ1GE9HNNPD2HTSjcdEPcLXQr8lQgDFEiD1MAw==";

    #[test]
    fn test_valid_signature_returns_ok_packs() {
        let packs =
            verify_with_test_key(VALID_MANIFEST, VALID_SIG).expect("valid signature should verify");
        assert_eq!(packs.seq(), 1);
        let ch = packs.channel("0.1").expect("0.1 channel present");
        assert_eq!(ch.tag(), "v0.1.0");
    }

    #[test]
    fn test_invalid_signature_bytes_returns_signature_error() {
        let err = verify_with_test_key(VALID_MANIFEST, TAMPERED_SIG).unwrap_err();
        assert_eq!(err, TrustError::Signature);
    }

    #[test]
    fn test_untrusted_key_signature_returns_unknown_key() {
        let err = verify_with_test_key(VALID_MANIFEST, UNTRUSTED_KEY_SIG).unwrap_err();
        assert_eq!(err, TrustError::UnknownKey);
    }

    #[test]
    fn test_invalid_format_returns_format_error() {
        assert_eq!(
            verify_packs(VALID_MANIFEST, b"not a minisig"),
            Err(TrustError::Format)
        );
        assert_eq!(
            verify_packs(VALID_MANIFEST, &[0xff, 0xfe]),
            Err(TrustError::Format)
        );
    }

    #[test]
    fn test_valid_signature_invalid_pack_json_returns_pack_error() {
        let err = verify_with_test_key(BAD_SCHEMA_MANIFEST, BAD_SCHEMA_SIG).unwrap_err();
        assert_eq!(
            err,
            TrustError::Pack(PacksError::UnsupportedSchema { schema: 2 })
        );
    }

    #[test]
    fn slot_zero_is_the_pack_signing_key() {
        const LINE: &str = "RWRhjU1quHNn7PKyTJ8pcRPC12gANdoB4Kz9DiO8JQSuM36rGsTL4MRJ";
        let bytes = crate::base64::decode(LINE).unwrap();
        assert_eq!(bytes.len(), 42);
        assert_eq!(&bytes[0..2], b"Ed");
        assert_eq!(bytes[2..10], KEY_IDS[0]);
        assert_eq!(bytes[10..42], PUBLIC_KEYS[0]);
        assert_eq!(KEY_IDS[0], [0x61, 0x8d, 0x4d, 0x6a, 0xb8, 0x73, 0x67, 0xec]);
        assert_eq!(
            PUBLIC_KEYS[0],
            [
                242, 178, 76, 159, 41, 113, 19, 194, 215, 104, 0, 53, 218, 1, 224, 172, 253, 14,
                35, 188, 37, 4, 174, 51, 126, 171, 26, 196, 203, 224, 196, 73
            ]
        );
    }

    #[test]
    fn slot_one_stays_all_zero() {
        assert_eq!(KEY_IDS[1], [0u8; 8]);
        assert_eq!(PUBLIC_KEYS[1], [0u8; 32]);
    }

    #[test]
    fn a_stella01_signature_is_an_unknown_key() {
        for sig in [VALID_SIG, TAMPERED_SIG] {
            assert_eq!(
                verify_packs(VALID_MANIFEST, sig),
                Err(TrustError::UnknownKey)
            );
        }
        assert_eq!(
            verify_packs(BAD_SCHEMA_MANIFEST, BAD_SCHEMA_SIG),
            Err(TrustError::UnknownKey)
        );
    }
}
