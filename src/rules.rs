//! The rules file (DIR1, DS103): the numbers of the simulation that live in the content pack.
//! It starts with the crowd: `crowd_target`, the number of enemies a horde run keeps alive, and
//! `cap_headroom_pct`, how far above the target the enemy cap stands. The cap is
//! `target * (100 + headroom) / 100`, rounded down, derived when a run is built and never stored
//! (260 for the bundled file); the headroom is the room the specials (elites, escorts, a
//! splitter's children, guardians, the boss's summons) spawn in while the waves are held at the
//! target. A mode or a stage may override the target, not the headroom.
//!
//! Unlike `stardust`, the simulation reads this file, so the pack must hold it (a required slot)
//! and the derived content hash covers it: a replay never depends on the client's build.
//!
//! The file is untrusted (ADR-020), as every content file is: a size limit before parsing, strict
//! parsing, the current versions only, and every number bounded. The pack's own target must keep
//! its derived cap within `limits::MAX_ENEMY_CAP` (ADR-SIM-7): a pack over the ceiling is
//! refused. An override is bounded by its own file (1 to `limits::MAX_CROWD_TARGET`) and clamped
//! when the run is built (`Rules::enemy_cap_for`), so the cap never passes the ceiling.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::pack::Pack;
use crate::validate::limits::MAX_ENEMY_CAP;
use crate::validate::{Error, Issue};
use crate::version;

/// The largest rules file read (a placeholder; ADR-020 decides that a limit exists).
pub const MAX_RULES_BYTES: usize = 4 * 1024;
/// The least and the most the cap stands above the target, in percent (DS101: 125 to 130%).
pub const MIN_CAP_HEADROOM_PCT: u8 = 25;
pub const MAX_CAP_HEADROOM_PCT: u8 = 30;

/// The rules file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RulesFile {
    pub schema_version: u32,
    pub sim_version: u32,
    /// The enemies a horde run keeps alive: wave spawns that fall due at it are held. From 1 to
    /// the most that keeps the cap within the engine ceiling (384 at a headroom of 30).
    pub crowd_target: u16,
    /// How far above the target the cap stands, in percent, from 25 to 30.
    pub cap_headroom_pct: u8,
}

/// A rules file that passed validation, and the SHA-256 of the bytes it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rules {
    file: RulesFile,
    sha256: [u8; 32],
}

/// The most a target can be at `headroom_pct` for its cap to stay within the engine ceiling.
fn max_target(headroom_pct: u8) -> u32 {
    u32::from(MAX_ENEMY_CAP) * 100 / (100 + u32::from(headroom_pct))
}

impl Rules {
    /// The bundled file.
    pub fn bundled() -> &'static Rules {
        Pack::bundled().content().rules()
    }

    /// Checks the size, parses strictly and validates: the one call for a file's bytes.
    pub fn parse(bytes: &[u8]) -> Result<Rules, Error> {
        let file: RulesFile = version::read_current(bytes, MAX_RULES_BYTES)?;
        validate(&file)?;
        Ok(Rules {
            file,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The crowd target of a run that overrides nothing.
    pub fn crowd_target(&self) -> u16 {
        self.file.crowd_target
    }

    /// How far above the target the cap stands, in percent.
    pub fn cap_headroom_pct(&self) -> u8 {
        self.file.cap_headroom_pct
    }

    /// The target a run resolves: `target` (an override) or the file's, kept so that its cap
    /// stays within the engine ceiling.
    pub fn resolved_target(&self, target: Option<u16>) -> u32 {
        u32::from(target.unwrap_or(self.file.crowd_target))
            .min(max_target(self.file.cap_headroom_pct))
    }

    /// The enemy cap of a run whose target is `target` (clamped as `resolved_target` does):
    /// `target * (100 + headroom) / 100`, rounded down. Never stored; at most the ceiling.
    pub fn enemy_cap_for(&self, target: u16) -> u32 {
        self.resolved_target(Some(target)) * (100 + u32::from(self.file.cap_headroom_pct)) / 100
    }

    /// The SHA-256 of the bytes it was parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &RulesFile) -> Result<(), Error> {
    let mut issues = Vec::new();
    let headroom = file.cap_headroom_pct;
    let headroom_ok = (MIN_CAP_HEADROOM_PCT..=MAX_CAP_HEADROOM_PCT).contains(&headroom);
    if !headroom_ok {
        issues.push(Issue {
            path: "$.cap_headroom_pct".into(),
            message: format!("{MIN_CAP_HEADROOM_PCT} to {MAX_CAP_HEADROOM_PCT} (found {headroom})"),
        });
    } else {
        let most = max_target(headroom);
        let target = u32::from(file.crowd_target);
        if target == 0 || target > most {
            issues.push(Issue {
                path: "$.crowd_target".into(),
                message: format!("1 to {most} at headroom {headroom} (found {target})"),
            });
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}
