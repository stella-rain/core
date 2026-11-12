//! The kind table of a run (core#271, DS16): the content IDs that name what an entity is, which
//! the app's Bestiary keys on, interned once into a `u16` index. An event, a snapshot and the
//! state carry the index, a `Copy` number: no allocation and no reference count on a hit, a
//! death or a view (ADR-SIM-5). The app maps an index back to its ID through `Engine::kinds`.

use std::fmt::Write;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::behaviour::{Action, Agent, Movement, Rule};
use crate::id::ContentId;
use crate::stage::{Boss, Stage};
use crate::validate::Issue;

/// The most kinds a run holds: what a `u16` index can name.
pub const MAX_KINDS: usize = 1 << 16;

/// The index of a kind in the run's `KindTable`. It is only meaningful with that run's table:
/// key anything that outlives the run on the ID.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct KindIndex(pub u16);

/// The distinct kind IDs of a run, sorted by ID (byte order), so the order depends on the
/// content and nothing else: the stage, or the pack for a pack's run, pins it through its hash.
/// An enemy's, an elite's, a boss's or a companion's kind is its sprite ID, a boss part's is
/// the part's own ID.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KindTable {
    ids: Vec<ContentId>,
}

impl KindTable {
    /// The table of `ids`, sorted and without repeats; refused, with a path, past `limit` kinds
    /// (`MAX_KINDS` for a run).
    pub fn from_ids<'a>(
        ids: impl IntoIterator<Item = &'a ContentId>,
        limit: usize,
    ) -> Result<KindTable, Issue> {
        let mut ids: Vec<ContentId> = ids.into_iter().cloned().collect();
        ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        ids.dedup();
        if ids.len() > limit.min(MAX_KINDS) {
            return Err(Issue {
                path: "$.kinds".into(),
                message: format!(
                    "{} kinds, at most {} can be told apart",
                    ids.len(),
                    limit.min(MAX_KINDS)
                ),
            });
        }
        Ok(KindTable { ids })
    }

    /// The table of a stage: every kind it can bring, whichever way it comes (a wave, a summon,
    /// a shrine's guardian, a leader's escort, a companion, the boss and its parts).
    pub fn of_stage(stage: &Stage) -> Result<KindTable, Issue> {
        KindTable::of_stage_and(stage, &[], &[], MAX_KINDS)
    }

    /// As `of_stage`, with the bosses a `time_limit` run brings later, the sprite IDs it makes
    /// them with, and its limit.
    pub fn of_stage_and(
        stage: &Stage,
        bosses: &[&Boss],
        extra: &[ContentId],
        limit: usize,
    ) -> Result<KindTable, Issue> {
        let mut found = Vec::new();
        visit_foes(stage, bosses, &mut |foe, _| match foe {
            Foe::Agent(agent) => found.push(&agent.base),
            Foe::Boss(boss) => {
                found.push(&boss.base);
                found.extend(boss.parts.iter().map(|part| &part.id));
            }
        });
        found.extend(extra);
        KindTable::from_ids(found, limit)
    }

    /// The index of `id`, `None` for an ID that is not a kind of the run.
    pub fn index_of(&self, id: &ContentId) -> Option<KindIndex> {
        self.ids
            .binary_search_by(|k| k.as_str().cmp(id.as_str()))
            .ok()
            .map(|i| KindIndex(u16::try_from(i).expect("a run has fewer than 65536 kinds")))
    }

    /// The index of a kind the run was built with: a spawn, which only brings what the table
    /// was built from.
    pub(crate) fn of(&self, id: &ContentId) -> KindIndex {
        self.index_of(id)
            .expect("the kind table holds every kind the run can bring")
    }

    /// The ID `index` names.
    pub fn id(&self, index: KindIndex) -> Option<&ContentId> {
        self.ids.get(usize::from(index.0))
    }

    /// Every kind ID, in index order.
    pub fn ids(&self) -> &[ContentId] {
        &self.ids
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// One enemy definition a stage can bring: a regular enemy, an elite, a summoned agent, a
/// shrine's guardian, a leader's escort or a companion (`Agent`), or a boss (`Boss`).
#[derive(Clone, Copy)]
pub(crate) enum Foe<'a> {
    Agent(&'a Agent),
    Boss(&'a Boss),
}

/// Calls `visit` with every agent and boss `stage` can bring, and the JSON path of each in the
/// stage: the companions, the waves' enemies, the boss, the agents their rules summon, the
/// guardians and escorts, and the boss's phases' summons. `bosses` are the bosses a
/// `time_limit` run brings later; their paths are `$.bosses[i]`. The kind table and the colour
/// check both walk the stage with this, so they cannot drift.
pub(crate) fn visit_foes<'a>(
    stage: &'a Stage,
    bosses: &[&'a Boss],
    visit: &mut impl FnMut(Foe<'a>, &str),
) {
    let mut path = String::from("$");
    for (i, agent) in stage.companions.iter().enumerate() {
        let at = path.len();
        let _ = write!(path, ".companions[{i}]");
        visit_agent(agent, &mut path, visit);
        path.truncate(at);
    }
    for (i, wave) in stage.waves.iter().enumerate() {
        let at = path.len();
        let _ = write!(path, ".waves[{i}].enemy");
        visit_agent(&wave.enemy, &mut path, visit);
        path.truncate(at);
    }
    if let Some(boss) = &stage.boss {
        let at = path.len();
        path.push_str(".boss");
        visit_boss(boss, &mut path, visit);
        path.truncate(at);
    }
    for (i, boss) in bosses.iter().enumerate() {
        let at = path.len();
        let _ = write!(path, ".bosses[{i}]");
        visit_boss(*boss, &mut path, visit);
        path.truncate(at);
    }
}

fn visit_rules<'a>(rules: &'a [Rule], path: &mut String, visit: &mut impl FnMut(Foe<'a>, &str)) {
    for (i, rule) in rules.iter().enumerate() {
        if let Action::Summon { agent, .. } = &rule.action {
            let at = path.len();
            let _ = write!(path, ".rules[{i}].do.agent");
            visit_agent(agent, path, visit);
            path.truncate(at);
        }
    }
}

/// Visits an agent at `path`, then what it brings.
pub(crate) fn visit_agent<'a>(
    agent: &'a Agent,
    path: &mut String,
    visit: &mut impl FnMut(Foe<'a>, &str),
) {
    visit(Foe::Agent(agent), path);
    visit_rules(&agent.rules, path, visit);
    if let Movement::Shrine { guardians, .. } = &agent.movement {
        let at = path.len();
        path.push_str(".movement.guardians.enemy");
        visit_agent(&guardians.enemy, path, visit);
        path.truncate(at);
    }
    if let Some(escort) = agent.elite.as_ref().and_then(|e| e.escort.as_ref()) {
        let at = path.len();
        path.push_str(".elite.escort.enemy");
        visit_agent(&escort.enemy, path, visit);
        path.truncate(at);
    }
}

/// Visits a boss at `path`, then the agents its phases summon.
pub(crate) fn visit_boss<'a>(
    boss: &'a Boss,
    path: &mut String,
    visit: &mut impl FnMut(Foe<'a>, &str),
) {
    visit(Foe::Boss(boss), path);
    for (i, phase) in boss.phases.iter().enumerate() {
        let at = path.len();
        let _ = write!(path, ".phases[{i}]");
        visit_rules(&phase.rules, path, visit);
        path.truncate(at);
    }
}
