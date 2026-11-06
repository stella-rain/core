//! Expertise (ADR-CONT-1): one level inside one run, for the one weapon, with no highest
//! level. This is its content: the points levels 1 to 10 each need, the growth factor for every
//! level after and the most a level needs, and for each weapon its three styles (a path: the
//! style, then two upgrades and a capstone, at the levels of the mode's level table,
//! `stage::LevelTable`) and its common pool, drawn at the other levels. No option has a stack
//! limit and none leaves its pool by being taken (the attack speed total is held at
//! `run_stats::MAX_ATTACK_SPEED_PCT`); cooldown reduction is no expertise option but Rune
//! Haste, a star reward. Each enemy's points per hit and per kill are in the stage
//! (`stage::ExpertisePoints`).
//!
//! Data picks, code acts: an option's effect is one of the run stats' effect kinds
//! (`StatEffect`), under the same names, or `range_pct`, the range reward's widening.
//! The gauge reads the thresholds, the growth and the maximum, the common offers
//! draw from the common pool, and the style offers show the styles and the chosen style's
//! upgrades and capstone (`sim`).
//!
//! The file is untrusted (ADR-SVC-1), as the rewards are: a size limit before parsing, strict
//! parsing, the current versions only, and every field bounded. The thresholds rise strictly,
//! the factor is above 1, no pool is empty, every weapon preset has three complete styles, and
//! every number is capped. Option and style IDs are the file's own, unique within a weapon. The
//! bundled file, `content/expertise.json`, is the default; its styles are placeholders built
//! from the effect kinds `core` has.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::id::ContentId;
use crate::pack::Pack;
use crate::registry::{self, Kind};
use crate::rewards::{MAX_AMOUNT, MAX_HP_AMOUNT, MAX_RANGE_PCT};
use crate::run_stats::{MAX_STEP, StatEffect};
use crate::validate::{Error, Issue};
use crate::version::{self, Versions};

/// The largest expertise file read (a tuning value; ADR-SVC-1 decides that one exists).
pub const MAX_EXPERTISE_BYTES: usize = 64 * 1024;
/// The levels whose thresholds the file lists: 1 to 10, so far as the style path's last level
/// (`stage::MAX_STYLE_LEVEL`).
pub const TABLE_LEVELS: usize = 10;
/// The most points one level needs, a threshold or the growth's maximum.
pub const MAX_LEVEL_POINTS: u32 = 1_000_000;
/// The largest growth factor, in percent: four times the level before.
pub const MAX_GROWTH_PCT: u16 = 400;
/// The styles of each weapon: exactly three.
pub const STYLES: usize = 3;
/// The most fixed choices a style offers at each of its two upgrades and its capstone.
pub const MAX_STYLE_CHOICES: usize = 3;
/// The most options in a weapon's common pool.
pub const MAX_COMMON_OPTIONS: usize = 32;
pub use crate::limits::MAX_SET_STATS;

/// One stat of a weapon base set or a style set (the equipment build): its value at weapon level
/// 1 to 10.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetStat {
    /// One of `attack_pct`, `attack_speed_pct`, `evade_pct`, `skill_cooldown_pct`, `range_pct`
    /// and `max_hp`, once per set.
    pub effect: StatEffect,
    /// Exactly `TABLE_LEVELS` entries, Lv1 to Lv10, never falling. Tenths of a percent, except
    /// `max_hp`: hit points in the stored scale of every other `max_hp` amount.
    pub by_level: Vec<u32>,
}

/// The largest entry of a set stat of `effect`, if a set may hold the effect: a tenths effect
/// up to `MAX_STEP * 10` (100 percent), `max_hp` up to `MAX_HP_AMOUNT`.
fn set_stat_most(effect: StatEffect) -> Option<u32> {
    match effect {
        StatEffect::AttackPct
        | StatEffect::AttackSpeedPct
        | StatEffect::EvadePct
        | StatEffect::SkillCooldownPct
        | StatEffect::RangePct => Some(MAX_STEP * 10),
        StatEffect::MaxHp => Some(MAX_HP_AMOUNT),
        _ => None,
    }
}

