//! The runes file (ADR-099, ADR-067: data picks, code acts): each on-hit rune's numbers. What a
//! rune does is code (`rules_v0`'s runes); when it fires and how hard is this file's. An on-hit
//! rune has a cooldown, a base damage and its own attack: phases as any attack's, whose hit
//! boxes are centred on the point the combo step hit instead of on the main character.
//!
//! Boomerang's attack is its throw (`rules_v0`'s runes): exactly two phases, out and back, each
//! active with a hit box. Homing missiles' is one missile's flight: exactly one phase, active
//! with a hit box. Only its entry has `missiles`, 1 to `MAX_HOMING_MISSILES` launched together,
//! and `range`, 1 to `MAX_ASSIST_RANGE`, within which they pick their targets; both are
//! required on it and refused on every other rune.
//!
//! The file is untrusted (ADR-020), as every content file is: a size limit before parsing,
//! strict parsing, the current versions only, every on-hit rune of the closed list once, and
//! bounds under hard caps (ADR-099 point 7): a cooldown of 1 to `MAX_RUNE_COOLDOWN_TICKS`, a
//! damage of 1 to `MAX_DAMAGE`, and an attack of 1 to `MAX_RUNE_PHASES` phases, checked as any
//! attack, that lasts no longer than the cooldown, so each rune has at most one attack going.
//! It spawns no bullets, so no bullet comes from a rune; it sets no damage of its own, the
//! rune's `damage` being its base; and it never slows the main character. Its first phase lasts
//! at least 2 ticks: it counts the tick the rune fires on, whose hits have landed already.
//! The bundled file, `content/runes.json`, is the default; the SHA-256 of whatever bytes a
//! run used is part of its content hash, so a run replay verifies only with the same file.

use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::attack::{AttackPhase, AttackRef, PhaseKind};
use crate::fixed::Fx;
use crate::registry;
use crate::rune::{Rune, RuneKind};
use crate::validate::limits::{MAX_ASSIST_RANGE, MAX_DAMAGE};
use crate::validate::{Error, Issue};
use crate::version::{SCHEMA_VERSION, SIM_VERSION};

/// The largest runes file read (a tuning value; ADR-020 decides that one exists).
pub const MAX_RUNES_BYTES: usize = 8 * 1024;
/// The longest cooldown of an on-hit rune: a minute.
pub const MAX_RUNE_COOLDOWN_TICKS: u32 = 3_600;
/// The most phases an on-hit rune's attack has.
pub const MAX_RUNE_PHASES: usize = 4;
/// The most missiles homing missiles launch at once (a tuning value; ADR-099 point 7 decides
/// that one exists).
pub const MAX_HOMING_MISSILES: u32 = 4;

/// The runes file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunesFile {
    pub schema_version: u32,
    pub sim_version: u32,
    /// Every on-hit rune's numbers, each rune once.
    pub on_hit: Vec<OnHitRune>,
}

/// An on-hit rune's numbers (ADR-099 point 2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OnHitRune {
    pub rune: Rune,
    /// Ticks it waits after it fires, 1 to `MAX_RUNE_COOLDOWN_TICKS`, at least its attack's
    /// length.
    pub cooldown_ticks: u32,
    /// The base damage of its attack's hits, 1 to `MAX_DAMAGE` (ADR-056).
    pub damage: u32,
    /// Its attack's phases, 1 to `MAX_RUNE_PHASES`: hit boxes centred on the point hit, facing
    /// the way the main character faces as it fires; a phase's `type` is its hits' damage type.
    pub attack: Vec<AttackPhase>,
    /// Homing missiles only: how many it launches, 1 to `MAX_HOMING_MISSILES`, each running
    /// `attack` as its flight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missiles: Option<u32>,
    /// Homing missiles only: how far from the main character its targets may be, 1 to
    /// `MAX_ASSIST_RANGE`, as aim assist's range (ADR-072).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Fx>,
}

/// A runes file that passed validation, and the SHA-256 of the bytes it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runes {
    on_hit: Vec<OnHitRune>,
    sha256: [u8; 32],
}

impl Runes {
    /// The bundled file's bytes.
    pub const BUNDLED: &'static [u8] = include_bytes!("../content/runes.json");

