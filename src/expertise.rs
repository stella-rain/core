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
//! upgrades and capstone (`sim`). A style level may also name mechanic kinds (`StyleKind`): a
//! step's charge or hold, a delayed repeat or a proc factor of a step's hits, the weapon's smash
//! effect (`reap` among them) or a weapon gauge that fills from hits and damage taken and spends
//! on a full-gauge attack (ADR-SIM-9). A repeat and a factor are runtime-only fields of the step
//! on the style's copy of the tree.
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

use crate::attack::AttackRef;
use crate::event::Press;
use crate::id::ContentId;
use crate::limits::{MAX_GAUGE_DAMAGE_PCT, MAX_WEAPON_GAUGE};
use crate::pack::Pack;
use crate::registry::{self, Kind};
use crate::rewards::{MAX_AMOUNT, MAX_HP_AMOUNT, MAX_RANGE_PCT};
use crate::run_stats::{MAX_STEP, StatEffect};
use crate::validate::limits::MAX_COMBO_DEPTH;
use crate::validate::{self, Error, Issue};
use crate::version::{self, Versions};
use crate::weapon::{
    Branches, Charge, Hold, RepeatAnchorKind, SmashEffect, Step, StepRepeat, Weapon,
};

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
/// The longest delay of a `repeat` kind, in ticks (a tuning placeholder: 2 seconds).
pub const MAX_REPEAT_AFTER_TICKS: u32 = 120;
/// The largest `proc_pct` of a `repeat` or `proc_factor` kind: a factor is a chance (a
/// placeholder).
pub const MAX_PROC_PCT: u32 = 100;
/// The most repeats on one step, summed over one style's path (a tuning placeholder).
pub const MAX_STEP_REPEATS: usize = 4;
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
    /// The weapon's smash effect (every smash, the dash branch's too) is `effect` in place of the
    /// base's (ADR-SIM-9 point 4). One per style path: a second is refused.
    SmashEffect { effect: SmashEffect },
    /// The step reached by `step` repeats: `after_ticks` after its last `active` phase ends,
    /// `repeat_pct` percent of its damage lands again on `anchor`, firing ready on-hit runes at
    /// `proc_pct` (absent: every ready rune, no draw). Not on a held step; at most
    /// `MAX_STEP_REPEATS` per step over one style's path.
    Repeat {
        step: Vec<Press>,
        after_ticks: u32,
        repeat_pct: u32,
        anchor: RepeatAnchorKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proc_pct: Option<u32>,
    },
    /// Each hit of the step reached by `step` fires a ready on-hit rune only when a draw falls
    /// under `proc_pct` (0 to `MAX_PROC_PCT`); one per step per style.
    ProcFactor { step: Vec<Press>, proc_pct: u32 },
    /// The style's weapon gauge: it fills from the player's normal (A) steps that hit and from the
    /// hit points lost, and a B press with it full plays the full-gauge attack and empties it
    /// (ADR-SIM-9 points 3 and 7). One per style path: a second is refused.
    Gauge(Gauge),
}

/// A weapon gauge's numbers, every one bounded by the validator and clamped again when the
/// world is built (ADR-SVC-1, ADR-SIM-5). Units are gauge units; the bar is `max` long.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Gauge {
    /// The gauge never passes it; full is `max` reached. 1 to `MAX_WEAPON_GAUGE`.
    #[schemars(range(min = 1, max = MAX_WEAPON_GAUGE))]
    pub max: u32,
    /// Added once to a normal step the first time one of its hit boxes hits an enemy; absent
    /// (0): no rule. At most `max`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub per_step: u32,
    /// Added for each enemy a normal step's hit box hits on a tick; absent (0): no rule. At most
    /// `max`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub per_hit: u32,
    /// A hit taken adds `max * lost * pct / (max_hp * 100)` (floor), `lost` the hit points really
    /// lost; absent (0): no rule. At most `MAX_GAUGE_DAMAGE_PCT`.
    #[serde(default, skip_serializing_if = "is_zero")]
    #[schemars(range(min = 0, max = MAX_GAUGE_DAMAGE_PCT))]
    pub from_damage_pct: u32,
    /// The most one step's hits add (the bound on a horde, ADR-SIM-5): 1 to `max`. The damage
    /// fill is not held to it.
    pub step_cap: u32,
    /// What a full gauge buys.
    pub spend: GaugeSpend,
}

