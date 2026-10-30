//! The savepoint file (ADR-SAVE-3): a fixed header, then the engine-state payload.

use std::fmt;

use crate::pack::Pack;
use crate::run_replay::{RunRecorder, RunReplay};

/// The version of the savepoint file layout. A change to the encoding bumps it.
pub const SAVEPOINT_FORMAT: u32 = 1;
/// The length of the header, in bytes.
pub const HEADER_LEN: usize = 160;
/// The most bytes a savepoint file may have.
pub const MAX_SAVEPOINT_BYTES: usize = 8 * 1024 * 1024;

/// Why a savepoint was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavepointError {
    /// Not a savepoint of this format.
    Format,
    /// Saved under another `sim_version`.
    SimVersion,
    /// Saved with another content pack.
    Pack,
    /// Truncated, altered or malformed.
    Damaged,
    /// The restored state does not hash as saved.
    HashMismatch,
    /// The replay prefix is not the run the savepoint was taken from.
    PrefixMismatch,
    /// The saved run had already ended.
    Ended,
}

impl fmt::Display for SavepointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("savepoint refused")
    }
}

impl std::error::Error for SavepointError {}

/// The fixed fields of a savepoint file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavepointHeader {
    pub format: u32,
    pub sim_version: u32,
    pub schema_version: u32,
    pub content_sha256: [u8; 32],
    pub tick: u32,
    pub state_hash: u64,
    pub history_sha256: [u8; 32],
    pub crate_version: String,
}

/// A savepoint file: its header and the state payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Savepoint {
    header: SavepointHeader,
    payload: Vec<u8>,
}

impl Savepoint {
    /// Reads the header checks only.
    pub fn from_bytes(_bytes: &[u8]) -> Result<Savepoint, SavepointError> {
        unimplemented!("SNAP-FILE step 4")
    }

    /// The header.
    pub fn header(&self) -> &SavepointHeader {
        &self.header
    }

    /// The file's bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let _ = &self.payload;
        unimplemented!("SNAP-FILE step 4")
    }
}

impl RunRecorder {
    /// The savepoint file of the run as it stands.
    pub fn save(&self) -> Vec<u8> {
        unimplemented!("SNAP-FILE step 5")
    }

    /// Restores a run from savepoint `bytes`, `prefix` and `pack`.
    pub fn resume_from_savepoint(
        _bytes: &[u8],
        _prefix: &RunReplay,
        _pack: &Pack,
    ) -> Result<RunRecorder, SavepointError> {
        unimplemented!("SNAP-FILE step 5")
    }
}
