//! The stage validator (ADR-020): a byte-size limit before parsing, strict parsing, registry
//! lookups with the introducing `sim_version`, and the static budgets of ADR-020 and ADR-036.
//! The app and `stage-verify` call the same code, and an invalid stage is not playable (fail
//! closed).
//!
//! Errors name where the problem is (`$.boss.phases[1].timeline[3]`) and what is wrong. They
//! never repeat text from the file, so a hostile stage cannot put its own words in front of a
//! player.
//!
//! Budgets here are tuning values; their existence is the decision. Dynamic budgets (bullets
//! alive, enemies alive) are the simulation's runtime caps, and so is any count that an
//! expression computes: the validator checks constants, the engine clamps the rest.

use std::fmt;

use serde::Deserialize;

use crate::attack::{
    AttackPhase, AttackRef, DamageType, Direction, Emitter, Expr, HitBox, PhaseKind, Profile,
};
use crate::behaviour::{
    Action, Agent, BossCall, Condition, Elite, Guardians, Movement, Rule, Selector, Skill,
};
use crate::id::ContentId;
use crate::presets;
use crate::registry::{self, Entry, Kind};
use crate::rewards::Rarity;
use crate::stage::{
    Boss, BossPart, Evade, ExpertisePoints, Levels, Phase, Player, Stage, Stars, Step, Tier,
    Transition, Wave,
};
use crate::trig;
use crate::version::{SCHEMA_VERSION, SIM_VERSION};
use crate::weapon::{Branches, Charge, Hold, SmashEffect, Weapon, WeaponRef};

/// Limits a stage must keep. The first block is ADR-020, the second ADR-036, the rest the
/// loadout and numeric ranges that ADR-012 and ADR-019 ask for.
pub mod limits {
    pub const MAX_STAGE_BYTES: usize = 256 * 1024;
    /// Five minutes at 60 ticks per second.
    pub const MAX_TICKS: u32 = 5 * 60 * 60;
    pub const MAX_WAVES: usize = 200;
    pub const MAX_COMPANIONS: usize = 4;
    pub const MAX_PHASES: usize = 8;
    pub const MAX_TITLE_CHARS: usize = 60;
    pub const MAX_DESCRIPTION_CHARS: usize = 500;
    /// Stage IDs are slugs (ADR-010).
    pub const MAX_STAGE_ID_CHARS: usize = 40;

    pub const MAX_RULES: usize = 8;
    pub const MAX_TIMELINE_STEPS: usize = 64;
    pub const MAX_PATTERN_DEPTH: u32 = 4;
    pub const MAX_PATTERN_NODES: u32 = 64;
    pub const MAX_EXPR_NODES: u32 = 16;
    pub const MAX_REPEAT: i32 = 100;
    /// Phases in one attack, and the ticks one lasts: 10 seconds (ADR-055).
    pub const MAX_ATTACK_PHASES: usize = 8;
    pub const MAX_PHASE_TICKS: u32 = 600;

    /// An evade's dash: at most 256 pixels, over at most a second, and at most a second of
    /// invulnerability (ADR-050).
    pub const MAX_EVADE_DISTANCE: i32 = 65_536;
    pub const MAX_EVADE_TICKS: u32 = 60;
    /// An evade holds at most 3 charges (ADR-074).
    pub const MAX_EVADE_CHARGES: u8 = 3;
    /// A charge (core#83): at most 8 levels over at most 10 seconds of game time, and a perfect
    /// bonus of at most +1000%.
    pub const MAX_CHARGE_LEVELS: usize = 8;
    pub const MAX_CHARGE_TICKS: u32 = 600;
    /// A held step's hold (ADR-084 point 6): at most 10 seconds, as a charge.
    pub const MAX_HOLD_TICKS: u32 = 600;
    /// A Main weapon's combos (ADR-070, ADR-084): at most 32 steps in its tree and dash branch
    /// together, no path from the root deeper than six steps (six attacks, AAAAAB at most); a
    /// window of at most 2 seconds and a dash-strike of at most 1 second.
    pub const MAX_COMBO_STEPS: usize = 32;
    pub const MAX_COMBO_DEPTH: u32 = 6;
    pub const MAX_COMBO_WINDOW_TICKS: u32 = 120;
    pub const MAX_DASH_STRIKE_TICKS: u32 = 60;
    /// Aim assist looks at most 512 pixels away (ADR-072).
    pub const MAX_ASSIST_RANGE: i32 = 131_072;
    /// The Sub weapon's pull (ADR-070): at most 64 pixels, over at most 1 second.
    pub const MAX_PULL_DISTANCE: i32 = 16_384;
    pub const MAX_PULL_TICKS: u32 = 60;
    /// A stun lasts at most 2 seconds (ADR-085 point 5, a placeholder bound).
    pub const MAX_STUN_TICKS: u32 = 120;
    /// Twenty seconds: a bullet at 2 pixels a tick crosses a 4,096-pixel map in 2,048 ticks,
    /// so a lifetime is a shorter one than the margin's (core#147).
    pub const MAX_BULLET_TICKS: u32 = 1_200;
    pub const MAX_PERFECT_PCT: u16 = 1000;
    pub const MAX_BOSS_PARTS: usize = 8;
    pub const MAX_TRANSITIONS: usize = 4;
    pub const MAX_PRESET_ARGS: usize = 8;
    pub const MAX_WAVE_COUNT: u16 = 100;
    /// Entries in a stage's `levels.xp`, and the experience in each (ADR-051).
    pub const MAX_LEVELS: usize = 100;
    pub const MAX_LEVEL_XP: u32 = 1_000_000;
    /// The experience an enemy gives.
    pub const MAX_ENEMY_XP: u32 = 1000;
    /// The expertise points an enemy or the boss gives for a hit and for a kill (ADR-078 §7).
    pub const MAX_EXPERTISE_PER_HIT: u32 = 1000;
    pub const MAX_EXPERTISE_PER_KILL: u32 = 100_000;
    /// Stardust (ADR-109, ADR-020), held times 10: the base an enemy or the boss earns for a kill
    /// (the bundled boss's is 2,000), the percent of the time factor (as `difficulties::MAX_PCT`),
    /// and the run's total, at which the counter saturates. At the largest base and percent a
    /// kill earns 1,000,000, so the total is reached in ten kills and never wraps a `u32`.
    pub const MAX_STARDUST_BASE: u32 = 100_000;
    pub const MAX_STARDUST_PCT: u32 = 1_000;
    pub const MAX_RUN_STARDUST: u32 = 10_000_000;
    /// Falling stars (ADR-079 §2, ADR-068): the drop chance is in 1000; the pity, the stars on
    /// the map, the pickup circle (64 pixels), the rerolls and banishes, and the drop table's
    /// tiers, each starting at most at the highest level, with weights of at most 10,000.
    pub const MAX_DROP_PER_MILLE: u16 = 1000;
    pub const MAX_PITY: u16 = 1000;
    pub const MAX_STARS_ON_MAP: u16 = 64;
    pub const MAX_PICKUP_RADIUS: i32 = 16_384;
    pub const MAX_REROLLS: u8 = 20;
    pub const MAX_BANISHES: u8 = 20;
    pub const MAX_TIERS: usize = 16;
    pub const MAX_TIER_LEVEL: u32 = 100;
    pub const MAX_RARITY_WEIGHT: u16 = 10_000;
    pub const MAX_SUMMON_COUNT: u8 = 8;
    /// Elites alive at once (ADR-068), among the enemies.
    pub const MAX_ELITES_ALIVE: u16 = 32;
    /// The most guardians one shrine rings itself with, or escorts one pack leader brings
    /// (ADR-090).
    pub const MAX_SHRINE_GUARDIANS: u8 = 16;
    /// The most a pack leader's aura adds to the damage of the enemies near it, in percent
    /// (ADR-090).
    pub const MAX_AURA_PERCENT: u16 = 200;

