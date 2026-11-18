//! The run's agent, boss and horde definitions.

use super::{BossIndex, DefIndex, RangedPool, colour_of};

use crate::behaviour::Action;
use crate::behaviour::Agent;
use crate::behaviour::Movement;
use crate::id::ContentId;
use crate::kinds::KindIndex;
use crate::kinds::KindTable;
use crate::sim::state::AgentKey;
use crate::sim::state::AgentSource;
use crate::stage::Boss;
use crate::stage::Stage;
use crate::threat_colours::ThreatColourIndex;
use crate::threat_colours::ThreatColours;
use crate::time_limit::EliteTier;
use crate::validate::Issue;

/// An agent definition and its kind.
#[derive(Clone, Debug)]
pub(crate) struct AgentDef {
    pub kind: KindIndex,
    pub agent: Agent,
    /// The ranged pool of each rule, by rule; empty when no rule is a `ranged` attack (RA1).
    pub ranged_pools: Vec<Option<RangedPool>>,
    /// Whether each rule is held to the view (an elite's or a companion's rule that lays
    /// something or casts a `Shot`); empty when no rule is.
    pub view_barred: Vec<bool>,
    /// The entry of the run's threat colour table its `threat_colour` names (snapshot only).
    pub colour: Option<ThreatColourIndex>,
    /// The leash may move an enemy of this definition (EC2b): `leash_eligible`. Decided once when
    /// the world is compiled; never hashed.
    pub leash: bool,
}

/// Whether the leash may move an enemy made from `agent` under `source`: a whitelist (DS87).
/// It roams (`roams`) and a wave or a regular wave enemy's summon made it; a guardian, an escort
/// and the boss's summons stay with their owner, and so do an elite's. `owner_is_elite` says the
/// wave enemy that owns a `Summoned` agent is an elite. The match is exhaustive.
pub(super) fn leash_eligible(agent: &Agent, source: AgentSource, owner_is_elite: bool) -> bool {
    let placed_freely = match source {
        AgentSource::Wave => true,
        AgentSource::Summoned => !owner_is_elite,
        AgentSource::Companion
        | AgentSource::BossRule
        | AgentSource::Guardian
        | AgentSource::Escort => false,
    };
    placed_freely && roams(agent)
}

/// Whether an agent drifts or chases rather than keeping to a fixed point of its own: its
/// movement is free (`follow`, `orbit`, `straight`) and no rule is a `move_to`. Both matches are
/// exhaustive, so a movement or an action added later has to be decided here.
pub(super) fn roams(agent: &Agent) -> bool {
    let free = match &agent.movement {
        Movement::Follow { .. } | Movement::Orbit { .. } | Movement::Straight { .. } => true,
        Movement::Hold { .. } | Movement::FleeWhenDisturbed { .. } | Movement::Shrine { .. } => {
            false
        }
    };
    free && !agent.rules.iter().any(|r| match &r.action {
        Action::MoveTo { .. } => true,
        Action::Cast { .. }
        | Action::Attack { .. }
        | Action::Summon { .. }
        | Action::Telegraph { .. } => false,
    })
}

/// Every agent definition of the run, by index. `keys` finds one by the `AgentKey` that names
/// it, which spawning does; the rules use the index kept on the agent.
#[derive(Clone, Debug, Default)]
pub(crate) struct AgentDefs {
    pub(super) defs: Vec<AgentDef>,
    pub(super) keys: Vec<(AgentKey, DefIndex)>,
}

impl AgentDefs {
    pub(in crate::sim) fn defs_iter(&self) -> impl Iterator<Item = &AgentDef> {
        self.defs.iter()
    }

    pub(super) fn push(
        &mut self,
        kinds: &KindTable,
        agent: &Agent,
        source: AgentSource,
        owner_is_elite: bool,
        colours: &ThreatColours,
    ) -> Result<DefIndex, Issue> {
        let at = u16::try_from(self.defs.len()).map_err(|_| Issue {
            path: "$".into(),
            message: format!("a run brings more than {} agent definitions", u16::MAX),
        })?;
        self.defs.push(AgentDef {
            kind: kinds.of(&agent.base),
            agent: agent.clone(),
            ranged_pools: Vec::new(),
            view_barred: Vec::new(),
            // A companion has none (the validator refuses the field), so the compile side and
            // `colour_of_owner`, which gives a companion's attacks none, agree even for a stage
            // that skipped validation.
            colour: match source {
                AgentSource::Companion => None,
                _ => colour_of(colours, agent.threat_colour.as_ref()),
            },
            leash: leash_eligible(agent, source, owner_is_elite),
        });
        Ok(DefIndex(at))
    }

    fn keyed(
        &mut self,
        kinds: &KindTable,
        key: AgentKey,
        agent: &Agent,
        owner_is_elite: bool,
        colours: &ThreatColours,
    ) -> Result<(), Issue> {
        let at = self.push(kinds, agent, key.source, owner_is_elite, colours)?;
        self.keys.push((key, at));
        Ok(())
    }

