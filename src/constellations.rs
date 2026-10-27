//! The constellations (core#NS3, app#336): sets of named stars, each with tiers. Holding
//! `threshold` of a set's members gives the tier's effects; reaching a higher tier replaces the
//! lower one's, it does not add to it. A tier lists the effects the set has **at that tier**, in
//! the reward effect list, so the engine applies the difference between two tiers
//! (`tier_delta`): the full tier's reroll count is the count in all, not on top of the small
//! tier's. Carried stars count as held (ADR-117).
//!
//! The file is untrusted (ADR-020), as every content file is: a size limit before parsing,
//! strict parsing, the current versions only, and every list bounded. A set has 2 to
//! `MAX_MEMBERS` members, each a named star of the rewards the file is read with and in no
//! other set; its tiers ascend strictly from 1 to the member count, the top tier being the
//! whole set; a tier's effects obey the caps of a reward's (`rewards::validate_entry` and
//! `validate_bundle`), they cannot be a random rune (which keys on a reward); an orbit is keyed on its set, the
//! same orbit at every tier and the orbs adding up, and counts toward `MAX_ORBITS` with the
//! orbit rewards, and a
//! higher tier holds at least what the lower one does. At most `MAX_CONSTELLATIONS` sets.
//!
//! `Rewards` holds the sets beside the rewards they were read against, as it holds its stage
//! skills, so every function that takes the rewards sees them. A pack's `constellations` file is
//! optional: a pack released without it has no sets, and stays valid. The simulation reads the
//! sets; the SHA-256 of the file is part of the pack's hash.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::id::ContentId;
use crate::registry;
use crate::rewards::{
    EffectEntry, MAX_ORBITS, RewardEffect, Rewards, validate_bundle, validate_entry,
};
use crate::run_stats;
use crate::validate::{Error, Issue};
use crate::version::{self, Versions};

/// The largest constellations file read (a tuning value; ADR-020 decides that one exists).
pub const MAX_CONSTELLATIONS_BYTES: usize = 32 * 1024;
/// The most sets a file lists (a placeholder bound, ADR-020).
pub const MAX_CONSTELLATIONS: usize = 16;
/// The fewest members of a set.
pub const MIN_MEMBERS: usize = 2;
/// The most members of a set.
pub const MAX_MEMBERS: usize = 4;
/// The most effects one tier holds, as a reward does.
pub const MAX_TIER_EFFECTS: usize = crate::rewards::MAX_REWARD_EFFECTS;

/// The constellations file: the sets of named stars.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConstellationsFile {
    pub constellations: Vec<Constellation>,
}

/// One set of named stars.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Constellation {
    /// Unique within the file; lowercase snake_case, at most 32 characters.
    pub id: ContentId,
    /// The name the frontend shows, 1 to `MAX_TITLE_CHARS` characters.
    pub name: String,
    /// Named stars of the rewards file, 2 to `MAX_MEMBERS`, each in no other set.
    pub members: Vec<ContentId>,
    /// From the smallest threshold to the member count.
    pub tiers: Vec<ConstellationTier>,
}

/// What a set gives once `threshold` of its members are held.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConstellationTier {
    /// Members held, from 1 to the member count; ascending over the tiers, the last being the
    /// member count.
    pub threshold: u8,
    /// The effects the set has at this tier in all: the reward effect list, at most
    /// `MAX_TIER_EFFECTS`.
    pub effects: Vec<EffectEntry>,
}

impl Constellation {
    /// The tiers reached with `held` members.
    pub fn tier_reached(&self, held: u8) -> u8 {
        let reached = self.tiers.iter().filter(|t| t.threshold <= held).count();
        u8::try_from(reached).unwrap_or(u8::MAX)
    }

    /// The members the next tier needs, or the whole set once the top tier is reached.
    pub fn needed(&self, held: u8) -> u8 {
        self.tiers
            .iter()
            .map(|t| t.threshold)
            .find(|t| *t > held)
            .or_else(|| self.tiers.last().map(|t| t.threshold))
            .unwrap_or(0)
    }
}

/// Sets that passed validation, and the SHA-256 of the bytes they came from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Constellations {
    sets: Vec<Constellation>,
    /// `None` for the default: no file, so a run's hashes are what they were before the file.
    sha256: Option<[u8; 32]>,
}