/// What a full gauge is spent on: a closed list, one variant now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum GaugeSpend {
    /// A B press with the gauge full plays `attack` in place of the step's own and empties the
    /// gauge. It is checked as a player's attack and differs from every attack of the weapon's
    /// base tree (the orb clear tells it by its compiled index).
    FullSmash { attack: AttackRef },
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
            // A common option is a stat: the kinds are a style path's (ADR-SIM-9 point 3).
            if !option.kinds.is_empty() {
                issue(
                    format!("{path}.kinds"),
                    "a common option names no kind".into(),
                );
            }
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
        match kind {
            StyleKind::ReplaceStep { step, grant } => {
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
            StyleKind::SmashEffect { effect } => {
                if let Err(found) = validate::validate_smash_effect(effect, registry::ENTRIES) {
                    for f in found {
                        issue(
                            format!("{at}.effect{}", f.path.trim_start_matches('$')),
                            f.message,
                        );
                    }
                }
            }
            StyleKind::Repeat {
                step,
                after_ticks,
                repeat_pct,
                proc_pct,
                ..
            } => {
                check_step_len(issue, &at, step);
                if !(1..=MAX_REPEAT_AFTER_TICKS).contains(after_ticks) {
                    issue(
                        format!("{at}.after_ticks"),
                        format!("1 to {MAX_REPEAT_AFTER_TICKS} (found {after_ticks})"),
                    );
                }
                if !(1..=crate::limits::MAX_PCT).contains(repeat_pct) {
                    issue(
                        format!("{at}.repeat_pct"),
                        format!("1 to {} (found {repeat_pct})", crate::limits::MAX_PCT),
                    );
                }
                check_proc_pct(issue, &at, *proc_pct);
            }
            StyleKind::ProcFactor { step, proc_pct } => {
                check_step_len(issue, &at, step);
                check_proc_pct(issue, &at, Some(*proc_pct));
            }
            StyleKind::Gauge(g) => check_gauge(issue, &at, g),
        }
    }
}

/// A gauge's numbers, each in its own range, and its full-gauge attack as a player's attack.
fn check_gauge(issue: &mut impl FnMut(String, String), at: &str, g: &Gauge) {
    if !(1..=MAX_WEAPON_GAUGE).contains(&g.max) {
        issue(
            format!("{at}.max"),
            format!("1 to {MAX_WEAPON_GAUGE} (found {})", g.max),
        );
    }
    // The fill bounds are read against the gauge's own length; a `max` out of range is its own issue.
    let most = g.max.min(MAX_WEAPON_GAUGE);
    for (key, v) in [("per_step", g.per_step), ("per_hit", g.per_hit)] {
        if v > most {
            issue(
                format!("{at}.{key}"),
                format!("0 to the gauge's max {most} (found {v})"),
            );
        }
    }
    if g.from_damage_pct > MAX_GAUGE_DAMAGE_PCT {
        issue(
            format!("{at}.from_damage_pct"),
            format!("0 to {MAX_GAUGE_DAMAGE_PCT} (found {})", g.from_damage_pct),
        );
    }
    if g.step_cap == 0 || g.step_cap > most {
        issue(
            format!("{at}.step_cap"),
            format!("1 to the gauge's max {most} (found {})", g.step_cap),
        );
    }
    let GaugeSpend::FullSmash { attack } = &g.spend;
    if let Err(found) = validate::validate_attack_ref(attack, registry::ENTRIES) {
        for f in found {
            issue(
                format!("{at}.spend.attack{}", f.path.trim_start_matches('$')),
                f.message,
            );
        }
    }
}

fn check_step_len(issue: &mut impl FnMut(String, String), at: &str, step: &[Press]) {
    if step.is_empty() || step.len() > MAX_COMBO_DEPTH as usize {
        issue(
            format!("{at}.step"),
            format!("1 to {MAX_COMBO_DEPTH} presses (found {})", step.len()),
        );
    }
}

