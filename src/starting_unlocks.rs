//! The starting unlocks (DS5 Q10, ADR-080, ADR-099 point 4a, ADR-124): the weapons, perk sets,
//! stage skills and runes a new save begins with. The pack lists them; the app seeds a new save
//! from the pack, so changing them is a content release, and a save that exists keeps what it
//! has. The simulation never reads this file: no outcome depends on it.
//!
//! The file is untrusted (ADR-020), as every content file is: a size limit before parsing,
//! strict parsing, the current versions only, and every list bounded: at most
//! `MAX_STARTING_UNLOCKS` IDs, none twice. Each ID must be one the game knows: a weapon preset,
//! a perk set, a stage skill or a rune of the pack's own files, so a pack never starts a save
//! with something it does not hold. `Pack` holds the file as an optional field: a pack released
//! before it has none, and then reports the starting unlocks as unknown (`None`), never a
//! guessed list. The bundled file, `content/starting_unlocks.json`, is part of the bundled pack.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::id::ContentId;
use crate::perk_sets::PerkSets;
use crate::registry;
use crate::rune::Rune;
use crate::runes::Runes;
use crate::stage_skill::StageSkills;
use crate::validate::{Error, Issue};
use crate::version::{SCHEMA_VERSION, SIM_VERSION};
use crate::weapons::Weapons;

/// The largest starting unlocks file read (a tuning value; ADR-020 decides that one exists).
pub const MAX_STARTING_UNLOCKS_BYTES: usize = 4 * 1024;
/// The most IDs one list holds (6 across the lists today; a tuning value).
pub const MAX_STARTING_UNLOCKS: usize = 32;

/// The starting unlocks file: the IDs a new save begins with, by kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StartingUnlocksFile {
    pub schema_version: u32,
    pub sim_version: u32,
    /// Weapon presets, at most `MAX_STARTING_UNLOCKS`.
    pub weapons: Vec<ContentId>,
    /// Perk sets of the pack's perk sets file.
    pub perk_sets: Vec<ContentId>,
    /// Stage skills of the pack's stage skills file.
    pub stage_skills: Vec<ContentId>,
    /// Runes of the pack's runes file.
    pub runes: Vec<ContentId>,
}

/// Starting unlocks that passed validation, and the SHA-256 of the bytes they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartingUnlocks {
    file: StartingUnlocksFile,
    sha256: [u8; 32],
}

impl StartingUnlocks {
    /// The bundled file's bytes.
    pub const BUNDLED: &'static [u8] = include_bytes!("../content/starting_unlocks.json");

    /// Checks the size, parses strictly and validates against the files the IDs name: the one
    /// call for a file's bytes.
    pub fn parse_with(
        bytes: &[u8],
        perk_sets: &PerkSets,
        stage_skills: &StageSkills,
        runes: &Runes,
        weapons: &Weapons,
    ) -> Result<StartingUnlocks, Error> {
        if bytes.len() > MAX_STARTING_UNLOCKS_BYTES {
            return Err(Error::TooLarge {
                size: bytes.len(),
                limit: MAX_STARTING_UNLOCKS_BYTES,
            });
        }
        // Versions first: a file for a newer game may have fields this one rejects.
        #[derive(Deserialize)]
        struct Versions {
            schema_version: u32,
            sim_version: u32,
        }
        let parse_error = |e: serde_json::Error| Error::Parse {
            line: e.line(),
            column: e.column(),
        };
        let versions: Versions = serde_json::from_slice(bytes).map_err(parse_error)?;
        if versions.schema_version != SCHEMA_VERSION || versions.sim_version != SIM_VERSION {
            return Err(Error::UnsupportedVersion {
                schema_version: versions.schema_version,
                sim_version: versions.sim_version,
            });
        }
        let file: StartingUnlocksFile = serde_json::from_slice(bytes).map_err(parse_error)?;
        validate(&file, perk_sets, stage_skills, runes, weapons)?;
        Ok(StartingUnlocks {
            file,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The weapon presets a new save owns.
    pub fn weapons(&self) -> &[ContentId] {
        &self.file.weapons
    }

    /// The perk sets a new save has open.
    pub fn perk_sets(&self) -> &[ContentId] {
        &self.file.perk_sets
    }

    /// The stage skills a new save has open.
    pub fn stage_skills(&self) -> &[ContentId] {
        &self.file.stage_skills
    }

    /// The runes a new save has open.
    pub fn runes(&self) -> &[ContentId] {
        &self.file.runes
    }

    /// The SHA-256 of the bytes they were parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Whether an ID is one the game holds.
trait KnownId: Fn(&ContentId) -> bool {}
impl<F: Fn(&ContentId) -> bool> KnownId for F {}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(
    file: &StartingUnlocksFile,
    perk_sets: &PerkSets,
    stage_skills: &StageSkills,
    runes: &Runes,
    weapons: &Weapons,
) -> Result<(), Error> {
    let mut issues = Vec::new();
    let weapon_known = |id: &ContentId| weapons.ids().any(|w| w == id);
    let perk_set_known = |id: &ContentId| perk_sets.index_of(id).is_some();
    let stage_skill_known = |id: &ContentId| stage_skills.index_of(id).is_some();
    let rune_known = |id: &ContentId| {
        Rune::from_id(&id.0).is_some_and(|rune| {
            runes.on_hit(rune).is_some() || runes.damage_over_time(rune).is_some()
        })
    };
    let lists: [(&str, &[ContentId], &dyn KnownId); 4] = [
        ("weapons", &file.weapons, &weapon_known),
        ("perk_sets", &file.perk_sets, &perk_set_known),
        ("stage_skills", &file.stage_skills, &stage_skill_known),
        ("runes", &file.runes, &rune_known),
    ];
    for (name, list, known) in lists {
        if list.len() > MAX_STARTING_UNLOCKS {
            issues.push(Issue {
                path: format!("$.{name}"),
                message: format!("at most {MAX_STARTING_UNLOCKS} IDs (found {})", list.len()),
            });
            continue;
        }
        for (i, id) in list.iter().enumerate() {
            let path = format!("$.{name}[{i}]");
            if !registry::is_well_formed(&id.0) {
                issues.push(Issue {
                    path,
                    message: "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
                });
            } else if list[..i].contains(id) {
                issues.push(Issue {
                    path,
                    message: "the same ID as an earlier entry".into(),
                });
            } else if !known(id) {
                issues.push(Issue {
                    path,
                    message: "not an ID the game holds".into(),
                });
            }
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}
