//! JSON Schema generated from the types (ADR-018). The files in `schema/` are this output,
//! checked in; `tests/schema.rs` fails when they are stale and rewrites them when run with
//! `UPDATE_SCHEMA=1`.

use schemars::schema_for;

use crate::event::DomainEvent;
use crate::input::Input;
use crate::replay::Replay;
use crate::snapshot::Snapshot;
use crate::stage::Stage;

/// Each schema's file name in `schema/` and its pretty-printed JSON, ending in a newline.
pub fn all() -> Vec<(&'static str, String)> {
    let pretty = |value: schemars::Schema| {
        let mut json = serde_json::to_string_pretty(&value).expect("a schema serialises");
        json.push('\n');
        json
    };
    vec![
        ("stage.schema.json", pretty(schema_for!(Stage))),
        ("input.schema.json", pretty(schema_for!(Input))),
        ("event.schema.json", pretty(schema_for!(DomainEvent))),
        ("snapshot.schema.json", pretty(schema_for!(Snapshot))),
        ("replay.schema.json", pretty(schema_for!(Replay))),
    ]
}