    /// What every default damage and hit-point value is multiplied by (ADR-103 decision 6), so
    /// that a percentage of a hit (an aura's +25%) is a whole number. The bounds below, `K` of
    /// ADR-056, the aggro constants and the bundled content all hold the scaled values.
    pub const DAMAGE_SCALE: u32 = 100;
    pub const MAX_PLAYER_HP: u32 = 99 * DAMAGE_SCALE;
    pub const MAX_PLAYER_ATK: u32 = 999 * DAMAGE_SCALE;
    pub const MAX_AGENT_HP: u32 = 100_000 * DAMAGE_SCALE;
    pub const MAX_BOSS_HP: u32 = 1_000_000 * DAMAGE_SCALE;
    /// The largest base damage. With the atk-up, the type bonus and the vulnerability at their
    /// caps and the match-up at its cap, a hit of it is `MAX_DAMAGE × 11 × 11 × 11 × 2`: below
    /// `u32::MAX` (ADR-056 decision 5), as `rules_v0::entity`'s tests check.
    pub const MAX_DAMAGE: u32 = 10_000 * DAMAGE_SCALE;
    /// At this defence a hit keeps about 1% of its damage (ADR-056, K = 100 × `DAMAGE_SCALE`).
    pub const MAX_DEFENCE: u32 = 10_000 * DAMAGE_SCALE;
    pub const MAX_BUFF_PCT: u16 = 500;
    pub const MAX_AGGRO: u32 = 1_000_000 * DAMAGE_SCALE;
    /// A number in an expression, an argument or a velocity-like field.
    pub const MAX_NUMBER: i32 = 1_000_000;
    /// A length in 1/256 pixels: 1,024 pixels.
    pub const MAX_LEN: i32 = 262_144;
    /// A hurt circle radius in 1/256 pixels: 256 pixels.
    pub const MAX_RADIUS: i32 = 65_536;
    /// The portrait field in 1/256 pixels (360 by 640 pixels), the map of a stage without one,
    /// and how far outside a map a point may be.
    pub const FIELD_W: i32 = 92_160;
    pub const FIELD_H: i32 = 163_840;
    pub const FIELD_MARGIN: i32 = 16_384;
    /// A stage's map: each side from 1 to 4,096 pixels (core#87).
    pub const MIN_MAP_SIDE: i32 = 256;
    pub const MAX_MAP_SIDE: i32 = 1_048_576;

    /// Problems reported before the rest are left out.
    pub const MAX_ISSUES: usize = 32;
}

use limits::*;

/// One problem and where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// For example `$.boss.phases[1].timeline[3]`; `$` is the file.
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Over `MAX_STAGE_BYTES`; the file is not parsed.
    TooLarge {
        size: usize,
        limit: usize,
    },
    /// Not JSON, or not the stage format (unknown or missing fields, wrong types).
    Parse {
        line: usize,
        column: usize,
    },
    /// Written by a newer game: listed greyed out as "update the app" (ADR-017).
    UnsupportedVersion {
        schema_version: u32,
        sim_version: u32,
    },
    Invalid(Vec<Issue>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::TooLarge { size, limit } => {
                write!(f, "the stage file is {size} bytes; the limit is {limit}")
            }
            Error::Parse { line, column } => {
                write!(f, "not a valid stage file (line {line}, column {column})")
            }
            Error::UnsupportedVersion {
                schema_version,
                sim_version,
            } => write!(
                f,
                "needs a newer version of the game (schema_version {schema_version}, \
                 sim_version {sim_version})"
            ),
            Error::Invalid(issues) => {
                for (i, issue) in issues.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{}: {}", issue.path, issue.message)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Error {}

/// Checks the size, parses strictly and validates: the one call for a stage file's bytes.
pub fn parse_and_validate(bytes: &[u8]) -> Result<Stage, Error> {
    if bytes.len() > MAX_STAGE_BYTES {
        return Err(Error::TooLarge {
            size: bytes.len(),
            limit: MAX_STAGE_BYTES,
        });
    }
    // Versions first: a file from a newer game may have fields this one rejects.
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
    check_versions(versions.schema_version, versions.sim_version)?;
    let stage: Stage = serde_json::from_slice(bytes).map_err(parse_error)?;
    validate(&stage)?;
    Ok(stage)
}

/// Checks the phases of one attack against the same budgets as a stage's inline attacks, for an
/// attack that takes `arity` arguments (the developer's presets, ADR-036). The problems are
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

/// Checks an attack outside a stage, a preset by ID with its arguments or an inline one, against
/// the same budgets as a stage's: a damage-area stage skill's (ADR-075). The problems are
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
    };
    ctx.weapon(preset);
    if ctx.issues.is_empty() {
        Ok(())
    } else {
        Err(ctx.issues)
    }
}

/// Whether the phase after an attack's last `active` one is a recovery with no spawn: where a
/// held step's hold begins (ADR-084 point 6).
fn holds_after_active(phases: &[AttackPhase]) -> bool {
    phases
        .iter()
        .rposition(|p| p.kind == PhaseKind::Active)
        .and_then(|last| phases.get(last + 1))
        .is_some_and(|p| p.kind == PhaseKind::Recovery && p.spawn.is_empty())
}

fn check_versions(schema_version: u32, sim_version: u32) -> Result<(), Error> {
    if schema_version == SCHEMA_VERSION && sim_version == SIM_VERSION {
        Ok(())
    } else {
        Err(Error::UnsupportedVersion {
            schema_version,
            sim_version,
        })
    }
}

/// Validates a parsed stage against the registry.
pub fn validate(stage: &Stage) -> Result<(), Error> {
    validate_with(stage, registry::ENTRIES)
}

