//! The run's definitions, resolved once when it starts (core#271): every agent a run can bring,
//! found by an index kept on the agent, so the rules never walk the stage to learn what an
//! enemy is. The stage is read-only for the whole run; what a boss changes when it comes
//! (`spawn_boss`) lives in the run's state.

use std::ops::Deref;

use crate::behaviour::{Action, Agent, Movement};
use crate::engine::{AgentKey, AgentSource};
use crate::kinds::{KindIndex, KindTable};
use crate::stage::{Boss, Stage};

/// An agent definition and its kind.
#[derive(Clone, Debug)]
pub(crate) struct AgentDef {
    pub kind: KindIndex,
    pub agent: Agent,
}

/// Every agent definition of the run, by index. `keys` finds one by the `AgentKey` that names
/// it, which spawning does; the rules use the index kept on the agent.
#[derive(Clone, Debug, Default)]
pub(crate) struct AgentDefs {
    defs: Vec<AgentDef>,
    keys: Vec<(AgentKey, u32)>,
    /// For each boss definition the run can bring, its summons by (phase, rule).
    boss_summons: Vec<Vec<(u32, u32, u32)>>,
}

impl AgentDefs {
    fn push(&mut self, kinds: &KindTable, agent: &Agent) -> u32 {
        self.defs.push(AgentDef {
            kind: kinds.of(&agent.base),
            agent: agent.clone(),
        });
        u32::try_from(self.defs.len() - 1).unwrap_or(u32::MAX)
    }

    fn keyed(&mut self, kinds: &KindTable, key: AgentKey, agent: &Agent) {
        let at = self.push(kinds, agent);
        self.keys.push((key, at));
    }

    /// The definitions of `stage` and of `bosses`, the bosses a run can bring (the stage's own,
    /// or a `time_limit` run's, in the order `boss_summons` is asked for).
    pub fn build(stage: &Stage, bosses: &[&Boss], kinds: &KindTable) -> AgentDefs {
        let mut defs = AgentDefs::default();
        let index = |i: usize| u32::try_from(i).unwrap_or(u32::MAX);
        for (i, agent) in stage.companions.iter().enumerate() {
            let key = AgentKey {
                source: AgentSource::Companion,
                index: index(i),
                rule: 0,
            };
            defs.keyed(kinds, key, agent);
        }
        for (i, wave) in stage.waves.iter().enumerate() {
            let at = |source, rule| AgentKey {
                source,
                index: index(i),
                rule,
            };
            defs.keyed(kinds, at(AgentSource::Wave, 0), &wave.enemy);
            for (r, rule) in wave.enemy.rules.iter().enumerate() {
                if let Action::Summon { agent, .. } = &rule.action {
                    defs.keyed(kinds, at(AgentSource::Summoned, index(r)), agent);
                }
            }
            if let Movement::Shrine { guardians, .. } = &wave.enemy.movement {
                defs.keyed(kinds, at(AgentSource::Guardian, 0), &guardians.enemy);
            }
            if let Some(escort) = wave.enemy.elite.as_ref().and_then(|e| e.escort.as_ref()) {
                defs.keyed(kinds, at(AgentSource::Escort, 0), &escort.enemy);
            }
        }
        for boss in bosses {
            let mut found = Vec::new();
            for (p, phase) in boss.phases.iter().enumerate() {
                for (r, rule) in phase.rules.iter().enumerate() {
                    if let Action::Summon { agent, .. } = &rule.action {
                        found.push((index(p), index(r), defs.push(kinds, agent)));
                    }
                }
            }
            defs.boss_summons.push(found);
        }
        defs.keys.sort_by_key(|(k, _)| *k);
        defs
    }

    /// The definition `key` names, for a spawn.
    pub fn of_key(&self, key: AgentKey) -> u32 {
        let at = self
            .keys
            .binary_search_by_key(&key, |(k, _)| *k)
            .expect("every agent a run can spawn has a definition");
        self.keys[at].1
    }

    /// The definition boss `boss` (its index in the list `build` was given) summons in rule
    /// `rule` of phase `phase`.
    pub fn of_boss_rule(&self, boss: u32, phase: u32, rule: u32) -> u32 {
        self.boss_summons[boss as usize]
            .iter()
            .find(|(p, r, _)| *p == phase && *r == rule)
            .map(|(_, _, d)| *d)
            .expect("a boss summon names the summon rule that made it")
    }

    pub fn get(&self, def: u32) -> &AgentDef {
        &self.defs[def as usize]
    }

    pub fn agent(&self, def: u32) -> &Agent {
        &self.defs[def as usize].agent
    }
}

/// The stage and the definitions resolved from it: what the rules read. It derefs to the stage.
#[derive(Clone, Debug)]
pub(crate) struct Run {
    pub stage: Stage,
    pub defs: AgentDefs,
}

impl Run {
    pub fn new(stage: Stage, bosses: &[&Boss], kinds: &KindTable) -> Run {
        let defs = AgentDefs::build(&stage, bosses, kinds);
        Run { stage, defs }
    }
}

#[cfg(test)]
impl Run {
    /// A run of `stage` for a unit test.
    pub fn for_test(stage: Stage) -> Run {
        let kinds = KindTable::of_stage(&stage).unwrap();
        let bosses: Vec<&Boss> = stage.boss.iter().collect();
        Run::new(stage.clone(), &bosses, &kinds)
    }
}

impl Deref for Run {
    type Target = Stage;

    fn deref(&self) -> &Stage {
        &self.stage
    }
}
