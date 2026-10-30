//! JSON Schema generated from the types (ADR-018). The files in `schema/` are this output,
//! checked in; `tests/schema.rs` fails when they are stale and rewrites them when run with
//! `UPDATE_SCHEMA=1`.

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;

use crate::attack_presets::AttackPresetsFile;
use crate::constellations::ConstellationsFile;
use crate::difficulties::DifficultiesFile;
use crate::event::DomainEvent;
use crate::expertise::ExpertiseFile;
use crate::input::Input;
use crate::match_ups::MatchUpsFile;
use crate::pack::PackFile;
use crate::packs::PacksFile;
use crate::perk_sets::PerkSetsFile;
use crate::replay::Replay;
use crate::rewards::RewardsFile;
use crate::rules::RulesFile;
use crate::run_replay::{RunReplay, RunVerified};
use crate::run_setup::RunSetup;
use crate::run_stats::RunStatsFile;
use crate::runes::RunesFile;
use crate::snapshot::Snapshot;
use crate::stage::Stage;
use crate::stage_skill::StageSkillsFile;
use crate::stardust::StardustFile;
use crate::starting_unlocks::StartingUnlocksFile;
use crate::time_limit::TimeLimitFile;
use crate::weapons::WeaponsFile;

/// Each schema's file name in `schema/` and its pretty-printed JSON, ending in a newline.
pub fn all() -> Vec<(&'static str, String)> {
    let mut generator = SchemaSettings::default().into_generator();

    macro_rules! register {
        ($($t:ty),* $(,)?) => {
            $(
                generator.subschema_for::<$t>();
            )*
        };
    }

    register!(
        Stage,
        Input,
        DomainEvent,
        Snapshot,
        Replay,
        RunSetup,
        RunReplay,
        RunVerified,
        RunStatsFile,
        RewardsFile,
        StageSkillsFile,
        ExpertiseFile,
        MatchUpsFile,
        TimeLimitFile,
        PerkSetsFile,
        RulesFile,
        StardustFile,
        StartingUnlocksFile,
        DifficultiesFile,
        AttackPresetsFile,
        WeaponsFile,
        ConstellationsFile,
        RunesFile,
        PackFile,
        PacksFile,
    );

    let mut definitions = generator.take_definitions(true);
    if let Some(attack_ref) = definitions.get_mut("AttackRef") {
        attach_preset_arg_bounds(attack_ref);
    }
    let mut bundle = serde_json::Map::new();
    bundle.insert(
        "$schema".to_string(),
        serde_json::Value::String("https://json-schema.org/draft/2020-12/schema".to_string()),
    );
    bundle.insert(
        "title".to_string(),
        serde_json::Value::String("Content".to_string()),
    );
    bundle.insert("$defs".to_string(), serde_json::Value::Object(definitions));

    let mut bundle_json = serde_json::to_string_pretty(&serde_json::Value::Object(bundle))
        .expect("bundle serialises");
    bundle_json.push('\n');

    fn attach_preset_arg_bounds(attack_ref: &mut serde_json::Value) {
        let preset = &mut attack_ref["oneOf"][0]["properties"]["preset"];
        if let serde_json::Value::Object(preset_map) = preset {
            let all_of: Vec<serde_json::Value> = crate::registry::ENTRIES
                .iter()
                .filter(|e| e.kind == crate::registry::Kind::Preset && !e.arg_bounds.is_empty())
                .map(|e| {
                    let prefix_items: Vec<serde_json::Value> = e
                        .arg_bounds
                        .iter()
                        .map(|b| {
                            serde_json::json!({
                                "type": "integer",
                                "minimum": b.min,
                                "maximum": b.max,
                            })
                        })
                        .collect();
                    let n = e.arg_bounds.len();
                    serde_json::json!({
                        "if": {
                            "properties": {
                                "id": {
                                    "const": e.id
                                }
                            },
                            "required": ["id"]
                        },
                        "then": {
                            "properties": {
                                "args": {
                                    "prefixItems": prefix_items,
                                    "minItems": n,
                                    "maxItems": n
                                }
                            }
                        }
                    })
                })
                .collect();
            preset_map.insert("allOf".to_string(), serde_json::Value::Array(all_of));
        }
    }

    fn stub(type_name: &str) -> String {
        let mut map = serde_json::Map::new();
        map.insert(
            "$schema".to_string(),
            serde_json::Value::String("https://json-schema.org/draft/2020-12/schema".to_string()),
        );
        map.insert(
            "$ref".to_string(),
            serde_json::Value::String(format!("content.schema.json#/$defs/{type_name}")),
        );
        let mut json =
            serde_json::to_string_pretty(&serde_json::Value::Object(map)).expect("stub serialises");
        json.push('\n');
        json
    }

    vec![
        ("content.schema.json", bundle_json),
        ("stage.schema.json", stub(&Stage::schema_name())),
        ("input.schema.json", stub(&Input::schema_name())),
        ("event.schema.json", stub(&DomainEvent::schema_name())),
        ("snapshot.schema.json", stub(&Snapshot::schema_name())),
        ("replay.schema.json", stub(&Replay::schema_name())),
        ("run_setup.schema.json", stub(&RunSetup::schema_name())),
        ("run_replay.schema.json", stub(&RunReplay::schema_name())),
        (
            "run_verified.schema.json",
            stub(&RunVerified::schema_name()),
        ),
        ("run_stats.schema.json", stub(&RunStatsFile::schema_name())),
        ("rewards.schema.json", stub(&RewardsFile::schema_name())),
        (
            "stage_skills.schema.json",
            stub(&StageSkillsFile::schema_name()),
        ),
        ("expertise.schema.json", stub(&ExpertiseFile::schema_name())),
        ("match_ups.schema.json", stub(&MatchUpsFile::schema_name())),
        (
            "time_limit.schema.json",
            stub(&TimeLimitFile::schema_name()),
        ),
        ("perk_sets.schema.json", stub(&PerkSetsFile::schema_name())),
        ("rules.schema.json", stub(&RulesFile::schema_name())),
        ("stardust.schema.json", stub(&StardustFile::schema_name())),
        (
            "starting_unlocks.schema.json",
            stub(&StartingUnlocksFile::schema_name()),
        ),
        (
            "difficulties.schema.json",
            stub(&DifficultiesFile::schema_name()),
        ),
        (
            "attack_presets.schema.json",
            stub(&AttackPresetsFile::schema_name()),
        ),
        ("weapons.schema.json", stub(&WeaponsFile::schema_name())),
        (
            "constellations.schema.json",
            stub(&ConstellationsFile::schema_name()),
        ),
        ("runes.schema.json", stub(&RunesFile::schema_name())),
        ("pack.schema.json", stub(&PackFile::schema_name())),
        ("packs.schema.json", stub(&PacksFile::schema_name())),
    ]
}