fn check_proc_pct(issue: &mut impl FnMut(String, String), at: &str, proc_pct: Option<u32>) {
    if let Some(p) = proc_pct.filter(|&p| p > MAX_PROC_PCT) {
        issue(
            format!("{at}.proc_pct"),
            format!("0 to {MAX_PROC_PCT} (found {p})"),
        );
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

/// One grant on a step per style, and one smash effect and one gauge: a second grant on a step an earlier level
/// of the same style granted, or a second smash effect or gauge, is refused at the later one.
fn check_step_once(issue: &mut impl FnMut(String, String), base: &str, style: &Style) {
    let mut seen: Vec<&Vec<Press>> = Vec::new();
    let mut factored: Vec<&Vec<Press>> = Vec::new();
    let mut smash_set = false;
    let mut gauge_set = false;
    let mut levels: Vec<(String, &[StyleKind])> = vec![(format!("{base}.kinds"), &style.kinds)];
    for (field, option) in style_levels(style) {
        if let Some(o) = option {
            levels.push((format!("{base}.{field}[0].kinds"), &o.kinds));
        }
    }
    for (path, kinds) in levels {
        for (k, kind) in kinds.iter().enumerate().take(MAX_LEVEL_KINDS) {
            match kind {
                StyleKind::ReplaceStep { step, .. } => {
                    if seen.contains(&step) {
                        issue(
                            format!("{path}[{k}].step"),
                            "the step is already granted by an earlier level of the style".into(),
                        );
                    } else {
                        seen.push(step);
                    }
                }
                StyleKind::SmashEffect { .. } => {
                    if smash_set {
                        issue(
                            format!("{path}[{k}]"),
                            "the style already sets the smash effect".into(),
                        );
                    }
                    smash_set = true;
                }
                StyleKind::Repeat { .. } => {}
                StyleKind::ProcFactor { step, .. } => {
                    if factored.contains(&step) {
                        issue(
                            format!("{path}[{k}].step"),
                            "the step already has a proc factor from an earlier level of the style"
                                .into(),
                        );
                    } else {
                        factored.push(step);
                    }
                }
                StyleKind::Gauge(_) => {
                    if gauge_set {
                        issue(
                            format!("{path}[{k}]"),
                            "the style already names a gauge".into(),
                        );
                    }
                    gauge_set = true;
                }
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

/// The step the `presses` reach from the root of a combo tree.
pub(crate) fn step_mut<'a>(branches: &'a mut Branches, presses: &[Press]) -> Option<&'a mut Step> {
    let (first, rest) = presses.split_first()?;
    let step = match first {
        Press::Main => branches.main.as_deref_mut(),
        Press::Smash => branches.smash.as_deref_mut(),
    }?;
    if rest.is_empty() {
        Some(step)
    } else {
        step_mut(&mut step.next, rest)
    }
}

/// Applies one `replace_step` grant to the step of `weapon` that `presses` reach from the root
/// of the combo tree: the grant sets the step's charge (clearing its hold) or its hold
/// (clearing its charge). `false`, nothing changed, when no step is there.
pub(crate) fn apply_grant(weapon: &mut Weapon, presses: &[Press], grant: &StepGrant) -> bool {
    let Some(step) = step_mut(&mut weapon.combo, presses) else {
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
/// `base` with the grants a style gives when it is chosen and `taken` places of its path
/// (0 to 3: the first upgrade, the second, the capstone) are taken, in path order: the style's
/// own kinds, then each taken place's one choice. A grant that finds no step is skipped (the
/// pack check refuses it before a run).
pub(crate) fn weapon_with_style(base: &Weapon, style: &Style, taken: usize) -> Weapon {
    let mut weapon = base.clone();
    let places = style_levels(style)
        .into_iter()
        .take(taken)
        .filter_map(|(_, option)| option.map(|o| o.kinds.as_slice()));
    for kinds in std::iter::once(style.kinds.as_slice()).chain(places) {
        for kind in kinds {
            match kind {
                StyleKind::ReplaceStep { step, grant } => {
                    apply_grant(&mut weapon, step, grant);
                }
                StyleKind::SmashEffect { effect } => weapon.smash = Some(*effect),
                StyleKind::Repeat {
                    step,
                    after_ticks,
                    repeat_pct,
                    anchor,
                    proc_pct,
                } => {
                    if let Some(s) = step_mut(&mut weapon.combo, step) {
                        s.repeats.push(StepRepeat {
                            after_ticks: *after_ticks,
                            repeat_pct: u16::try_from(*repeat_pct).unwrap_or(u16::MAX),
                            anchor: *anchor,
                            proc_pct: proc_pct.map(|p| u16::try_from(p).unwrap_or(u16::MAX)),
                        });
                    }
                }
                StyleKind::ProcFactor { step, proc_pct } => {
                    if let Some(s) = step_mut(&mut weapon.combo, step) {
                        s.proc_pct = Some(u16::try_from(*proc_pct).unwrap_or(u16::MAX));
                    }
                }
                // The gauge changes no step: the rules read it beside the weapon (`World::gauge_of`).
                StyleKind::Gauge(_) => {}
            }
        }
    }
    weapon
}

/// Every attack a weapon's combo and dash trees can start: a step's, a charge level's (and its
/// perfect one), a hold's loop and release.
pub(crate) fn weapon_attacks(weapon: &Weapon) -> Vec<&AttackRef> {
    fn branches<'a>(b: &'a Branches, out: &mut Vec<&'a AttackRef>) {
        for step in [&b.main, &b.smash].into_iter().flatten() {
            out.push(&step.attack);
            if let Some(charge) = &step.charge {
                for level in &charge.levels {
                    out.push(&level.attack);
                    out.extend(level.perfect_attack.as_ref());
                }
            }
            if let Some(hold) = &step.hold {
                out.push(&hold.loop_attack);
                out.push(&hold.release);
            }
            branches(&step.next, out);
        }
    }
    let mut out = Vec::new();
    branches(&weapon.combo, &mut out);
    branches(&weapon.dash, &mut out);
    out
}
