//! The threat colour table (ADR-CONT-1 point 1, ADR-BRDG-4 point 9): the colours an enemy's
//! attack areas may be drawn in, one entry per colour, an ID and an RGB value. An enemy (`Agent`)
//! or a boss names one entry by ID (`threat_colour`); the snapshot's enemy attack and zone
//! views carry that entry's position as a `ThreatColourIndex`. The colour decides nothing in the
//! simulation: the state hash never reads it.
//!
//! An entry's index is its position in the file, 0-based. It is meaningful only with the run's
//! table (`Engine::threat_colours`), which the frontend reads once per run and remaps by ID.
//! An entry's ID is never renamed or removed once released (ADR-CONT-2). An enemy that names
//! no entry carries no index, and the frontend draws its own hostile colour: the table has no
//! default entry.
//!
//! The file is an optional member of the pack (`threat_colours`) and untrusted (ADR-SVC-1): a
//! size limit before parsing, strict parsing, the current versions only, at most
//! `MAX_THREAT_COLOURS` entries, each ID well formed and unique. A pack without the member has
//! an empty table, and an enemy naming a colour then fails closed (`Pack::from_bytes`,
//! `Engine::new_with`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::id::ContentId;
use crate::registry;
use crate::validate::{Error, Issue};
use crate::version::{self, Versions};

/// The largest threat colour file read (a tuning value; ADR-SVC-1 decides that one exists).
pub const MAX_THREAT_COLOURS_BYTES: usize = 4 * 1024;
/// The most entries one table has (a tuning value).
pub const MAX_THREAT_COLOURS: usize = 16;

/// The index of an entry in the run's threat colour table. It is only meaningful with that
/// run's table: key anything that outlives the run on the ID.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct ThreatColourIndex(pub u16);

/// One colour of the table: an ID and its RGB value, 0 to 255 each. Alpha and brightness are
/// the frontend's. The ID is lowercase snake_case, unique in the file, and never renamed or
/// removed once released (ADR-CONT-2): the frontend remaps by it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreatColour {
    pub id: ContentId,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// The threat colour file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreatColoursFile {
    /// At most `MAX_THREAT_COLOURS` entries; an empty list is valid.
    #[schemars(length(max = MAX_THREAT_COLOURS))]
    pub colours: Vec<ThreatColour>,
}

/// A threat colour file that passed validation, and the SHA-256 of the bytes it came from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreatColours {
    colours: Vec<ThreatColour>,
    sha256: [u8; 32],
}

impl ThreatColours {
    /// Checks the size and the versions given (the pack's), parses strictly and validates: the
    /// one call for a file's bytes.
    pub fn parse(bytes: &[u8], versions: Versions) -> Result<ThreatColours, Error> {
        let file: ThreatColoursFile =
            version::read_member(bytes, MAX_THREAT_COLOURS_BYTES, versions)?;
        validate(&file)?;
        Ok(ThreatColours {
            colours: file.colours,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// Every entry, in file order: an entry's position is its index.
    pub fn colours(&self) -> &[ThreatColour] {
        &self.colours
    }

    /// The index of the entry `id`, `None` for an ID the table does not hold.
    pub fn index_of(&self, id: &str) -> Option<ThreatColourIndex> {
        self.colours
            .iter()
            .position(|c| c.id.as_str() == id)
            .map(|i| ThreatColourIndex(u16::try_from(i).expect("at most MAX_THREAT_COLOURS")))
    }

    pub fn len(&self) -> usize {
        self.colours.len()
    }

    pub fn is_empty(&self) -> bool {
        self.colours.is_empty()
    }

    /// The SHA-256 of the bytes it was parsed from (zeros for the empty table of a pack
    /// without the member).
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &ThreatColoursFile) -> Result<(), Error> {
    let mut issues = Vec::new();
    if file.colours.len() > MAX_THREAT_COLOURS {
        issues.push(Issue {
            path: "$.colours".into(),
            message: format!(
                "at most {MAX_THREAT_COLOURS} entries (found {})",
                file.colours.len()
            ),
        });
        return Err(Error::Invalid(issues));
    }
    for (i, colour) in file.colours.iter().enumerate() {
        if !registry::is_well_formed(colour.id.as_str()) {
            issues.push(Issue {
                path: format!("$.colours[{i}].id"),
                message: "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
            });
        } else if file.colours[..i].iter().any(|c| c.id == colour.id) {
            issues.push(Issue {
                path: format!("$.colours[{i}].id"),
                message: "the same colour as an earlier entry".into(),
            });
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}

/// Checks that every threat colour `stage` and `bosses` (the bosses a `time_limit` run brings
/// later) name is an entry of `table`: the first that is not is refused at its path, with a
/// message that never repeats the name. A stage that names none passes with any table.
pub(crate) fn check(
    stage: &crate::stage::Stage,
    bosses: &[&crate::stage::Boss],
    table: &ThreatColours,
) -> Result<(), Issue> {
    let mut found = None;
    crate::kinds::visit_foes(stage, bosses, &mut |foe, path| {
        let name = match foe {
            crate::kinds::Foe::Agent(a) => a.threat_colour.as_ref(),
            crate::kinds::Foe::Boss(b) => b.threat_colour.as_ref(),
        };
        if found.is_none() && name.is_some_and(|n| table.index_of(n.as_str()).is_none()) {
            found = Some(format!("{path}.threat_colour"));
        }
    });
    match found {
        None => Ok(()),
        Some(path) => Err(Issue {
            path,
            message: NO_SUCH_COLOUR.into(),
        }),
    }
}

/// The message of a threat colour the table does not hold.
pub(crate) const NO_SUCH_COLOUR: &str = "no such threat colour in the pack's table";

/// As `check`, for the enemies and bosses of a `time_limit` mode file: its waves' enemies, its
/// elite types' enemies and its boss definitions, at their paths in the file. A pack checks
/// them at load, so a pack that loads can be run.
pub(crate) fn check_time_limit(
    mode: &crate::time_limit::TimeLimit,
    table: &ThreatColours,
) -> Result<(), Issue> {
    use std::fmt::Write;
    let mut found = None;
    let mut visit = |foe: crate::kinds::Foe<'_>, path: &str| {
        let name = match foe {
            crate::kinds::Foe::Agent(a) => a.threat_colour.as_ref(),
            crate::kinds::Foe::Boss(b) => b.threat_colour.as_ref(),
        };
        if found.is_none() && name.is_some_and(|n| table.index_of(n.as_str()).is_none()) {
            found = Some(format!("{path}.threat_colour"));
        }
    };
    let mut path = String::new();
    for (i, wave) in mode.waves().iter().enumerate() {
        path.clear();
        let _ = write!(path, "$.waves[{i}].enemy");
        crate::kinds::visit_agent(&wave.enemy, &mut path, &mut visit);
    }
    for (i, elite) in mode.elite_types().iter().enumerate() {
        path.clear();
        let _ = write!(path, "$.elite_types[{i}].enemy");
        crate::kinds::visit_agent(&elite.enemy, &mut path, &mut visit);
    }
    for (i, boss) in mode.bosses().iter().enumerate() {
        path.clear();
        let _ = write!(path, "$.bosses[{i}]");
        crate::kinds::visit_boss(boss, &mut path, &mut visit);
    }
    match found {
        None => Ok(()),
        Some(path) => Err(Issue {
            path,
            message: NO_SUCH_COLOUR.into(),
        }),
    }
}
