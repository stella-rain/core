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

use crate::event::Press;
use crate::id::ContentId;
use crate::pack::Pack;
use crate::registry::{self, Kind};
use crate::rewards::{MAX_AMOUNT, MAX_HP_AMOUNT, MAX_RANGE_PCT};
use crate::run_stats::{MAX_STEP, StatEffect};
use crate::validate::limits::MAX_COMBO_DEPTH;
use crate::validate::{self, Error, Issue};
use crate::version::{self, Versions};
use crate::weapon::{Branches, Charge, Hold, Step, Weapon};

/// The largest expertise file read (a tuning value; ADR-SVC-1 decides that one exists).
pub const MAX_EXPERTISE_BYTES: usize = 64 * 1024;
/// The levels whose thresholds the file lists: 1 to 10, so far as the style path's last level
/// (`stage::MAX_STYLE_LEVEL`).
pub const TABLE_LEVELS: usize = 10;
/// The most points one level needs, a threshold or the growth's maximum.
pub const MAX_LEVEL_POINTS: u32 = 1_000_000;
/// The largest growth factor, in percent: four times the level before.
pub const MAX_GROWTH_PCT: u16 = 400;
/// The styles of each weapon: exactly three. The plain base is no style (ADR-SIM-9 point 2): it
/// is the weapon's base tree, with no entry here.
pub const STYLES: usize = 3;
/// The fixed choices a style offers at each of its two upgrades and its capstone: exactly one
/// (ADR-SIM-9 point 3).
pub const MAX_STYLE_CHOICES: usize = 1;
/// The most mechanic kinds one level names (a tuning placeholder): the style pick's, an
/// upgrade's choice's or the capstone's.
pub const MAX_LEVEL_KINDS: usize = 4;
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
    /// No stat: a level that grants only mechanic `kinds`. Its `amount` is absent or 0 and its
    /// `kinds` are not empty; a rule of `sim_version` 2.
    None,
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
            ExpertiseEffect::None | ExpertiseEffect::RangePct => return None,
        })
    }

    /// The largest amount of this kind.
    pub fn max_amount(self) -> u32 {
        match self {
            ExpertiseEffect::None => 0,
            ExpertiseEffect::RangePct => MAX_RANGE_PCT,
            ExpertiseEffect::MaxHp => MAX_HP_AMOUNT,
            _ => MAX_AMOUNT,
        }
    }
}

/// A grant on a step of the base tree: a charge or a hold, never both (ADR-SIM-9 point 3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum StepGrant {
    /// The step charges while its button stays held as it starts.
    Charge(Charge),
    /// The step holds past its last active phase.
    Hold(Hold),
}

/// A mechanic a style level grants: a closed list built into `core`, its numbers in JSON
/// (ADR-SIM-9 point 3, ADR-CONT-1). Every kind is bounded and validated alone and against the
/// weapon's base tree. Only the kinds that are listed exist.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StyleKind {
    /// The step reached by `step` (the presses from the root of the combo tree, 1 to
    /// `MAX_COMBO_DEPTH` long: `[main, main, main]` is AAA, `[smash]` is B from neutral) gets
    /// `grant`. A grant sets the step's charge (clearing its hold) or its hold (clearing its
    /// charge), so a step never ends with both; two grants on one step inside one style are
    /// refused.
    ReplaceStep { step: Vec<Press>, grant: StepGrant },
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// One option: a candidate of a common offer, or a fixed choice of a style.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpertiseOption {
    /// The option's own ID, unique within its weapon.
    pub id: ContentId,
    pub effect: ExpertiseEffect,
    /// What taking it adds to the effect's total, from 1 to the effect's most; absent (0) for
    /// the effect `none`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub amount: u32,
    /// The mechanic kinds it grants, at most `MAX_LEVEL_KINDS`; absent or empty for none
    /// (`sim_version` 2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<StyleKind>,
}

/// A style: a path chosen at the level table's style level, held for the run. Its
/// own effect applies when it is chosen at `sim_version` 1 (a version 2 run applies the style's
/// `stats` set instead: the set is everything); it then offers the fixed choices of `upgrade_1` at the
/// table's first upgrade level, `upgrade_2` at the second and `capstone` at the capstone's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Style {
    /// The style's own ID, unique within its weapon.
    pub id: ContentId,
    pub effect: ExpertiseEffect,
    /// What choosing it adds to the effect's total, from 1 to the effect's most; absent (0) for
    /// the effect `none`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub amount: u32,
    /// The mechanic kinds the style pick grants, at most `MAX_LEVEL_KINDS`; absent or empty for
    /// none (`sim_version` 2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<StyleKind>,
    /// The set the style grants in the equipment build, replacing the weapon's base set when
    /// the style is picked: at most `MAX_SET_STATS` stats. Absent or empty: the empty set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stats: Vec<SetStat>,
    /// The first upgrade's one choice.
    pub upgrade_1: Vec<ExpertiseOption>,
    /// The second upgrade's one choice.
    pub upgrade_2: Vec<ExpertiseOption>,
    /// The capstone's one choice.
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
            check_level(
                &mut issue,
                &base,
                style.effect,
                style.amount,
                &style.kinds,
                sim_version,
            );
            check_set(&mut issue, &format!("{base}.stats"), &style.stats);
            for (field, choices) in [
                ("upgrade_1", &style.upgrade_1),
                ("upgrade_2", &style.upgrade_2),
                ("capstone", &style.capstone),
            ] {
                let path = format!("{base}.{field}");
                if choices.len() != MAX_STYLE_CHOICES {
                    issue(
                        path.clone(),
                        format!("exactly 1 choice (found {})", choices.len()),
                    );
                }
                for (c, choice) in choices.iter().enumerate() {
                    let path = format!("{path}[{c}]");
                    check_option(&mut issue, &mut seen, &path, choice, sim_version);
                }
            }
            check_step_once(&mut issue, &base, style);
        }
        for (c, option) in weapon.common.iter().enumerate() {
            let path = at(&format!("common[{c}]"));
            check_option(&mut issue, &mut seen, &path, option, sim_version);
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
    sim_version: u32,
) {
    check_id(issue, seen, path, &option.id);
    check_level(
        issue,
        path,
        option.effect,
        option.amount,
        &option.kinds,
        sim_version,
    );
}

