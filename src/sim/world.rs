//! The run's definitions, resolved once when it starts (core#271): every agent a run can bring,
//! found by an index kept on the agent, so the rules never walk the stage to learn what an
//! enemy is. The stage is read-only for the whole run; what a boss changes when it comes
//! (`spawn_boss`) lives in the run's state.

use std::ops::Deref;
use std::sync::Arc;

use crate::attack::{AttackPhase, AttackRef, Emitter, ZoneShape};
use crate::attack_presets::AttackPresets;
use crate::behaviour::{Action, Agent, Movement};
use crate::engine::{AgentKey, AgentSource};
use crate::fixed::Fx;
use crate::id::ContentId;
use crate::kinds::{KindIndex, KindTable};
use crate::rules::Rules;
use crate::run_content::RunContent;
use crate::stage::{Boss, Stage};
use crate::time_limit::EliteTier;
use crate::validate::Issue;
use crate::weapon::{Weapon, WeaponRef};

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
    /// The same ID, shared, so a `PlayerHit` names it without allocating (PHIT).
    pub preset_id: Option<Arc<str>>,
    /// The program of each phase's `spawn`, in the order of `phases`.
    pub programs: Vec<ProgramIndex>,
    /// The ranged-token pool a ranged start of this attack draws from (RA1, DS83).
    pub pool: RangedPool,
}

/// The two pools of the ranged tokens (RA1, DS83): an attack that lays a line zone takes the
/// line token, any other the zone tokens. Decided once, where the attack is compiled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RangedPool {
    Zone,
    Line,
}

impl RangedPool {
    /// Both pools, in `index` order.
    pub const ALL: [RangedPool; 2] = [RangedPool::Zone, RangedPool::Line];

    /// The pool's place in `ALL`, for per-pool arrays.
    pub fn index(self) -> usize {
        self as usize
    }

    /// The pool of an attack with these phases: `Line` if any phase spawns a zone of shape
    /// `line` (also inside an orb's burst), else `Zone`.
    pub fn of(phases: &[AttackPhase]) -> RangedPool {
        fn lays_line(list: &[Emitter]) -> bool {
            list.iter().any(|n| match n {
                Emitter::Zone { shape, .. } => matches!(shape, ZoneShape::Line { .. }),
                Emitter::Orb { burst, .. } => lays_line(burst),
                Emitter::Repeat { body, .. }
                | Emitter::Ring { body, .. }
                | Emitter::Spread { body, .. }
                | Emitter::Rotate { body, .. } => lays_line(body),
                Emitter::Wait { .. } => false,
            })
        }
        if phases.iter().any(|p| lays_line(&p.spawn)) {
            RangedPool::Line
        } else {
            RangedPool::Zone
        }
    }
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
    /// The ranged pool of each rule, by rule; empty when no rule is a `ranged` attack (RA1).
    pub ranged_pools: Vec<Option<RangedPool>>,
    /// The leash may move an enemy of this definition (EC2b): `leash_eligible`. Decided once when
    /// the world is compiled; never hashed.
    pub leash: bool,
}

