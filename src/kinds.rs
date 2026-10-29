//! The kind table of a run (core#271, DS16): the content IDs that name what an entity is, which
//! the app's Bestiary keys on, interned once into a `u16` index. An event, a snapshot and the
//! state carry the index, a `Copy` number: no allocation and no reference count on a hit, a
//! death or a view (ADR-058). The app maps an index back to its ID through `Engine::kinds`.

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
        for agent in &stage.companions {
            agent_kinds(agent, &mut found);
        }
        for wave in &stage.waves {
            agent_kinds(&wave.enemy, &mut found);
        }
        for boss in stage.boss.iter().chain(bosses.iter().copied()) {
            boss_kinds(boss, &mut found);
        }
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

fn rules_kinds<'a>(rules: &'a [Rule], out: &mut Vec<&'a ContentId>) {
    for rule in rules {
        if let Action::Summon { agent, .. } = &rule.action {
            agent_kinds(agent, out);
        }
    }
}

fn agent_kinds<'a>(agent: &'a Agent, out: &mut Vec<&'a ContentId>) {
    out.push(&agent.base);
    rules_kinds(&agent.rules, out);
    if let Movement::Shrine { guardians, .. } = &agent.movement {
        agent_kinds(&guardians.enemy, out);
    }
    if let Some(escort) = agent.elite.as_ref().and_then(|e| e.escort.as_ref()) {
        agent_kinds(&escort.enemy, out);
    }
}

fn boss_kinds<'a>(boss: &'a Boss, out: &mut Vec<&'a ContentId>) {
    out.push(&boss.base);
    for part in &boss.parts {
        out.push(&part.id);
    }
    for phase in &boss.phases {
        rules_kinds(&phase.rules, out);
    }
}
