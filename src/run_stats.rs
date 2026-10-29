//! Run stats (ADR-079 §4, ADR-067): the stats a stat point can raise during a run, each with
//! its step and its limit. They are content, not code: a JSON list of any length up to
//! `MAX_STATS`, read in its order, so a stat is added, removed or retuned by editing the file.
//! Data picks, code acts: each stat names one of the effect kinds built in here
//! (`StatEffect`), and a new kind of effect is a code change. Stats that share an effect add
//! up.
//!
//! The file is untrusted (ADR-020), because it may later come from outside the app: a size
//! limit before parsing, strict parsing, the current versions only, and every field bounded.
//! Stat IDs must be well-formed and unique within the file; they are the file's own, not
//! registry entries, so a file can bring a stat this build has never seen as long as its
//! effect kind is one this build knows. The bundled file, `content/run_stats.json`, is the
//! default; the SHA-256 of whatever bytes a run used is part of its content hash, so a run
//! replay verifies only with the same file.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::id::ContentId;
use crate::pack::Pack;
use crate::registry;
use crate::validate::limits::MAX_PLAYER_HP;
use crate::validate::{Error, Issue};
use crate::version::{self, Versions};

/// The largest run stats file read (a tuning value; ADR-020 decides that one exists).
pub const MAX_RUN_STATS_BYTES: usize = 16 * 1024;
/// The most stats a file lists.
pub const MAX_STATS: usize = 32;
/// The largest step: a percentage; for hit points, `MAX_HP_STEP`.
pub const MAX_STEP: u32 = 100;
/// The largest step of hit points: `MAX_STEP` at the design values, scaled (ADR-103).
pub const MAX_HP_STEP: u32 = MAX_STEP * crate::validate::limits::DAMAGE_SCALE;
/// The most points one stat takes.
pub const MAX_LIMIT: u8 = 20;
/// The longest title, in characters.
pub const MAX_TITLE_CHARS: usize = 32;
/// The longest description, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 160;

/// The largest immunity to a new stun after one ends, in ticks (ADR-085 point 4): a placeholder
/// bound, as the other limits are (ADR-020 decides that one exists).
pub const MAX_STUN_IMMUNE_TICKS: u32 = 600;

/// The most the attack speed total's cap can be, in percent: the ceiling the run stats file's
/// `max_attack_speed_pct` may not pass (ADR-020), and the largest total one of the options
/// weighed in app#298 allowed. Expertise options have no stack limit, so the total saturates
/// at the file's cap (ADR-099).
pub const MAX_ATTACK_SPEED_PCT: u32 = 800;

/// What an attack does when an enemy or the boss makes it and the attack names none, when the
/// file names none: 100 shown hit points, scaled (ADR-103). The file's value is bounded by
/// `MAX_PLAYER_HP`, the most the main character holds.
pub const DEFAULT_ENEMY_ATTACK_DAMAGE: u32 = 100 * crate::validate::limits::DAMAGE_SCALE;

/// The cap of a file that names none: the cap before it was content (app#298 kept it).
pub const DEFAULT_MAX_ATTACK_SPEED_PCT: u32 = 500;

/// The attack speed total `total` held to `cap`, and `cap` to `MAX_ATTACK_SPEED_PCT`, so that
/// an unvalidated cap cannot pass the ceiling.
pub fn capped_attack_speed_pct(total: u32, cap: u32) -> u32 {
    total.min(cap.min(MAX_ATTACK_SPEED_PCT))
}

/// What a point in a stat does, `step` times over. The closed list of version 0's effects;
/// percentages add up over every stat with the same effect and round down.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum StatEffect {
    /// `step` more hit points at most, and as many healed.
    MaxHp,
    /// `step` percent more damage from every attack of the main character.
    AttackPct,
    /// `step` percent faster attacks: a combo step's length divided by `1 + step / 100`
    /// (ADR-070), each of its phases at least 1 tick.
    AttackSpeedPct,
    /// A `step` percent shorter evade cooldown (at least 1 tick).
    EvadePct,
    /// `step` percent more movement speed.
    SpeedPct,
    /// `step` percent more experience from every kill, rounded down (ADR-068: the build that
    /// unlocks rarer stars sooner).
    XpPct,
}

impl StatEffect {
    /// Every effect, in the order the state keeps their totals.
    pub const ALL: [StatEffect; 6] = [
        StatEffect::MaxHp,
        StatEffect::AttackPct,
        StatEffect::AttackSpeedPct,
        StatEffect::EvadePct,
        StatEffect::SpeedPct,
        StatEffect::XpPct,
    ];
}

/// One stat a point can raise.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunStat {
    /// The stat's own ID, unique within the file.
    pub id: ContentId,
    pub effect: StatEffect,
    /// What one point adds, from 1 to `MAX_STEP` (`MAX_HP_STEP` for hit points).
    pub step: u32,
    /// The most points the stat takes in a run, from 1 to `MAX_LIMIT`.
    pub limit: u8,
}

/// The stun rules (ADR-085 points 4 and 6): run rules read with the stats, as they are content
/// too. Both are tuning values; that each is bounded is the decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StunRules {
    /// Ticks after a stun ends in which the enemy cannot be stunned again, 0 to
    /// `MAX_STUN_IMMUNE_TICKS`.
    pub immune_ticks: u32,
    /// The share of a stun's ticks an elite takes, in percent, 0 to 100.
    pub elite_pct: u32,
}

