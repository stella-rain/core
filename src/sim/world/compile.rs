//! World::new and the resolvers it runs once when the run is built.

use super::grants::{
    any_step, collect_bleeds, grant_breakers, grant_counters, grant_gauges, grant_repeat_breaks,
    grant_spirits, grant_tempos, grant_triggers, grant_weapons,
};
use super::{
    AgentDefs, BossDef, BossIndex, HordeDefs, HordeSpec, ProgramIndex, RangedPool, Tuning, World,
};
use crate::behaviour::Action;
use crate::behaviour::Skill;
use crate::id::ContentId;
use crate::kinds::KindTable;
use crate::rewards::EquipmentRules;
use crate::run_content::RunContent;
use crate::sim::attack_table::{self, AttackKeys};
use crate::sim::drops::DropTable;
use crate::sim::state::AgentSource;
use crate::stage::Boss;
use crate::stage::Stage;
use crate::threat_colours::ThreatColourIndex;
use crate::validate::Issue;
use crate::weapon::WeaponRef;
use std::sync::Arc;

impl World {
    /// Compiles `stage` and, for a `time_limit` run, its `horde`.
    pub fn new(
        stage: Stage,
        horde: Option<HordeSpec>,
        kinds: &KindTable,
        content: &RunContent,
    ) -> Result<World, Issue> {
        let colours = content.threat_colours();
        // The threat colour each enemy names must be an entry of the run's table: the stage's
        // enemies and bosses, and the bosses a horde brings.
        {
            let horde_bosses: Vec<&Boss> = horde
                .iter()
                .flat_map(|h| h.bosses.iter().chain(std::iter::once(&h.template)))
                .collect();
            crate::threat_colours::check(&stage, &horde_bosses, colours)?;
            // An agent naming a `poise` needs rules naming `flinch_ticks`: it could never flinch.
            if content.rules().flinch_ticks() == 0 {
                let mut found = None;
                crate::kinds::visit_foes(&stage, &horde_bosses, &mut |foe, path| {
                    if let crate::kinds::Foe::Agent(a) = foe {
                        if found.is_none() && a.poise.is_some() {
                            found = Some(format!("{path}.poise"));
                        }
                    }
                });
                if let Some(path) = found {
                    return Err(Issue {
                        path,
                        message: "needs flinch_ticks in the run's rules".into(),
                    });
                }
            }
            // A gauge needs the ticks of its stagger, from the rules or its own definition.
            if content.rules().staggered_ticks() == 0 {
                let mut found = None;
                crate::kinds::visit_foes(&stage, &horde_bosses, &mut |foe, path| {
                    let (max, ticks) = match foe {
                        crate::kinds::Foe::Agent(a) => (a.stagger_max, a.staggered_ticks),
                        crate::kinds::Foe::Boss(b) => (b.stagger_max, b.staggered_ticks),
                    };
                    if found.is_none() && max.is_some() && ticks.is_none() {
                        found = Some(format!("{path}.stagger_max"));
                    }
                });
                if let Some(path) = found {
                    return Err(Issue {
                        path,
                        message: "needs staggered_ticks, in the run's rules or its own".into(),
                    });
                }
            }
        }
        let mut defs = AgentDefs::build(&stage, kinds, colours)?;
        let mut bosses = Vec::new();
        let mut push = |defs: &mut AgentDefs, boss: Boss| -> Result<BossIndex, Issue> {
            let at = u16::try_from(bosses.len()).map_err(|_| Issue {
                path: "$".into(),
                message: format!("a run brings more than {} boss definitions", u16::MAX),
            })?;
            bosses.push(BossDef::compile(defs, kinds, colours, boss)?);
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
        let grant_weapons = grant_weapons(main, weapon.as_ref(), content);
        let grant_gauges = grant_gauges(main, weapon.as_ref(), content);
        let grant_spirits = grant_spirits(main, weapon.as_ref(), content);
        let grant_counters = grant_counters(main, weapon.as_ref(), content);
        let grant_tempos = grant_tempos(main, weapon.as_ref(), content);
        let grant_triggers = grant_triggers(main, weapon.as_ref(), content);
        let grant_breakers = grant_breakers(main, weapon.as_ref(), content);
        let grant_repeat_breaks = grant_repeat_breaks(main, weapon.as_ref(), content);
        // A stage's own weapon is judged by the content that runs it (ADR-CONT-3 point 6): the
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
        // A boss that outruns a player cannot be kited.
        if let Some(chase) = bosses.iter().find_map(|b| b.boss.chase)
            && chase.speed.0 >= rules.player_speed().0
        {
            return Err(Issue {
                path: "$.boss.chase.speed".into(),
                message: format!(
                    "below the players' speed, {} (found {})",
                    rules.player_speed().0,
                    chase.speed.0
                ),
            });
        }
        let target = stage.crowd_target.unwrap_or_else(|| rules.crowd_target());
        let mut world = World {
            crowd_target: rules.resolved_target(Some(target)),
            enemy_cap: rules.enemy_cap_for(target),
            tuning: Tuning::of(rules),
            director: horde.is_some(),
            drops: DropTable::default(),
            equipment_rules: EquipmentRules::default(),
            drop_weight_pct: Vec::new(),
            styles_offered: None,
            stage,
            defs,
            bosses,
            horde,
            attacks: Vec::new(),
            attack_keys: AttackKeys::default(),
            program_colours: Vec::new(),
            attack_presets: Arc::new(content.attack_presets().clone()),
            has_dash: false,
            has_advance: false,
            has_repeat: false,
            repeat_bleeds: Vec::new(),
            has_proc: false,
            has_stagger: false,
            has_down: false,
            has_counter: false,
            emitters: Vec::new(),
            rune_attacks: Vec::new(),
            grant_weapons,
            has_gauge: grant_gauges.iter().any(Option::is_some),
            grant_gauges,
            has_spirit: grant_spirits.iter().any(Option::is_some),
            grant_spirits,
            has_counter_bonus: grant_counters.iter().any(Option::is_some),
            grant_counters,
            has_tempo: grant_tempos.iter().any(Option::is_some),
            grant_tempos,
            has_trigger: grant_triggers.iter().any(Option::is_some),
            grant_triggers,
            grant_rings: Vec::new(),
            has_breaker: grant_breakers.iter().any(Option::is_some),
            grant_breakers,
            has_repeat_break: grant_repeat_breaks.iter().any(Option::is_some),
            grant_repeat_breaks,
            weapon,
        };
        // The shared flinch ID is built here, not on the first flinch of a steady tick.
        super::snapshot::warm_flinch_id();
        attack_table::compile_run(&mut world, content)?;
        world.resolve_ranged_pools();
        world.resolve_view_bars();
        world.resolve_program_colours();
        world.has_dash = world
            .attacks
            .iter()
            .any(|a| a.phases.iter().any(|p| p.dash.is_some()));
        // A counter bonus's slide also needs magnetism to run.
        world.has_advance = world
            .attacks
            .iter()
            .any(|a| a.phases.iter().any(|p| p.advance.is_some()))
            || world
                .grant_counters
                .iter()
                .flatten()
                .any(|c| c.advance.0 > 0);
        // One walk of each style copy's tree, once (ADR-SIM-9 point 3).
        for copy in world.grant_weapons.iter().flatten() {
            world.has_repeat |= any_step(&copy.combo, &|s| !s.repeats.is_empty());
            world.has_proc |= any_step(&copy.combo, &|s| s.proc_pct.is_some());
            collect_bleeds(&copy.combo, &mut world.repeat_bleeds);
        }
        world.has_stagger = world
            .defs
            .defs_iter()
            .any(|d| d.agent.stagger_max.is_some())
            || world.bosses.iter().any(|b| b.boss.stagger_max.is_some());
        world.has_down = world.tuning.down_ticks > 0;
        world.has_counter = world
            .attacks
            .iter()
            .any(|a| a.phases.iter().any(|p| p.counterable));
        Ok(world)
    }

    /// Fills `program_colours` from the compiled attacks, unless none has a colour.
    fn resolve_program_colours(&mut self) {
        if self.attacks.iter().all(|a| a.colour.is_none()) {
            return;
        }
        self.program_colours = vec![None; self.emitters.len()];
        for attack in &self.attacks {
            for program in &attack.programs {
                self.program_colours[program.0 as usize] = attack.colour;
            }
        }
    }

    /// The threat colour of the area a zone with `program` made: `None` for an enemy that
    /// names none, a friendly area, and every run of a pack that names no colour.
    pub(crate) fn program_colour(&self, program: ProgramIndex) -> Option<ThreatColourIndex> {
        self.program_colours
            .get(program.0 as usize)
            .copied()
            .flatten()
    }

    /// The threat colour of the attacks `owner` starts: the definition's, `None` for a player,
    /// a companion and an owner that is gone.
    pub(crate) fn colour_of_owner(
        &self,
        s: &super::state::State,
        owner: super::entity::Who,
    ) -> Option<ThreatColourIndex> {
        use super::entity::Who;
        match owner {
            Who::Enemy(i) => self.defs.get(s.enemies.get(i)?.def).colour,
            Who::Boss | Who::BossPart(_) => s.boss.as_ref().and_then(|b| self.boss(b.boss).colour),
            Who::Player(_) | Who::Companion(_) => None,
        }
    }

    /// Sets each agent definition's `ranged_pools`, now that the attacks are compiled.
    fn resolve_ranged_pools(&mut self) {
        for i in 0..self.defs.defs.len() {
            // An elite is never held to the tokens (the validator refuses `ranged` on one).
            if self.defs.defs[i].agent.elite.is_some() {
                continue;
            }
            let colour = self.defs.defs[i].colour;
            let pools: Vec<Option<RangedPool>> = self.defs.defs[i]
                .agent
                .rules
                .iter()
                .map(|rule| match &rule.action {
                    Action::Attack {
                        attack,
                        ranged: true,
                    } => Some(self.attacks[usize::from(self.attack_index(attack, colour).0)].pool),
                    _ => None,
                })
                .collect();
            if pools.iter().any(Option::is_some) {
                self.defs.defs[i].ranged_pools = pools;
            }
        }
    }

    /// Sets each elite's and companion's `view_barred`, now that the attacks are compiled.
    fn resolve_view_bars(&mut self) {
        for i in 0..self.defs.defs.len() {
            let def = &self.defs.defs[i];
            let companion = self
                .defs
                .keys
                .iter()
                .any(|(k, at)| usize::from(at.0) == i && k.source == AgentSource::Companion);
            if def.agent.elite.is_none() && !companion {
                continue;
            }
            let colour = def.colour;
            let barred: Vec<bool> = def
                .agent
                .rules
                .iter()
                .map(|rule| match &rule.action {
                    Action::Attack { attack, .. } => {
                        self.attacks[usize::from(self.attack_index(attack, colour).0)].lays
                    }
                    Action::Cast {
                        skill: Skill::Shot { .. },
                    } => true,
                    _ => false,
                })
                .collect();
            if barred.iter().any(|b| *b) {
                self.defs.defs[i].view_barred = barred;
            }
        }
    }
}
