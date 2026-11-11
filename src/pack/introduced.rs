//! The `introduced` table: the IDs the released content holds and the version each came in at.

use std::sync::OnceLock;

use super::{Introduced, IntroducedId, Pack, PackError, PackFile, WeaponIds};
use crate::difficulties::Difficulties;
use crate::expertise::{Expertise, WeaponExpertise};
use crate::id::ContentId;
use crate::perk_sets::{PerkEffect, PerkSets};
use crate::rewards::{RewardEffect, Rewards};
use crate::run_content::RunContent;
use crate::run_stats::RunStats;
use crate::stage_skill::StageSkills;
use crate::time_limit::TimeLimit;
use crate::validate::{Error, Issue};

/// Every ID one weapon's expertise introduces, with its path in the expertise file, in the
/// file's order.
fn expertise_ids(w: usize, weapon: &WeaponExpertise) -> Vec<(String, &ContentId)> {
    let mut ids = Vec::new();
    for (s, style) in weapon.styles.iter().enumerate() {
        let base = format!("$.expertise.weapons[{w}].styles[{s}]");
        ids.push((format!("{base}.id"), &style.id));
        for (field, choices) in [
            ("upgrade_1", &style.upgrade_1),
            ("upgrade_2", &style.upgrade_2),
            ("capstone", &style.capstone),
        ] {
            for (c, choice) in choices.iter().enumerate() {
                ids.push((format!("{base}.{field}[{c}].id"), &choice.id));
            }
        }
    }
    for (c, option) in weapon.common.iter().enumerate() {
        ids.push((
            format!("$.expertise.weapons[{w}].common[{c}].id"),
            &option.id,
        ));
    }
    ids
}

/// The `introduced` table of the bundled pack: the IDs the released content holds. Read without
/// the version check (`Pack::bundled` calls `check_introduced`, which calls this).
fn released() -> &'static Introduced {
    static RELEASED: OnceLock<Introduced> = OnceLock::new();
    RELEASED.get_or_init(|| {
        serde_json::from_slice::<PackFile>(Pack::BUNDLED)
            .expect("the bundled pack parses")
            .introduced
    })
}

/// The released entries of one weapon's expertise; none for a weapon the release lacks.
fn released_expertise(weapon: &ContentId) -> &'static [IntroducedId] {
    released()
        .expertise
        .iter()
        .find(|e| e.weapon == *weapon)
        .map_or(&[], |e| e.ids.as_slice())
}

/// Checks one list of the table, at `at`, against the IDs of its file with their paths. An ID
/// the `released` entries (the bundled pack's) do not hold is new, and is recorded at the pack's
/// own `sim_version` (ADR-REPL-2, ADR-CONT-2); a released ID keeps the version it was released at.
fn check_ids(
    issues: &mut Vec<Issue>,
    at: &str,
    listed: &[IntroducedId],
    ids: &[(String, &ContentId)],
    sim_version: u32,
    released: &[IntroducedId],
) {
    for (i, entry) in listed.iter().enumerate() {
        if !ids.iter().any(|(_, id)| **id == entry.id) {
            issues.push(Issue {
                path: format!("{at}[{i}].id"),
                message: "not an ID of the file".into(),
            });
        } else if listed[..i].iter().any(|e| e.id == entry.id) {
            issues.push(Issue {
                path: format!("{at}[{i}].id"),
                message: "the same ID as an earlier entry".into(),
            });
        }
        if entry.sim_version > sim_version {
            issues.push(Issue {
                path: format!("{at}[{i}].sim_version"),
                message: format!(
                    "introduced in sim_version {}, newer than the pack's {sim_version}",
                    entry.sim_version
                ),
            });
        } else if entry.sim_version < sim_version && !released.iter().any(|r| r.id == entry.id) {
            issues.push(Issue {
                path: format!("{at}[{i}].sim_version"),
                message: format!(
                    "a new ID is introduced in the pack's sim_version {sim_version}, not {}",
                    entry.sim_version
                ),
            });
        }
    }
    for (path, id) in ids {
        if !listed.iter().any(|e| e.id == **id) {
            issues.push(Issue {
                path: path.clone(),
                message: format!("not listed in {at}"),
            });
        }
    }
}