/// What taking an option or a style does: one of the run stats' effect kinds, under the same
/// names, or a wider combo.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ExpertiseEffect {
    MaxHp,
    AttackPct,
    AttackSpeedPct,
    EvadePct,
    SpeedPct,
    XpPct,
    /// Every combo step's hit box `amount` percent larger, as the range reward widens it,
    /// at most `MAX_RANGE_PCT`.
    RangePct,
}

impl ExpertiseEffect {
    /// The run stats' effect kind it adds to, if it is one.
    pub fn stat(self) -> Option<StatEffect> {
        Some(match self {
            ExpertiseEffect::MaxHp => StatEffect::MaxHp,
            ExpertiseEffect::AttackPct => StatEffect::AttackPct,
            ExpertiseEffect::AttackSpeedPct => StatEffect::AttackSpeedPct,
            ExpertiseEffect::EvadePct => StatEffect::EvadePct,
            ExpertiseEffect::SpeedPct => StatEffect::SpeedPct,
            ExpertiseEffect::XpPct => StatEffect::XpPct,
            ExpertiseEffect::RangePct => return None,
        })
    }

    /// The largest amount of this kind.
    pub fn max_amount(self) -> u32 {
        match self {
            ExpertiseEffect::RangePct => MAX_RANGE_PCT,
            ExpertiseEffect::MaxHp => MAX_HP_AMOUNT,
            _ => MAX_AMOUNT,
        }
    }
}

/// One option: a candidate of a common offer, or a fixed choice of a style.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpertiseOption {
    /// The option's own ID, unique within its weapon.
    pub id: ContentId,
    pub effect: ExpertiseEffect,
    /// What taking it adds to the effect's total, from 1 to the effect's most.
    pub amount: u32,
}

/// A style: a path chosen at the level table's style level, held for the run. Its
/// own effect applies when it is chosen in offers mode (an equipment run applies the style's
/// `stats` set instead: the set is everything); it then offers the fixed choices of `upgrade_1` at the
/// table's first upgrade level, `upgrade_2` at the second and `capstone` at the capstone's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Style {
    /// The style's own ID, unique within its weapon.
    pub id: ContentId,
    pub effect: ExpertiseEffect,
    /// What choosing it adds to the effect's total, from 1 to the effect's most.
    pub amount: u32,
    /// The set the style grants in the equipment build, replacing the weapon's base set when
    /// the style is picked: at most `MAX_SET_STATS` stats. Absent or empty: the empty set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stats: Vec<SetStat>,
    /// The first upgrade's choices, 1 to `MAX_STYLE_CHOICES`.
    pub upgrade_1: Vec<ExpertiseOption>,
    /// The second upgrade's choices, 1 to `MAX_STYLE_CHOICES`.
    pub upgrade_2: Vec<ExpertiseOption>,
    /// The capstone's choices, 1 to `MAX_STYLE_CHOICES`.
    pub capstone: Vec<ExpertiseOption>,
}

/// One weapon's expertise.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeaponExpertise {
    /// A weapon preset of the registry.
    pub weapon: ContentId,
    /// Exactly `STYLES` styles, in the order the style offer shows them.
    pub styles: Vec<Style>,
    /// The common pool, 1 to `MAX_COMMON_OPTIONS` options, in the order offers draw from.
    pub common: Vec<ExpertiseOption>,
    /// The set the weapon grants in the equipment build until a style is picked: at most
    /// `MAX_SET_STATS` stats. Absent or empty: the empty set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base: Vec<SetStat>,
}

/// The expertise file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpertiseFile {
    /// The points each of levels 1 to 10 needs from the level before, the first from 0:
    /// exactly `TABLE_LEVELS`, rising strictly, each from 1 to `max_points`.
    pub thresholds: Vec<u32>,
    /// Each level after 10 needs the level before's points times `growth_pct / 100`: above 100,
    /// at most `MAX_GROWTH_PCT`.
    pub growth_pct: u16,
    /// The most points any level needs, where the growth saturates: at least the last
    /// threshold, at most `MAX_LEVEL_POINTS`.
    pub max_points: u32,
    /// Every weapon preset of the registry, each once.
    pub weapons: Vec<WeaponExpertise>,
}

/// Expertise content that passed validation, and the SHA-256 of the bytes it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expertise {
    thresholds: Vec<u32>,
    growth_pct: u16,
    max_points: u32,
    weapons: Vec<WeaponExpertise>,
    sha256: [u8; 32],
}

