//! The run's definitions, resolved once when it starts (core#271): every agent a run can bring,
//! found by an index kept on the agent, so the rules never walk the stage to learn what an
//! enemy is. The stage is read-only for the whole run; what a boss changes when it comes
//! (`spawn_boss`) lives in the run's state.

use std::ops::Deref;

use crate::attack::{AttackPhase, AttackRef, Emitter};
use crate::behaviour::{Action, Agent, Movement};
use crate::engine::{AgentKey, AgentSource};
use crate::id::ContentId;
use crate::kinds::{KindIndex, KindTable};
use crate::run_content::RunContent;
use crate::stage::{Boss, Stage};
use crate::time_limit::EliteTier;
use crate::validate::Issue;

use super::attack_table::{self, AttackKeys};

/// An agent definition of the run, by its place in `AgentDefs`: what an agent keeps of its
/// definition (a `u16`, so a run brings at most `u16::MAX` of them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DefIndex(pub u16);

/// A boss definition of the run, by its place in `World::bosses`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct BossIndex(pub u16);

/// An attack of the run, by its place in `World::attacks`: what a run of the attack keeps of
/// its phases (a `u16`, so a run brings at most `u16::MAX` of them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct AttackIndex(pub u16);

/// One phase's emitter nodes, by their place in `World::emitters`: what an attack task keeps of
/// its program.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ProgramIndex(pub u32);

/// An attack of the run, compiled once: a preset taken apart or the inline phases, as the
/// attack is before attack speed or range changes it.
#[derive(Clone, Debug)]
pub(crate) struct AttackDef {
    /// Empty for a preset the registry does not define or an attack with no phases: one that
    /// starts nothing.
    pub phases: Vec<AttackPhase>,
    /// The preset it came from, for the frontend to pick its animation.
    pub preset: Option<ContentId>,
    /// The program of each phase's `spawn`, in the order of `phases`.
    pub programs: Vec<ProgramIndex>,
}

impl DefIndex {
    fn at(self) -> usize {
        usize::from(self.0)
    }
}

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
    keys: Vec<(AgentKey, DefIndex)>,
}

impl AgentDefs {
    pub(super) fn agents(&self) -> impl Iterator<Item = &Agent> {
        self.defs.iter().map(|d| &d.agent)
    }

    fn push(&mut self, kinds: &KindTable, agent: &Agent) -> Result<DefIndex, Issue> {
        let at = u16::try_from(self.defs.len()).map_err(|_| Issue {
            path: "$".into(),
            message: format!("a run brings more than {} agent definitions", u16::MAX),
        })?;
        self.defs.push(AgentDef {
            kind: kinds.of(&agent.base),
            agent: agent.clone(),
        });
        Ok(DefIndex(at))
    }

    fn keyed(&mut self, kinds: &KindTable, key: AgentKey, agent: &Agent) -> Result<(), Issue> {
        let at = self.push(kinds, agent)?;
        self.keys.push((key, at));
        Ok(())
    }

