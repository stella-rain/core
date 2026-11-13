//! Count budgets and cross-file checks of a pack.

use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::value::RawValue;

use super::{
    MAX_ATTACK_PRESETS, MAX_BOSS_TICKS, MAX_CONSTELLATIONS, MAX_DAMAGE_OVER_TIME_RUNES,
    MAX_DIFFICULTIES, MAX_ELITE_TABLE_TIERS, MAX_ELITE_TYPE_DEFINITIONS, MAX_EXPERTISE_COMMON,
    MAX_EXPERTISE_IDS, MAX_EXPERTISE_WEAPONS, MAX_MID_BOSS_POOL, MAX_ON_HIT_RUNES, MAX_PERK_SETS,
    MAX_RARITIES, MAX_REWARDS, MAX_RUN_STATS, MAX_STAGE_SKILLS, MAX_STARDUST_MODES,
    MAX_STARTING_UNLOCKS, MAX_TIME_LIMIT_WAVES, MAX_TREES, MAX_WEAPONS, PackError, PackFile,
};
use crate::attack_presets::AttackPresets;
use crate::difficulties::Difficulties;
use crate::expertise::{
    Expertise, GaugeSpend, MAX_STEP_REPEATS, Spirit, SpiritEffect, StyleKind, apply_grant,
    step_mut, style_levels, weapon_attacks,
};
use crate::perk_sets::PerkSets;
use crate::rewards::Rewards;
use crate::run_setup::{DEFAULT_DIFFICULTY, DEFAULT_PERK_SET};
use crate::star_chart;
use crate::time_limit::TimeLimit;
use crate::validate::{self, Error, Issue};
use crate::weapons::Weapons;

/// The counts of a file's lists, read leniently: a file of the wrong shape is left to its own
/// parser, which refuses it.
fn counts<'a, T: Deserialize<'a>>(raw: &'a RawValue) -> Option<T> {
    serde_json::from_str(raw.get()).ok()
}

#[derive(Deserialize)]
struct ModeLists {
    #[serde(default)]
    modes: Vec<IgnoredAny>,
}

#[derive(Deserialize)]
struct ColourLists {
    #[serde(default)]
    colours: Vec<IgnoredAny>,
}

/// The lists of the star chart file, counted.
#[derive(Deserialize)]
struct StarChartLists {
    #[serde(default)]
    weapons: Vec<IgnoredAny>,
    #[serde(default)]
    perk_sets: Vec<IgnoredAny>,
    #[serde(default)]
    stage_skills: Vec<IgnoredAny>,
    #[serde(default)]
    stats: Vec<StatPriceLists>,
    #[serde(default)]
    styles: Vec<StyleRowLists>,
    #[serde(default)]
    expertise_multiplier: Vec<IgnoredAny>,
    #[serde(default)]
    lumine_levels: Vec<IgnoredAny>,
}

#[derive(Deserialize)]
struct StyleRowLists {
    #[serde(default)]
    styles: Vec<IgnoredAny>,
}

#[derive(Deserialize)]
struct StatPriceLists {
    #[serde(default)]
    prices: Vec<IgnoredAny>,
}

#[derive(Deserialize)]
struct ConstellationLists {
    #[serde(default)]
    constellations: Vec<IgnoredAny>,
}

#[derive(Deserialize)]
struct Lists {
    #[serde(default)]
    stats: Vec<IgnoredAny>,
    #[serde(default)]
    rewards: Vec<IgnoredAny>,
    #[serde(default)]
    rarities: Vec<IgnoredAny>,
    #[serde(default)]
    stage_skills: Vec<IgnoredAny>,
    #[serde(default)]
    perk_sets: Vec<IgnoredAny>,
    #[serde(default)]
    difficulties: Vec<IgnoredAny>,
    #[serde(default)]
    weapons: Vec<WeaponLists>,
    #[serde(default)]
    on_hit: Vec<IgnoredAny>,
    #[serde(default)]
    damage_over_time: Vec<IgnoredAny>,
    #[serde(default)]
    waves: Vec<IgnoredAny>,
    #[serde(default)]
    boss_ticks: Vec<IgnoredAny>,
    #[serde(default)]
    mid_boss_pool: Vec<IgnoredAny>,
    #[serde(default)]
    elite_table: Vec<IgnoredAny>,
    #[serde(default)]
    elite_types: Vec<IgnoredAny>,
}

