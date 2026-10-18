//! JSON Schema generated from the types (ADR-018). The files in `schema/` are this output,
//! checked in; `tests/schema.rs` fails when they are stale and rewrites them when run with
//! `UPDATE_SCHEMA=1`.

use schemars::schema_for;

use crate::event::DomainEvent;
use crate::input::Input;
use crate::replay::Replay;
use crate::rewards::RewardsFile;
use crate::run_replay::RunReplay;
use crate::run_setup::RunSetup;
use crate::run_stats::RunStatsFile;
use crate::snapshot::Snapshot;
use crate::stage::Stage;
use crate::stage_skill::StageSkillsFile;

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
        ("run_setup.schema.json", pretty(schema_for!(RunSetup))),
        ("run_replay.schema.json", pretty(schema_for!(RunReplay))),
        ("run_stats.schema.json", pretty(schema_for!(RunStatsFile))),
        ("rewards.schema.json", pretty(schema_for!(RewardsFile))),
        (
            "stage_skills.schema.json",
            pretty(schema_for!(StageSkillsFile)),
        ),
    ]
}