/// A level's stat and kinds: a stat effect has an amount from 1 to its most; `none` has none
/// and names at least one kind; the kinds, and `none`, are `sim_version` 2's, at most
/// `MAX_LEVEL_KINDS`, each bounded alone.
fn check_level(
    issue: &mut impl FnMut(String, String),
    path: &str,
    effect: ExpertiseEffect,
    amount: u32,
    kinds: &[StyleKind],
    sim_version: u32,
) {
    if effect == ExpertiseEffect::None {
        if sim_version < 2 {
            issue(
                format!("{path}.effect"),
                "the effect none needs sim_version 2".into(),
            );
        } else if kinds.is_empty() {
            issue(
                format!("{path}.effect"),
                "the effect none needs at least one kind".into(),
            );
        }
        if amount != 0 {
            issue(
                format!("{path}.amount"),
                format!("none takes no amount (found {amount})"),
            );
        }
    } else {
        check_amount(issue, path, effect, amount);
    }
    if kinds.is_empty() {
        return;
    }
    let at = format!("{path}.kinds");
    if sim_version < 2 {
        issue(at.clone(), "kinds need sim_version 2".into());
    }
    if kinds.len() > MAX_LEVEL_KINDS {
        issue(
            at.clone(),
            format!("at most {MAX_LEVEL_KINDS} kinds (found {})", kinds.len()),
        );
    }
    for (k, kind) in kinds.iter().enumerate().take(MAX_LEVEL_KINDS) {
        let at = format!("{at}[{k}]");
        let StyleKind::ReplaceStep { step, grant } = kind;
        if step.is_empty() || step.len() > MAX_COMBO_DEPTH as usize {
            issue(
                format!("{at}.step"),
                format!("1 to {MAX_COMBO_DEPTH} presses (found {})", step.len()),
            );
        }
        if let Err(found) = validate::validate_step_grant(grant, registry::ENTRIES) {
            for f in found {
                issue(
                    format!("{at}.grant{}", f.path.trim_start_matches('$')),
                    f.message,
                );
            }
        }
    }
}

/// The levels of a style's path in order, as the simulation takes them: the style pick's own
/// kinds, then the one choice of each upgrade and the capstone.
pub(crate) fn style_levels(style: &Style) -> [(&'static str, Option<&ExpertiseOption>); 3] {
    [
        ("upgrade_1", style.upgrade_1.first()),
        ("upgrade_2", style.upgrade_2.first()),
        ("capstone", style.capstone.first()),
    ]
}

/// One grant on a step per style: a second grant on a step an earlier level of the same style
/// granted is refused at the later one.
fn check_step_once(issue: &mut impl FnMut(String, String), base: &str, style: &Style) {
    let mut seen: Vec<&Vec<Press>> = Vec::new();
    let mut levels: Vec<(String, &[StyleKind])> = vec![(format!("{base}.kinds"), &style.kinds)];
    for (field, option) in style_levels(style) {
        if let Some(o) = option {
            levels.push((format!("{base}.{field}[0].kinds"), &o.kinds));
        }
    }
    for (path, kinds) in levels {
        for (k, kind) in kinds.iter().enumerate().take(MAX_LEVEL_KINDS) {
            let StyleKind::ReplaceStep { step, .. } = kind;
            if seen.contains(&step) {
                issue(
                    format!("{path}[{k}].step"),
                    "the step is already granted by an earlier level of the style".into(),
                );
            } else {
                seen.push(step);
            }
        }
    }
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

/// Applies one `replace_step` grant to the step of `weapon` that `presses` reach from the root
/// of the combo tree: the grant sets the step's charge (clearing its hold) or its hold
/// (clearing its charge). `false`, nothing changed, when no step is there.
pub(crate) fn apply_grant(weapon: &mut Weapon, presses: &[Press], grant: &StepGrant) -> bool {
    fn at<'a>(branches: &'a mut Branches, presses: &[Press]) -> Option<&'a mut Step> {
        let (first, rest) = presses.split_first()?;
        let step = match first {
            Press::Main => branches.main.as_deref_mut(),
            Press::Smash => branches.smash.as_deref_mut(),
        }?;
        if rest.is_empty() {
            Some(step)
        } else {
            at(&mut step.next, rest)
        }
    }
    let Some(step) = at(&mut weapon.combo, presses) else {
        return false;
    };
    match grant {
        StepGrant::Charge(c) => {
            step.charge = Some(c.clone());
            step.hold = None;
        }
        StepGrant::Hold(h) => {
            step.hold = Some(h.clone());
            step.charge = None;
        }
    }
    true
}