/// The lists of the starting unlocks file, counted.
#[derive(Deserialize)]
struct UnlockLists {
    #[serde(default)]
    weapons: Vec<IgnoredAny>,
    #[serde(default)]
    perk_sets: Vec<IgnoredAny>,
    #[serde(default)]
    stage_skills: Vec<IgnoredAny>,
    #[serde(default)]
    runes: Vec<IgnoredAny>,
}

/// The lists of the weapons file, counted.
#[derive(Deserialize)]
struct AttackPresetsLists {
    #[serde(default)]
    presets: Vec<IgnoredAny>,
}

/// The lists of the weapons file, counted.
#[derive(Deserialize)]
struct WeaponsLists {
    #[serde(default)]
    weapons: Vec<TreeLists>,
}

#[derive(Deserialize)]
struct TreeLists {
    #[serde(default)]
    trees: Vec<IgnoredAny>,
}

#[derive(Deserialize)]
struct WeaponLists {
    #[serde(default)]
    common: Vec<IgnoredAny>,
}

/// The held-step checks that read a preset's phases, for every tree of the pack's weapons against
/// the pack's own attack presets (ADR-CONT-3 point 6, CS1b-1): the weapons file alone cannot say.
pub(super) fn check_holds(weapons: &Weapons, presets: &AttackPresets) -> Result<(), PackError> {
    let mut issues = Vec::new();
    for (w, entry) in weapons.list().iter().enumerate() {
        for (t, tree) in entry.trees.iter().enumerate() {
            for found in crate::presets::hold_problems(&tree.weapon, presets) {
                issues.push(validate::Issue {
                    path: format!(
                        "$.weapons[{w}].trees[{t}].weapon{}",
                        found.path.trim_start_matches('$')
                    ),
                    message: found.message,
                });
            }
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(PackError::File {
            file: "weapons",
            error: validate::Error::Invalid(issues),
        })
    }
}

/// A style's grants against the weapon's base tree (ADR-SIM-9 points 2 and 3), for every weapon of
/// the expertise file that has a base tree in the weapons file (an inline weapon has none, and a
/// weapon with no tree is skipped). The grants are applied in path order, the style's own kinds
/// and then each upgrade's and the capstone's one choice, to a copy of the tree: a step that is
/// no step of the tree is refused at its `step`, and every issue the weapon validator or the
/// preset hold check finds on the tree the grant made is refused at the grant.
pub(super) fn check_style_grants(
    expertise: &Expertise,
    weapons: &Weapons,
    presets: &AttackPresets,
) -> Result<(), PackError> {
    let mut issues = Vec::new();
    for (w, we) in expertise.weapons().iter().enumerate() {
        let Some(base) = weapons.tree(we.weapon.as_str(), None) else {
            continue;
        };
        for (s, style) in we.styles.iter().enumerate() {
            let at = format!("$.weapons[{w}].styles[{s}]");
            let mut levels = vec![(format!("{at}.kinds"), style.kinds.as_slice())];
            for (field, option) in style_levels(style) {
                if let Some(o) = option {
                    levels.push((format!("{at}.{field}[0].kinds"), o.kinds.as_slice()));
                }
            }
            let mut tree = base.clone();
            // Each repeat kind's path and step, for the checks that need the whole path applied.
            let mut repeats: Vec<(String, &Vec<crate::event::Press>)> = Vec::new();
            let mut gauges: Vec<(String, &crate::attack::AttackRef)> = Vec::new();
            let mut spirits: Vec<(String, &Spirit)> = Vec::new();
            'style: for (path, kinds) in levels {
                for (k, kind) in kinds.iter().enumerate() {
                    let at = format!("{path}[{k}]");
                    let field = match kind {
                        StyleKind::ReplaceStep { step, grant } => {
                            if !apply_grant(&mut tree, step, grant) {
                                issues.push(validate::Issue {
                                    path: format!("{at}.step"),
                                    message: "no such step in the weapon's base tree".into(),
                                });
                                break 'style;
                            }
                            "grant"
                        }
                        StyleKind::SmashEffect { effect } => {
                            tree.smash = Some(*effect);
                            "effect"
                        }
                        StyleKind::Repeat { step, .. } | StyleKind::ProcFactor { step, .. } => {
                            if step_mut(&mut tree.combo, step).is_none() {
                                issues.push(validate::Issue {
                                    path: format!("{at}.step"),
                                    message: "no such step in the weapon's base tree".into(),
                                });
                                break 'style;
                            }
                            if matches!(kind, StyleKind::Repeat { .. }) {
                                let same = repeats.iter().filter(|(_, s)| *s == step).count();
                                if same >= MAX_STEP_REPEATS {
                                    issues.push(validate::Issue {
                                        path: at.clone(),
                                        message: format!(
                                            "at most {MAX_STEP_REPEATS} repeats on one step"
                                        ),
                                    });
                                } else {
                                    repeats.push((at.clone(), step));
                                }
                            }
                            continue;
                        }
                        StyleKind::Gauge(g) => {
                            let GaugeSpend::FullSmash { attack } = &g.spend;
                            gauges.push((format!("{at}.spend.attack"), attack));
                            continue;
                        }
                        StyleKind::Spirit(s) => {
                            spirits.push((at.clone(), s));
                            continue;
                        }
                        // The counter bonus needs the run's perfect evade and the weapon's dash
                        // branch, which this check cannot see: a pack lacking either has a
                        // counter that never opens.
                        StyleKind::CounterBonus(_) => continue,
                    };
                    let found = validate::validate_weapon_preset(&tree, crate::registry::ENTRIES)
                        .err()
                        .into_iter()
                        .flatten()
                        .chain(crate::presets::hold_problems(&tree, presets))
                        .collect::<Vec<_>>();
                    if !found.is_empty() {
                        issues.extend(found.into_iter().map(|f| validate::Issue {
                            path: format!("{at}.{field}"),
                            message: format!("{} (the tree at {})", f.message, f.path),
                        }));
                        break 'style;
                    }
                }
            }
            // A held step does not repeat: the grants may come after the repeat, so the check
            // reads the tree the whole path made.
            for (at, step) in repeats {
                if step_mut(&mut tree.combo, step).is_some_and(|s| s.hold.is_some()) {
                    issues.push(validate::Issue {
                        path: at,
                        message: "a held step does not repeat".into(),
                    });
                }
            }
            // The spirit is raised by the full-gauge smash and its extra bleed stacks ride a bleed
            // smash: both read the tree and the gauges the whole path made.
            for (at, spirit) in spirits {
                if gauges.is_empty() {
                    issues.push(validate::Issue {
                        path: at.clone(),
                        message: "nothing raises the level: the style names no full-smash gauge"
                            .into(),
                    });
                }
                for (l, level) in spirit.levels.iter().enumerate() {
                    for (e, effect) in level.iter().enumerate() {
                        if matches!(effect, SpiritEffect::SmashBleedStacks { .. })
                            && tree.smash != Some(crate::weapon::SmashEffect::Bleed {})
                        {
                            issues.push(validate::Issue {
                                path: format!("{at}.levels[{l}][{e}]"),
                                message: "needs a bleed smash".into(),
                            });
                        }
                    }
                }
            }
            // A full-gauge attack equal to an attack the style's tree can play would clear orbs
            // as an ordinary step (the run tells it apart by its compiled index alone).
            let played = weapon_attacks(base)
                .into_iter()
                .chain(weapon_attacks(&tree));
            for (path, attack) in gauges {
                if played.clone().any(|a| a == attack) {
                    issues.push(validate::Issue {
                        path,
                        message: "the same as an attack of the base tree".into(),
                    });
                }
            }
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(PackError::File {
            file: "expertise",
            error: validate::Error::Invalid(issues),
        })
    }
}

fn budget(path: impl Into<String>, found: usize, limit: usize) -> Result<(), PackError> {
    if found > limit {
        Err(PackError::Budget {
            path: path.into(),
            limit,
            found,
        })
    } else {
        Ok(())
    }
}

pub(super) fn check_budgets(pack: &PackFile) -> Result<(), PackError> {
    let introduced = &pack.introduced;
    budget(
        "$.introduced.run_stats",
        introduced.run_stats.len(),
        MAX_RUN_STATS,
    )?;
    budget(
        "$.introduced.rewards",
        introduced.rewards.len(),
        MAX_REWARDS,
    )?;
    budget(
        "$.introduced.stage_skills",
        introduced.stage_skills.len(),
        MAX_STAGE_SKILLS,
    )?;
    budget(
        "$.introduced.perk_sets",
        introduced.perk_sets.len(),
        MAX_PERK_SETS,
    )?;
    budget(
        "$.introduced.difficulties",
        introduced.difficulties.len(),
        MAX_DIFFICULTIES,
    )?;
    budget(
        "$.introduced.expertise",
        introduced.expertise.len(),
        MAX_EXPERTISE_WEAPONS,
    )?;
    if let Some(listed) = &introduced.constellations {
        budget(
            "$.introduced.constellations",
            listed.len(),
            MAX_CONSTELLATIONS,
        )?;
    }
    for (w, weapon) in introduced.expertise.iter().enumerate() {
        budget(
            format!("$.introduced.expertise[{w}].ids"),
            weapon.ids.len(),
            MAX_EXPERTISE_IDS,
        )?;
    }
    if let Some(l) = counts::<Lists>(&pack.run_stats) {
        budget("$.run_stats.stats", l.stats.len(), MAX_RUN_STATS)?;
    }
    if let Some(l) = counts::<Lists>(&pack.rewards) {
        budget("$.rewards.rewards", l.rewards.len(), MAX_REWARDS)?;
        budget("$.rewards.rarities", l.rarities.len(), MAX_RARITIES)?;
    }
    if let Some(l) = counts::<Lists>(&pack.stage_skills) {
        budget(
            "$.stage_skills.stage_skills",
            l.stage_skills.len(),
            MAX_STAGE_SKILLS,
        )?;
    }
    if let Some(l) = counts::<Lists>(&pack.expertise) {
        budget(
            "$.expertise.weapons",
            l.weapons.len(),
            MAX_EXPERTISE_WEAPONS,
        )?;
        for (w, weapon) in l.weapons.iter().enumerate() {
            budget(
                format!("$.expertise.weapons[{w}].common"),
                weapon.common.len(),
                MAX_EXPERTISE_COMMON,
            )?;
        }
    }
    if let Some(l) = counts::<Lists>(&pack.runes) {
        budget("$.runes.on_hit", l.on_hit.len(), MAX_ON_HIT_RUNES)?;
        budget(
            "$.runes.damage_over_time",
            l.damage_over_time.len(),
            MAX_DAMAGE_OVER_TIME_RUNES,
        )?;
    }
    if let Some(l) = counts::<Lists>(&pack.perk_sets) {
        budget("$.perk_sets.perk_sets", l.perk_sets.len(), MAX_PERK_SETS)?;
    }
    if let Some(l) = counts::<Lists>(&pack.difficulties) {
        budget(
            "$.difficulties.difficulties",
            l.difficulties.len(),
            MAX_DIFFICULTIES,
        )?;
    }
    if let Some(l) = counts::<AttackPresetsLists>(&pack.attack_presets) {
        budget(
            "$.attack_presets.presets",
            l.presets.len(),
            MAX_ATTACK_PRESETS,
        )?;
    }
    if let Some(l) = counts::<WeaponsLists>(&pack.weapons) {
        budget("$.weapons.weapons", l.weapons.len(), MAX_WEAPONS)?;
        for (w, weapon) in l.weapons.iter().enumerate() {
            budget(
                format!("$.weapons.weapons[{w}].trees"),
                weapon.trees.len(),
                MAX_TREES,
            )?;
        }
    }
    if let Some(l) = pack
        .starting_unlocks
        .as_deref()
        .and_then(counts::<UnlockLists>)
    {
        for (name, found) in [
            ("weapons", l.weapons.len()),
            ("perk_sets", l.perk_sets.len()),
            ("stage_skills", l.stage_skills.len()),
            ("runes", l.runes.len()),
        ] {
            budget(
                format!("$.starting_unlocks.{name}"),
                found,
                MAX_STARTING_UNLOCKS,
            )?;
        }
    }
    if let Some(l) = pack
        .threat_colours
        .as_deref()
        .and_then(counts::<ColourLists>)
    {
        budget(
            "$.threat_colours.colours",
            l.colours.len(),
            crate::threat_colours::MAX_THREAT_COLOURS,
        )?;
    }
    if let Some(l) = pack.stardust.as_deref().and_then(counts::<ModeLists>) {
        budget("$.stardust.modes", l.modes.len(), MAX_STARDUST_MODES)?;
    }
    if let Some(l) = pack
        .star_chart
        .as_deref()
        .and_then(counts::<StarChartLists>)
    {
        for (name, found) in [
            ("weapons", l.weapons.len()),
            ("perk_sets", l.perk_sets.len()),
            ("stage_skills", l.stage_skills.len()),
            ("stats", l.stats.len()),
            ("styles", l.styles.len()),
        ] {
            budget(format!("$.star_chart.{name}"), found, star_chart::MAX_ITEMS)?;
        }
        for (i, row) in l.styles.iter().enumerate() {
            budget(
                format!("$.star_chart.styles[{i}].styles"),
                row.styles.len(),
                crate::run_setup::MAX_UNLOCKED_STYLES,
            )?;
        }
        for (i, stat) in l.stats.iter().enumerate() {
            budget(
                format!("$.star_chart.stats[{i}].prices"),
                stat.prices.len(),
                star_chart::MAX_STAT_LEVELS,
            )?;
        }
        budget(
            "$.star_chart.expertise_multiplier",
            l.expertise_multiplier.len(),
            star_chart::MAX_LEVELS,
        )?;
        budget(
            "$.star_chart.lumine_levels",
            l.lumine_levels.len(),
            star_chart::MAX_LEVELS,
        )?;
    }
    if let Some(l) = pack
        .constellations
        .as_deref()
        .and_then(counts::<ConstellationLists>)
    {
        budget(
            "$.constellations.constellations",
            l.constellations.len(),
            MAX_CONSTELLATIONS,
        )?;
    }
    if let Some(l) = counts::<Lists>(&pack.modes.time_limit) {
        budget(
            "$.modes.time_limit.waves",
            l.waves.len(),
            MAX_TIME_LIMIT_WAVES,
        )?;
        budget(
            "$.modes.time_limit.boss_ticks",
            l.boss_ticks.len(),
            MAX_BOSS_TICKS,
        )?;
        budget(
            "$.modes.time_limit.mid_boss_pool",
            l.mid_boss_pool.len(),
            MAX_MID_BOSS_POOL,
        )?;
        budget(
            "$.modes.time_limit.elite_table",
            l.elite_table.len(),
            MAX_ELITE_TABLE_TIERS,
        )?;
        budget(
            "$.modes.time_limit.elite_types",
            l.elite_types.len(),
            MAX_ELITE_TYPE_DEFINITIONS,
        )?;
    }
    Ok(())
}

/// A run at `sim_version` 2 or later needs the rewards file's equipment rules; the run start
/// refuses a pack without them, so the pack is refused at load (the same text and condition).
pub(super) fn check_equipment_rules(sim_version: u32, rewards: &Rewards) -> Result<(), PackError> {
    if sim_version >= 2 && rewards.equipment().is_none() {
        return Err(PackError::File {
            file: "rewards",
            error: Error::Invalid(vec![Issue {
                path: "$.equipment".into(),
                message: "sim_version 2 needs the rewards file's equipment rules".into(),
            }]),
        });
    }
    Ok(())
}

/// A difficulty's own boss times are at most one more than the `time_limit` mode's pool has
/// bosses (one mid-boss for each time but the last, each pool entry once): the run start
/// refuses more when that difficulty is chosen, so the pack is refused at load. A difficulty
/// without `boss_ticks` uses the mode's own list, which the mode file checks.
pub(super) fn check_difficulty_boss_times(
    difficulties: &Difficulties,
    time_limit: &TimeLimit,
) -> Result<(), PackError> {
    let pool = time_limit.mid_boss_pool().len();
    let issues: Vec<Issue> = difficulties
        .list()
        .iter()
        .enumerate()
        .filter(|(_, d)| {
            d.boss_ticks
                .as_ref()
                .is_some_and(|t| t.len().saturating_sub(1) > pool)
        })
        .map(|(i, _)| Issue {
            path: format!("$.difficulties[{i}].boss_ticks"),
            message: "more mid-boss times than the time_limit mode's pool has bosses".into(),
        })
        .collect();
    if issues.is_empty() {
        Ok(())
    } else {
        Err(PackError::File {
            file: "difficulties",
            error: Error::Invalid(issues),
        })
    }
}

/// The perk set a run names none of starts with is in the file: the run start refuses a pack
/// without it.
pub(super) fn check_default_perk_set(perk_sets: &PerkSets) -> Result<(), PackError> {
    if perk_sets
        .sets()
        .iter()
        .any(|s| s.id.as_str() == DEFAULT_PERK_SET)
    {
        return Ok(());
    }
    Err(PackError::File {
        file: "perk_sets",
        error: Error::Invalid(vec![Issue {
            path: "$.perk_sets".into(),
            message: format!(
                "the default perk set ({DEFAULT_PERK_SET}) is missing: a run that names none starts with it"
            ),
        }]),
    })
}

/// The difficulty a run names none of starts with is in the file: the run start refuses a pack
/// without it.
pub(super) fn check_default_difficulty(difficulties: &Difficulties) -> Result<(), PackError> {
    if difficulties
        .list()
        .iter()
        .any(|d| d.id.as_str() == DEFAULT_DIFFICULTY)
    {
        return Ok(());
    }
    Err(PackError::File {
        file: "difficulties",
        error: Error::Invalid(vec![Issue {
            path: "$.difficulties".into(),
            message: format!(
                "the default difficulty ({DEFAULT_DIFFICULTY}) is missing: a run that names none starts with it"
            ),
        }]),
    })
}

/// A perk's drop-weight category is a category of the rewards' equipment rules.
pub(super) fn check_perk_categories(
    perk_sets: &PerkSets,
    rewards: &Rewards,
) -> Result<(), PackError> {
    let mut issues = Vec::new();
    for (i, set) in perk_sets.sets().iter().enumerate() {
        for (e, entry) in set.effects.iter().enumerate() {
            let Some(category) = entry.category else {
                continue;
            };
            let known = rewards.equipment().is_some_and(|r| {
                r.categories
                    .iter()
                    .any(|c| c.id.as_str() == category.as_str())
            });
            if !known {
                issues.push(Issue {
                    path: format!("$.perk_sets[{i}].effects[{e}].category"),
                    message: "not a category of the rewards' equipment rules".into(),
                });
            }
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(PackError::File {
            file: "perk_sets",
            error: Error::Invalid(issues),
        })
    }
}