    /// The definitions of `stage`'s companions and waves; those of its bosses are added by
    /// `BossDef::compile`.
    fn build(stage: &Stage, kinds: &KindTable) -> Result<AgentDefs, Issue> {
        let mut defs = AgentDefs::default();
        let index = |i: usize| u32::try_from(i).unwrap_or(u32::MAX);
        for (i, agent) in stage.companions.iter().enumerate() {
            let key = AgentKey {
                source: AgentSource::Companion,
                index: index(i),
                rule: 0,
            };
            defs.keyed(kinds, key, agent)?;
        }
        for (i, wave) in stage.waves.iter().enumerate() {
            let at = |source, rule| AgentKey {
                source,
                index: index(i),
                rule,
            };
            defs.keyed(kinds, at(AgentSource::Wave, 0), &wave.enemy)?;
            for (r, rule) in wave.enemy.rules.iter().enumerate() {
                if let Action::Summon { agent, .. } = &rule.action {
                    defs.keyed(kinds, at(AgentSource::Summoned, index(r)), agent)?;
                }
            }
            if let Movement::Shrine { guardians, .. } = &wave.enemy.movement {
                defs.keyed(kinds, at(AgentSource::Guardian, 0), &guardians.enemy)?;
            }
            if let Some(escort) = wave.enemy.elite.as_ref().and_then(|e| e.escort.as_ref()) {
                defs.keyed(kinds, at(AgentSource::Escort, 0), &escort.enemy)?;
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
    /// The summon rules, by (phase, rule).
    pub summons: Vec<(u32, u32, DefIndex)>,
}

impl BossDef {
    fn compile(defs: &mut AgentDefs, kinds: &KindTable, boss: Boss) -> Result<BossDef, Issue> {
        let index = |i: usize| u32::try_from(i).unwrap_or(u32::MAX);
        let mut summons = Vec::new();
        for (p, phase) in boss.phases.iter().enumerate() {
            for (r, rule) in phase.rules.iter().enumerate() {
                if let Action::Summon { agent, .. } = &rule.action {
                    summons.push((index(p), index(r), defs.push(kinds, agent)?));
                }
            }
        }
        Ok(BossDef {
            kind: kinds.of(&boss.base),
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
    /// The mode's elite table (ADR-090), by start.
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

/// The stage and everything compiled from it, immutable for the run: what the rules read. It
/// derefs to the stage.
#[derive(Clone, Debug)]
pub(crate) struct World {
    pub stage: Stage,
    pub defs: AgentDefs,
    /// Every boss the run can bring: the stage's own, or a `time_limit` run's, each at its
    /// own place in the mode file and the template's once for each sprite it stands in for.
    pub bosses: Vec<BossDef>,
    pub horde: Option<HordeDefs>,
    /// Every attack the run can start, compiled once; found by `attack_index`.
    pub attacks: Vec<AttackDef>,
    attack_keys: AttackKeys,
    /// The nodes of every phase's `spawn`, by `ProgramIndex`.
    pub emitters: Vec<Vec<Emitter>>,
    /// The attack of each on-hit rune, by `Rune::index`; none for a rune the run's file does
    /// not give an attack.
    pub rune_attacks: Vec<Option<AttackIndex>>,
}

impl World {
    /// Compiles `stage` and, for a `time_limit` run, its `horde`.
    pub fn new(
        stage: Stage,
        horde: Option<HordeSpec>,
        kinds: &KindTable,
        content: &RunContent,
    ) -> Result<World, Issue> {
        let mut defs = AgentDefs::build(&stage, kinds)?;
        let mut bosses = Vec::new();
        let mut push = |defs: &mut AgentDefs, boss: Boss| -> Result<BossIndex, Issue> {
            let at = u16::try_from(bosses.len()).map_err(|_| Issue {
                path: "$".into(),
                message: format!("a run brings more than {} boss definitions", u16::MAX),
            })?;
            bosses.push(BossDef::compile(defs, kinds, boss)?);
            Ok(BossIndex(at))
        };
        if let Some(boss) = &stage.boss {
            push(&mut defs, boss.clone())?;
        }
        let horde = match horde {
            None => None,
            Some(spec) => {
                // The mode file's definitions first, then the template once for each sprite of
                // the pool and the final boss that has none, in the order they are listed.
                let mut by_sprite: Vec<(ContentId, BossIndex)> = Vec::new();
                for boss in spec.bosses {
                    let sprite = boss.base.clone();
                    let at = push(&mut defs, boss)?;
                    if !by_sprite.iter().any(|(s, _)| *s == sprite) {
                        by_sprite.push((sprite, at));
                    }
                }
                let mut of_sprite = |defs: &mut AgentDefs, sprite: &ContentId| {
                    if let Some((_, at)) = by_sprite.iter().find(|(s, _)| s == sprite) {
                        return Ok(*at);
                    }
                    let mut boss = spec.template.clone();
                    boss.base = sprite.clone();
                    let at = push(defs, boss)?;
                    by_sprite.push((sprite.clone(), at));
                    Ok::<_, Issue>(at)
                };
                let mut pool = Vec::new();
                for sprite in &spec.pool {
                    pool.push(of_sprite(&mut defs, sprite)?);
                }
                let final_boss = of_sprite(&mut defs, &spec.final_boss)?;
                Some(HordeDefs {
                    pool,
                    final_boss,
                    length_ticks: spec.length_ticks,
                    elite_table: spec.elite_table,
                    elite_waves: spec.elite_waves,
                })
            }
        };
        let mut world = World {
            stage,
            defs,
            bosses,
            horde,
            attacks: Vec::new(),
            attack_keys: AttackKeys::default(),
            emitters: Vec::new(),
            rune_attacks: Vec::new(),
        };
        attack_table::compile_run(&mut world, content)?;
        Ok(world)
    }

    /// Compiles `attack` unless the run has it already, and returns its index.
    pub fn compile_attack(&mut self, attack: &AttackRef) -> Result<AttackIndex, Issue> {
        self.attack_keys
            .compile(&mut self.attacks, &mut self.emitters, attack)
    }

    /// The index of `attack`, which the run compiled when it started.
    pub fn attack_index(&self, attack: &AttackRef) -> AttackIndex {
        self.attack_keys
            .find(attack)
            .expect("every attack a run can start was compiled")
    }

    pub fn boss(&self, index: BossIndex) -> &BossDef {
        &self.bosses[usize::from(index.0)]
    }

    /// The definition the boss `boss` summons in rule `rule` of phase `phase`.
    pub fn boss_summon(&self, boss: BossIndex, phase: u32, rule: u32) -> DefIndex {
        self.boss(boss)
            .summons
            .iter()
            .find(|(p, r, _)| *p == phase && *r == rule)
            .map(|(_, _, d)| *d)
            .expect("a boss summon names the summon rule that made it")
    }
}

#[cfg(test)]
impl World {
    /// A run of `stage` for a unit test.
    pub fn for_test(stage: Stage) -> World {
        let kinds = KindTable::of_stage(&stage).unwrap();
        World::new(stage, None, &kinds, RunContent::bundled()).unwrap()
    }
}

impl Deref for World {
    type Target = Stage;

    fn deref(&self) -> &Stage {
        &self.stage
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_content::RunContent;

    #[test]
    fn a_run_with_more_than_u16_max_definitions_is_refused() {
        let stage = RunContent::bundled().trial().clone();
        let kinds = KindTable::of_stage(&stage).unwrap();
        let agent = stage.waves[0].enemy.clone();
        let mut defs = AgentDefs::default();
        for i in 0..=u16::MAX {
            assert_eq!(defs.push(&kinds, &agent).unwrap(), DefIndex(i));
        }
        let refused = defs.push(&kinds, &agent).unwrap_err();
        assert!(refused.message.contains("65535"), "{refused:?}");
    }
}
