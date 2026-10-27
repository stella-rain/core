//! The Stardust file (ADR-101 point 4, ADR-080, DS6 Q10): the rules the app pays a failed run's
//! Stardust by, one entry per mode. A clear pays all the Stardust a run earned; a fail pays
//! `pay_pct` of it and, while `bank` is on, adds `bank_pct` of it (rounded up, never more than
//! what is left after the pay) to a bank kept up to `bank_cap`. The next clear of the same mode
//! at the same difficulty pays the whole bank. The bank's key is therefore the pair (mode,
//! difficulty); the entry here is a mode's and holds for every difficulty.
//!
//! Core does not pay, split or bank Stardust: the app does, and keeps the bank in its save
//! (ADR-101 point 7). Core counts what a run earns (ADR-109, `rules_v0::stardust`); this file
//! only carries the numbers the app reads, so no simulation outcome depends on it.
//!
//! The file is untrusted (ADR-020), as every content file is: a size limit before parsing,
//! strict parsing, the current versions only, and every number bounded: at most `MAX_MODES`
//! entries, each a mode of the registry (ADR-026) once, `pay_pct` and `bank_pct` from 0 to 100
//! with a sum of at most 100, and `bank_cap` at most `MAX_BANK_CAP`. `Pack` holds the file as
//! an optional field: a pack released before it yields the bundled file (`Stardust::bundled`),
//! so older packs still load. A mode without an entry has none of these rules: what the app
//! does for it is the app's choice, never a guess made here (`Stardust::mode` is `None`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::id::ContentId;
use crate::pack::Pack;
use crate::registry::{self, Kind};
use crate::validate::{Error, Issue};
use crate::version::{self, Versions};

/// The largest Stardust file read (a tuning value; ADR-020 decides that one exists).
pub const MAX_STARDUST_BYTES: usize = 4 * 1024;
/// The most modes one file has entries for (3 modes exist today).
pub const MAX_MODES: usize = 16;
/// The hard ceiling of any mode's `bank_cap`, in whole Stardust (a tuning value, ADR-101
/// point 5).
pub const MAX_BANK_CAP: u32 = 1_000_000;
/// The largest share: a percent.
pub const MAX_PCT: u32 = 100;

/// One mode's Stardust rules.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModeStardust {
    /// A mode ID of the registry, once in the file.
    pub id: ContentId,
    /// Whether a fail banks anything and a clear pays a bank. Off: a fail pays `pay_pct` and
    /// banks nothing; a bank already saved stays until it is on again.
    pub bank: bool,
    /// The percent of a fail's Stardust that is paid at once, rounded down, from 0 to 100.
    pub pay_pct: u32,
    /// The percent of a fail's Stardust that is banked, rounded up and never more than what the
    /// pay leaves, from 0 to 100; with `pay_pct` at most 100.
    pub bank_pct: u32,
    /// The most a bank of this mode (at each difficulty) holds, at most `MAX_BANK_CAP`;
    /// Stardust that would pass it is lost.
    pub bank_cap: u32,
}

/// The Stardust file: one entry per mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StardustFile {
    /// At most `MAX_MODES` entries, one per mode.
    pub modes: Vec<ModeStardust>,
}

/// A Stardust file that passed validation, and the SHA-256 of the bytes it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stardust {
    file: StardustFile,
    sha256: [u8; 32],
}

impl Stardust {
    /// The bundled file: what a pack without the file yields.
    pub fn bundled() -> &'static Stardust {
        Pack::bundled().stardust()
    }

    /// Checks the size and the versions given (the pack's), parses strictly and validates: the one
    /// call for a file's bytes.
    pub fn parse(bytes: &[u8], versions: Versions) -> Result<Stardust, Error> {
        let file: StardustFile = version::read_member(bytes, MAX_STARDUST_BYTES, versions)?;
        validate(&file)?;
        Ok(Stardust {
            file,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// Every mode's entry, in file order.
    pub fn modes(&self) -> &[ModeStardust] {
        &self.file.modes
    }

    /// The entry of mode `id`, `None` for a mode the file has none for.
    pub fn mode(&self, id: &str) -> Option<&ModeStardust> {
        self.file.modes.iter().find(|m| m.id.0 == id)
    }

    /// The SHA-256 of the bytes it was parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &StardustFile) -> Result<(), Error> {
    let mut issues = Vec::new();
    if file.modes.len() > MAX_MODES {
        issues.push(Issue {
            path: "$.modes".into(),
            message: format!("at most {MAX_MODES} entries (found {})", file.modes.len()),
        });
        return Err(Error::Invalid(issues));
    }
    for (i, mode) in file.modes.iter().enumerate() {
        let path = format!("$.modes[{i}]");
        let mut bad = |field: &str, message: String| {
            issues.push(Issue {
                path: format!("{path}{field}"),
                message,
            });
        };
        if !registry::is_well_formed(&mode.id.0) {
            bad(
                ".id",
                "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
            );
        } else if file.modes[..i].iter().any(|m| m.id == mode.id) {
            bad(".id", "the same mode as an earlier entry".into());
        } else if registry::find(registry::ENTRIES, Kind::Mode, &mode.id.0).is_none() {
            bad(".id", "not a mode the game holds".into());
        }
        let mut shares_ok = true;
        for (field, pct) in [(".pay_pct", mode.pay_pct), (".bank_pct", mode.bank_pct)] {
            if pct > MAX_PCT {
                bad(field, format!("a percent from 0 to {MAX_PCT}"));
                shares_ok = false;
            }
        }
        if shares_ok && mode.pay_pct + mode.bank_pct > MAX_PCT {
            bad(
                "",
                format!("pay_pct and bank_pct add up to at most {MAX_PCT}"),
            );
        }
        if mode.bank_cap > MAX_BANK_CAP {
            bad(".bank_cap", format!("at most {MAX_BANK_CAP}"));
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}
