//! UI text table for titles and descriptions outside the hashed content pack (TX1).
//!
//! Untrusted: size limit, strict JSON parsing, no control characters, bounded character lengths.

use std::collections::BTreeMap;
use std::fmt;

use crate::pack::Pack;

/// The largest text table read (64 KiB).
pub const MAX_TEXT_BYTES: usize = 64 * 1024;

/// One text entry holding a title and an optional description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEntry {
    pub title: String,
    pub description: String,
}

/// Why a text table was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextError {
    /// Over `MAX_TEXT_BYTES`.
    TooLarge { size: usize, limit: usize },
    /// Not valid JSON, duplicate keys, or unknown fields.
    Parse { line: usize, column: usize },
    /// An entry failed text validation (length or control characters).
    Invalid { key: String, issue: String },
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { size, limit } => {
                write!(f, "text table is {size} bytes, over the limit of {limit}")
            }
            Self::Parse { line, column } => {
                write!(f, "text table is not valid at line {line}, column {column}")
            }
            Self::Invalid { key, issue } => write!(f, "entry `{key}` is invalid: {issue}"),
        }
    }
}

impl std::error::Error for TextError {}

/// A table mapping `"<file>/<id>"` to `TextEntry`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextTable {
    entries: BTreeMap<String, TextEntry>,
}

impl TextTable {
    pub const MAX_BYTES: usize = MAX_TEXT_BYTES;
    pub const BUNDLED: &'static [u8] = b"{}";

    /// Parse a text table from untrusted UTF-8 JSON bytes.
    pub fn from_bytes(_bytes: &[u8]) -> Result<Self, TextError> {
        // Stub for red test
        Err(TextError::Parse { line: 1, column: 1 })
    }

    /// Look up an entry by full key `"<file>/<id>"`.
    pub fn get(&self, key: &str) -> Option<&TextEntry> {
        self.entries.get(key)
    }

    /// Check that every shown ID in `pack` has a corresponding entry in this table.
    pub fn check_pack(&self, _pack: &Pack) -> Result<(), Vec<String>> {
        Err(vec!["not implemented".into()])
    }
}