impl Expertise {
    /// The bundled expertise content.
    pub fn bundled() -> &'static Expertise {
        Pack::bundled().content().expertise()
    }

    /// Checks the size and the versions given (the pack's), parses strictly and validates: the one
    /// call for a file's bytes.
    pub fn parse(bytes: &[u8], versions: Versions) -> Result<Expertise, Error> {
        let file: ExpertiseFile = version::read_member(bytes, MAX_EXPERTISE_BYTES, versions)?;
        validate(&file, versions.sim_version)?;
        Ok(Expertise {
            thresholds: file.thresholds,
            growth_pct: file.growth_pct,
            max_points: file.max_points,
            weapons: file.weapons,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The points each of levels 1 to 10 needs from the level before.
    pub fn thresholds(&self) -> &[u32] {
        &self.thresholds
    }

    /// The growth factor after level 10, in percent.
    pub fn growth_pct(&self) -> u16 {
        self.growth_pct
    }

    /// The most points any level needs.
    pub fn max_points(&self) -> u32 {
        self.max_points
    }

    /// Every weapon's expertise, in the file's order.
    pub fn weapons(&self) -> &[WeaponExpertise] {
        &self.weapons
    }

    /// The expertise of the weapon preset `id`.
    pub fn weapon(&self, id: &ContentId) -> Option<&WeaponExpertise> {
        self.weapons.iter().find(|w| &w.weapon == id)
    }

    /// The SHA-256 of the bytes it was parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// The weapon presets a file must cover: the registry's, no newer than this `sim_version`.
fn weapon_presets(sim_version: u32) -> impl Iterator<Item = &'static str> {
    registry::ENTRIES
        .iter()
        .filter(move |e| e.kind == Kind::Weapon && e.introduced.le(&sim_version))
        .map(|e| e.id)
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &ExpertiseFile, sim_version: u32) -> Result<(), Error> {
    let mut issues = Vec::new();
    let mut issue = |path: String, message: String| issues.push(Issue { path, message });

    if file.thresholds.len() != TABLE_LEVELS {
        issue(
            "$.thresholds".into(),
            format!(
                "exactly {TABLE_LEVELS} thresholds (found {})",
                file.thresholds.len()
            ),
        );
    }
    for (i, &points) in file.thresholds.iter().enumerate() {
        let at = format!("$.thresholds[{i}]");
        if !(1..=file.max_points).contains(&points) {
            issue(
                at,
                format!("1 to max_points, {} (found {points})", file.max_points),
            );
        } else if i > 0 && points <= file.thresholds[i - 1] {
            issue(at, "more than the threshold before".into());
        }
    }
    let last = file.thresholds.last().copied().unwrap_or(1);
    if !(last..=MAX_LEVEL_POINTS).contains(&file.max_points) {
        issue(
            "$.max_points".into(),
            format!(
                "the last threshold to {MAX_LEVEL_POINTS} (found {})",
                file.max_points
            ),
        );
    }
    if !(101..=MAX_GROWTH_PCT).contains(&file.growth_pct) {
        issue(
            "$.growth_pct".into(),
            format!("101 to {MAX_GROWTH_PCT} (found {})", file.growth_pct),
        );
    }

    let missing: Vec<&str> = weapon_presets(sim_version)
        .filter(|id| !file.weapons.iter().any(|w| w.weapon.as_str() == *id))
        .collect();
    if !missing.is_empty() {
        issue(
            "$.weapons".into(),
            format!("every weapon preset, missing {}", missing.join(", ")),
        );
    }
    for (w, weapon) in file.weapons.iter().enumerate() {
        let at = |field: &str| format!("$.weapons[{w}].{field}");
        if !weapon_presets(sim_version).any(|id| id == weapon.weapon.as_str()) {
            issue(at("weapon"), "not a weapon preset".into());
        } else if file.weapons[..w].iter().any(|x| x.weapon == weapon.weapon) {
            issue(at("weapon"), "the same weapon as an earlier entry".into());
        }
        if weapon.styles.len() != STYLES {
            issue(
                at("styles"),
                format!("exactly {STYLES} styles (found {})", weapon.styles.len()),
            );
        }
        check_set(&mut issue, &at("base"), &weapon.base);
        if weapon.common.is_empty() || weapon.common.len() > MAX_COMMON_OPTIONS {
            issue(
                at("common"),
                format!(
                    "1 to {MAX_COMMON_OPTIONS} options (found {})",
                    weapon.common.len()
                ),
            );
        }

        // Every ID of the weapon, styles and options alike, in the file's order.
        let mut seen: Vec<&ContentId> = Vec::new();
        for (s, style) in weapon.styles.iter().enumerate() {
            let base = at(&format!("styles[{s}]"));
            check_id(&mut issue, &mut seen, &base, &style.id);
            check_amount(&mut issue, &base, style.effect, style.amount);
            check_set(&mut issue, &format!("{base}.stats"), &style.stats);
            for (field, choices) in [
                ("upgrade_1", &style.upgrade_1),
                ("upgrade_2", &style.upgrade_2),
                ("capstone", &style.capstone),
            ] {
                let path = format!("{base}.{field}");
                if choices.is_empty() || choices.len() > MAX_STYLE_CHOICES {
                    issue(
                        path.clone(),
                        format!("1 to {MAX_STYLE_CHOICES} choices (found {})", choices.len()),
                    );
                }
                for (c, choice) in choices.iter().enumerate() {
                    let path = format!("{path}[{c}]");
                    check_option(&mut issue, &mut seen, &path, choice);
                }
            }
        }
        for (c, option) in weapon.common.iter().enumerate() {
            let path = at(&format!("common[{c}]"));
            check_option(&mut issue, &mut seen, &path, option);
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}

fn check_option<'a>(
    issue: &mut impl FnMut(String, String),
    seen: &mut Vec<&'a ContentId>,
    path: &str,
    option: &'a ExpertiseOption,
) {
    check_id(issue, seen, path, &option.id);
    check_amount(issue, path, option.effect, option.amount);
}

fn check_id<'a>(
    issue: &mut impl FnMut(String, String),
    seen: &mut Vec<&'a ContentId>,
    path: &str,
    id: &'a ContentId,
) {
    if !registry::is_well_formed(id.as_str()) {
        issue(
            format!("{path}.id"),
            "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
        );
    } else if seen.contains(&id) {
        issue(
            format!("{path}.id"),
            "the same ID as an earlier style or option of the weapon".into(),
        );
    }
    seen.push(id);
}

fn check_amount(
    issue: &mut impl FnMut(String, String),
    path: &str,
    effect: ExpertiseEffect,
    amount: u32,
) {
    let most = effect.max_amount();
    if !(1..=most).contains(&amount) {
        issue(
            format!("{path}.amount"),
            format!("1 to {most} (found {amount})"),
        );
    }
}

/// A set: at most `MAX_SET_STATS` stats, each effect allowed and once, each with exactly
/// `TABLE_LEVELS` entries within the effect's bound and never falling.
fn check_set(issue: &mut impl FnMut(String, String), path: &str, set: &[SetStat]) {
    if set.len() > MAX_SET_STATS {
        issue(
            path.to_string(),
            format!("at most {MAX_SET_STATS} stats (found {})", set.len()),
        );
    }
    for (i, stat) in set.iter().enumerate().take(MAX_SET_STATS) {
        let at = format!("{path}[{i}]");
        let most = set_stat_most(stat.effect);
        if most.is_none() {
            issue(
                format!("{at}.effect"),
                "a set holds attack_pct, attack_speed_pct, evade_pct, skill_cooldown_pct, range_pct or max_hp".into(),
            );
        } else if set[..i].iter().any(|x| x.effect == stat.effect) {
            issue(
                format!("{at}.effect"),
                "the same effect as an earlier stat of the set".into(),
            );
        }
        if stat.by_level.len() != TABLE_LEVELS {
            issue(
                format!("{at}.by_level"),
                format!(
                    "exactly {TABLE_LEVELS} entries (found {})",
                    stat.by_level.len()
                ),
            );
        }
        for (n, &value) in stat.by_level.iter().enumerate().take(TABLE_LEVELS) {
            let path = format!("{at}.by_level[{n}]");
            if let Some(most) = most.filter(|&m| value > m) {
                issue(path, format!("0 to {most} (found {value})"));
            } else if n > 0 && value < stat.by_level[n - 1] {
                issue(path, "not less than the level before".into());
            }
        }
    }
}