/// The run stats file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunStatsFile {
    /// The stats, in the order a stat offer lists them and the setup's priority falls back on.
    pub stats: Vec<RunStat>,
    /// The stun rules.
    pub stun: StunRules,
    /// The most the attack speed total reaches, in percent, 1 to `MAX_ATTACK_SPEED_PCT`
    /// (app#298); absent, `DEFAULT_MAX_ATTACK_SPEED_PCT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_attack_speed_pct: Option<u32>,
    /// The damage an enemy's or the boss's attack does when it names none, in stored hit points,
    /// 1 to `MAX_PLAYER_HP`; absent, `DEFAULT_ENEMY_ATTACK_DAMAGE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enemy_attack_damage: Option<u32>,
}

/// Run stats that passed validation, and the SHA-256 of the bytes they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunStats {
    stats: Vec<RunStat>,
    stun: StunRules,
    max_attack_speed_pct: u32,
    enemy_attack_damage: u32,
    sha256: [u8; 32],
}

impl RunStats {
    /// The bundled run stats, the default of every run.
    pub fn bundled() -> &'static RunStats {
        Pack::bundled().content().run_stats()
    }

    /// Checks the size and the versions given (the pack's), parses strictly and validates: the one
    /// call for a file's bytes.
    pub fn parse(bytes: &[u8], versions: Versions) -> Result<RunStats, Error> {
        let file: RunStatsFile = version::read_member(bytes, MAX_RUN_STATS_BYTES, versions)?;
        validate(&file)?;
        Ok(RunStats {
            stats: file.stats,
            stun: file.stun,
            max_attack_speed_pct: file
                .max_attack_speed_pct
                .unwrap_or(DEFAULT_MAX_ATTACK_SPEED_PCT),
            enemy_attack_damage: file
                .enemy_attack_damage
                .unwrap_or(DEFAULT_ENEMY_ATTACK_DAMAGE),
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The stats, in the file's order.
    pub fn stats(&self) -> &[RunStat] {
        &self.stats
    }

    /// The stun rules of the file.
    pub fn stun(&self) -> StunRules {
        self.stun
    }

    /// The most the attack speed total reaches, in percent: the file's, or the default.
    pub fn max_attack_speed_pct(&self) -> u32 {
        self.max_attack_speed_pct
    }

    /// The damage an enemy's or the boss's attack does when it names none: the file's, or the
    /// default.
    pub fn enemy_attack_damage(&self) -> u32 {
        self.enemy_attack_damage
    }

    /// The SHA-256 of the bytes they were parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &RunStatsFile) -> Result<(), Error> {
    let mut issues = Vec::new();
    let mut issue = |path: String, message: String| issues.push(Issue { path, message });
    if file.stats.is_empty() || file.stats.len() > MAX_STATS {
        issue(
            "$.stats".into(),
            format!("1 to {MAX_STATS} stats (found {})", file.stats.len()),
        );
    }
    for (i, stat) in file.stats.iter().enumerate() {
        let at = |field: &str| format!("$.stats[{i}].{field}");
        if !registry::is_well_formed(stat.id.as_str()) {
            issue(
                at("id"),
                "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
            );
        } else if file.stats[..i].iter().any(|s| s.id == stat.id) {
            issue(at("id"), "the same ID as an earlier stat".into());
        }
        let most_step = match stat.effect {
            StatEffect::MaxHp => MAX_HP_STEP,
            _ => MAX_STEP,
        };
        if !(1..=most_step).contains(&stat.step) {
            issue(
                at("step"),
                format!("1 to {most_step} (found {})", stat.step),
            );
        }
        if !(1..=MAX_LIMIT).contains(&stat.limit) {
            issue(
                at("limit"),
                format!("1 to {MAX_LIMIT} (found {})", stat.limit),
            );
        }
    }
    if file.stun.immune_ticks > MAX_STUN_IMMUNE_TICKS {
        issue(
            "$.stun.immune_ticks".into(),
            format!(
                "0 to {MAX_STUN_IMMUNE_TICKS} (found {})",
                file.stun.immune_ticks
            ),
        );
    }
    if file.stun.elite_pct > MAX_STEP {
        issue(
            "$.stun.elite_pct".into(),
            format!("0 to {MAX_STEP} (found {})", file.stun.elite_pct),
        );
    }
    if let Some(cap) = file.max_attack_speed_pct
        && !(1..=MAX_ATTACK_SPEED_PCT).contains(&cap)
    {
        issue(
            "$.max_attack_speed_pct".into(),
            format!("1 to {MAX_ATTACK_SPEED_PCT} (found {cap})"),
        );
    }
    if let Some(damage) = file.enemy_attack_damage
        && !(1..=MAX_PLAYER_HP).contains(&damage)
    {
        issue(
            "$.enemy_attack_damage".into(),
            format!("1 to {MAX_PLAYER_HP} (found {damage})"),
        );
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}

/// Why `text` is not `min` to `max` characters of printable text, if it is not.
pub(crate) fn text_problem(text: &str, min: usize, max: usize) -> Option<String> {
    let chars = text.chars().count();
    if !(min..=max).contains(&chars) {
        Some(format!("{min} to {max} characters (found {chars})"))
    } else if text.chars().any(char::is_control) {
        Some("control characters are not allowed".into())
    } else {
        None
    }
}
