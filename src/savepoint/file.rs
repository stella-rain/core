//! The savepoint file (ADR-SAVE-3): a fixed header, then the engine-state payload
//! (`encode_state`). `RunRecorder::save` writes one at a tick; `RunRecorder::resume_from_savepoint`
//! reads it with the run's replay prefix and the pack, and returns a live recorder without
//! re-simulating the prefix. A savepoint is valid only when `SAVEPOINT_FORMAT`, `SIM_VERSION`
//! and the pack's content hash match; the crate version is stored for information and never
//! compared. Every refusal is one `SavepointError`, and the caller falls back to
//! `RunRecorder::resume_with_pack`.
//!
//! Layout, all integers little-endian, the payload after the 160-byte header:
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 4 | magic `SRSV` |
//! | 4 | 4 | `format` |
//! | 8 | 4 | `sim_version` |
//! | 12 | 4 | `schema_version` (information) |
//! | 16 | 32 | `content_sha256` of the pack, for the run's mode |
//! | 48 | 4 | `tick` |
//! | 52 | 8 | `state_hash` |
//! | 60 | 32 | `history_sha256`: SHA-256 of the replay prefix's canonical bytes at save time |
//! | 92 | 4 | `payload_len` |
//! | 96 | 32 | `checksum`: SHA-256 of bytes `0..96`, then the payload |
//! | 128 | 32 | `crate_version`, UTF-8, NUL-padded (information, outside the checksum) |
//!
//! The file is untrusted (ADR-SVC-1): the header is read and checked field by field before the
//! payload is touched, and `payload_len` is compared, never used to allocate.

use std::fmt;

use sha2::{Digest, Sha256};

use crate::content_hash::{sha256, to_hex};
use crate::engine::{Engine, Outcome};
use crate::pack::Pack;
use crate::replay::hash_to_hex;
use crate::run_replay::{
    History, MAX_RUN_REPLAY_BYTES, RunReplay, content_sha256_with, unpack_history,
};
use crate::version::{SCHEMA_VERSION, SIM_VERSION};

use super::{decode_state, encode_state};

/// The version of the savepoint file layout. A change to the encoding bumps it, and the golden
/// test refuses a change that does not.
pub const SAVEPOINT_FORMAT: u32 = 1;
/// The length of the header, in bytes.
pub const HEADER_LEN: usize = 160;
/// The most bytes a savepoint file may have; a larger one is `Damaged` without being read.
pub const MAX_SAVEPOINT_BYTES: usize = 8 * 1024 * 1024;

const MAGIC: [u8; 4] = *b"SRSV";
/// Where the checksummed header part ends.
const CHECKSUM_AT: usize = 96;
const CRATE_VERSION_AT: usize = 128;
const CRATE_VERSION_LEN: usize = HEADER_LEN - CRATE_VERSION_AT;

/// Why a savepoint was refused. One variant per cause; none carries text from the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavepointError {
    /// Not a savepoint of this `SAVEPOINT_FORMAT`.
    Format,
    /// Saved under another `sim_version`.
    SimVersion,
    /// Saved with another content pack.
    Pack,
    /// Truncated, altered or malformed: the length, the checksum or the payload is wrong.
    Damaged,
    /// The restored state does not hash as the header says.
    HashMismatch,
    /// The replay prefix is not the run the savepoint was taken from.
    PrefixMismatch,
    /// The saved run had already ended.
    Ended,
}

impl fmt::Display for SavepointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SavepointError::Format => "not a savepoint of this format",
            SavepointError::SimVersion => "the savepoint is of another simulation version",
            SavepointError::Pack => "the savepoint is of another content pack",
            SavepointError::Damaged => "the savepoint is damaged",
            SavepointError::HashMismatch => "the savepoint's state does not hash as saved",
            SavepointError::PrefixMismatch => "the replay prefix is not this savepoint's run",
            SavepointError::Ended => "the savepoint's run has ended",
        })
    }
}

impl std::error::Error for SavepointError {}

/// The fixed fields of a savepoint file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavepointHeader {
    /// `SAVEPOINT_FORMAT` at save time.
    pub format: u32,
    /// The simulation rules version of the run.
    pub sim_version: u32,
    /// The schema version at save time; stored for information.
    pub schema_version: u32,
    /// The SHA-256 of the content pack, for the run's mode (`content_sha256_with`).
    pub content_sha256: [u8; 32],
    /// The ticks the run had completed.
    pub tick: u32,
    /// The state hash at that tick.
    pub state_hash: u64,
    /// The SHA-256 of the replay prefix's canonical bytes at save time.
    pub history_sha256: [u8; 32],
    /// The crate version that wrote the file; stored for information and never compared.
    pub crate_version: String,
}