/// `ids` with their paths, the `i`th at `$.{list}[i].id`.
fn paths<'a>(list: &str, ids: impl Iterator<Item = &'a ContentId>) -> Vec<(String, &'a ContentId)> {
    ids.enumerate()
        .map(|(i, id)| (format!("$.{list}[{i}].id"), id))
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn check_introduced(
    introduced: &Introduced,
    sim_version: u32,
    run_stats: &RunStats,
    rewards: &Rewards,
    stage_skills: &StageSkills,
    expertise: &Expertise,
    perk_sets: &PerkSets,
    difficulties: &Difficulties,
    time_limit: &TimeLimit,
) -> Result<(), PackError> {
    let mut issues = Vec::new();
    let stats = paths("run_stats.stats", run_stats.stats().iter().map(|s| &s.id));
    let reward_ids = paths("rewards.rewards", rewards.rewards().iter().map(|r| &r.id));
    let skills = paths(
        "stage_skills.stage_skills",
        stage_skills.skills().iter().map(|s| &s.id),
    );
    for (at, listed, ids, released) in [
        (
            "$.introduced.run_stats",
            &introduced.run_stats,
            &stats,
            &released().run_stats,
        ),
        (
            "$.introduced.rewards",
            &introduced.rewards,
            &reward_ids,
            &released().rewards,
        ),
        (
            "$.introduced.stage_skills",
            &introduced.stage_skills,
            &skills,
            &released().stage_skills,
        ),
    ] {
        check_ids(&mut issues, at, listed, ids, sim_version, released);
    }
    let sets = paths(
        "perk_sets.perk_sets",
        perk_sets.sets().iter().map(|s| &s.id),
    );
    check_ids(
        &mut issues,
        "$.introduced.perk_sets",
        &introduced.perk_sets,
        &sets,
        sim_version,
        &released().perk_sets,
    );
    let difficulty_ids = paths(
        "difficulties.difficulties",
        difficulties.list().iter().map(|d| &d.id),
    );
    check_ids(
        &mut issues,
        "$.introduced.difficulties",
        &introduced.difficulties,
        &difficulty_ids,
        sim_version,
        &released().difficulties,
    );
    let elites = paths(
        "modes.time_limit.elite_types",
        time_limit.elite_types().iter().map(|t| &t.id),
    );
    check_ids(
        &mut issues,
        "$.introduced.elite_types",
        &introduced.elite_types,
        &elites,
        sim_version,
        &released().elite_types,
    );
    if let Some(listed) = &introduced.constellations {
        let sets = paths(
            "constellations.constellations",
            rewards.constellations().sets().iter().map(|c| &c.id),
        );
        check_ids(
            &mut issues,
            "$.introduced.constellations",
            listed,
            &sets,
            sim_version,
            released().constellations.as_deref().unwrap_or(&[]),
        );
    }
    for (k, entry) in introduced.expertise.iter().enumerate() {
        let at = format!("$.introduced.expertise[{k}]");
        let found = expertise
            .weapons()
            .iter()
            .position(|w| w.weapon == entry.weapon);
        if introduced.expertise[..k]
            .iter()
            .any(|e| e.weapon == entry.weapon)
        {
            issues.push(Issue {
                path: format!("{at}.weapon"),
                message: "the same weapon as an earlier entry".into(),
            });
        } else if let Some(w) = found {
            let ids = expertise_ids(w, &expertise.weapons()[w]);
            check_ids(
                &mut issues,
                &format!("{at}.ids"),
                &entry.ids,
                &ids,
                sim_version,
                released_expertise(&entry.weapon),
            );
        } else {
            issues.push(Issue {
                path: format!("{at}.weapon"),
                message: "not a weapon of the expertise".into(),
            });
        }
    }
    for (w, weapon) in expertise.weapons().iter().enumerate() {
        if !introduced
            .expertise
            .iter()
            .any(|e| e.weapon == weapon.weapon)
        {
            issues.push(Issue {
                path: format!("$.expertise.weapons[{w}].weapon"),
                message: "not listed in $.introduced.expertise".into(),
            });
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(PackError::Introduced(issues))
    }
}

const OFFER_EFFECT_MESSAGE: &str =
    "an offer effect is deprecated: not allowed in content new at sim_version 2";

fn is_offer_reward(effect: RewardEffect) -> bool {
    matches!(
        effect,
        RewardEffect::OfferCandidates | RewardEffect::Rerolls | RewardEffect::Banishes
    )
}

/// The offer effects are deprecated (ADR-CONT-2): in a pack of `sim_version` 2 or more an entry
/// may hold one only if `introduced` records its ID below 2, so released content keeps them
/// and new content cannot add them. A constellation the pack lists no version for is not
/// provably released and is refused (ADR-SVC-1). A pack of `sim_version` 1 is not checked.
pub(super) fn check_offer_effects(
    introduced: &Introduced,
    sim_version: u32,
    rewards: &Rewards,
    perk_sets: &PerkSets,
) -> Result<(), PackError> {
    if sim_version < 2 {
        return Ok(());
    }
    let released = |list: Option<&[IntroducedId]>, id: &ContentId| {
        list.is_some_and(|l| l.iter().any(|e| e.id == *id && e.sim_version < 2))
    };
    let invalid = |file: &'static str, issues: Vec<Issue>| {
        (!issues.is_empty()).then_some(PackError::File {
            file,
            error: Error::Invalid(issues),
        })
    };
    let issue = |path: String| Issue {
        path,
        message: OFFER_EFFECT_MESSAGE.into(),
    };
    let mut found = Vec::new();
    for (i, r) in rewards.rewards().iter().enumerate() {
        if released(Some(&introduced.rewards), &r.id) {
            continue;
        }
        for (k, e) in r.effects.iter().enumerate() {
            if is_offer_reward(e.effect) {
                found.push(issue(format!("$.rewards[{i}].effects[{k}].effect")));
            }
        }
    }
    if let Some(e) = invalid("rewards", found) {
        return Err(e);
    }
    let mut found = Vec::new();
    for (i, s) in perk_sets.sets().iter().enumerate() {
        if released(Some(&introduced.perk_sets), &s.id) {
            continue;
        }
        for (k, e) in s.effects.iter().enumerate() {
            if matches!(
                e.effect,
                PerkEffect::ExtraRerolls | PerkEffect::ExtraBanishes
            ) {
                found.push(issue(format!("$.perk_sets[{i}].effects[{k}].effect")));
            }
        }
    }
    if let Some(e) = invalid("perk_sets", found) {
        return Err(e);
    }
    let mut found = Vec::new();
    for (i, c) in rewards.constellations().sets().iter().enumerate() {
        if released(introduced.constellations.as_deref(), &c.id) {
            continue;
        }
        for (t, tier) in c.tiers.iter().enumerate() {
            for (k, e) in tier.effects.iter().enumerate() {
                if is_offer_reward(e.effect) {
                    found.push(issue(format!(
                        "$.constellations[{i}].tiers[{t}].effects[{k}].effect"
                    )));
                }
            }
        }
    }
    match invalid("constellations", found) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// `ids` with the version each was released at in `released`, or `sim_version` when new.
fn stamp(released: &[IntroducedId], ids: Vec<&ContentId>, sim_version: u32) -> Vec<IntroducedId> {
    ids.into_iter()
        .map(|id| IntroducedId {
            id: id.clone(),
            sim_version: released
                .iter()
                .find(|r| r.id == *id)
                .map_or(sim_version, |r| r.sim_version),
        })
        .collect()
}

/// The table of `content`'s IDs: an ID the released table holds keeps the version it was
/// released at, a new one is introduced in `sim_version` (ADR-CONT-2 point 6, ADR-REPL-2).
pub(super) fn introduced_at(content: &RunContent, sim_version: u32) -> Introduced {
    let released = released();
    let list = |released: &[IntroducedId], ids: Vec<&ContentId>| stamp(released, ids, sim_version);
    Introduced {
        run_stats: list(
            &released.run_stats,
            content.run_stats().stats().iter().map(|s| &s.id).collect(),
        ),
        rewards: list(
            &released.rewards,
            content.rewards().rewards().iter().map(|r| &r.id).collect(),
        ),
        stage_skills: list(
            &released.stage_skills,
            content
                .stage_skills()
                .skills()
                .iter()
                .map(|s| &s.id)
                .collect(),
        ),
        expertise: content
            .expertise()
            .weapons()
            .iter()
            .enumerate()
            .map(|(w, weapon)| WeaponIds {
                weapon: weapon.weapon.clone(),
                ids: list(
                    released_expertise(&weapon.weapon),
                    expertise_ids(w, weapon)
                        .into_iter()
                        .map(|(_, id)| id)
                        .collect(),
                ),
            })
            .collect(),
        perk_sets: list(
            &released.perk_sets,
            content.perk_sets().sets().iter().map(|s| &s.id).collect(),
        ),
        difficulties: list(
            &released.difficulties,
            content
                .difficulties()
                .list()
                .iter()
                .map(|d| &d.id)
                .collect(),
        ),
        elite_types: list(
            &released.elite_types,
            content
                .time_limit()
                .elite_types()
                .iter()
                .map(|t| &t.id)
                .collect(),
        ),
        constellations: Some(list(
            released.constellations.as_deref().unwrap_or(&[]),
            content
                .constellations()
                .sets()
                .iter()
                .map(|c| &c.id)
                .collect(),
        ))
        .filter(|l| !l.is_empty()),
    }
}

#[cfg(test)]
mod tests;
