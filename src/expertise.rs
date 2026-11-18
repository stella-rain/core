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
//! step's charge or hold, a charge whose levels come sooner, a delayed repeat or a proc factor of
//! a step's hits, the weapon's smash
//! effect (`reap` among them) or a weapon gauge that fills from hits and damage taken and spends
//! on a full-gauge attack, or a spirit level that the full-gauge smash raises and a countdown
//! lowers, each level with its own effects, or a counter bonus that a near miss of the evade opens
//! and the first dash-branch step inside its window takes, or a combo tempo whose stacks, gained
//! from the swings that hit, speed the attacks the combo starts, or a giant trigger that the
//! full-gauge smash starts and that runs a time with a closed list of effects, or a breaker whose
//! closed list of effects scales the stagger and the poise damage the player deals, the damage of
//! a smash on a boss part and the burst round a target the player staggers; a path may name it at
//! several levels, the effects stacking in path order (ADR-SIM-9). A repeat and a factor are runtime-only fields of the
//! step on the style's copy of the tree.
//!
//! The file is untrusted (ADR-SVC-1), as the rewards are: a size limit before parsing, strict
//! parsing, the current versions only, and every field bounded. The thresholds rise strictly,
//! the factor is above 1, no pool is empty, every weapon preset has three complete styles, and
//! every number is capped. Option and style IDs are the file's own, unique within a weapon. The
//! bundled file, `content/expertise.json`, is the default; its styles are placeholders built
//! from the effect kinds `core` has.

use sha2::{Digest, Sha256};

use crate::attack::AttackRef;
use crate::event::Press;
use crate::fixed::Fx;
use crate::id::ContentId;
use crate::limits::{
    MAX_COUNTER_WINDOW_TICKS, MAX_GAUGE_DAMAGE_PCT, MAX_PCT, MAX_SPIRIT_DECAY_TICKS,
    MAX_SPIRIT_LEVEL, MAX_TEMPO_STACKS, MAX_TRIGGER_TICKS, MAX_WEAPON_GAUGE,
};
use crate::pack::Pack;
use crate::registry::{self, Kind};
use crate::rewards::{MAX_BURST_RADIUS_PX, MAX_RANGE_PCT};
use crate::run_stats::MAX_ATTACK_SPEED_PCT;
use crate::validate::limits::{MAX_COMBO_DEPTH, MAX_DAMAGE};
use crate::validate::{self, Error, Issue};
use crate::version::{self, Versions};
use crate::weapon::{Branches, RepeatBleed, Step, StepRepeat, Weapon};

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
/// The most effects one giant trigger kind names (a bound: one per `TriggerEffect` variant).
pub const MAX_TRIGGER_EFFECTS: usize = 6;
/// The most effects one combo tempo kind names (a bound: one per `TempoEffect` variant).
pub const MAX_TEMPO_EFFECTS: usize = 5;
/// The largest `pct` of a giant trigger's `hold_move_pct`: full speed, the `Hold.move_pct` ceiling.
pub const MAX_HOLD_MOVE_PCT: u32 = 100;
/// The longest delay of a `repeat` kind, in ticks (a tuning placeholder: 2 seconds).
pub const MAX_REPEAT_AFTER_TICKS: u32 = 120;
/// The largest `proc_pct` of a `repeat` or `proc_factor` kind: a factor is a chance (a
/// placeholder).
pub const MAX_PROC_PCT: u32 = 100;
/// The largest `chance_pct` of a `repeat_breaks_orbs` kind: a chance (a placeholder).
pub const MAX_ORB_BREAK_PCT: u32 = 100;
/// The most stacks one repeat of a `repeat_bleed` kind adds (a tuning placeholder: the spirit's cap).
pub const MAX_REPEAT_BLEED_STACKS: u32 = 4;
/// Ticks between two damage ticks of the repeat bleed: the runes bleed's own interval, not
/// content and not scaled by rune haste (it is no rune).
pub const REPEAT_BLEED_INTERVAL_TICKS: u32 = 20;
/// The largest `pct` of a `charge_sooner` kind: a share of the charge's ticks taken off (a bound,
/// not a design number).
pub const MAX_CHARGE_SOONER_PCT: u32 = 90;
/// The most repeats on one step, summed over one style's path (a tuning placeholder).
pub const MAX_STEP_REPEATS: usize = 4;
/// The most extra bleed stacks a spirit level's `smash_bleed_stacks` adds to a smash hit (a
/// tuning placeholder).
pub const MAX_SPIRIT_BLEED_STACKS: u32 = 4;
/// The most options in a weapon's common pool.
pub const MAX_COMMON_OPTIONS: usize = 32;
pub use crate::limits::MAX_SET_STATS;

