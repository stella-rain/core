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
use crate::perk_sets::PerkSets;
use crate::rewards::Rewards;
use crate::star_chart;
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
    expertise_multiplier: Vec<IgnoredAny>,
    #[serde(default)]
    lumine_levels: Vec<IgnoredAny>,
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
        ] {
            budget(format!("$.star_chart.{name}"), found, star_chart::MAX_ITEMS)?;
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