impl Constellations {
    /// Checks the size and the versions given (the pack's), parses strictly and validates against
    /// `rewards`, whose named stars the members must be: the one call for a file's bytes.
    pub fn parse_with(
        bytes: &[u8],
        versions: Versions,
        rewards: &Rewards,
    ) -> Result<Constellations, Error> {
        let file: ConstellationsFile =
            version::read_member(bytes, MAX_CONSTELLATIONS_BYTES, versions)?;
        validate(&file, rewards)?;
        Ok(Constellations {
            sets: file.constellations,
            sha256: Some(Sha256::digest(bytes).into()),
        })
    }

    /// The sets, in the file's order.
    pub fn sets(&self) -> &[Constellation] {
        &self.sets
    }

    /// Whether there is no set.
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    /// The place of the set the member `reward` is in, if it is in one.
    pub fn set_of(&self, reward: &ContentId) -> Option<usize> {
        self.sets.iter().position(|s| s.members.contains(reward))
    }

    /// The SHA-256 of the bytes they were parsed from; `None` without a file.
    pub fn sha256(&self) -> Option<[u8; 32]> {
        self.sha256
    }
}

/// What identifies an effect for the difference between two tiers.
fn same_kind(a: &EffectEntry, b: &EffectEntry) -> bool {
    (a.effect, &a.skill, a.damage_type) == (b.effect, &b.skill, b.damage_type)
}

/// A tier's effects with those of one kind merged: the amounts and radii add up, the depth is
/// the greatest, the kill interval the shortest, as the picks of a run combine them.
fn totals(effects: &[EffectEntry]) -> Vec<EffectEntry> {
    let mut out: Vec<EffectEntry> = Vec::new();
    for e in effects {
        match out.iter_mut().find(|t| same_kind(t, e)) {
            Some(t) => {
                t.amount = Some(t.amount.unwrap_or(0).saturating_add(e.amount.unwrap_or(0)));
                t.radius = t.radius.map(|r| r.saturating_add(e.radius.unwrap_or(0)));
                t.depth = t.depth.max(e.depth);
                t.kills = t.kills.min(e.kills);
            }
            None => out.push(e.clone()),
        }
    }
    out
}

/// The effects to apply to go from the tier `lower` (none: no tier yet) to `tier`: for each
/// kind, what `tier` has beyond `lower`, so that the two applied one after the other give
/// `tier`'s totals and not their sum. Kinds with nothing beyond are left out.
pub fn tier_delta(lower: Option<&ConstellationTier>, tier: &ConstellationTier) -> Vec<EffectEntry> {
    let before = lower.map(|t| totals(&t.effects)).unwrap_or_default();
    totals(&tier.effects)
        .into_iter()
        .filter_map(|mut e| {
            if let Some(old) = before.iter().find(|b| same_kind(b, &e)) {
                let amount = e
                    .amount
                    .unwrap_or(0)
                    .saturating_sub(old.amount.unwrap_or(0));
                if amount == 0 {
                    return None;
                }
                e.amount = Some(amount);
                e.radius = e.radius.map(|r| r.saturating_sub(old.radius.unwrap_or(0)));
            }
            Some(e)
        })
        .collect()
}