/// A savepoint file: its header and the state payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Savepoint {
    header: SavepointHeader,
    payload: Vec<u8>,
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    let mut v = [0; 4];
    v.copy_from_slice(&bytes[at..at + 4]);
    u32::from_le_bytes(v)
}

fn array_at<const N: usize>(bytes: &[u8], at: usize) -> [u8; N] {
    let mut v = [0; N];
    v.copy_from_slice(&bytes[at..at + N]);
    v
}

/// SHA-256 of the header up to the checksum, then the payload.
fn checksum(head: &[u8], payload: &[u8]) -> [u8; 32] {
    let mut sha = Sha256::new();
    sha.update(head);
    sha.update(payload);
    sha.finalize().into()
}

impl Savepoint {
    /// Reads and checks the header and the checksum, nothing that needs a pack or a prefix, so
    /// a caller can show a slot's tick without one. The checks run in the order `Format`,
    /// `Damaged` (under a header's length), `SimVersion`, `Damaged` (size, length, checksum);
    /// nothing past byte 8 is read before the format is right.
    pub fn from_bytes(bytes: &[u8]) -> Result<Savepoint, SavepointError> {
        if bytes.len() < 8 || bytes[..4] != MAGIC || u32_at(bytes, 4) != SAVEPOINT_FORMAT {
            return Err(SavepointError::Format);
        }
        if bytes.len() < HEADER_LEN {
            return Err(SavepointError::Damaged);
        }
        if u32_at(bytes, 8) != SIM_VERSION {
            return Err(SavepointError::SimVersion);
        }
        if bytes.len() > MAX_SAVEPOINT_BYTES {
            return Err(SavepointError::Damaged);
        }
        let payload = &bytes[HEADER_LEN..];
        if usize::try_from(u32_at(bytes, 92)) != Ok(payload.len())
            || array_at::<32>(bytes, CHECKSUM_AT) != checksum(&bytes[..CHECKSUM_AT], payload)
        {
            return Err(SavepointError::Damaged);
        }
        let version = &bytes[CRATE_VERSION_AT..HEADER_LEN];
        let version = version.split(|b| *b == 0).next().unwrap_or_default();
        let state_hash = u64::from_le_bytes(array_at(bytes, 52));
        Ok(Savepoint {
            header: SavepointHeader {
                format: u32_at(bytes, 4),
                sim_version: u32_at(bytes, 8),
                schema_version: u32_at(bytes, 12),
                content_sha256: array_at(bytes, 16),
                tick: u32_at(bytes, 48),
                state_hash,
                history_sha256: array_at(bytes, 60),
                crate_version: String::from_utf8_lossy(version).into_owned(),
            },
            payload: payload.to_vec(),
        })
    }

    /// The header.
    pub fn header(&self) -> &SavepointHeader {
        &self.header
    }

    /// The file's bytes: the header with its checksum, then the payload.
    pub fn to_bytes(&self) -> Vec<u8> {
        let h = &self.header;
        let mut out = Vec::with_capacity(HEADER_LEN + self.payload.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&h.format.to_le_bytes());
        out.extend_from_slice(&h.sim_version.to_le_bytes());
        out.extend_from_slice(&h.schema_version.to_le_bytes());
        out.extend_from_slice(&h.content_sha256);
        out.extend_from_slice(&h.tick.to_le_bytes());
        out.extend_from_slice(&h.state_hash.to_le_bytes());
        out.extend_from_slice(&h.history_sha256);
        let len = u32::try_from(self.payload.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&len.to_le_bytes());
        let sum = checksum(&out, &self.payload);
        out.extend_from_slice(&sum);
        let mut version = [0; CRATE_VERSION_LEN];
        let text = h.crate_version.as_bytes();
        let n = text.len().min(CRATE_VERSION_LEN);
        version[..n].copy_from_slice(&text[..n]);
        out.extend_from_slice(&version);
        out.extend_from_slice(&self.payload);
        out
    }
}

