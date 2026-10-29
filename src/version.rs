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
/// that holds it (ADR-CONT-3), checked here so that a future migration has them (ADR-REPL-5
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

/// One step from an older version pair to the next: `apply` turns the file's JSON at `from`
/// into the JSON of the version after it (ADR-REPL-5 point 4). The production table is empty
/// until the freeze at P3.
pub(crate) struct Migration {
    pub from: Versions,
    pub apply: fn(serde_json::Value) -> serde_json::Value,
}

/// Why a replay file was not loaded. Numbers and positions only, never text from the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReplayLoadError {
    /// Over the file's size limit; it is not parsed.
    TooLarge { size: usize, limit: usize },
    /// Recorded by an older version that has no migration.
    Older(Versions),
    /// Either version is above this game's.
    Newer(Versions),
    /// Not JSON, no versions, or the full parse failed at a known version.
    Damaged { line: usize, column: usize },
}

fn damaged(e: serde_json::Error) -> ReplayLoadError {
    ReplayLoadError::Damaged {
        line: e.line(),
        column: e.column(),
    }
}

/// Reads a replay file: the size limit first, then the versions alone (`header`), then the
/// strict parse (ADR-017, ADR-SVC-1). Either version above `current` is `Newer`, whatever else
/// the file holds; both equal is parsed as it is; below, a migration for exactly that pair is
/// applied to the JSON first (positions of a `Damaged` after a migration are in the migrated
/// bytes), and without one the file is `Older`.
pub(crate) fn read_replay<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
    limit: usize,
    current: Versions,
    migrations: &[Migration],
    header: fn(&[u8]) -> Result<Versions, serde_json::Error>,
) -> Result<T, ReplayLoadError> {
    if bytes.len() > limit {
        return Err(ReplayLoadError::TooLarge {
            size: bytes.len(),
            limit,
        });
    }
    let found = header(bytes).map_err(damaged)?;
    if found.schema_version > current.schema_version || found.sim_version > current.sim_version {
        return Err(ReplayLoadError::Newer(found));
    }
    if found == current {
        return serde_json::from_slice(bytes).map_err(damaged);
    }
    let Some(migration) = migrations.iter().find(|m| m.from == found) else {
        return Err(ReplayLoadError::Older(found));
    };
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(damaged)?;
    let migrated = serde_json::to_vec(&(migration.apply)(value)).map_err(damaged)?;
    serde_json::from_slice(&migrated).map_err(damaged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    const CURRENT: Versions = Versions {
        schema_version: 2,
        sim_version: 2,
    };

    #[derive(serde::Deserialize, PartialEq, Debug)]
    #[serde(deny_unknown_fields)]
    struct Doc {
        schema_version: u32,
        sim_version: u32,
        value: u32,
    }

    fn header(bytes: &[u8]) -> Result<Versions, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    fn v(schema_version: u32, sim_version: u32) -> Versions {
        Versions {
            schema_version,
            sim_version,
        }
    }

    fn from_one_one(mut doc: Value) -> Value {
        let map = doc.as_object_mut().unwrap();
        let old = map.remove("old_value").unwrap();
        map.insert("value".into(), old);
        map.insert("schema_version".into(), json!(2));
        map.insert("sim_version".into(), json!(2));
        doc
    }

    const TABLE: &[Migration] = &[Migration {
        from: Versions {
            schema_version: 1,
            sim_version: 1,
        },
        apply: from_one_one,
    }];

    fn read(body: &Value, table: &[Migration]) -> Result<Doc, ReplayLoadError> {
        read_raw(body.to_string().as_bytes(), table)
    }

    fn read_raw(bytes: &[u8], table: &[Migration]) -> Result<Doc, ReplayLoadError> {
        read_replay(bytes, 1024, CURRENT, table, header)
    }

    fn doc(schema_version: u32, sim_version: u32) -> Value {
        json!({ "schema_version": schema_version, "sim_version": sim_version, "value": 7 })
    }

    fn is_damaged<T: std::fmt::Debug>(r: Result<T, ReplayLoadError>) -> bool {
        matches!(r, Err(ReplayLoadError::Damaged { .. }))
    }

    #[test]
    fn a_newer_file_is_newer_even_with_fields_this_version_rejects() {
        assert_eq!(read(&doc(3, 2), &[]), Err(ReplayLoadError::Newer(v(3, 2))));
        let mut future = doc(2, 3);
        future["future"] = json!(1);
        assert_eq!(read(&future, &[]), Err(ReplayLoadError::Newer(v(2, 3))));
        // A mixed pair: the newer side wins.
        assert_eq!(
            read(&doc(3, 1), TABLE),
            Err(ReplayLoadError::Newer(v(3, 1)))
        );
    }

    #[test]
    fn an_older_file_with_a_migration_loads_migrated() {
        let old = json!({ "schema_version": 1, "sim_version": 1, "old_value": 7 });
        assert_eq!(
            read(&old, TABLE),
            Ok(Doc {
                schema_version: 2,
                sim_version: 2,
                value: 7
            })
        );
    }

    #[test]
    fn an_older_file_without_a_migration_is_older() {
        let old = json!({ "schema_version": 1, "sim_version": 1, "old_value": 7 });
        assert_eq!(read(&old, &[]), Err(ReplayLoadError::Older(v(1, 1))));
        assert_eq!(
            read(&doc(0, 0), TABLE),
            Err(ReplayLoadError::Older(v(0, 0)))
        );
    }

    #[test]
    fn a_damaged_file_is_never_older() {
        let mut bad = doc(2, 2);
        bad["value"] = json!("x");
        assert!(is_damaged(read(&bad, TABLE)));
        let migrated_bad = json!({ "schema_version": 1, "sim_version": 1, "old_value": "x" });
        assert!(is_damaged(read(&migrated_bad, TABLE)));
        assert!(is_damaged(read_raw(
            b"{\"schema_version\": 2, \"sim",
            TABLE
        )));
        assert!(is_damaged(read_raw(b"", TABLE)));
        assert!(is_damaged(read_raw(b"{}", TABLE)));
        assert!(is_damaged(read(
            &json!({ "sim_version": 2, "value": 7 }),
            TABLE
        )));
    }

    #[test]
    fn the_header_ignores_unknown_fields() {
        let mut old = json!({ "schema_version": 1, "sim_version": 1, "old_value": 7 });
        old["extra"] = json!(1);
        assert_eq!(read(&old, &[]), Err(ReplayLoadError::Older(v(1, 1))));
    }

    #[test]
    fn the_size_limit_comes_before_any_parse() {
        let bytes = vec![b'x'; 1025];
        assert_eq!(
            read_raw(&bytes, TABLE),
            Err(ReplayLoadError::TooLarge {
                size: 1025,
                limit: 1024
            })
        );
    }
}
