//! File and rule versions (ADR-017). Both stay 0 until the freeze at P3 (ADR-035): until then
//! types and rules change in place, and the replay corpus is regenerated on purpose.

/// Structure of stage, event and replay files.
pub const SCHEMA_VERSION: u32 = 0;

/// Simulation rules.
pub const SIM_VERSION: u32 = 0;

/// Why a versioned file was not read. Paths and numbers only, never text from the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReadError {
    /// Over the file's size limit; it is not parsed.
    TooLarge { size: usize, limit: usize },
    /// Not JSON, or not the file's format (unknown or missing fields, wrong types).
    Parse { line: usize, column: usize },
    /// Written for another game.
    UnsupportedVersion {
        schema_version: u32,
        sim_version: u32,
    },
}

/// Checks both versions against this game's.
pub(crate) fn check(schema_version: u32, sim_version: u32) -> Result<(), ReadError> {
    if schema_version == SCHEMA_VERSION && sim_version == SIM_VERSION {
        Ok(())
    } else {
        Err(ReadError::UnsupportedVersion {
            schema_version,
            sim_version,
        })
    }
}

/// The versions a file is read at: a standalone file's own (a stage, a run setup, a pack), or,
/// for a content file inside a pack, the pack's (CS1a-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct Versions {
    pub schema_version: u32,
    pub sim_version: u32,
}

impl Versions {
    /// This game's.
    pub const CURRENT: Versions = Versions {
        schema_version: SCHEMA_VERSION,
        sim_version: SIM_VERSION,
    };
}

fn parse_error(e: serde_json::Error) -> ReadError {
    ReadError::Parse {
        line: e.line(),
        column: e.column(),
    }
}

/// Reads a versioned file (a pack, a stage or a run setup): the size limit first, then the
/// versions alone, then the strict parse (ADR-017, ADR-SVC-1). The versions are read before the
/// rest because a file for a newer game may have fields this one rejects, and that file is
/// "needs a newer version", not a parse error.
pub(crate) fn read_current<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
    limit: usize,
) -> Result<T, ReadError> {
    if bytes.len() > limit {
        return Err(ReadError::TooLarge {
            size: bytes.len(),
            limit,
        });
    }
    // Not `deny_unknown_fields`: only the two versions are read here.
    let versions: Versions = serde_json::from_slice(bytes).map_err(parse_error)?;
    check(versions.schema_version, versions.sim_version)?;
    serde_json::from_slice(bytes).map_err(parse_error)
}

/// Reads a content file: it has no versions of its own and is read at the versions of the pack
/// that holds it (ADR-CONT-3), checked here so that a future migration has them (ADR-REPL-1
/// point 4). The size limit first, then the versions, then the strict parse.
pub(crate) fn read_member<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
    limit: usize,
    versions: Versions,
) -> Result<T, ReadError> {
    if bytes.len() > limit {
        return Err(ReadError::TooLarge {
            size: bytes.len(),
            limit,
        });
    }
    check(versions.schema_version, versions.sim_version)?;
    serde_json::from_slice(bytes).map_err(parse_error)
}
