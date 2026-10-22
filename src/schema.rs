//! JSON Schema generated from the types (ADR-018). The files in `schema/` are this output,
//! checked in; `tests/schema.rs` fails when they are stale and rewrites them when run with
//! `UPDATE_SCHEMA=1`.

use schemars::schema_for;

use crate::constellations::ConstellationsFile;
use crate::difficulties::DifficultiesFile;
use crate::event::DomainEvent;
use crate::expertise::ExpertiseFile;
use crate::input::Input;
use crate::match_ups::MatchUpsFile;
use crate::pack::PackFile;
use crate::perk_sets::PerkSetsFile;
use crate::replay::Replay;
use crate::rewards::RewardsFile;
use crate::run_replay::RunReplay;
use crate::run_setup::RunSetup;
use crate::run_stats::RunStatsFile;
use crate::runes::RunesFile;
use crate::snapshot::Snapshot;
use crate::stage::Stage;
use crate::stage_skill::StageSkillsFile;
use crate::starting_unlocks::StartingUnlocksFile;
use crate::time_limit::TimeLimitFile;

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
        ("expertise.schema.json", pretty(schema_for!(ExpertiseFile))),
        ("match_ups.schema.json", pretty(schema_for!(MatchUpsFile))),
        ("time_limit.schema.json", pretty(schema_for!(TimeLimitFile))),
        ("perk_sets.schema.json", pretty(schema_for!(PerkSetsFile))),
        (
            "starting_unlocks.schema.json",
            pretty(schema_for!(StartingUnlocksFile)),
        ),
        (
            "difficulties.schema.json",
            pretty(schema_for!(DifficultiesFile)),
        ),
        (
            "constellations.schema.json",
            pretty(schema_for!(ConstellationsFile)),
        ),
        ("runes.schema.json", pretty(schema_for!(RunesFile))),
        ("pack.schema.json", pretty(schema_for!(PackFile))),
    ]
}
