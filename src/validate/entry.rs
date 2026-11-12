//! The entry points: bytes or a parsed stage, an attack, a mode file and a weapon preset.

use super::limits::*;
use super::{Ctx, Foe};
use crate::attack::{AttackPhase, AttackRef};
use crate::expertise::StepGrant;
use crate::registry::{self, Entry};
use crate::stage::{Levels, Stage, Stars, Wave};
use crate::version::{self};
use crate::weapon::{SmashEffect, Weapon};

use super::{Error, Issue};

/// Checks the size, parses strictly and validates: the one call for a stage file's bytes.
pub fn parse_and_validate(bytes: &[u8]) -> Result<Stage, Error> {
    let stage: Stage = version::read_current(bytes, MAX_STAGE_BYTES)?;
    validate(&stage)?;
    Ok(stage)
}

/// Checks the phases of one attack against the same budgets as a stage's inline attacks, for an
/// attack that takes `arity` arguments (the developer's presets, ADR-SIM-4). The problems are
/// reported at paths from the attack: `$[0].spawn[0].body[1]`.
pub fn validate_attack(
    phases: &[AttackPhase],
    arity: u8,
    registry: &[Entry],
) -> Result<(), Vec<Issue>> {
    let mut ctx = Ctx {
        registry,
        sim_version: crate::version::SIM_VERSION,
        map: crate::stage::Map::PORTRAIT,
        parts: Vec::new(),
        path: Vec::new(),
        issues: Vec::new(),
        left_out: false,
        arity,
        foe: Foe::Enemy,
        ranged_ok: false,
        loop_rounds: 0,
    };
    ctx.phases(phases);
    if ctx.issues.is_empty() {
        Ok(())
    } else {
        Err(ctx.issues)
    }
}

/// Checks the waves of a run whose mode file holds its own schedule (`time_limit`): each as a
/// stage's wave is checked, on `map`, starting before `length_ticks`, with up to `max_count`
/// enemies. How many waves and how many enemies in all is the mode's to bound. Paths are from
/// the list: `$[0].enemy.hp`.
pub fn validate_waves(
    waves: &[Wave],
    map: crate::stage::Map,
    length_ticks: u32,
    max_count: u16,
) -> Result<(), Vec<Issue>> {
    let mut ctx = Ctx {
        registry: registry::ENTRIES,
        sim_version: crate::version::SIM_VERSION,
        map,
        parts: Vec::new(),
        path: Vec::new(),
        issues: Vec::new(),
        left_out: false,
        arity: 0,
        foe: Foe::Other,
        ranged_ok: false,
        loop_rounds: 0,
    };
    for (i, w) in waves.iter().enumerate() {
        ctx.idx(i, |c| c.wave(w, length_ticks, max_count));
    }
    if ctx.issues.is_empty() {
        Ok(())
    } else {
        Err(ctx.issues)
    }
}

/// Checks the experience curve and the star drop table of a run whose mode file holds its own
/// (`time_limit`), as a stage's `levels` and `stars` are checked. Paths are from the file's
/// fields: `$.levels.xp[0]`, `$.stars.tiers[1].from_level`.
pub fn validate_levels_and_stars(
    levels: &Levels,
    stars: &Stars,
    map: crate::stage::Map,
) -> Result<(), Vec<Issue>> {
    let mut ctx = Ctx {
        registry: registry::ENTRIES,
        sim_version: crate::version::SIM_VERSION,
        map,
        parts: Vec::new(),
        path: Vec::new(),
        issues: Vec::new(),
        left_out: false,
        arity: 0,
        foe: Foe::Other,
        ranged_ok: false,
        loop_rounds: 0,
    };
    ctx.key("levels", |c| c.levels(levels));
    ctx.key("stars", |c| c.stars(stars));
    if ctx.issues.is_empty() {
        Ok(())
    } else {
        Err(ctx.issues)
    }
}

/// Checks an attack outside a stage, a preset by ID with its arguments or an inline one, against
/// the same budgets as a stage's: a damage-area stage skill's. The problems are
/// reported at paths from the attack: `$.preset.id`, `$.inline[0].ticks`.
pub fn validate_attack_ref(attack: &AttackRef, registry: &[Entry]) -> Result<(), Vec<Issue>> {
    let mut ctx = Ctx {
        registry,
        sim_version: crate::version::SIM_VERSION,
        map: crate::stage::Map::PORTRAIT,
        parts: Vec::new(),
        path: Vec::new(),
        issues: Vec::new(),
        left_out: false,
        arity: 0,
        foe: Foe::Other,
        ranged_ok: false,
        loop_rounds: 0,
    };
    ctx.attack_ref(attack);
    if ctx.issues.is_empty() {
        Ok(())
    } else {
        Err(ctx.issues)
    }
}

