//! `Issue` and `Error`: the problems the validator reports and the error of a stage file.

use std::fmt;

use crate::version::ReadError;

/// One problem and where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// For example `$.boss.phases[1].timeline[3]`; `$` is the file.
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Over `MAX_STAGE_BYTES`; the file is not parsed.
    TooLarge {
        size: usize,
        limit: usize,
    },
    /// Not JSON, or not the stage format (unknown or missing fields, wrong types).
    Parse {
        line: usize,
        column: usize,
    },
    /// Written by a newer game: listed greyed out as "update the app" (ADR-REPL-1).
    UnsupportedVersion {
        schema_version: u32,
        sim_version: u32,
    },
    Invalid(Vec<Issue>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::TooLarge { size, limit } => {
                write!(f, "the stage file is {size} bytes; the limit is {limit}")
            }
            Error::Parse { line, column } => {
                write!(f, "not a valid stage file (line {line}, column {column})")
            }
            Error::UnsupportedVersion {
                schema_version,
                sim_version,
            } => write!(
                f,
                "needs a newer version of the game (schema_version {schema_version}, \
                 sim_version {sim_version})"
            ),
            Error::Invalid(issues) => {
                for (i, issue) in issues.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{}: {}", issue.path, issue.message)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<ReadError> for Error {
    fn from(e: ReadError) -> Error {
        match e {
            ReadError::TooLarge { size, limit } => Error::TooLarge { size, limit },
            ReadError::Parse { line, column } => Error::Parse { line, column },
            ReadError::UnsupportedVersion {
                schema_version,
                sim_version,
            } => Error::UnsupportedVersion {
                schema_version,
                sim_version,
            },
        }
    }
}
