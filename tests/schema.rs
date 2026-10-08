//! The checked-in JSON Schema matches the types (ADR-018). After changing a type, regenerate:
//! `UPDATE_SCHEMA=1 cargo test --test schema`, and commit the diff with the change.

use std::path::Path;

#[test]
fn checked_in_schema_is_current() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
    let update = std::env::var_os("UPDATE_SCHEMA").is_some();
    let mut stale = Vec::new();
    for (name, json) in stella_rain_core::schema::all() {
        let path = dir.join(name);
        if update {
            std::fs::write(&path, &json).unwrap();
        } else if std::fs::read_to_string(&path).ok().as_deref() != Some(json.as_str()) {
            stale.push(name);
        }
    }
    assert!(
        stale.is_empty(),
        "stale schema files {stale:?}: run `UPDATE_SCHEMA=1 cargo test --test schema`"
    );
}