    /// The definitions of `stage`'s companions and waves; those of its bosses are added by
    /// `BossDef::compile`.
    pub(super) fn build(
        stage: &Stage,
        kinds: &KindTable,
        colours: &ThreatColours,
    ) -> Result<AgentDefs, Issue> {
        let mut defs = AgentDefs::default();
        let index = |i: usize| u32::try_from(i).unwrap_or(u32::MAX);
        for (i, agent) in stage.companions.iter().enumerate() {
            let key = AgentKey {
                source: AgentSource::Companion,
                index: index(i),
                rule: 0,
            };
            defs.keyed(kinds, key, agent, false, colours)?;
        }
        for (i, wave) in stage.waves.iter().enumerate() {
            let at = |source, rule| AgentKey {
                source,
                index: index(i),
                rule,
            };
            defs.keyed(kinds, at(AgentSource::Wave, 0), &wave.enemy, false, colours)?;
            for (r, rule) in wave.enemy.rules.iter().enumerate() {
                if let Action::Summon { agent, .. } = &rule.action {
                    let owner_is_elite = wave.enemy.elite.is_some();
                    defs.keyed(
                        kinds,
                        at(AgentSource::Summoned, index(r)),
                        agent,
                        owner_is_elite,
                        colours,
                    )?;
                }
            }
            if let Movement::Shrine { guardians, .. } = &wave.enemy.movement {
                defs.keyed(
                    kinds,
                    at(AgentSource::Guardian, 0),
                    &guardians.enemy,
                    true,
                    colours,
                )?;
            }
            if let Some(escort) = wave.enemy.elite.as_ref().and_then(|e| e.escort.as_ref()) {
                defs.keyed(
                    kinds,
                    at(AgentSource::Escort, 0),
                    &escort.enemy,
                    true,
                    colours,
                )?;
            }
        }
        defs.keys.sort_by_key(|(k, _)| *k);
        Ok(defs)
    }

    /// The definition `key` names, for a spawn.
    pub fn of_key(&self, key: AgentKey) -> DefIndex {
        let at = self
            .keys
            .binary_search_by_key(&key, |(k, _)| *k)
            .expect("every agent a run can spawn has a definition");
        self.keys[at].1
    }

    pub fn get(&self, def: DefIndex) -> &AgentDef {
        &self.defs[def.at()]
    }

    /// Test code only: a definition's agent, to give it a stagger gauge.
    #[cfg(test)]
    pub(in crate::sim) fn agent_mut(&mut self, def: DefIndex) -> &mut Agent {
        &mut self.defs[def.at()].agent
    }

    /// How many definitions the run has: what a saved `DefIndex` is checked against.
    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn agent(&self, def: DefIndex) -> &Agent {
        &self.defs[def.at()].agent
    }
}

/// A boss definition of the run, compiled once: the boss as the stage or the mode file gives
/// it, its kind, and the agent definition each of its summon rules makes. A boss that comes
/// later in the run (`spawn_boss`) is a `BossState` over one of these, with its own offset;
/// the definition is never changed.
#[derive(Clone, Debug)]
pub(crate) struct BossDef {
    pub boss: Boss,
    pub kind: KindIndex,
    /// The entry of the run's threat colour table its `threat_colour` names (snapshot only).
    pub colour: Option<ThreatColourIndex>,
    /// The summon rules, by (phase, rule).
    pub summons: Vec<(u32, u32, DefIndex)>,
}

impl BossDef {
    pub(super) fn compile(
        defs: &mut AgentDefs,
        kinds: &KindTable,
        colours: &ThreatColours,
        boss: Boss,
    ) -> Result<BossDef, Issue> {
        let index = |i: usize| u32::try_from(i).unwrap_or(u32::MAX);
        let mut summons = Vec::new();
        for (p, phase) in boss.phases.iter().enumerate() {
            for (r, rule) in phase.rules.iter().enumerate() {
                if let Action::Summon { agent, .. } = &rule.action {
                    summons.push((
                        index(p),
                        index(r),
                        defs.push(kinds, agent, AgentSource::BossRule, true, colours)?,
                    ));
                }
            }
        }
        Ok(BossDef {
            kind: kinds.of(&boss.base),
            colour: colour_of(colours, boss.threat_colour.as_ref()),
            boss,
            summons,
        })
    }
}

/// What a `time_limit` run's bosses are compiled from (core#212): the mode file's boss
/// definitions and the placeholder, the sprites of the pool and of the final boss, and what the
/// elite minutes read.
#[derive(Clone, Debug)]
pub(crate) struct HordeSpec {
    pub pool: Vec<ContentId>,
    pub final_boss: ContentId,
    /// The boss every one is made from, with its sprite replaced, when the mode file gives no
    /// definition for its sprite: a placeholder.
    pub template: Boss,
    /// The mode file's boss definitions (core#336), by sprite.
    pub bosses: Vec<Boss>,
    /// The run's length: an elite minute comes only before it.
    pub length_ticks: u32,
    /// The mode's elite table, by start.
    pub elite_table: Vec<EliteTier>,
    /// For each elite type of the mode file, the stage wave that holds its definition: the
    /// type's enemy, experience and expertise, never spawned as a wave (count 0). A kill's
    /// experience and expertise are the wave's.
    pub elite_waves: Vec<(ContentId, u32)>,
}

/// A `time_limit` run's bosses and elites, fixed for the run: the boss of each pool entry and
/// of the final boss are `World::bosses` indices, so a boss coming is an index and an offset.
#[derive(Clone, Debug)]
pub(crate) struct HordeDefs {
    /// One per pool entry.
    pub pool: Vec<BossIndex>,
    pub final_boss: BossIndex,
    pub length_ticks: u32,
    pub elite_table: Vec<EliteTier>,
    pub elite_waves: Vec<(ContentId, u32)>,
}
