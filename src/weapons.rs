//! The weapons file (WP1, DS20 item 2): each weapon's combo trees as content data, so that a
//! weapon is balanced as content and not as code. A weapon is a registered weapon ID (ADR-CONT-2)
//! with exactly `MAX_TREES` (one) base tree (ADR-SIM-9 point 2, ADR-REPL-5); a tree is a
//! `Weapon`, the stage format's own type, under a tree ID of its own. A stage names one by ID and tree
//! (`WeaponRef::Preset`), or holds an inline weapon. The attack presets are content too
//! (ADR-CONT-3, CS1b-1, `attack_presets.rs`).
//!
//! The file is untrusted (ADR-SVC-1), as every content file is: a size limit before parsing,
//! strict parsing, the current versions only, and every list bounded: at most `MAX_WEAPONS`
//! weapons with unique registered IDs, exactly `MAX_TREES` tree (one) each with a well-formed
//! ID, and each tree checked as a stage's inline weapon is (`validate::validate_weapon_preset`).
//! The SHA-256 of the file's bytes is part of a run's content hash through the pack, so a run
//! replay verifies only with the weapons it was made with.
//! The bundled file, `content/weapons.json`, is part of the bundled pack.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::id::ContentId;
use crate::pack::Pack;
use crate::registry::{self, Kind};
use crate::validate::{self, Error, Issue};
use crate::version::{self, Versions};
use crate::weapon::{Weapon, WeaponRef};

/// The largest weapons file read (a tuning value; ADR-SVC-1 decides that one exists).
pub const MAX_WEAPONS_BYTES: usize = 512 * 1024;
/// The most weapons the file lists (1 today; a tuning value).
pub const MAX_WEAPONS: usize = 16;
/// The trees one weapon has: exactly one, its base tree (ADR-SIM-9 point 2). The tree's ID and
/// the setup's `weapon_tree` stay, so a setup naming `<weapon>_base` or none still loads.
pub const MAX_TREES: usize = 1;

/// One tree of a weapon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeaponTree {
    /// The tree's own ID, unique within its weapon: lowercase snake_case, `<weapon>_base` for
    /// tree 1 by convention.
    pub id: ContentId,
    pub weapon: Weapon,
}

/// A weapon and its trees.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeaponEntry {
    /// A registered weapon ID, unique within the file.
    pub id: ContentId,
    /// The trees: exactly `MAX_TREES`, the base tree.
    pub trees: Vec<WeaponTree>,
}

/// The weapons file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeaponsFile {
    /// The weapons, 1 to `MAX_WEAPONS`.
    pub weapons: Vec<WeaponEntry>,
}

/// Weapons that passed validation, and the SHA-256 of the bytes they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Weapons {
    list: Vec<WeaponEntry>,
    sha256: [u8; 32],
}

impl Weapons {
    /// The bundled weapons, the default of every run.
    pub fn bundled() -> &'static Weapons {
        Pack::bundled().content().weapons()
    }

    /// Checks the size and the versions given (the pack's), parses strictly and validates: the one
    /// call for a file's bytes.
    pub fn parse(bytes: &[u8], versions: Versions) -> Result<Weapons, Error> {
        let file: WeaponsFile = version::read_member(bytes, MAX_WEAPONS_BYTES, versions)?;
        validate(&file)?;
        Ok(Weapons {
            list: file.weapons,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The weapons, in the file's order.
    pub fn list(&self) -> &[WeaponEntry] {
        &self.list
    }

    /// The ID of every weapon that has a definition.
    pub fn ids(&self) -> impl Iterator<Item = &ContentId> {
        self.list.iter().map(|w| &w.id)
    }

    /// The IDs of a weapon's trees, tree 1 first (ADR-REPL-5); none for an ID that is no weapon.
    pub fn tree_ids(&self, weapon: &str) -> impl Iterator<Item = &ContentId> {
        self.list
            .iter()
            .find(|w| w.id.as_str() == weapon)
            .into_iter()
            .flat_map(|w| w.trees.iter().map(|t| &t.id))
    }

    /// What is wrong with naming `tree` for the weapon `weapon`, if anything (ADR-SVC-1: fail
    /// closed, and never echo the file's text): not a well-formed ID, or not one of the
    /// weapon's trees. A weapon with no definition is left to the weapon's own check.
    pub fn tree_problem(&self, weapon: &str, tree: &str) -> Option<&'static str> {
        if !registry::is_well_formed(tree) {
            Some("not a valid ID (lowercase snake_case, at most 32 characters)")
        } else if self.tree_ids(weapon).next().is_some()
            && !self.tree_ids(weapon).any(|t| t.as_str() == tree)
        {
            Some("no such tree for this weapon")
        } else {
            None
        }
    }

    /// A weapon's tree: the one `tree` names, or the first when `None`. `None` for a weapon with
    /// no definition or a tree it does not have.
    pub fn tree(&self, id: &str, tree: Option<&str>) -> Option<&Weapon> {
        let entry = self.list.iter().find(|w| w.id.as_str() == id)?;
        match tree {
            None => entry.trees.first(),
            Some(tree) => entry.trees.iter().find(|t| t.id.as_str() == tree),
        }
        .map(|t| &t.weapon)
    }

    /// The weapon a reference names: the preset's definition (in its tree), or the inline
    /// weapon itself. `None` for a preset that has no definition, or a tree it does not have.
    pub fn resolve<'a>(&'a self, weapon: &'a WeaponRef<Weapon>) -> Option<&'a Weapon> {
        match weapon {
            WeaponRef::Preset { id, tree } => {
                self.tree(id.as_str(), tree.as_ref().map(|t| t.as_str()))
            }
            WeaponRef::Inline(w) => Some(w),
        }
    }

    /// The SHA-256 of the bytes they were parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &WeaponsFile) -> Result<(), Error> {
    let mut issues = Vec::new();
    let mut issue = |path: String, message: String| issues.push(Issue { path, message });
    if file.weapons.is_empty() || file.weapons.len() > MAX_WEAPONS {
        issue(
            "$.weapons".into(),
            format!("1 to {MAX_WEAPONS} weapons (found {})", file.weapons.len()),
        );
    }
    for (i, w) in file.weapons.iter().enumerate().take(MAX_WEAPONS) {
        let at = format!("$.weapons[{i}]");
        if registry::find(registry::ENTRIES, Kind::Weapon, w.id.as_str()).is_none() {
            issue(format!("{at}.id"), "not a registered weapon".into());
        } else if file.weapons[..i].iter().any(|x| x.id == w.id) {
            issue(
                format!("{at}.id"),
                "the same ID as an earlier weapon".into(),
            );
        }
        if w.trees.len() != MAX_TREES {
            issue(
                format!("{at}.trees"),
                format!("exactly one base tree (found {})", w.trees.len()),
            );
        }
        for (t, tree) in w.trees.iter().enumerate().take(MAX_TREES) {
            let at = format!("{at}.trees[{t}]");
            if !registry::is_well_formed(tree.id.as_str()) {
                issue(
                    format!("{at}.id"),
                    "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
                );
            } else if w.trees[..t].iter().any(|x| x.id == tree.id) {
                issue(format!("{at}.id"), "the same ID as an earlier tree".into());
            }
            if let Err(found) = validate::validate_weapon_preset(&tree.weapon, registry::ENTRIES) {
                for f in found {
                    issue(
                        format!("{at}.weapon{}", f.path.trim_start_matches('$')),
                        f.message,
                    );
                }
            }
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}
