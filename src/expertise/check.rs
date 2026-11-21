//! The validator: the file's own checks, the level and option checks, and the ID and set checks.

use super::kind_checks::{
    check_breaker, check_combo_tempo, check_counter_bonus, check_gauge, check_giant_trigger,
    check_proc_pct, check_spirit, check_step_len, check_step_once,
};
use super::types::set_stat_most;
use super::{
    ExpertiseEffect, ExpertiseFile, ExpertiseOption, MAX_CHARGE_SOONER_PCT, MAX_COMMON_OPTIONS,
    MAX_GROWTH_PCT, MAX_LEVEL_KINDS, MAX_LEVEL_POINTS, MAX_ORB_BREAK_PCT, MAX_REPEAT_AFTER_TICKS,
    MAX_REPEAT_BLEED_STACKS, MAX_SET_STATS, MAX_STYLE_CHOICES, REPEAT_BLEED_INTERVAL_TICKS, STYLES,
    SetStat, StyleKind, TABLE_LEVELS,
};
use crate::id::ContentId;
use crate::registry::{self, Kind};
use crate::rewards::{MAX_BURST_RADIUS_PX, MAX_RANGE_PCT};
use crate::validate::limits::MAX_DAMAGE;
use crate::validate::{self, Error, Issue};

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
                check_step_len(issue, &at, step);
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
            StyleKind::RepeatRangePct { pct } => {
                if !(1..=MAX_RANGE_PCT).contains(pct) {
                    issue(
                        format!("{at}.pct"),
                        format!("1 to {MAX_RANGE_PCT} (found {pct})"),
                    );
                }
            }
            StyleKind::RepeatProcPct { proc_pct } => check_proc_pct(issue, &at, Some(*proc_pct)),
            // The size stack's numbers are the base tree's own field: nothing to bound here.
            StyleKind::RepeatSizeStack {} => {}
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