/// Checks a weapon preset as a stage's inline weapon is checked, without a boss (so a part to
/// aim at is never found): the developer's weapon presets. Paths are from the preset:
/// `$.combo.main.charge.levels[0].at_ticks`.
pub fn validate_weapon_preset(preset: &Weapon, registry: &[Entry]) -> Result<(), Vec<Issue>> {
    let mut ctx = Ctx {
        registry,
        sim_version: crate::version::SIM_VERSION,
        map: crate::stage::Map::PORTRAIT,
        parts: Vec::new(),
        path: Vec::new(),
        issues: Vec::new(),
        left_out: false,
        arity: 0,
        foe: Foe::Other,
        ranged_ok: false,
        loop_rounds: 0,
    };
    ctx.weapon(preset);
    if ctx.issues.is_empty() {
        Ok(())
    } else {
        Err(ctx.issues)
    }
}

/// Validates a parsed stage against the registry.
pub fn validate(stage: &Stage) -> Result<(), Error> {
    validate_with(stage, registry::ENTRIES)
}

/// As `validate`, against a given registry (tests use one with newer and deprecated IDs).
pub fn validate_with(stage: &Stage, registry: &[Entry]) -> Result<(), Error> {
    version::check(stage.schema_version, stage.sim_version)?;
    let mut ctx = Ctx {
        registry,
        sim_version: stage.sim_version,
        map: stage.map(),
        parts: stage
            .boss
            .iter()
            .flat_map(|b| b.parts.iter().map(|p| p.id.as_str()))
            .collect(),
        path: Vec::new(),
        issues: Vec::new(),
        left_out: false,
        arity: 0,
        foe: Foe::Other,
        ranged_ok: false,
        loop_rounds: 0,
    };
    ctx.stage(stage);
    if ctx.left_out {
        ctx.path.clear();
        ctx.issues.push(Issue {
            path: "$".into(),
            message: "more problems were found; fix these first".into(),
        });
    }
    if ctx.issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(ctx.issues))
    }
}

/// Checks a style's smash effect as the weapon validator checks a weapon's own, so every number is
/// capped by the same bounds (ADR-SIM-9 point 4). Paths are from the effect: `$.stacks`.
pub fn validate_smash_effect(effect: &SmashEffect, registry: &[Entry]) -> Result<(), Vec<Issue>> {
    let mut ctx = Ctx {
        registry,
        sim_version: crate::version::SIM_VERSION,
        map: crate::stage::Map::PORTRAIT,
        parts: Vec::new(),
        path: Vec::new(),
        issues: Vec::new(),
        left_out: false,
        arity: 0,
        foe: Foe::Other,
        ranged_ok: false,
        loop_rounds: 0,
    };
    ctx.smash(effect);
    if ctx.issues.is_empty() {
        Ok(())
    } else {
        Err(ctx.issues)
    }
}

/// Checks a style's step grant as the weapon validator checks a charge or a hold, so every number
/// is capped by the same bounds (ADR-SIM-9 point 3). Paths are from the grant:
/// `$.charge.levels[0].at_ticks`, `$.hold.max_ticks`.
pub fn validate_step_grant(grant: &StepGrant, registry: &[Entry]) -> Result<(), Vec<Issue>> {
    let mut ctx = Ctx {
        registry,
        sim_version: crate::version::SIM_VERSION,
        map: crate::stage::Map::PORTRAIT,
        parts: Vec::new(),
        path: Vec::new(),
        issues: Vec::new(),
        left_out: false,
        arity: 0,
        foe: Foe::Other,
        ranged_ok: false,
        loop_rounds: 0,
    };
    match grant {
        StepGrant::Charge(c) => ctx.key("charge", |ctx| ctx.charge(c)),
        StepGrant::Hold(h) => ctx.key("hold", |ctx| ctx.hold(h)),
    }
    if ctx.issues.is_empty() {
        Ok(())
    } else {
        Err(ctx.issues)
    }
}
