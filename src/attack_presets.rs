//! The attack presets file (CS1b-1, DS20 item 2): the developer's attacks as content data, so
//! that a preset is balanced by a content release and not by a code change (ADR-SIM-4: "Presets
//! are content"). A preset is a registered preset ID (ADR-026) with its arguments and its
//! phases, written in the stage's own attack format (`AttackPhase`, with `{"arg": n}` for an
//! argument). A stage names one by ID and arguments (`AttackRef::Preset`), or holds an inline
//! copy; `presets.rs` takes one apart.
//!
//! The IDs stay in the registry (ADR-CONT-2): the file gives each registered preset its
//! definition and may not add an ID. Letting the pack introduce preset IDs is a later change, as
//! ADR-CONT-7 point 7 says for weapons. `args` names each argument and must be as long as the
//! registry's arity, which the phases must read exactly.
//!
//! The file is untrusted (ADR-020), as every content file is: a size limit before parsing,
//! strict parsing, the current versions only, and every list bounded: at most
//! `MAX_ATTACK_PRESETS` presets, each phase list checked as a stage's inline attack is
//! (`validate::validate_attack`). The SHA-256 of the file's bytes is part of a run's content
//! hash, through the pack or the derived hash, so a run replay verifies only with the presets it
//! was made with. The bundled file, `content/attack_presets.json`, is part of the bundled pack.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::attack::AttackPhase;
use crate::id::ContentId;
use crate::pack::Pack;
use crate::presets;
use crate::registry::{self, Kind};
use crate::validate::{self, Error, Issue};
use crate::version::{self, Versions};

/// The largest attack presets file read (a tuning value; ADR-020 decides that one exists).
pub const MAX_ATTACK_PRESETS_BYTES: usize = 128 * 1024;
/// The most presets the file lists (9 today; a tuning value).
pub const MAX_ATTACK_PRESETS: usize = 64;

/// One argument of a preset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArgSpec {
    /// What the argument is, unique within its preset: lowercase snake_case.
    pub name: ContentId,
}

/// An attack preset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttackPreset {
    /// A registered preset ID, unique within the file.
    pub id: ContentId,
    /// The arguments in order: as many as the registry says the preset takes.
    pub args: Vec<ArgSpec>,
    /// The attack, with its arguments still as `arg` nodes.
    pub phases: Vec<AttackPhase>,
}

/// The attack presets file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttackPresetsFile {
    /// The presets, 1 to `MAX_ATTACK_PRESETS`; every registered preset is among them.
    pub presets: Vec<AttackPreset>,
}

/// Attack presets that passed validation, and the SHA-256 of the bytes they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttackPresets {
    list: Vec<AttackPreset>,
    sha256: [u8; 32],
}

impl AttackPresets {
    /// The bundled attack presets, the default of every run.
    pub fn bundled() -> &'static AttackPresets {
        Pack::bundled().content().attack_presets()
    }

    /// Checks the size and the versions given (the pack's), parses strictly and validates: the
    /// one call for a file's bytes.
    pub fn parse(bytes: &[u8], versions: Versions) -> Result<AttackPresets, Error> {
        let file: AttackPresetsFile =
            version::read_member(bytes, MAX_ATTACK_PRESETS_BYTES, versions)?;
        validate(&file)?;
        Ok(AttackPresets {
            list: file.presets,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The presets, in the file's order.
    pub fn list(&self) -> &[AttackPreset] {
        &self.list
    }

    /// The ID of every preset that has a definition.
    pub fn ids(&self) -> impl Iterator<Item = &ContentId> {
        self.list.iter().map(|p| &p.id)
    }

    /// A preset's definition, with its arguments still as `arg` nodes.
    pub fn definition(&self, id: &str) -> Option<&[AttackPhase]> {
        self.list
            .iter()
            .find(|p| p.id.0 == id)
            .map(|p| p.phases.as_slice())
    }

    /// The SHA-256 of the bytes they were parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &AttackPresetsFile) -> Result<(), Error> {
    let mut issues = Vec::new();
    let mut issue = |path: String, message: String| issues.push(Issue { path, message });
    if file.presets.is_empty() || file.presets.len() > MAX_ATTACK_PRESETS {
        issue(
            "$.presets".into(),
            format!(
                "1 to {MAX_ATTACK_PRESETS} presets (found {})",
                file.presets.len()
            ),
        );
    }
    for (i, p) in file.presets.iter().enumerate().take(MAX_ATTACK_PRESETS) {
        let at = format!("$.presets[{i}]");
        let Some(entry) = registry::find(registry::ENTRIES, Kind::Preset, &p.id.0) else {
            issue(format!("{at}.id"), "not a registered attack preset".into());
            continue;
        };
        if file.presets[..i].iter().any(|x| x.id == p.id) {
            issue(
                format!("{at}.id"),
                "the same ID as an earlier preset".into(),
            );
        }
        let arity = usize::from(entry.args);
        if p.args.len() != arity {
            issue(
                format!("{at}.args"),
                format!(
                    "this preset takes {arity} arguments (found {})",
                    p.args.len()
                ),
            );
        }
        for (k, arg) in p.args.iter().enumerate() {
            let at = format!("{at}.args[{k}]");
            if !registry::is_well_formed(&arg.name.0) {
                issue(
                    format!("{at}.name"),
                    "not a valid name (lowercase snake_case, at most 32 characters)".into(),
                );
            } else if p.args[..k].iter().any(|x| x.name == arg.name) {
                issue(
                    format!("{at}.name"),
                    "the same name as an earlier argument".into(),
                );
            }
        }
        if let Err(found) = validate::validate_attack(&p.phases, entry.args, registry::ENTRIES) {
            for f in found {
                issue(
                    format!("{at}.phases{}", f.path.trim_start_matches('$')),
                    f.message,
                );
            }
        }
        let used = presets::highest_arg(&p.phases).map_or(0, |n| usize::from(n) + 1);
        if used != arity {
            issue(
                format!("{at}.phases"),
                format!("uses {used} arguments; the registry says {arity}"),
            );
        }
    }
    for entry in registry::ENTRIES.iter().filter(|e| e.kind == Kind::Preset) {
        if !file.presets.iter().any(|p| p.id.0 == entry.id) {
            issue(
                "$.presets".into(),
                format!("the registered preset `{}` is missing", entry.id),
            );
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}