/// As `validate`, against a given registry (tests use one with newer and deprecated IDs).
pub fn validate_with(stage: &Stage, registry: &[Entry]) -> Result<(), Error> {
    check_versions(stage.schema_version, stage.sim_version)?;
    let mut ctx = Ctx {
        registry,
        sim_version: stage.sim_version,
        map: stage.map(),
        parts: stage
            .boss
            .iter()
            .flat_map(|b| b.parts.iter().map(|p| p.id.0.as_str()))
            .collect(),
        path: Vec::new(),
        issues: Vec::new(),
        left_out: false,
        arity: 0,
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

enum Seg {
    Key(&'static str),
    Index(usize),
}

struct Ctx<'a> {
    registry: &'a [Entry],
    sim_version: u32,
    /// Points are checked against it.
    map: crate::stage::Map,
    /// Boss part IDs, which conditions and selectors may name.
    parts: Vec<&'a str>,
    path: Vec<Seg>,
    issues: Vec<Issue>,
    left_out: bool,
    /// How many arguments the attack being checked takes: 0 for an inline attack in a stage.
    arity: u8,
}

impl<'a> Ctx<'a> {
    fn fail(&mut self, message: impl Into<String>) {
        if self.issues.len() >= MAX_ISSUES {
            self.left_out = true;
            return;
        }
        let mut path = String::from("$");
        for seg in &self.path {
            match seg {
                Seg::Key(k) => {
                    path.push('.');
                    path.push_str(k);
                }
                Seg::Index(i) => path.push_str(&format!("[{i}]")),
            }
        }
        self.issues.push(Issue {
            path,
            message: message.into(),
        });
    }

    fn key<T>(&mut self, key: &'static str, f: impl FnOnce(&mut Self) -> T) -> T {
        self.path.push(Seg::Key(key));
        let out = f(self);
        self.path.pop();
        out
    }

    fn idx<T>(&mut self, index: usize, f: impl FnOnce(&mut Self) -> T) -> T {
        self.path.push(Seg::Index(index));
        let out = f(self);
        self.path.pop();
        out
    }

    /// `value` must be in `lo..=hi`.
    fn within(&mut self, key: &'static str, value: i64, lo: i64, hi: i64) {
        if value < lo || value > hi {
            self.key(key, |c| {
                c.fail(format!("must be between {lo} and {hi} (found {value})"))
            });
        }
    }

    /// A list must hold at most `max` items; returns whether its items should be checked.
    fn at_most(&mut self, key: &'static str, len: usize, max: usize, what: &str) -> bool {
        if len > max {
            self.key(key, |c| {
                c.fail(format!("at most {max} {what} (found {len})"))
            });
            return false;
        }
        true
    }

    /// As `at_most`, for the list at the current path.
    fn at_most_here(&mut self, len: usize, max: usize, what: &str) -> bool {
        if len > max {
            self.fail(format!("at most {max} {what} (found {len})"));
            return false;
        }
        true
    }

    /// A content ID of `kind` that exists and is available to this stage's `sim_version`.
    fn content_id(&mut self, key: &'static str, id: &ContentId, kind: Kind) -> Option<Entry> {
        self.key(key, |c| {
            if !registry::is_well_formed(&id.0) {
                c.fail("not a valid content ID (lowercase snake_case, at most 32 characters)");
                return None;
            }
            match registry::find(c.registry, kind, &id.0) {
                None => {
                    c.fail(format!("no such {kind:?} ID in the registry"));
                    None
                }
                Some(e) if e.introduced > c.sim_version => {
                    c.fail(format!(
                        "introduced in sim_version {}, newer than this stage's {}",
                        e.introduced, c.sim_version
                    ));
                    None
                }
                Some(e) => Some(*e),
            }
        })
    }

    fn point(&mut self, key: &'static str, p: &crate::fixed::Point) {
        self.key(key, |c| {
            c.within(
                "x",
                i64::from(p.x.0),
                -i64::from(FIELD_MARGIN),
                i64::from(c.map.width.0) + i64::from(FIELD_MARGIN),
            );
            c.within(
                "y",
                i64::from(p.y.0),
                -i64::from(FIELD_MARGIN),
                i64::from(c.map.height.0) + i64::from(FIELD_MARGIN),
            );
        });
    }

    /// Each side bounded (core#87), and the start on the map itself, not in its margin.
    fn map(&mut self, m: &crate::stage::Map) {
        let side = |c: &mut Self, key, v: crate::fixed::Fx| {
            c.within(
                key,
                i64::from(v.0),
                i64::from(MIN_MAP_SIDE),
                i64::from(MAX_MAP_SIDE),
            );
        };
        side(self, "width", m.width);
        side(self, "height", m.height);
        if let Some(start) = m.start {
            self.key("start", |c| {
                c.within("x", i64::from(start.x.0), 0, i64::from(m.width.0));
                c.within("y", i64::from(start.y.0), 0, i64::from(m.height.0));
            });
        }
        if let Some(facing) = m.facing {
            self.within(
                "facing",
                i64::from(facing),
                0,
                i64::from(crate::trig::PER_TURN) - 1,
            );
        }
    }

    fn ticks(&mut self, key: &'static str, ticks: u32, lo: u32) {
        self.within(key, i64::from(ticks), i64::from(lo), i64::from(MAX_TICKS));
    }

    fn stage(&mut self, s: &Stage) {
        if let Some(map) = &s.map {
            self.key("map", |c| c.map(map));
        }
        self.key("id", |c| {
            let ok = (1..=MAX_STAGE_ID_CHARS).contains(&s.id.len())
                && s.id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !s.id.starts_with('-')
                && !s.id.ends_with('-');
            if !ok {
                c.fail(format!(
                    "a stage ID is 1 to {MAX_STAGE_ID_CHARS} characters of a-z, 0-9 and -, \
                     not starting or ending with -"
                ));
            }
        });
        let title = s.title.chars().count();
        if title == 0 {
            self.key("title", |c| c.fail("must not be empty"));
        } else {
            self.within("title", title as i64, 1, MAX_TITLE_CHARS as i64);
        }
        self.within(
            "description",
            s.description.chars().count() as i64,
            0,
            MAX_DESCRIPTION_CHARS as i64,
        );
        self.ticks("length_ticks", s.length_ticks, 1);
        self.key("player", |c| c.player(&s.player));
        if self.at_most(
            "companions",
            s.companions.len(),
            MAX_COMPANIONS,
            "companions",
        ) {
            self.key("companions", |c| {
                for (i, a) in s.companions.iter().enumerate() {
                    c.idx(i, |c| {
                        c.agent(a, true);
                        if a.elite.is_some() {
                            c.key("elite", |c| c.fail("a companion cannot be an elite"));
                        }
                        if a.profile.is_some() {
                            c.key("profile", |c| {
                                c.fail("a companion has no match-up profile (ADR-100)")
                            });
                        }
                    });
                }
            });
        }
        if self.at_most("waves", s.waves.len(), MAX_WAVES, "waves") {
            self.key("waves", |c| {
                for (i, w) in s.waves.iter().enumerate() {
                    c.idx(i, |c| c.wave(w, s.length_ticks, MAX_WAVE_COUNT));
                }
            });
        }
        if let Some(boss) = &s.boss {
            self.key("boss", |c| c.boss(boss));
        }
        if let Some(levels) = &s.levels {
            self.key("levels", |c| c.levels(levels));
        }
        if let Some(table) = s.level_table
            && let Some(problem) = table.problem()
        {
            self.key("level_table", |c| c.fail(problem));
        }
        if let Some(f) = &s.stardust {
            self.key("stardust", |c| {
                let max = i64::from(MAX_STARDUST_PCT);
                c.within("start_pct", i64::from(f.start_pct), 1, max);
                c.within("end_pct", i64::from(f.end_pct), 1, max);
                if (1..=max).contains(&i64::from(f.start_pct))
                    && (1..=max).contains(&i64::from(f.end_pct))
                    && f.end_pct < f.start_pct
                {
                    c.key("end_pct", |c| c.fail("not below the factor at the start"));
                }
            });
        }
        if let Some(cap) = s.elite_cap {
            self.within("elite_cap", i64::from(cap), 1, i64::from(MAX_ELITES_ALIVE));
        }
        if let Some(stars) = &s.stars {
            self.key("stars", |c| {
                if s.levels.is_none() {
                    c.fail("needs `levels`: a star's rarity follows the player's level".to_owned());
                }
                c.stars(stars);
            });
        }
    }

    fn stars(&mut self, st: &Stars) {
        self.within(
            "drop_per_mille",
            i64::from(st.drop_per_mille),
            0,
            i64::from(MAX_DROP_PER_MILLE),
        );
        self.within("pity", i64::from(st.pity), 1, i64::from(MAX_PITY));
        self.within("cap", i64::from(st.cap), 1, i64::from(MAX_STARS_ON_MAP));
        self.within(
            "pickup_radius",
            i64::from(st.pickup_radius.0),
            1,
            i64::from(MAX_PICKUP_RADIUS),
        );
        self.within("rerolls", i64::from(st.rerolls), 0, i64::from(MAX_REROLLS));
        self.within(
            "banishes",
            i64::from(st.banishes),
            0,
            i64::from(MAX_BANISHES),
        );
        if st.tiers.is_empty() {
            self.key("tiers", |c| c.fail("needs at least one tier".to_owned()));
        } else if self.at_most("tiers", st.tiers.len(), MAX_TIERS, "tiers") {
            self.key("tiers", |c| {
                let mut previous = 0;
                for (i, tier) in st.tiers.iter().enumerate() {
                    c.idx(i, |c| c.tier(tier, i, previous));
                    previous = tier.from_level;
                }
            });
        }
    }

    /// Tier `index` of a drop table, after one that starts at level `previous`.
    fn tier(&mut self, tier: &Tier, index: usize, previous: u32) {
        let level = tier.from_level;
        if index == 0 && level != 1 {
            self.key("from_level", |c| {
                c.fail(format!("the first tier starts at level 1 (found {level})"))
            });
        } else if index > 0 && !(previous + 1..=MAX_TIER_LEVEL).contains(&level) {
            self.key("from_level", |c| {
                c.fail(format!(
                    "must be above the tier before ({previous}) and at most {MAX_TIER_LEVEL}                      (found {level})"
                ))
            });
        }
        self.key("weights", |c| {
            for rarity in Rarity::ALL {
                let weight = tier.weights.get(rarity);
                if weight > MAX_RARITY_WEIGHT {
                    c.fail(format!(
                        "{rarity:?} weighs at most {MAX_RARITY_WEIGHT} (found {weight})"
                    ));
                }
            }
            if tier.weights.total() == 0 {
                c.fail("at least one rarity must weigh more than 0".to_owned());
            }
        });
    }

    fn levels(&mut self, l: &Levels) {
        if l.xp.is_empty() {
            self.key("xp", |c| c.fail("needs at least one level".to_owned()));
        } else if self.at_most("xp", l.xp.len(), MAX_LEVELS, "levels") {
            self.key("xp", |c| {
                for (i, xp) in l.xp.iter().enumerate() {
                    if !(1..=MAX_LEVEL_XP).contains(xp) {
                        c.idx(i, |c| {
                            c.fail(format!("must be between 1 and {MAX_LEVEL_XP} (found {xp})"))
                        });
                    }
                }
            });
        }
    }

    fn player(&mut self, p: &Player) {
        self.content_id("base", &p.base, Kind::Asset);
        self.within("hp", i64::from(p.hp), 1, i64::from(MAX_PLAYER_HP));
        self.within("atk", i64::from(p.atk), 1, i64::from(MAX_PLAYER_ATK));
        self.within("defence", i64::from(p.defence), 0, i64::from(MAX_DEFENCE));
        self.key("weapon", |c| c.weapon_ref(&p.weapon, Self::weapon));
        self.key("evade", |c| c.evade(&p.evade));
    }

    /// A weapon preset's ID, or an inline weapon checked by `inline`.
    fn weapon_ref<W>(&mut self, w: &WeaponRef<W>, inline: impl FnOnce(&mut Self, &W)) {
        match w {
            WeaponRef::Preset { id } => self.key("preset", |c| {
                c.content_id("id", id, Kind::Weapon);
            }),
            WeaponRef::Inline(w) => self.key("inline", |c| inline(c, w)),
        }
    }

    /// The weapon's combo tree (ADR-084): at most `MAX_COMBO_STEPS` steps in all, none deeper
    /// than `MAX_COMBO_DEPTH`, each attack and charge checked, and its smash effect.
    fn weapon(&mut self, w: &Weapon) {
        let mut steps = 0;
        self.key("combo", |c| c.branches(&w.combo, 1, &mut steps));
        self.key("dash", |c| c.branches(&w.dash, 1, &mut steps));
        if steps > MAX_COMBO_STEPS {
            self.key("combo", |c| {
                c.fail(format!(
                    "at most {MAX_COMBO_STEPS} steps in a weapon's combos"
                ))
            });
        }
        self.within(
            "window_ticks",
            i64::from(w.window_ticks),
            0,
            i64::from(MAX_COMBO_WINDOW_TICKS),
        );
        self.within(
            "dash_strike_ticks",
            i64::from(w.dash_strike_ticks),
            0,
            i64::from(MAX_DASH_STRIKE_TICKS),
        );
        self.within(
            "assist_range",
            i64::from(w.assist_range.0),
            0,
            i64::from(MAX_ASSIST_RANGE),
        );
        if let Some(effect) = &w.smash {
            self.key("smash", |c| c.smash(effect));
        }
    }

    fn branches(&mut self, b: &Branches, depth: u32, steps: &mut usize) {
        for (key, step) in [("main", &b.main), ("smash", &b.smash)] {
            let Some(step) = step else {
                continue;
            };
            *steps += 1;
            self.key(key, |c| {
                if depth > MAX_COMBO_DEPTH {
                    c.fail(format!("a combo is at most {MAX_COMBO_DEPTH} steps deep"));
                    return;
                }
                c.key("attack", |c| c.attack_ref(&step.attack));
                if let Some(unlock) = &step.unlock {
                    c.content_id("unlock", unlock, Kind::Upgrade);
                }
                if let Some(charge) = &step.charge {
                    c.key("charge", |c| c.charge(charge));
                }
                if let Some(hold) = &step.hold {
                    if step.charge.is_some() {
                        c.key("hold", |c| {
                            c.fail("a step has a charge or a hold, never both")
                        });
                    }
                    c.key("hold", |c| c.hold(hold));
                    if presets::phases(&step.attack).is_some_and(|p| !holds_after_active(&p)) {
                        c.key("attack", |c| {
                            c.fail(
                                "a held step's attack has a recovery with no spawn after its last active phase, where the hold begins",
                            )
                        });
                    }
                }
                c.key("next", |c| c.branches(&step.next, depth + 1, steps));
            });
        }
    }

    /// A held step's hold (ADR-084 point 6): the ticks held and the movement are bounded, the
    /// loop's phases are all `active`, and both attacks are checked as any attack's are.
    fn hold(&mut self, h: &Hold) {
        self.within(
            "max_ticks",
            i64::from(h.max_ticks),
            1,
            i64::from(MAX_HOLD_TICKS),
        );
        self.within("move_pct", i64::from(h.move_pct), 0, 100);
        self.key("loop", |c| {
            c.attack_ref(&h.loop_attack);
            if presets::phases(&h.loop_attack)
                .is_some_and(|p| p.iter().any(|q| q.kind != PhaseKind::Active))
            {
                c.fail("a loop's phases are all active");
            }
        });
        self.key("release", |c| c.attack_ref(&h.release));
    }

    fn smash(&mut self, effect: &SmashEffect) {
        match *effect {
            SmashEffect::Pull { distance, ticks } | SmashEffect::Knockback { distance, ticks } => {
                self.within(
                    "distance",
                    i64::from(distance.0),
                    0,
                    i64::from(MAX_PULL_DISTANCE),
                );
                self.within("ticks", i64::from(ticks), 1, i64::from(MAX_PULL_TICKS));
            }
            // No numbers of its own: the rune's come from the runes file, bounded there.
            SmashEffect::Bleed {} => {}
            SmashEffect::Stun { ticks } => {
                self.within("ticks", i64::from(ticks), 1, i64::from(MAX_STUN_TICKS));
            }
        }
    }

    /// A charged step (ADR-070). Level 0 is the step's own attack; the levels here are 1 and
    /// up, after 0, rising strictly and all before `max_ticks`; a perfect window ends by the next
    /// level (or `max_ticks`).
    fn charge(&mut self, c: &Charge) {
        self.within(
            "max_ticks",
            i64::from(c.max_ticks),
            1,
            i64::from(MAX_CHARGE_TICKS),
        );
        self.within("move_pct", i64::from(c.move_pct), 0, 100);
        if c.levels.is_empty() {
            self.key("levels", |c| c.fail("a charge has at least one level"));
            return;
        }
        if !self.at_most("levels", c.levels.len(), MAX_CHARGE_LEVELS, "levels") {
            return;
        }
        self.key("levels", |ctx| {
            for (i, l) in c.levels.iter().enumerate() {
                let next = c.levels.get(i + 1).map_or(c.max_ticks, |n| n.at_ticks);
                ctx.idx(i, |ctx| {
                    if i == 0 && l.at_ticks == 0 {
                        ctx.key("at_ticks", |c| {
                            c.fail("level 1 comes after 0 (level 0 is the step's own attack)")
                        });
                    }
                    if i > 0 && l.at_ticks <= c.levels[i - 1].at_ticks {
                        ctx.key("at_ticks", |c| c.fail("levels rise strictly"));
                    }
                    if l.at_ticks >= c.max_ticks {
                        ctx.key("at_ticks", |c| c.fail("a level comes before max_ticks"));
                    }
                    if u64::from(l.at_ticks) + u64::from(l.perfect_ticks) > u64::from(next) {
                        ctx.key("perfect_ticks", |c| {
                            c.fail("a perfect window ends by the next level, or max_ticks")
                        });
                    }
                    ctx.within(
                        "perfect_pct",
                        i64::from(l.perfect_pct),
                        0,
                        i64::from(MAX_PERFECT_PCT),
                    );
                    ctx.key("attack", |c| c.attack_ref(&l.attack));
                    if let Some(a) = &l.perfect_attack {
                        ctx.key("perfect_attack", |c| c.attack_ref(a));
                    }
                });
            }
        });
    }

    fn evade(&mut self, e: &Evade) {
        self.within(
            "distance",
            i64::from(e.distance.0),
            0,
            i64::from(MAX_EVADE_DISTANCE),
        );
        self.within("ticks", i64::from(e.ticks), 1, i64::from(MAX_EVADE_TICKS));
        self.within(
            "invulnerable_ticks",
            i64::from(e.invulnerable_ticks),
            0,
            i64::from(MAX_EVADE_TICKS),
        );
        self.ticks("cooldown_ticks", e.cooldown_ticks, 0);
        let most = i64::from(MAX_EVADE_CHARGES);
        self.within("charges", i64::from(e.charges), 1, most);
        if let Some(max) = e.max_charges {
            self.within(
                "max_charges",
                i64::from(max),
                i64::from(e.charges).max(1),
                most,
            );
        }
    }

    fn charges(&mut self, charges: Option<u16>) {
        if let Some(n) = charges {
            self.within("charges", i64::from(n), 1, i64::from(u16::MAX));
        }
    }

    fn agent(&mut self, a: &Agent, summon_ok: bool) {
        self.content_id("base", &a.base, Kind::Asset);
        if let Some(p) = &a.palette {
            self.content_id("palette", p, Kind::Palette);
        }
        self.within("hp", i64::from(a.hp), 1, i64::from(MAX_AGENT_HP));
        self.within("defence", i64::from(a.defence), 0, i64::from(MAX_DEFENCE));
        if let Some(p) = &a.profile {
            self.key("profile", |c| c.profile(p));
        }
        self.within("radius", i64::from(a.radius.0), 1, i64::from(MAX_RADIUS));
        self.key("movement", |c| {
            c.movement(&a.movement);
            match (&a.movement, &a.elite) {
                (Movement::FleeWhenDisturbed { .. }, None) => {
                    c.fail("flee_when_disturbed needs an elite")
                }
                (Movement::Shrine { .. }, None) => c.fail("shrine needs an elite"),
                (
                    Movement::Shrine { .. } | Movement::FleeWhenDisturbed { .. },
                    Some(Elite { mimic: Some(_), .. }),
                ) => c.fail("a mimic is neither a shrine nor a goblin"),
                _ => {}
            }
        });
        self.rules("rules", &a.rules, summon_ok);
        if let Some(elite) = &a.elite {
            self.key("elite", |c| {
                c.within("range", i64::from(elite.range.0), 1, i64::from(MAX_LEN));
                if matches!(a.movement, Movement::Shrine { .. })
                    && elite.boss_call != BossCall::Stay
                {
                    c.key("boss_call", |c| c.fail("a shrine's boss_call is stay"));
                } else if elite.mimic.is_some() && elite.boss_call != BossCall::Join {
                    c.key("boss_call", |c| c.fail("a mimic's boss_call is join"));
                }
                if let Some(mimic) = &elite.mimic {
                    c.key("mimic", |c| {
                        c.content_id("disguise", &mimic.disguise, Kind::Asset);
                        c.within(
                            "reveal_radius",
                            i64::from(mimic.reveal_radius.0),
                            1,
                            i64::from(MAX_LEN),
                        );
                    });
                }
                if let Some(aura) = &elite.aura {
                    if elite.mimic.is_some() {
                        c.key("aura", |c| c.fail("a mimic has no aura"));
                    }
                    c.key("aura", |c| {
                        c.within("radius", i64::from(aura.radius.0), 1, i64::from(MAX_LEN));
                        c.within(
                            "damage_percent",
                            i64::from(aura.damage_percent),
                            1,
                            i64::from(MAX_AURA_PERCENT),
                        );
                    });
                }
                if let Some(escort) = &elite.escort {
                    if elite.mimic.is_some() {
                        c.key("escort", |c| c.fail("a mimic has no escort"));
                    }
                    c.key("escort", |c| c.guardians(escort));
                }
            });
        }
    }

    /// A shrine's guardians or a pack leader's escort: a bounded count and ring of ordinary
    /// enemies.
    fn guardians(&mut self, g: &Guardians) {
        self.within(
            "count",
            i64::from(g.count),
            1,
            i64::from(MAX_SHRINE_GUARDIANS),
        );
        self.within("radius", i64::from(g.radius.0), 1, i64::from(MAX_LEN));
        self.key("enemy", |c| {
            c.agent(&g.enemy, false);
            if g.enemy.elite.is_some() {
                c.key("elite", |c| c.fail("a guardian is an ordinary enemy"));
            }
        });
    }

    fn movement(&mut self, m: &Movement) {
        match m {
            Movement::Orbit { radius } => {
                self.within("radius", i64::from(radius.0), 1, i64::from(MAX_LEN))
            }
            Movement::Follow { distance, speed } => {
                self.within("distance", i64::from(distance.0), 0, i64::from(MAX_LEN));
                if let Some(speed) = speed {
                    self.within("speed", i64::from(speed.0), 1, i64::from(MAX_LEN));
                }
            }
            Movement::FleeWhenDisturbed {
                disturb_radius,
                speed,
                leave_after_ticks,
            } => {
                let max = i64::from(MAX_LEN);
                self.within("disturb_radius", i64::from(disturb_radius.0), 1, max);
                self.within("speed", i64::from(speed.0), 1, max);
                self.ticks("leave_after_ticks", *leave_after_ticks, 1);
            }
            Movement::Shrine {
                fight_ticks,
                guardians,
            } => {
                self.ticks("fight_ticks", *fight_ticks, 1);
                self.key("guardians", |c| c.guardians(guardians));
            }
            Movement::Hold { at } => self.point("at", at),
            Movement::Straight { vx, vy } => {
                let max = i64::from(MAX_LEN);
                self.within("vx", i64::from(vx.0), -max, max);
                self.within("vy", i64::from(vy.0), -max, max);
            }
        }
    }

    fn rules(&mut self, key: &'static str, rules: &[Rule], summon_ok: bool) {
        if self.at_most(key, rules.len(), MAX_RULES, "rules") {
            self.key(key, |c| {
                for (i, r) in rules.iter().enumerate() {
                    c.idx(i, |c| c.rule(r, summon_ok));
                }
            });
        }
    }

    fn rule(&mut self, r: &Rule, summon_ok: bool) {
        self.key("when", |c| c.condition(&r.when));
        self.key("target", |c| c.selector(&r.target));
        self.key("do", |c| c.action(&r.action, summon_ok));
        self.ticks("cooldown_ticks", r.cooldown_ticks, 0);
        self.charges(r.charges);
    }

    fn condition(&mut self, cond: &Condition) {
        match cond {
            Condition::Always => {}
            Condition::HpBelow { who, pct } => {
                self.key("who", |c| c.selector(who));
                self.within("pct", i64::from(*pct), 0, 100);
            }
            Condition::HasStatus { who, status } => {
                self.key("who", |c| c.selector(who));
                self.content_id("status", status, Kind::Status);
            }
            Condition::AggroAbove { who, value } => {
                self.key("who", |c| c.selector(who));
                self.within("value", i64::from(*value), 0, i64::from(MAX_AGGRO));
            }
            Condition::PartBroken { part } => self.part("part", part),
            Condition::TimeAbove { ticks } => self.ticks("ticks", *ticks, 0),
            Condition::NearTarget { distance } => {
                self.within("distance", i64::from(distance.0), 1, i64::from(MAX_LEN))
            }
        }
    }

    fn selector(&mut self, s: &Selector) {
        if let Selector::Part(id) = s {
            self.part("part", id);
        }
    }

    /// A boss part the stage defines.
    fn part(&mut self, key: &'static str, id: &ContentId) {
        if !self.parts.contains(&id.0.as_str()) {
            self.key(key, |c| c.fail("the boss has no part with this ID"));
        }
    }

    fn action(&mut self, a: &Action, summon_ok: bool) {
        match a {
            Action::Cast { skill } => self.key("skill", |c| c.skill(skill)),
            Action::Attack { attack, .. } => self.key("attack", |c| c.attack_ref(attack)),
            Action::Summon { agent, count } => {
                if !summon_ok {
                    self.fail("a summoned agent cannot summon");
                    return;
                }
                self.within("count", i64::from(*count), 1, i64::from(MAX_SUMMON_COUNT));
                self.key("agent", |c| c.agent(agent, false));
            }
            Action::MoveTo { to, ticks } => {
                self.point("to", to);
                self.ticks("ticks", *ticks, 1);
            }
            Action::Telegraph { ticks } => self.ticks("ticks", *ticks, 1),
        }
    }

    fn skill(&mut self, s: &Skill) {
        match s {
            Skill::Shot { pattern, damage } => {
                self.key("pattern", |c| c.attack_ref(pattern));
                self.within("damage", i64::from(*damage), 1, i64::from(MAX_DAMAGE));
            }
            Skill::AtkUp {
                value_pct,
                duration_ticks,
            }
            | Skill::Vulnerable {
                value_pct,
                duration_ticks,
            } => {
                self.within(
                    "value_pct",
                    i64::from(*value_pct),
                    1,
                    i64::from(MAX_BUFF_PCT),
                );
                self.ticks("duration_ticks", *duration_ticks, 1);
            }
            Skill::Slow {
                value_pct,
                duration_ticks,
            } => {
                self.within("value_pct", i64::from(*value_pct), 1, 100);
                self.ticks("duration_ticks", *duration_ticks, 1);
            }
            Skill::Heal { amount } => {
                self.within("amount", i64::from(*amount), 1, i64::from(MAX_DAMAGE))
            }
        }
    }

    fn wave(&mut self, w: &Wave, length_ticks: u32, max_count: u16) {
        if w.at_tick >= length_ticks {
            self.key("at_tick", |c| c.fail("must be before the stage ends"));
        }
        self.key("enemy", |c| c.agent(&w.enemy, true));
        self.point("spawn", &w.spawn);
        if let Some(around) = w.around {
            self.within("around", i64::from(around.0), 1, i64::from(MAX_LEN));
        }
        self.within("count", i64::from(w.count), 1, i64::from(max_count));
        self.ticks("every_ticks", w.every_ticks, u32::from(w.count > 1));
        if let Some(xp) = w.xp {
            self.within("xp", i64::from(xp), 0, i64::from(MAX_ENEMY_XP));
        }
        if let Some(points) = &w.expertise {
            self.expertise_points(points);
        }
        if let Some(base) = w.stardust {
            self.within("stardust", i64::from(base), 0, i64::from(MAX_STARDUST_BASE));
        }
    }

    fn expertise_points(&mut self, points: &ExpertisePoints) {
        self.key("expertise", |c| {
            c.within(
                "hit",
                i64::from(points.hit),
                0,
                i64::from(MAX_EXPERTISE_PER_HIT),
            );
            c.within(
                "kill",
                i64::from(points.kill),
                0,
                i64::from(MAX_EXPERTISE_PER_KILL),
            );
        });
    }

    fn boss(&mut self, b: &Boss) {
        self.content_id("base", &b.base, Kind::Asset);
        if let Some(p) = &b.palette {
            self.content_id("palette", p, Kind::Palette);
        }
        self.within("hp", i64::from(b.hp), 1, i64::from(MAX_BOSS_HP));
        if let Some(base) = b.stardust {
            self.within("stardust", i64::from(base), 0, i64::from(MAX_STARDUST_BASE));
        }
        self.within("defence", i64::from(b.defence), 0, i64::from(MAX_DEFENCE));
        if let Some(p) = &b.profile {
            self.key("profile", |c| c.profile(p));
        }
        self.within("radius", i64::from(b.radius.0), 1, i64::from(MAX_RADIUS));
        self.point("spawn", &b.spawn);
        if self.at_most("parts", b.parts.len(), MAX_BOSS_PARTS, "boss parts") {
            self.key("parts", |c| {
                for (i, p) in b.parts.iter().enumerate() {
                    let repeated = b.parts[..i].iter().any(|q| q.id == p.id);
                    c.idx(i, |c| c.boss_part(p, repeated));
                }
            });
        }
        if let Some(points) = &b.expertise {
            self.expertise_points(points);
        }
        if let Some(call) = &b.call_elites {
            self.key("call_elites", |c| {
                if call.after_ticks.is_none() && call.below_pct.is_none() {
                    c.fail("needs `after_ticks`, `below_pct` or both");
                }
                if let Some(t) = call.after_ticks {
                    c.ticks("after_ticks", t, 0);
                }
                if let Some(pct) = call.below_pct {
                    c.within("below_pct", i64::from(pct), 1, 100);
                }
            });
        }
        if b.phases.is_empty() {
            self.key("phases", |c| c.fail("the boss needs at least one phase"));
        } else if self.at_most("phases", b.phases.len(), MAX_PHASES, "phases") {
            self.key("phases", |c| {
                for (i, p) in b.phases.iter().enumerate() {
                    c.idx(i, |c| c.phase(p, i + 1 == b.phases.len()));
                }
            });
        }
    }

    fn boss_part(&mut self, p: &BossPart, repeated: bool) {
        self.key("id", |c| {
            if !registry::is_well_formed(&p.id.0) {
                c.fail("not a valid part ID (lowercase snake_case, at most 32 characters)");
            } else if repeated {
                c.fail("two parts have this ID");
            }
        });
        self.content_id("asset", &p.asset, Kind::Asset);
        self.within("hp", i64::from(p.hp), 1, i64::from(MAX_AGENT_HP));
        self.within("defence", i64::from(p.defence), 0, i64::from(MAX_DEFENCE));
        self.within("radius", i64::from(p.radius.0), 1, i64::from(MAX_RADIUS));
        self.key("offset", |c| {
            let max = i64::from(MAX_LEN);
            c.within("x", i64::from(p.offset.x.0), -max, max);
            c.within("y", i64::from(p.offset.y.0), -max, max);
        });
        if let Some(profile) = &p.profile {
            self.key("profile", |c| c.profile(profile));
        }
    }

    /// A match-up profile (ADR-100 point 7): each type at most once in each list, and never in
    /// both. The one blanket resist is the parser's: `resist` holds a single value.
    fn profile(&mut self, p: &Profile) {
        let types = DamageType::ALL.len();
        for (key, list) in [("weak", &p.weak), ("strong", &p.strong)] {
            if !self.at_most(key, list.len(), types, "types") {
                continue;
            }
            self.key(key, |c| {
                for (i, ty) in list.iter().enumerate() {
                    if list[..i].contains(ty) {
                        c.idx(i, |c| c.fail("this type is already in the list"));
                    } else if key == "strong" && p.weak.contains(ty) {
                        c.idx(i, |c| c.fail("this type is also in `weak`"));
                    }
                }
            });
        }
    }

    fn phase(&mut self, p: &Phase, last: bool) {
        match (&p.until, last) {
            (Some(cond), false) => self.key("until", |c| c.condition(cond)),
            (None, false) => self.key("until", |c| c.fail("every phase but the last needs one")),
            (Some(_), true) => self.key("until", |c| {
                c.fail("the last phase lasts until the boss dies")
            }),
            (None, true) => {}
        }
        if self.at_most(
            "transition",
            p.transition.len(),
            MAX_TRANSITIONS,
            "transitions",
        ) {
            self.key("transition", |c| {
                for (i, t) in p.transition.iter().enumerate() {
                    if let Transition::InvulnerableTicks { ticks } = t {
                        c.idx(i, |c| c.ticks("ticks", *ticks, 1));
                    }
                }
            });
        }
        if self.at_most(
            "timeline",
            p.timeline.len(),
            MAX_TIMELINE_STEPS,
            "timeline steps",
        ) {
            self.key("timeline", |c| {
                for (i, s) in p.timeline.iter().enumerate() {
                    c.idx(i, |c| c.step(s, i + 1 == p.timeline.len()));
                }
            });
        }
        self.rules("rules", &p.rules, true);
    }

    fn step(&mut self, s: &Step, last: bool) {
        match s {
            Step::MoveTo { to, ticks } => {
                self.point("to", to);
                self.ticks("ticks", *ticks, 1);
            }
            Step::Wait { ticks } | Step::Telegraph { ticks } => self.ticks("ticks", *ticks, 1),
            Step::Attack { attack, big } => {
                self.key("attack", |c| c.attack_ref(attack));
                if *big && !matches!(attack, AttackRef::Preset { .. }) {
                    self.key("big", |c| {
                        c.fail("only a preset can be big: its ID names it")
                    });
                }
            }
            Step::Loop => {
                if !last {
                    self.fail("`loop` must be the last step; later steps would never run");
                }
            }
        }
    }

    fn attack_ref(&mut self, a: &AttackRef) {
        match a {
            AttackRef::Preset { id, args } => {
                self.key("preset", |c| {
                    let entry = c.content_id("id", id, Kind::Preset);
                    if !c.at_most("args", args.len(), MAX_PRESET_ARGS, "arguments") {
                        return;
                    }
                    if let Some(e) = entry {
                        if usize::from(e.args) != args.len() {
                            c.key("args", |c| {
                                c.fail(format!(
                                    "this preset takes {} arguments (found {})",
                                    e.args,
                                    args.len()
                                ))
                            });
                        }
                    }
                    c.key("args", |c| {
                        for (i, v) in args.iter().enumerate() {
                            c.idx(i, |c| c.number(*v));
                        }
                    });
                });
            }
            AttackRef::Inline(phases) => self.key("inline", |c| c.phases(phases)),
        }
    }

    /// An attack's phases (ADR-055). The nodes they spawn count against one pattern budget.
    fn phases(&mut self, phases: &[AttackPhase]) {
        if phases.is_empty() {
            self.fail("an attack has at least one phase");
            return;
        }
        if !self.at_most_here(phases.len(), MAX_ATTACK_PHASES, "phases") {
            return;
        }
        let mut count = 0;
        for (i, p) in phases.iter().enumerate() {
            let going = self.idx(i, |c| {
                c.attack_phase(p);
                c.key("spawn", |c| c.emitters(&p.spawn, 1, &mut count))
            });
            if !going {
                return;
            }
        }
    }

    /// One phase: its ticks, movement, hit box, damage and re-hits are bounded. A re-hit needs
    /// an active phase with a hit box, since nothing else hits.
    fn attack_phase(&mut self, p: &AttackPhase) {
        self.within("ticks", i64::from(p.ticks), 1, i64::from(MAX_PHASE_TICKS));
        self.within("move_pct", i64::from(p.move_pct), 0, 100);
        if let Some(h) = &p.hit_box {
            self.key("hit_box", |c| c.hit_box(h));
        }
        if let Some(d) = p.damage {
            self.within("damage", i64::from(d), 1, i64::from(MAX_DAMAGE));
        }
        if let Some(r) = p.rehit_ticks {
            if p.kind != PhaseKind::Active || p.hit_box.is_none() {
                self.key("rehit_ticks", |c| {
                    c.fail("only an active phase with a hit box hits again")
                });
            } else {
                self.within("rehit_ticks", i64::from(r), 1, i64::from(MAX_PHASE_TICKS));
            }
        }
    }

    /// A hit box's radius and offset are at most `MAX_RADIUS`, and an arc's half angle is 0 to
    /// 180 degrees.
    fn hit_box(&mut self, h: &HitBox) {
        let (HitBox::Circle { offset, radius } | HitBox::Arc { offset, radius, .. }) = h;
        let r = i64::from(MAX_RADIUS);
        self.within("radius", i64::from(radius.0), 1, r);
        self.key("offset", |c| {
            c.within("forward", i64::from(offset.forward.0), -r, r);
            c.within("right", i64::from(offset.right.0), -r, r);
        });
        if let HitBox::Arc { half_angle, .. } = h {
            self.within(
                "half_angle",
                i64::from(*half_angle),
                0,
                i64::from(180 * trig::PER_DEGREE),
            );
        }
    }

    fn number(&mut self, v: i32) {
        self.within(
            "value",
            i64::from(v),
            -i64::from(MAX_NUMBER),
            i64::from(MAX_NUMBER),
        );
    }

    /// Checks a list of emitters at `depth`, counting nodes in `count`. Returns false once a
    /// limit is hit, so that the rest of the tree is not walked.
    fn emitters(&mut self, list: &[Emitter], depth: u32, count: &mut u32) -> bool {
        for (i, e) in list.iter().enumerate() {
            *count += 1;
            if depth > MAX_PATTERN_DEPTH {
                self.idx(i, |c| {
                    c.fail(format!(
                        "a pattern is at most {MAX_PATTERN_DEPTH} levels deep"
                    ))
                });
                return false;
            }
            if *count > MAX_PATTERN_NODES {
                self.idx(i, |c| {
                    c.fail(format!("a pattern has at most {MAX_PATTERN_NODES} nodes"))
                });
                return false;
            }
            if !self.idx(i, |c| c.emitter(e, depth, count)) {
                return false;
            }
        }
        true
    }

    fn emitter(&mut self, e: &Emitter, depth: u32, count: &mut u32) -> bool {
        match e {
            Emitter::Fire {
                speed,
                direction,
                bullet,
                ticks,
            } => {
                if let Some(t) = ticks {
                    self.key("ticks", |c| c.expr(t));
                    if let Expr::Const(v) = t {
                        self.within(
                            "ticks",
                            i64::from(*v),
                            1,
                            i64::from(limits::MAX_BULLET_TICKS),
                        );
                    }
                }
                self.key("speed", |c| c.expr(speed));
                self.key("direction", |c| match direction {
                    Direction::Aimed { offset } => c.key("offset", |c| c.expr(offset)),
                    Direction::Absolute { angle } | Direction::Relative { angle } => {
                        c.key("angle", |c| c.expr(angle))
                    }
                    Direction::Sequential { step } => c.key("step", |c| c.expr(step)),
                });
                if let Some(b) = bullet {
                    self.content_id("bullet", b, Kind::Bullet);
                }
                true
            }
            Emitter::Repeat {
                count: n,
                interval_ticks,
                body,
            } => {
                self.repeat_count("count", n);
                self.constant_ticks("interval_ticks", interval_ticks);
                self.key("body", |c| c.emitters(body, depth + 1, count))
            }
            Emitter::Ring { count: n, body } => {
                self.repeat_count("count", n);
                self.key("body", |c| c.emitters(body, depth + 1, count))
            }
            Emitter::Spread {
                count: n,
                angle,
                body,
            } => {
                self.repeat_count("count", n);
                self.key("angle", |c| c.expr(angle));
                self.key("body", |c| c.emitters(body, depth + 1, count))
            }
            Emitter::Rotate { angle, body } => {
                self.key("angle", |c| c.expr(angle));
                self.key("body", |c| c.emitters(body, depth + 1, count))
            }
            Emitter::Wait { ticks } => {
                self.constant_ticks("ticks", ticks);
                true
            }
            Emitter::OnBullet {
                after_ticks,
                body,
                then,
            } => {
                self.constant_ticks("after_ticks", after_ticks);
                self.key("body", |c| c.emitters(body, depth + 1, count))
                    && self.key("then", |c| c.emitters(then, depth + 1, count))
            }
        }
    }

    /// A repeat, ring or spread count: a constant must be in `1..=MAX_REPEAT`; a computed one
    /// is clamped by the engine.
    fn repeat_count(&mut self, key: &'static str, e: &Expr) {
        self.key(key, |c| c.expr(e));
        if let Expr::Const(v) = e {
            self.within(key, i64::from(*v), 1, i64::from(MAX_REPEAT));
        }
    }

    /// A tick count: a constant must be in `0..=MAX_TICKS`; a computed one is clamped.
    fn constant_ticks(&mut self, key: &'static str, e: &Expr) {
        self.key(key, |c| c.expr(e));
        if let Expr::Const(v) = e {
            self.within(key, i64::from(*v), 0, i64::from(MAX_TICKS));
        }
    }

    /// An expression of at most `MAX_EXPR_NODES` nodes. An inline attack in a stage takes no
    /// arguments, so `arg` is not valid there; presets, which do, are checked with `validate_attack`.
    fn expr(&mut self, e: &Expr) {
        let mut nodes = 0;
        self.expr_node(e, self.arity, &mut nodes);
    }

    fn expr_node(&mut self, e: &Expr, args: u8, nodes: &mut u32) {
        *nodes += 1;
        if *nodes > MAX_EXPR_NODES {
            if *nodes == MAX_EXPR_NODES + 1 {
                self.fail(format!("an expression has at most {MAX_EXPR_NODES} nodes"));
            }
            return;
        }
        match e {
            Expr::Arg(i) => {
                if *i >= args {
                    self.fail("there is no such argument in an inline attack");
                }
            }
            Expr::LoopIndex | Expr::PhaseTime => {}
            Expr::Const(v) => self.number(*v),
            Expr::Rand(b) => self.pair("rand", b, args, nodes),
            Expr::Add(b) => self.pair("add", b, args, nodes),
            Expr::Sub(b) => self.pair("sub", b, args, nodes),
            Expr::Mul(b) => self.pair("mul", b, args, nodes),
            Expr::Div(b) => self.pair("div", b, args, nodes),
            Expr::Min(b) => self.pair("min", b, args, nodes),
            Expr::Max(b) => self.pair("max", b, args, nodes),
            Expr::Clamp(b) => self.key("clamp", |c| {
                c.idx(0, |c| c.expr_node(&b.0, args, nodes));
                c.idx(1, |c| c.expr_node(&b.1, args, nodes));
                c.idx(2, |c| c.expr_node(&b.2, args, nodes));
            }),
        }
    }

    fn pair(&mut self, name: &'static str, b: &(Expr, Expr), args: u8, nodes: &mut u32) {
        self.key(name, |c| {
            c.idx(0, |c| c.expr_node(&b.0, args, nodes));
            c.idx(1, |c| c.expr_node(&b.1, args, nodes));
        });
    }
}
