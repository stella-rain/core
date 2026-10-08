//! Share codes (ADR-010, ADR-018): a stage and its replay as text, for players without GitHub.
//! The two files go into one JSON object as strings, so their exact bytes survive (the replay
//! names the SHA-256 of the stage file, and the content hash covers both, ADR-034). The object
//! is compressed with zlib (the `miniz_oxide` crate) and base64-encoded.
//!
//! A code is untrusted (ADR-020). Its length is limited before anything is decoded, the
//! decompressed size is limited while decompressing, so a small code cannot expand into a
//! large file, and the two files inside must each be within their own limits.

use std::fmt;

use miniz_oxide::deflate::compress_to_vec_zlib;
use miniz_oxide::inflate::{TINFLStatus, decompress_to_vec_zlib_with_limit};
use serde::{Deserialize, Serialize};

use crate::base64;
use crate::replay::MAX_REPLAY_BYTES;
use crate::validate::limits::MAX_STAGE_BYTES;

/// A code is at most this many characters.
pub const MAX_SHARE_CODE_CHARS: usize = 512 * 1024;

/// The decompressed container is at most this many bytes: two files at their limits, escaped.
pub const MAX_SHARE_DECOMPRESSED_BYTES: usize = 1024 * 1024;

/// Zlib's best compression. Only decoding has to be stable across versions of the crate.
const LEVEL: u8 = 9;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Container {
    stage: String,
    replay: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareError {
    /// The code (or the text to encode) is longer than the limit.
    TooLong,
    /// Not base64.
    Base64(base64::Error),
    /// Not compressed data, or damaged.
    Corrupt,
    /// It decompresses to more than `MAX_SHARE_DECOMPRESSED_BYTES`.
    TooLarge,
    /// The decompressed text is not the container.
    Parse,
    /// A file is over its own size limit.
    FileTooLarge,
    /// A file is not text (stage and replay files are JSON).
    NotText,
}

impl fmt::Display for ShareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShareError::TooLong => f.write_str("the share code is too long"),
            ShareError::Base64(e) => write!(f, "the share code is not valid: {e}"),
            ShareError::Corrupt => f.write_str("the share code is damaged"),
            ShareError::TooLarge => f.write_str("the share code expands to too much data"),
            ShareError::Parse => f.write_str("the share code does not hold a stage and a replay"),
            ShareError::FileTooLarge => f.write_str("a file in the share code is too large"),
            ShareError::NotText => f.write_str("a file to share is not text"),
        }
    }
}

impl std::error::Error for ShareError {}

/// The share code for a stage file and its replay file.
pub fn encode(stage_bytes: &[u8], replay_bytes: &[u8]) -> Result<String, ShareError> {
    if stage_bytes.len() > MAX_STAGE_BYTES || replay_bytes.len() > MAX_REPLAY_BYTES {
        return Err(ShareError::FileTooLarge);
    }
    let text = |bytes: &[u8]| {
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| ShareError::NotText)
    };
    let container = Container {
        stage: text(stage_bytes)?,
        replay: text(replay_bytes)?,
    };
    let json = serde_json::to_vec(&container).expect("a container serialises");
    let code = base64::encode(&compress_to_vec_zlib(&json, LEVEL));
    if code.len() > MAX_SHARE_CODE_CHARS {
        return Err(ShareError::TooLong);
    }
    Ok(code)
}

/// The stage file and the replay file inside a share code, byte for byte as they were encoded.
/// Neither is validated here: pass them to `replay::verify`.
pub fn decode(code: &str) -> Result<(Vec<u8>, Vec<u8>), ShareError> {
    if code.len() > MAX_SHARE_CODE_CHARS {
        return Err(ShareError::TooLong);
    }
    let compressed = base64::decode(code).map_err(ShareError::Base64)?;
    let json = decompress_to_vec_zlib_with_limit(&compressed, MAX_SHARE_DECOMPRESSED_BYTES)
        .map_err(|e| match e.status {
            TINFLStatus::HasMoreOutput => ShareError::TooLarge,
            _ => ShareError::Corrupt,
        })?;
    let container: Container = serde_json::from_slice(&json).map_err(|_| ShareError::Parse)?;
    if container.stage.len() > MAX_STAGE_BYTES || container.replay.len() > MAX_REPLAY_BYTES {
        return Err(ShareError::FileTooLarge);
    }
    Ok((container.stage.into_bytes(), container.replay.into_bytes()))
}