mod types;

use types::set_stat_most;
pub use types::{
    Breaker, BreakerEffect, BurstEffect, ComboTempo, CounterBonus, CounterEffect, ExpertiseEffect,
    ExpertiseFile, ExpertiseOption, Gauge, GaugeSpend, GiantTrigger, SetStat, Spirit, SpiritEffect,
    StepGrant, Style, StyleKind, TempoEffect, TriggerEffect, WeaponExpertise,
};

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
                Earlier::default(),
            );
            check_set(&mut issue, &format!("{base}.stats"), &style.stats);
            let mut earlier = Earlier::default().with(&style.kinds);
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
                let before = earlier;
                for (c, choice) in choices.iter().enumerate() {
                    let path = format!("{path}[{c}]");
                    check_option(&mut issue, &mut seen, &path, choice, sim_version, before);
                    earlier = earlier.with(&choice.kinds);
                }
            }
            check_step_once(&mut issue, &base, style);
        }
        for (c, option) in weapon.common.iter().enumerate() {
            let path = at(&format!("common[{c}]"));
            check_option(
                &mut issue,
                &mut seen,
                &path,
                option,
                sim_version,
                Earlier::default(),
            );
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

/// The kinds an earlier level of the same style path already names: a later level naming one
/// again states only what it changes (ADR-SIM-9 points 2 to 7).
#[derive(Clone, Copy, Default)]
struct Earlier {
    spirit: bool,
    counter: bool,
    tempo: bool,
    trigger: bool,
}

impl Earlier {
    /// This set and every kind `kinds` names.
    fn with(mut self, kinds: &[StyleKind]) -> Self {
        for kind in kinds {
            match kind {
                StyleKind::Spirit(_) => self.spirit = true,
                StyleKind::CounterBonus(_) => self.counter = true,
                StyleKind::ComboTempoPct(_) => self.tempo = true,
                StyleKind::GiantTrigger(_) => self.trigger = true,
                _ => {}
            }
        }
        self
    }
}

fn check_option<'a>(
    issue: &mut impl FnMut(String, String),
    seen: &mut Vec<&'a ContentId>,
    path: &str,
    option: &'a ExpertiseOption,
    sim_version: u32,
    earlier: Earlier,
) {
    check_id(issue, seen, path, &option.id);
    check_level(
        issue,
        path,
        option.effect,
        option.amount,
        &option.kinds,
        sim_version,
        earlier,
    );
}