    /// The bundled runes, the default of every run.
    pub fn bundled() -> &'static Runes {
        static BUNDLED: OnceLock<Runes> = OnceLock::new();
        BUNDLED.get_or_init(|| Runes::parse(Runes::BUNDLED).expect("the bundled runes"))
    }

    /// Checks the size, parses strictly and validates: the one call for a file's bytes.
    pub fn parse(bytes: &[u8]) -> Result<Runes, Error> {
        if bytes.len() > MAX_RUNES_BYTES {
            return Err(Error::TooLarge {
                size: bytes.len(),
                limit: MAX_RUNES_BYTES,
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
        let file: RunesFile = serde_json::from_slice(bytes).map_err(parse_error)?;
        validate(&file)?;
        Ok(Runes {
            on_hit: file.on_hit,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The numbers of on-hit rune `rune`; none for a damage-over-time rune.
    pub fn on_hit(&self, rune: Rune) -> Option<&OnHitRune> {
        self.on_hit.iter().find(|r| r.rune == rune)
    }

    /// The SHA-256 of the bytes it was parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &RunesFile) -> Result<(), Error> {
    let mut issues = Vec::new();
    let mut fail = |path: String, message: String| issues.push(Issue { path, message });
    for (i, r) in file.on_hit.iter().enumerate() {
        let at = |field: &str| format!("$.on_hit[{i}].{field}");
        if r.rune.kind() != RuneKind::OnHit {
            fail(at("rune"), "an on-hit rune".into());
        } else if file.on_hit[..i].iter().any(|o| o.rune == r.rune) {
            fail(at("rune"), "each rune once".into());
        }
        let cooldown_ok = (1..=MAX_RUNE_COOLDOWN_TICKS).contains(&r.cooldown_ticks);
        if !cooldown_ok {
            fail(
                at("cooldown_ticks"),
                format!(
                    "1 to {MAX_RUNE_COOLDOWN_TICKS} (found {})",
                    r.cooldown_ticks
                ),
            );
        }
        if !(1..=MAX_DAMAGE).contains(&r.damage) {
            fail(
                at("damage"),
                format!("1 to {MAX_DAMAGE} (found {})", r.damage),
            );
        }
        attack(r, cooldown_ok, &at("attack"), &mut fail);
        homing(r, &at, &mut fail);
    }
    if Rune::ALL
        .iter()
        .any(|r| r.kind() == RuneKind::OnHit && !file.on_hit.iter().any(|o| o.rune == *r))
    {
        fail("$.on_hit".into(), "every on-hit rune once".into());
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}

/// A thrown rune's attack, at `path`: two legs, out and back, each active with a hit box.
fn throw(phases: &[AttackPhase], path: &str, fail: &mut impl FnMut(String, String)) {
    if phases.len() != 2 {
        fail(
            path.into(),
            format!("2 phases, out and back (found {})", phases.len()),
        );
        return;
    }
    for (j, p) in phases.iter().enumerate() {
        if p.kind != PhaseKind::Active {
            fail(format!("{path}[{j}].kind"), "an active leg".into());
        } else if p.hit_box.is_none() {
            fail(
                format!("{path}[{j}].hit_box"),
                "a leg with a hit box".into(),
            );
        }
    }
}

/// Homing missiles' own numbers, `missiles` and `range`: required on it, refused on the others.
fn homing(r: &OnHitRune, at: &impl Fn(&str) -> String, fail: &mut impl FnMut(String, String)) {
    let own = r.rune == Rune::HomingMissiles;
    match r.missiles {
        None if own => fail(at("missiles"), "the missiles it launches".into()),
        Some(_) if !own => fail(at("missiles"), "homing missiles' own".into()),
        Some(n) if !(1..=MAX_HOMING_MISSILES).contains(&n) => fail(
            at("missiles"),
            format!("1 to {MAX_HOMING_MISSILES} (found {n})"),
        ),
        _ => {}
    }
    match r.range {
        None if own => fail(at("range"), "the range of its targets".into()),
        Some(_) if !own => fail(at("range"), "homing missiles' own".into()),
        Some(Fx(range)) if !(1..=MAX_ASSIST_RANGE).contains(&range) => fail(
            at("range"),
            format!("1 to {MAX_ASSIST_RANGE} (found {range})"),
        ),
        _ => {}
    }
}

/// Homing missiles' attack, at `path`: one flight, active with a hit box.
fn flight(phases: &[AttackPhase], path: &str, fail: &mut impl FnMut(String, String)) {
    let [p] = phases else {
        fail(
            path.into(),
            format!("1 phase, the flight (found {})", phases.len()),
        );
        return;
    };
    if p.kind != PhaseKind::Active {
        fail(format!("{path}[0].kind"), "an active flight".into());
    } else if p.hit_box.is_none() {
        fail(
            format!("{path}[0].hit_box"),
            "a flight with a hit box".into(),
        );
    }
}

/// An on-hit rune's attack, at `path`.
fn attack(r: &OnHitRune, cooldown_ok: bool, path: &str, fail: &mut impl FnMut(String, String)) {
    let phases = &r.attack;
    if phases.is_empty() || phases.len() > MAX_RUNE_PHASES {
        fail(
            path.into(),
            format!("1 to {MAX_RUNE_PHASES} phases (found {})", phases.len()),
        );
        return;
    }
    if let Err(found) =
        crate::validate::validate_attack_ref(&AttackRef::Inline(phases.clone()), registry::ENTRIES)
    {
        // The attack's paths start at `$.inline`.
        for f in found {
            let rest = f.path.strip_prefix("$.inline").unwrap_or(&f.path[1..]);
            fail(format!("{path}{rest}"), f.message);
        }
        return;
    }
    for (j, p) in phases.iter().enumerate() {
        let at = |field: &str| format!("{path}[{j}].{field}");
        if !p.spawn.is_empty() {
            fail(at("spawn"), "no bullets from a rune".into());
        }
        if p.damage.is_some() {
            fail(at("damage"), "the rune's damage is its base".into());
        }
        if p.move_pct != 100 {
            fail(at("move_pct"), "never slows the main character".into());
        }
    }
    if phases[0].ticks < 2 {
        fail(
            format!("{path}[0].ticks"),
            format!("at least 2 (found {})", phases[0].ticks),
        );
    }
    if r.rune == Rune::Boomerang {
        throw(phases, path, fail);
    } else if r.rune == Rune::HomingMissiles {
        flight(phases, path, fail);
    } else if !phases
        .iter()
        .any(|p| p.kind == PhaseKind::Active && p.hit_box.is_some())
    {
        fail(path.into(), "an active phase with a hit box".into());
    }
    let length = phases
        .iter()
        .fold(0u32, |sum, p| sum.saturating_add(p.ticks));
    if cooldown_ok && length > r.cooldown_ticks {
        fail(
            path.into(),
            format!(
                "at most the cooldown, {} ticks (found {length})",
                r.cooldown_ticks
            ),
        );
    }
}