/// The savepoint file of `engine` and `prefix`, the replay of the run up to it.
pub(crate) fn save(engine: &Engine, prefix: &RunReplay) -> Vec<u8> {
    let (_, content, ..) = engine.savepoint_parts();
    let content_sha256 = content_sha256_with(prefix.setup.mode.as_str(), content)
        .expect("a mode that runs has content");
    Savepoint {
        header: SavepointHeader {
            format: SAVEPOINT_FORMAT,
            sim_version: SIM_VERSION,
            schema_version: SCHEMA_VERSION,
            content_sha256,
            tick: engine.tick(),
            state_hash: engine.state_hash(),
            history_sha256: sha256(&prefix.to_bytes()),
            crate_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        payload: encode_state(engine),
    }
    .to_bytes()
}

/// The engine of a savepoint and the history of its prefix, after every check, in this order:
/// the file's (`Savepoint::from_bytes`), the pack, the prefix's setup and size, the payload,
/// the state hash, the prefix against the header, the prefix's own unpacking, and the run
/// still running. Nothing is simulated.
pub(crate) fn restore(
    bytes: &[u8],
    prefix: &RunReplay,
    pack: &Pack,
) -> Result<(Engine, History), SavepointError> {
    let file = Savepoint::from_bytes(bytes)?;
    let head = file.header();
    if content_sha256_with(prefix.setup.mode.as_str(), pack.content()) != Some(head.content_sha256)
    {
        return Err(SavepointError::Pack);
    }
    let prefix_bytes = prefix.to_bytes();
    let fresh = Engine::start_with(&prefix.setup, pack.content());
    let fresh = match fresh {
        Ok(fresh)
            if prefix.content_sha256 == to_hex(&head.content_sha256)
                && prefix_bytes.len() <= MAX_RUN_REPLAY_BYTES =>
        {
            fresh
        }
        _ => return Err(SavepointError::PrefixMismatch),
    };
    let (state, ledger, counted) =
        decode_state(&file.payload, &fresh).map_err(|_| SavepointError::Damaged)?;
    let engine = fresh.with_saved(state, ledger, counted);
    if engine.tick() != head.tick {
        return Err(SavepointError::Damaged);
    }
    if engine.state_hash() != head.state_hash {
        return Err(SavepointError::HashMismatch);
    }
    if prefix.ticks != head.tick
        || prefix.final_hash != hash_to_hex(head.state_hash)
        || sha256(&prefix_bytes) != head.history_sha256
    {
        return Err(SavepointError::PrefixMismatch);
    }
    let history =
        unpack_history(prefix, pack.content()).map_err(|_| SavepointError::PrefixMismatch)?;
    if engine.outcome() != Outcome::Running {
        return Err(SavepointError::Ended);
    }
    Ok((engine, history))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Savepoint {
        Savepoint {
            header: SavepointHeader {
                format: SAVEPOINT_FORMAT,
                sim_version: SIM_VERSION,
                schema_version: SCHEMA_VERSION,
                content_sha256: [0xA1; 32],
                tick: 0x0102_0304,
                state_hash: 0x1112_1314_1516_1718,
                history_sha256: [0xB2; 32],
                crate_version: "0.1.0".to_owned(),
            },
            payload: vec![7, 8, 9],
        }
    }

    #[test]
    fn the_header_is_160_bytes_with_the_documented_offsets() {
        assert_eq!(HEADER_LEN, 160);
        let bytes = sample().to_bytes();
        assert_eq!(bytes.len(), HEADER_LEN + 3);
        assert_eq!(&bytes[0..4], b"SRSV");
        assert_eq!(&bytes[4..8], &1u32.to_le_bytes());
        assert_eq!(&bytes[8..12], &SIM_VERSION.to_le_bytes());
        assert_eq!(&bytes[12..16], &SCHEMA_VERSION.to_le_bytes());
        assert_eq!(&bytes[16..48], &[0xA1; 32]);
        assert_eq!(&bytes[48..52], &[4, 3, 2, 1]);
        assert_eq!(
            &bytes[52..60],
            &[0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11]
        );
        assert_eq!(&bytes[60..92], &[0xB2; 32]);
        assert_eq!(&bytes[92..96], &[3, 0, 0, 0]);
        let sum = checksum(&bytes[..96], &[7, 8, 9]);
        assert_eq!(&bytes[96..128], &sum);
        assert_eq!(&bytes[128..133], b"0.1.0");
        assert!(bytes[133..160].iter().all(|b| *b == 0));
        assert_eq!(&bytes[160..], &[7, 8, 9]);
    }

    #[test]
    fn a_header_reads_back_what_was_written() {
        let sp = sample();
        assert_eq!(Savepoint::from_bytes(&sp.to_bytes()), Ok(sp));
    }

    #[test]
    fn a_format_one_higher_is_format_never_damaged() {
        let mut bytes = sample().to_bytes();
        bytes[4..8].copy_from_slice(&(SAVEPOINT_FORMAT + 1).to_le_bytes());
        assert_eq!(Savepoint::from_bytes(&bytes), Err(SavepointError::Format));
        bytes.truncate(8);
        assert_eq!(Savepoint::from_bytes(&bytes), Err(SavepointError::Format));
    }

    #[test]
    fn a_crate_version_over_32_bytes_is_cut() {
        let mut sp = sample();
        sp.header.crate_version = "1".repeat(40);
        let back = Savepoint::from_bytes(&sp.to_bytes()).unwrap();
        assert_eq!(back.header().crate_version, "1".repeat(32));
    }
}