/// A level's stat and kinds: a stat effect has an amount from 1 to its most; `none` has none
/// and names at least one kind; the kinds, and `none`, are `sim_version` 2's, at most
/// `MAX_LEVEL_KINDS`, each bounded alone; `earlier` names the kinds a level before it on the
/// style path already named.
fn check_level(
    issue: &mut impl FnMut(String, String),
    path: &str,
    effect: ExpertiseEffect,
    amount: u32,
    kinds: &[StyleKind],
    sim_version: u32,
    earlier: Earlier,
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
            StyleKind::RepeatBreaksOrbs {
                chance_pct,
                radius_px,
            } => {
                if !(1..=MAX_ORB_BREAK_PCT).contains(chance_pct) {
                    issue(
                        format!("{at}.chance_pct"),
                        format!("1 to {MAX_ORB_BREAK_PCT} (found {chance_pct})"),
                    );
                }
                if !(1..=MAX_BURST_RADIUS_PX).contains(radius_px) {
                    issue(
                        format!("{at}.radius_px"),
                        format!("1 to {MAX_BURST_RADIUS_PX} (found {radius_px})"),
                    );
                }
            }
            StyleKind::RepeatBleed {
                stacks,
                duration_ticks,
                damage,
                max_stacks,
            } => {
                if !(1..=MAX_REPEAT_BLEED_STACKS).contains(stacks) {
                    issue(
                        format!("{at}.stacks"),
                        format!("1 to {MAX_REPEAT_BLEED_STACKS} (found {stacks})"),
                    );
                }
                let longest = crate::runes::MAX_DOT_DURATION_TICKS;
                if !(REPEAT_BLEED_INTERVAL_TICKS..=longest).contains(duration_ticks) {
                    issue(
                        format!("{at}.duration_ticks"),
                        format!(
                            "{REPEAT_BLEED_INTERVAL_TICKS} to {longest} (found {duration_ticks})"
                        ),
                    );
                }
                if !(1..=MAX_DAMAGE).contains(damage) {
                    issue(
                        format!("{at}.damage"),
                        format!("1 to {MAX_DAMAGE} (found {damage})"),
                    );
                }
                let most = crate::runes::MAX_DOT_STACKS;
                if *max_stacks < (*stacks).max(1) || *max_stacks > most {
                    issue(
                        format!("{at}.max_stacks"),
                        format!("{} to {most} (found {max_stacks})", (*stacks).max(1)),
                    );
                }
            }
            StyleKind::ProcFactor { step, proc_pct } => {
                check_step_len(issue, &at, step);
                check_proc_pct(issue, &at, Some(*proc_pct));
            }
            StyleKind::ChargeSooner { step, pct } => {
                check_step_len(issue, &at, step);
                if !(1..=MAX_CHARGE_SOONER_PCT).contains(pct) {
                    issue(
                        format!("{at}.pct"),
                        format!("1 to {MAX_CHARGE_SOONER_PCT} (found {pct})"),
                    );
                }
            }
            StyleKind::Gauge(g) => check_gauge(issue, &at, g),
            StyleKind::Spirit(s) => check_spirit(issue, &at, s),
            StyleKind::CounterBonus(c) => check_counter_bonus(issue, &at, c, earlier.counter),
            StyleKind::ComboTempoPct(t) => check_combo_tempo(issue, &at, t, earlier.tempo),
            StyleKind::GiantTrigger(t) => check_giant_trigger(issue, &at, t, earlier.trigger),
            StyleKind::Breaker(b) => check_breaker(issue, &at, b),
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

/// A spirit's numbers, each in its own range; a level names an effect once.
fn check_spirit(issue: &mut impl FnMut(String, String), at: &str, s: &Spirit) {
    if !(1..=u32::from(MAX_SPIRIT_DECAY_TICKS)).contains(&s.decay_ticks) {
        issue(
            format!("{at}.decay_ticks"),
            format!("1 to {MAX_SPIRIT_DECAY_TICKS} (found {})", s.decay_ticks),
        );
    }
    if s.levels.is_empty() || s.levels.len() > usize::from(MAX_SPIRIT_LEVEL) {
        issue(
            format!("{at}.levels"),
            format!("1 to {MAX_SPIRIT_LEVEL} levels (found {})", s.levels.len()),
        );
    }
    for (l, level) in s
        .levels
        .iter()
        .enumerate()
        .take(usize::from(MAX_SPIRIT_LEVEL))
    {
        let mut seen = [false; 3];
        for (e, effect) in level.iter().enumerate() {
            let at = format!("{at}.levels[{l}][{e}]");
            let (key, v, most) = match effect {
                SpiritEffect::DamagePct { pct } => ("pct", *pct, MAX_PCT),
                SpiritEffect::RangePct { pct } => ("pct", *pct, MAX_RANGE_PCT),
                SpiritEffect::SmashBleedStacks { stacks } => {
                    ("stacks", *stacks, MAX_SPIRIT_BLEED_STACKS)
                }
            };
            if !(1..=most).contains(&v) {
                issue(format!("{at}.{key}"), format!("1 to {most} (found {v})"));
            }
            let slot = &mut seen[usize::from(effect.name())];
            if *slot {
                issue(at, "the level already names this effect".into());
            }
            *slot = true;
        }
    }
}

/// A combo tempo's numbers, each in its own range; it names an effect once and, unless an
/// earlier level of the path named the kind (`later`), names `speed_pct` and keeps the stacks'
/// total speed within the attack speed ceiling (a later level's fold is `check_step_once`'s).
fn check_combo_tempo(
    issue: &mut impl FnMut(String, String),
    at: &str,
    t: &ComboTempo,
    later: bool,
) {
    let max = u64::from(MAX_TEMPO_STACKS);
    if !(1..=max).contains(&u64::from(t.max_stacks)) {
        issue(
            format!("{at}.max_stacks"),
            format!("1 to {MAX_TEMPO_STACKS} (found {})", t.max_stacks),
        );
    }
    let least = usize::from(!later);
    if t.effects.len() < least || t.effects.len() > MAX_TEMPO_EFFECTS {
        issue(
            format!("{at}.effects"),
            format!(
                "{least} to {MAX_TEMPO_EFFECTS} effects (found {})",
                t.effects.len()
            ),
        );
    }
    let mut seen = [false; MAX_TEMPO_EFFECTS];
    let mut speed = None;
    for (e, effect) in t.effects.iter().enumerate().take(MAX_TEMPO_EFFECTS) {
        let at = format!("{at}.effects[{e}]");
        if let TempoEffect::SpeedPct { pct } = effect {
            if !(1..=MAX_ATTACK_SPEED_PCT).contains(pct) {
                issue(
                    format!("{at}.pct"),
                    format!("1 to {MAX_ATTACK_SPEED_PCT} (found {pct})"),
                );
            }
            speed.get_or_insert(*pct);
        }
        if let TempoEffect::RangePctAtFull { pct } = effect
            && !(1..=MAX_RANGE_PCT).contains(pct)
        {
            issue(
                format!("{at}.pct"),
                format!("1 to {MAX_RANGE_PCT} (found {pct})"),
            );
        }
        let slot = &mut seen[usize::from(effect.name())];
        if *slot {
            issue(at, "the kind already names this effect".into());
        }
        *slot = true;
    }
    if !later && !t.effects.is_empty() && t.effects.len() <= MAX_TEMPO_EFFECTS {
        match speed {
            None => issue(format!("{at}.effects"), "names no speed_pct".into()),
            Some(pct) if (1..=MAX_ATTACK_SPEED_PCT).contains(&pct) => {
                let total = u64::from(t.max_stacks) * u64::from(pct);
                if total > u64::from(MAX_ATTACK_SPEED_PCT) {
                    issue(
                        format!("{at}.effects"),
                        format!(
                            "max_stacks times speed_pct is at most {MAX_ATTACK_SPEED_PCT} (found {total})"
                        ),
                    );
                }
            }
            Some(_) => {}
        }
    }
}

/// A giant trigger's numbers, each in its own range; it names an effect once, and names at
/// least one unless an earlier level of the path named the kind (`later`).
fn check_giant_trigger(
    issue: &mut impl FnMut(String, String),
    at: &str,
    t: &GiantTrigger,
    later: bool,
) {
    if !(1..=u32::from(MAX_TRIGGER_TICKS)).contains(&t.ticks) {
        issue(
            format!("{at}.ticks"),
            format!("1 to {MAX_TRIGGER_TICKS} (found {})", t.ticks),
        );
    }
    let least = usize::from(!later);
    if t.effects.len() < least || t.effects.len() > MAX_TRIGGER_EFFECTS {
        issue(
            format!("{at}.effects"),
            format!(
                "{least} to {MAX_TRIGGER_EFFECTS} effects (found {})",
                t.effects.len()
            ),
        );
    }
    let mut seen = [false; MAX_TRIGGER_EFFECTS];
    for (e, effect) in t.effects.iter().enumerate().take(MAX_TRIGGER_EFFECTS) {
        let at = format!("{at}.effects[{e}]");
        let bound = match effect {
            TriggerEffect::RangePct { pct } => Some((*pct, 1, MAX_RANGE_PCT)),
            TriggerEffect::SmashPct { pct } => Some((*pct, 1, MAX_PCT)),
            TriggerEffect::RallyPct { pct } => Some((*pct, 101, MAX_PCT)),
            TriggerEffect::HoldMovePct { pct } => Some((*pct, 1, MAX_HOLD_MOVE_PCT)),
            TriggerEffect::SuperArmour {} | TriggerEffect::EndRing {} => None,
        };
        if let Some((pct, lo, hi)) = bound
            && !(lo..=hi).contains(&pct)
        {
            issue(format!("{at}.pct"), format!("{lo} to {hi} (found {pct})"));
        }
        let slot = &mut seen[usize::from(effect.name())];
        if *slot {
            issue(at, "the kind already names this effect".into());
        }
        *slot = true;
    }
}

/// A breaker's numbers, each in its own range; it names an effect once, its burst too.
fn check_breaker(issue: &mut impl FnMut(String, String), at: &str, b: &Breaker) {
    if b.effects.is_empty() || b.effects.len() > 5 {
        issue(
            format!("{at}.effects"),
            format!("1 to 5 effects (found {})", b.effects.len()),
        );
    }
    let mut seen = [false; 5];
    for (e, effect) in b.effects.iter().enumerate().take(5) {
        let at = format!("{at}.effects[{e}]");
        match effect {
            BreakerEffect::StaggerPct { pct }
            | BreakerEffect::PoisePct { pct }
            | BreakerEffect::PartBreakPct { pct }
            | BreakerEffect::StaggerTicksPct { pct } => {
                if !(101..=MAX_PCT).contains(pct) {
                    issue(
                        format!("{at}.pct"),
                        format!("101 to {MAX_PCT} (found {pct})"),
                    );
                }
            }
            BreakerEffect::OnStaggerBurst { radius_px, effects } => {
                if !(1..=MAX_BURST_RADIUS_PX).contains(radius_px) {
                    issue(
                        format!("{at}.radius_px"),
                        format!("1 to {MAX_BURST_RADIUS_PX} (found {radius_px})"),
                    );
                }
                check_burst(issue, &at, effects);
            }
        }
        let slot = &mut seen[usize::from(effect.name())];
        if *slot {
            issue(at, "the kind already names this effect".into());
        }
        *slot = true;
    }
}

/// A burst's effects: 1 to 3, each in its own range, each variant once.
fn check_burst(issue: &mut impl FnMut(String, String), at: &str, effects: &[BurstEffect]) {
    if effects.is_empty() || effects.len() > 3 {
        issue(
            format!("{at}.effects"),
            format!("1 to 3 effects (found {})", effects.len()),
        );
    }
    let mut seen = [false; 3];
    for (e, effect) in effects.iter().enumerate().take(3) {
        let at = format!("{at}.effects[{e}]");
        match effect {
            BurstEffect::DamagePct { pct } => {
                if !(1..=MAX_PCT).contains(pct) {
                    issue(format!("{at}.pct"), format!("1 to {MAX_PCT} (found {pct})"));
                }
            }
            BurstEffect::StunTicks { ticks } => {
                let max = validate::limits::MAX_STUN_TICKS;
                if !(1..=max).contains(ticks) {
                    issue(format!("{at}.ticks"), format!("1 to {max} (found {ticks})"));
                }
            }
            BurstEffect::KnockDown {} => {}
        }
        let slot = &mut seen[usize::from(effect.name())];
        if *slot {
            issue(at, "the burst already names this effect".into());
        }
        *slot = true;
    }
}

/// A counter bonus's numbers, each in its own range; it names an effect once, and names at
/// least one unless an earlier level of the path named the kind (`later`).
fn check_counter_bonus(
    issue: &mut impl FnMut(String, String),
    at: &str,
    c: &CounterBonus,
    later: bool,
) {
    if !(1..=u32::from(MAX_COUNTER_WINDOW_TICKS)).contains(&c.window_ticks) {
        issue(
            format!("{at}.window_ticks"),
            format!("1 to {MAX_COUNTER_WINDOW_TICKS} (found {})", c.window_ticks),
        );
    }
    let least = usize::from(!later);
    if c.effects.len() < least || c.effects.len() > 6 {
        issue(
            format!("{at}.effects"),
            format!("{least} to 6 effects (found {})", c.effects.len()),
        );
    }
    let mut seen = [false; 6];
    for (e, effect) in c.effects.iter().enumerate().take(6) {
        let at = format!("{at}.effects[{e}]");
        let (key, v, lo, hi) = match effect {
            CounterEffect::RefundEvade {} => {
                if std::mem::replace(&mut seen[usize::from(effect.name())], true) {
                    issue(at, "the kind already names this effect".into());
                }
                continue;
            }
            CounterEffect::GaugeFillPct { pct } => ("pct", i64::from(*pct), 1, 100),
            CounterEffect::DamagePct { pct } => ("pct", i64::from(*pct), 1, i64::from(MAX_PCT)),
            CounterEffect::StaggerPct { pct } => ("pct", i64::from(*pct), 101, i64::from(MAX_PCT)),
            CounterEffect::StunTicks { ticks } => (
                "ticks",
                i64::from(*ticks),
                1,
                i64::from(validate::limits::MAX_STUN_TICKS),
            ),
            CounterEffect::Advance { distance } => (
                "distance",
                i64::from(distance.0),
                i64::from(Fx::px(1).0),
                i64::from(Fx::px(crate::limits::MAX_ADVANCE_DISTANCE).0),
            ),
        };
        if !(lo..=hi).contains(&v) {
            issue(format!("{at}.{key}"), format!("{lo} to {hi} (found {v})"));
        }
        let slot = &mut seen[usize::from(effect.name())];
        if *slot {
            issue(at, "the kind already names this effect".into());
        }
        *slot = true;
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

/// One grant on a step per style, and one smash effect and one gauge: a second grant on a step an
/// earlier level of the same style granted, or a second smash effect or gauge, is refused at the
/// later one. The spirit, counter bonus, combo tempo and giant trigger are named once per level
/// and may be named at every level of the path.
fn check_step_once(issue: &mut impl FnMut(String, String), base: &str, style: &Style) {
    let mut seen: Vec<&Vec<Press>> = Vec::new();
    let mut factored: Vec<&Vec<Press>> = Vec::new();
    let mut hastened: Vec<&Vec<Press>> = Vec::new();
    let mut smash_set = false;
    let mut orbs_set = false;
    let mut bleed_set = false;
    let mut gauge_set = false;
    // The tempo's folded `max_stacks` and `speed_pct` over the levels so far, once one named it.
    let mut tempo: Option<(u32, u32)> = None;
    let mut levels: Vec<(String, &[StyleKind])> = vec![(format!("{base}.kinds"), &style.kinds)];
    for (field, option) in style_levels(style) {
        if let Some(o) = option {
            levels.push((format!("{base}.{field}[0].kinds"), &o.kinds));
        }
    }
    for (path, kinds) in levels {
        // A path may name the breaker, spirit, counter bonus, combo tempo and giant trigger at
        // every level (their numbers stack in path order), a level only once.
        let mut breaker_here = false;
        let mut spirit_here = false;
        let mut counter_here = false;
        let mut tempo_here = false;
        let mut trigger_here = false;
        for (k, kind) in kinds.iter().enumerate().take(MAX_LEVEL_KINDS) {
            match kind {
                StyleKind::Breaker(_) => {
                    if breaker_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names a breaker".into(),
                        );
                    }
                    breaker_here = true;
                }
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
                StyleKind::RepeatBreaksOrbs { .. } => {
                    if orbs_set {
                        issue(
                            format!("{path}[{k}]"),
                            "the style already breaks orbs".into(),
                        );
                    }
                    orbs_set = true;
                }
                StyleKind::RepeatBleed { .. } => {
                    if bleed_set {
                        issue(
                            format!("{path}[{k}]"),
                            "the style already bleeds on its repeats".into(),
                        );
                    }
                    bleed_set = true;
                }
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
                StyleKind::ChargeSooner { step, .. } => {
                    if hastened.contains(&step) {
                        issue(
                            format!("{path}[{k}].step"),
                            "the step's charge is already made sooner by an earlier level of the \
                             style"
                                .into(),
                        );
                    } else {
                        hastened.push(step);
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
                StyleKind::Spirit(_) => {
                    if spirit_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names a spirit".into(),
                        );
                    }
                    spirit_here = true;
                }
                StyleKind::CounterBonus(_) => {
                    if counter_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names a counter bonus".into(),
                        );
                    }
                    counter_here = true;
                }
                StyleKind::ComboTempoPct(t) => {
                    if tempo_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names a combo tempo".into(),
                        );
                    }
                    tempo_here = true;
                    check_tempo_fold(issue, &format!("{path}[{k}]"), t, &mut tempo);
                }
                StyleKind::GiantTrigger(_) => {
                    if trigger_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names a giant trigger".into(),
                        );
                    }
                    trigger_here = true;
                }
            }
        }
    }
}