/// Validates a parsed file against `rewards`: one issue per problem, with its path, never text
/// from the file.
pub fn validate(file: &ConstellationsFile, rewards: &Rewards) -> Result<(), Error> {
    let mut issues = Vec::new();
    let mut issue = |path: String, message: String| issues.push(Issue { path, message });
    if file.constellations.len() > MAX_CONSTELLATIONS {
        issue(
            "$.constellations".into(),
            format!(
                "at most {MAX_CONSTELLATIONS} sets (found {})",
                file.constellations.len()
            ),
        );
    }
    for (i, set) in file.constellations.iter().enumerate() {
        let at = |field: &str| format!("$.constellations[{i}].{field}");
        if !registry::is_well_formed(&set.id.0) {
            issue(
                at("id"),
                "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
            );
        } else if file.constellations[..i].iter().any(|s| s.id == set.id) {
            issue(at("id"), "the same ID as an earlier set".into());
        }
        if let Some(problem) = run_stats::text_problem(&set.name, 1, run_stats::MAX_TITLE_CHARS) {
            issue(at("name"), problem);
        }
        if !(MIN_MEMBERS..=MAX_MEMBERS).contains(&set.members.len()) {
            issue(
                at("members"),
                format!(
                    "{MIN_MEMBERS} to {MAX_MEMBERS} members (found {})",
                    set.members.len()
                ),
            );
        }
        for (k, member) in set.members.iter().enumerate() {
            let at = |field: &str| format!("$.constellations[{i}].members[{k}]{field}");
            match rewards.index_of(member).map(|p| &rewards.rewards()[p]) {
                None => issue(at(""), "no such reward in the rewards file".into()),
                Some(r) if !r.named => issue(at(""), "a member is a named star".into()),
                Some(_) => {}
            }
            let twice = set.members[..k].contains(member)
                || file.constellations[..i]
                    .iter()
                    .any(|s| s.members.contains(member));
            if twice {
                issue(at(""), "a named star is in at most one set".into());
            }
        }
        validate_tiers(set, &at, rewards, &mut issue);
        let mut orbits = set
            .tiers
            .iter()
            .flat_map(|t| &t.effects)
            .filter_map(|e| e.orbit.as_ref());
        if let Some(first) = orbits.next() {
            if orbits.any(|o| o != first) {
                issue(
                    at("tiers"),
                    "a set's tiers give the same orbit, with more orbs".into(),
                );
            }
        }
    }
    let gives_orbit = |e: &EffectEntry| e.effect == RewardEffect::Orbit;
    let orbits = rewards
        .rewards()
        .iter()
        .filter(|r| r.effects.iter().any(gives_orbit))
        .count()
        + file
            .constellations
            .iter()
            .filter(|s| s.tiers.iter().flat_map(|t| &t.effects).any(gives_orbit))
            .count();
    if orbits > MAX_ORBITS {
        issue(
            "$.constellations".into(),
            format!("at most {MAX_ORBITS} orbits in all, rewards and sets together"),
        );
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}

fn validate_tiers(
    set: &Constellation,
    at: &dyn Fn(&str) -> String,
    rewards: &Rewards,
    issue: &mut dyn FnMut(String, String),
) {
    let members = set.members.len();
    if set.tiers.is_empty() || set.tiers.len() > members {
        issue(
            at("tiers"),
            format!("1 to {members} tiers (found {})", set.tiers.len()),
        );
    } else if usize::from(set.tiers[set.tiers.len() - 1].threshold) != members {
        issue(
            at("tiers"),
            format!("the last tier is the whole set: threshold {members}"),
        );
    }
    for (t, tier) in set.tiers.iter().enumerate() {
        let tier_at = |field: &str| at(&format!("tiers[{t}].{field}"));
        let ascends = t == 0 || set.tiers[t - 1].threshold < tier.threshold;
        if !(1..=members).contains(&usize::from(tier.threshold)) {
            issue(
                tier_at("threshold"),
                format!("1 to {members} (found {})", tier.threshold),
            );
        } else if !ascends {
            issue(
                tier_at("threshold"),
                "thresholds ascend: more than the tier before".into(),
            );
        }
        if tier.effects.is_empty() || tier.effects.len() > MAX_TIER_EFFECTS {
            issue(
                tier_at("effects"),
                format!(
                    "1 to {MAX_TIER_EFFECTS} effects (found {})",
                    tier.effects.len()
                ),
            );
        }
        for (k, entry) in tier.effects.iter().enumerate() {
            let entry_at = |field: &str| at(&format!("tiers[{t}].effects[{k}].{field}"));
            if entry.effect == RewardEffect::RandomRune {
                issue(
                    entry_at("effect"),
                    "a set's tier cannot give a random rune".into(),
                );
            } else {
                validate_entry(
                    entry,
                    &entry_at,
                    rewards.stage_skills(),
                    rewards.max_rune_haste_pct(),
                    issue,
                );
            }
        }
        validate_bundle(
            &tier.effects,
            rewards.max_rune_haste_pct(),
            &|field: &str| tier_at(field),
            issue,
        );
        if t > 0 {
            let (low, high) = (totals(&set.tiers[t - 1].effects), totals(&tier.effects));
            let holds_less = low.iter().any(|l| {
                high.iter()
                    .find(|h| same_kind(h, l))
                    .is_none_or(|h| h.amount < l.amount)
            });
            if holds_less {
                issue(
                    tier_at("effects"),
                    "a tier holds at least the effects of the tier before it".into(),
                );
            }
        }
    }
}