/// Whether the leash may move an enemy made from `agent` under `source`: a whitelist (DS87).
/// It roams (`roams`) and a wave or a regular wave enemy's summon made it; a guardian, an escort
/// and the boss's summons stay with their owner, and so do an elite's. `owner_is_elite` says the
/// wave enemy that owns a `Summoned` agent is an elite. The match is exhaustive.
fn leash_eligible(agent: &Agent, source: AgentSource, owner_is_elite: bool) -> bool {
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
fn roams(agent: &Agent) -> bool {
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
    defs: Vec<AgentDef>,
    keys: Vec<(AgentKey, DefIndex)>,
}

impl AgentDefs {
    pub(super) fn agents(&self) -> impl Iterator<Item = &Agent> {
        self.defs.iter().map(|d| &d.agent)
    }

    fn push(
        &mut self,
        kinds: &KindTable,
        agent: &Agent,
        source: AgentSource,
        owner_is_elite: bool,
    ) -> Result<DefIndex, Issue> {
        let at = u16::try_from(self.defs.len()).map_err(|_| Issue {
            path: "$".into(),
            message: format!("a run brings more than {} agent definitions", u16::MAX),
        })?;
        self.defs.push(AgentDef {
            kind: kinds.of(&agent.base),
            agent: agent.clone(),
            ranged_pools: Vec::new(),
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
    ) -> Result<(), Issue> {
        let at = self.push(kinds, agent, key.source, owner_is_elite)?;
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
            defs.keyed(kinds, key, agent, false)?;
        }
        for (i, wave) in stage.waves.iter().enumerate() {
            let at = |source, rule| AgentKey {
                source,
                index: index(i),
                rule,
            };
            defs.keyed(kinds, at(AgentSource::Wave, 0), &wave.enemy, false)?;
            for (r, rule) in wave.enemy.rules.iter().enumerate() {
                if let Action::Summon { agent, .. } = &rule.action {
                    let owner_is_elite = wave.enemy.elite.is_some();
                    defs.keyed(
                        kinds,
                        at(AgentSource::Summoned, index(r)),
                        agent,
                        owner_is_elite,
                    )?;
                }
            }
            if let Movement::Shrine { guardians, .. } = &wave.enemy.movement {
                defs.keyed(kinds, at(AgentSource::Guardian, 0), &guardians.enemy, true)?;
            }
            if let Some(escort) = wave.enemy.elite.as_ref().and_then(|e| e.escort.as_ref()) {
                defs.keyed(kinds, at(AgentSource::Escort, 0), &escort.enemy, true)?;
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
                    summons.push((
                        index(p),
                        index(r),
                        defs.push(kinds, agent, AgentSource::BossRule, true)?,
                    ));
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

/// The run's rules numbers (CS1b-3): the designer-facing fields of the pack's `rules.json`, read
/// once when the run is built, and the spawn rectangle derived from the view and the margin. A
/// `Copy` struct of plain values, so a reader costs a field read and allocates nothing.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tuning {
    pub player_speed: i32,
    pub player_radius: i32,
    pub orb_radius: i32,
    pub hit_invulnerable_ticks: u32,
    pub aggro_per_damage: u32,
    pub aggro_per_buff: u32,
    pub aggro_per_heal_point: u32,
    pub aggro_decay: u32,
    /// How far a regular enemy may stray before the leash moves it, and where it lands (EC2, DS80).
    pub leash_reach_x: Fx,
    pub leash_reach_y: Fx,
    pub leash_land_x: Fx,
    pub leash_land_y: Fx,
    /// The spawn rectangle's half width (VS1, DS109) and half height: the view (`VIEW_HALF_WIDTH`, `VIEW_HALF_HEIGHT`) and the file's margin.
    pub spawn_edge_x: Fx,
    pub spawn_edge_y: Fx,
    pub ranged_token_cooldown_ticks: u32,
    pub ranged_zone_tokens: u32,
    pub ranged_line_tokens: u32,
    pub orbit_step: i32,
    pub boss_arrival_above: i32,
    pub default_xp: u32,
    pub offer_size: usize,
    pub defence_k: u64,
    pub crit_damage_base_pct: u64,
}

impl Tuning {
    fn of(rules: &Rules) -> Tuning {
        let margin = rules.spawn_margin();
        Tuning {
            player_speed: rules.player_speed().0,
            player_radius: rules.player_radius().0,
            orb_radius: rules.orb_radius().0,
            hit_invulnerable_ticks: rules.hit_invulnerable_ticks(),
            aggro_per_damage: rules.aggro_per_damage(),
            aggro_per_buff: rules.aggro_per_buff(),
            aggro_per_heal_point: rules.aggro_per_heal_point(),
            aggro_decay: rules.aggro_decay(),
            leash_reach_x: rules.leash_reach_x(),
            leash_reach_y: rules.leash_reach_y(),
            leash_land_x: rules.leash_land_x(),
            leash_land_y: rules.leash_land_y(),
            spawn_edge_x: Fx(super::VIEW_HALF_WIDTH.0 + margin.0),
            spawn_edge_y: Fx(super::VIEW_HALF_HEIGHT.0 + margin.0),
            ranged_token_cooldown_ticks: rules.ranged_token_cooldown_ticks(),
            ranged_zone_tokens: rules.ranged_zone_tokens(),
            ranged_line_tokens: rules.ranged_line_tokens(),
            orbit_step: i32::try_from(rules.orbit_step()).unwrap_or(i32::MAX),
            boss_arrival_above: rules.boss_arrival_above().0,
            default_xp: rules.default_xp(),
            offer_size: usize::from(rules.offer_size()),
            defence_k: u64::from(rules.defence_k()),
            crit_damage_base_pct: u64::from(rules.crit_damage_base_pct()),
        }
    }
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
    /// The crowd target of the run (DIR1, DS103): the stage's override or the pack's, kept so
    /// that the cap stays within the engine ceiling. Resolved once here.
    pub crowd_target: u32,
    /// The most enemies alive at once: the target and the pack's headroom, derived here and
    /// never stored in a file. Every spawn past it is dropped and counted.
    pub enemy_cap: u32,
    /// The run's rules numbers, from the pack's `rules.json`.
    pub tuning: Tuning,
    /// Whether the spawn director acts: a horde run holds its wave spawns at the target; a
    /// stage's waves are an authored timeline and keep their timing.
    pub director: bool,
    /// Every attack the run can start, compiled once; found by `attack_index`.
    pub attacks: Vec<AttackDef>,
    attack_keys: AttackKeys,
    /// The run's attack presets (CS1b-1): what a preset reference compiles from.
    attack_presets: Arc<AttackPresets>,
    /// Some compiled attack has a phase that dashes: the tick looks for a dasher only then.
    pub has_dash: bool,
    /// The nodes of every phase's `spawn`, by `ProgramIndex`.
    pub emitters: Vec<Vec<Emitter>>,
    /// The attack of each on-hit rune, by `Rune::index`; none for a rune the run's file does
    /// not give an attack.
    pub rune_attacks: Vec<Option<AttackIndex>>,
    /// The main character's weapon, resolved from the run's weapons file once (WP1): none for a
    /// weapon the file does not define.
    weapon: Option<Weapon>,
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
        let main = &stage.player.weapon;
        if let WeaponRef::Preset {
            id,
            tree: Some(tree),
        } = main
        {
            if let Some(message) = content.weapons().tree_problem(id.as_str(), tree.as_str()) {
                return Err(Issue {
                    path: "$.player.weapon.preset.tree".into(),
                    message: message.into(),
                });
            }
        }
        let weapon = content.weapons().resolve(main).cloned();
        // A stage's own weapon is judged by the content that runs it (ADR-CONT-7 point 6): the
        // held-step checks that read a preset's phases cannot run before the presets are known.
        if let WeaponRef::Inline(w) = main {
            if let Some(found) = crate::presets::hold_problems(w, content.attack_presets())
                .into_iter()
                .next()
            {
                return Err(Issue {
                    path: format!(
                        "$.player.weapon.inline{}",
                        found.path.trim_start_matches('$')
                    ),
                    message: found.message,
                });
            }
        }
        let rules = content.rules();
        let target = stage.crowd_target.unwrap_or_else(|| rules.crowd_target());
        let mut world = World {
            crowd_target: rules.resolved_target(Some(target)),
            enemy_cap: rules.enemy_cap_for(target),
            tuning: Tuning::of(rules),
            director: horde.is_some(),
            stage,
            defs,
            bosses,
            horde,
            attacks: Vec::new(),
            attack_keys: AttackKeys::default(),
            attack_presets: Arc::new(content.attack_presets().clone()),
            has_dash: false,
            emitters: Vec::new(),
            rune_attacks: Vec::new(),
            weapon,
        };
        attack_table::compile_run(&mut world, content)?;
        world.resolve_ranged_pools();
        world.has_dash = world
            .attacks
            .iter()
            .any(|a| a.phases.iter().any(|p| p.dash.is_some()));
        Ok(world)
    }

    /// Sets each agent definition's `ranged_pools`, now that the attacks are compiled.
    fn resolve_ranged_pools(&mut self) {
        for i in 0..self.defs.defs.len() {
            // An elite is never held to the tokens (the validator refuses `ranged` on one).
            if self.defs.defs[i].agent.elite.is_some() {
                continue;
            }
            let pools: Vec<Option<RangedPool>> = self.defs.defs[i]
                .agent
                .rules
                .iter()
                .map(|rule| match &rule.action {
                    Action::Attack {
                        attack,
                        ranged: true,
                    } => Some(self.attacks[usize::from(self.attack_index(attack).0)].pool),
                    _ => None,
                })
                .collect();
            if pools.iter().any(Option::is_some) {
                self.defs.defs[i].ranged_pools = pools;
            }
        }
    }

    /// The main character's weapon: the file's definition, or the stage's inline one.
    pub fn weapon(&self) -> Option<&Weapon> {
        self.weapon.as_ref()
    }

    /// Compiles `attack` unless the run has it already, and returns its index.
    pub fn compile_attack(&mut self, attack: &AttackRef) -> Result<AttackIndex, Issue> {
        let index = self.attack_keys.compile(
            &mut self.attacks,
            &mut self.emitters,
            &self.attack_presets,
            attack,
        )?;
        self.has_dash |= self.attacks[usize::from(index.0)]
            .phases
            .iter()
            .any(|p| p.dash.is_some());
        Ok(index)
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

impl World {
    /// The attack whose phases carry `program`, for the hit event to name.
    pub(crate) fn attack_of(&self, program: ProgramIndex) -> Option<AttackIndex> {
        let at = self
            .attacks
            .iter()
            .position(|a| a.programs.contains(&program))?;
        u16::try_from(at).ok().map(AttackIndex)
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
            assert_eq!(
                defs.push(&kinds, &agent, AgentSource::Wave, false).unwrap(),
                DefIndex(i)
            );
        }
        let refused = defs
            .push(&kinds, &agent, AgentSource::Wave, false)
            .unwrap_err();
        assert!(refused.message.contains("65535"), "{refused:?}");
    }

    fn agent_of(movement: serde_json::Value, rules: serde_json::Value) -> Agent {
        serde_json::from_value(serde_json::json!({
            "base": "bat", "hp": 10, "radius": 2048, "movement": movement, "rules": rules }))
        .unwrap()
    }

    #[test]
    fn only_a_free_mover_without_a_move_to_rule_roams() {
        use serde_json::json;
        let at = json!({ "x": 0, "y": 0 });
        let free = [
            json!({ "type": "follow", "distance": 0 }),
            json!({ "type": "orbit", "radius": 1000 }),
            json!({ "type": "straight", "vx": 0, "vy": 0 }),
        ];
        for m in free {
            assert!(roams(&agent_of(m, json!([]))), "free mover");
        }
        let anchored = [
            json!({ "type": "hold", "at": at }),
            json!({ "type": "flee_when_disturbed", "disturb_radius": 1, "speed": 1,
                    "leave_after_ticks": 1 }),
        ];
        for m in anchored {
            assert!(!roams(&agent_of(m, json!([]))), "anchored mover");
        }
        let move_to = json!([{ "when": { "type": "always" }, "target": "self",
            "do": { "type": "move_to", "to": at, "ticks": 5 }, "cooldown_ticks": 0 }]);
        let straight = json!({ "type": "straight", "vx": 0, "vy": 0 });
        assert!(!roams(&agent_of(straight, move_to)), "a move_to rule");
    }

    #[test]
    fn leash_eligible_by_source_and_owner() {
        use serde_json::json;
        let still = || agent_of(json!({ "type": "straight", "vx": 0, "vy": 0 }), json!([]));
        let held = agent_of(
            json!({ "type": "hold", "at": { "x": 0, "y": 0 } }),
            json!([]),
        );
        assert!(leash_eligible(&still(), AgentSource::Wave, false));
        assert!(!leash_eligible(&held, AgentSource::Wave, false));
        assert!(leash_eligible(&still(), AgentSource::Summoned, false));
        assert!(!leash_eligible(&still(), AgentSource::Summoned, true));
        assert!(!leash_eligible(&still(), AgentSource::Guardian, true));
        assert!(!leash_eligible(&still(), AgentSource::Escort, true));
        assert!(!leash_eligible(&still(), AgentSource::BossRule, true));
        assert!(!leash_eligible(&still(), AgentSource::Companion, false));
    }
}