/// A later level's combo tempo laid over the folded one: `max_stacks` never falls (the stacks
/// held never pass the cap) and `max_stacks` times `speed_pct` stays within the attack speed
/// ceiling at every place of the path, refused at the level that breaks it. The first naming is
/// `check_combo_tempo`'s.
fn check_tempo_fold(
    issue: &mut impl FnMut(String, String),
    at: &str,
    t: &ComboTempo,
    folded: &mut Option<(u32, u32)>,
) {
    let speed = t.effects.iter().find_map(|e| match e {
        TempoEffect::SpeedPct { pct } => Some(*pct),
        _ => None,
    });
    let Some((max, pct)) = *folded else {
        *folded = Some((t.max_stacks, speed.unwrap_or(0)));
        return;
    };
    if t.max_stacks < max {
        issue(
            format!("{at}.max_stacks"),
            format!(
                "at least the earlier level's {max} (found {})",
                t.max_stacks
            ),
        );
    }
    let max = max.max(t.max_stacks);
    let pct = speed
        .filter(|p| (1..=MAX_ATTACK_SPEED_PCT).contains(p))
        .unwrap_or(pct);
    let total = u64::from(max) * u64::from(pct);
    if total > u64::from(MAX_ATTACK_SPEED_PCT) {
        let key = if speed.is_some() {
            "effects"
        } else {
            "max_stacks"
        };
        issue(
            format!("{at}.{key}"),
            format!(
                "max_stacks times speed_pct, folded over the path, is at most {MAX_ATTACK_SPEED_PCT} (found {total})"
            ),
        );
    }
    *folded = Some((max, pct));
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

/// A level's `at_ticks` made `pct` percent sooner: `at * (100 - pct) / 100`, floored, at least 1.
/// A `pct` past 100 counts as 100 (the validator holds it to `MAX_CHARGE_SOONER_PCT`).
pub(crate) fn sooner_at(at: u32, pct: u32) -> u32 {
    let kept = u64::from(100 - pct.min(100));
    u32::try_from(u64::from(at) * kept / 100)
        .unwrap_or(u32::MAX)
        .max(1)
}

/// Makes each level of the charge on the step `presses` reach `pct` percent sooner. `false`,
/// nothing changed, when no step is there or it has no charge.
pub(crate) fn charge_sooner(weapon: &mut Weapon, presses: &[Press], pct: u32) -> bool {
    let Some(charge) = step_mut(&mut weapon.combo, presses).and_then(|s| s.charge.as_mut()) else {
        return false;
    };
    for level in &mut charge.levels {
        level.at_ticks = sooner_at(level.at_ticks, pct);
    }
    true
}

/// `base` with the grants a style gives when it is chosen and `taken` places of its path
/// (0 to 3: the first upgrade, the second, the capstone) are taken, in path order: the style's
/// own kinds, then each taken place's one choice. A grant that finds no step, or a
/// `charge_sooner` that finds no charge, is skipped (the pack check refuses it before a run). The
/// charge's levels come sooner here, at world build: no state, no hash field.
pub(crate) fn weapon_with_style(base: &Weapon, style: &Style, taken: usize) -> Weapon {
    let mut weapon = base.clone();
    let mut bleed = None;
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
                            bleed: None,
                        });
                    }
                }
                // Nor does the orb-breaking kind change a step: `World::repeat_break_of`.
                StyleKind::RepeatBreaksOrbs { .. } => {}
                // The repeat bleed is stamped on every repeat once all the kinds are read.
                StyleKind::RepeatBleed {
                    stacks,
                    duration_ticks,
                    damage,
                    max_stacks,
                } => {
                    bleed = Some(RepeatBleed {
                        stacks: u8::try_from(*stacks).unwrap_or(u8::MAX),
                        duration_ticks: u16::try_from(*duration_ticks).unwrap_or(u16::MAX),
                        damage: (*damage).min(MAX_DAMAGE),
                        max_stacks: u8::try_from(*max_stacks).unwrap_or(u8::MAX),
                    });
                }
                StyleKind::ProcFactor { step, proc_pct } => {
                    if let Some(s) = step_mut(&mut weapon.combo, step) {
                        s.proc_pct = Some(u16::try_from(*proc_pct).unwrap_or(u16::MAX));
                    }
                }
                // A path with no charge at this place is refused by the pack check.
                StyleKind::ChargeSooner { step, pct } => {
                    charge_sooner(&mut weapon, step, *pct);
                }
                // The gauge changes no step: the rules read it beside the weapon (`World::gauge_of`).
                StyleKind::Gauge(_) => {}
                // Nor does the spirit: the rules read it beside the weapon (`World::spirit_of`).
                StyleKind::Spirit(_) => {}
                // Nor does the counter bonus (`World::counter_of`).
                StyleKind::CounterBonus(_) => {}
                // Nor does the combo tempo (`World::tempo_of`).
                StyleKind::ComboTempoPct(_) => {}
                StyleKind::GiantTrigger(_) => {}
                // Nor does the breaker (`World::breaker_of`).
                StyleKind::Breaker(_) => {}
            }
        }
    }
    // A repeat granted by a later level carries the bleed too: it is read when the repeat is
    // queued, so a path that has taken the kind bleeds on every repeat it holds.
    if bleed.is_some() {
        for_each_step_mut(&mut weapon.combo, &mut |step| {
            for repeat in &mut step.repeats {
                repeat.bleed = bleed;
            }
        });
    }
    weapon
}

/// Runs `f` on every step of a combo tree, a step before the steps it reaches.
fn for_each_step_mut(branches: &mut Branches, f: &mut impl FnMut(&mut Step)) {
    for step in [branches.main.as_deref_mut(), branches.smash.as_deref_mut()]
        .into_iter()
        .flatten()
    {
        f(step);
        for_each_step_mut(&mut step.next, f);
    }
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
