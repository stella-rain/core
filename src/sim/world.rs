//! The run's definitions, resolved once when it starts (core#271): every agent a run can bring,
//! found by an index kept on the agent, so the rules never walk the stage to learn what an
//! enemy is. The stage is read-only for the whole run; what a boss changes when it comes
//! (`spawn_boss`) lives in the run's state.

use std::ops::Deref;
use std::sync::Arc;

use super::drops::DropTable;
use crate::attack::{AttackPhase, AttackRef, Emitter, PhaseKind, ZoneShape};
use crate::attack_presets::AttackPresets;
use crate::behaviour::{Action, Skill};
use crate::expertise::Gauge;
use crate::id::ContentId;
use crate::kinds::KindTable;
use crate::rewards::EquipmentRules;
use crate::run_content::RunContent;
use crate::sim::state::AgentSource;
use crate::stage::{Boss, Stage};
use crate::threat_colours::{ThreatColourIndex, ThreatColours};
use crate::validate::Issue;
use crate::weapon::{RepeatBleed, Weapon, WeaponRef};

use super::state::PlayerIdx;

use super::attack_table::{self, AttackKeys};
use super::{VIEW_HALF_HEIGHT, VIEW_HALF_WIDTH};

mod defs;
mod grants;
mod tuning;
pub(crate) use defs::{AgentDef, AgentDefs, BossDef, HordeDefs, HordeSpec};
#[cfg(test)]
use defs::{leash_eligible, roams};
pub(crate) use grants::{
    BreakerGrant, BurstGrant, CounterGrant, OrbBreakGrant, SpiritGrant, TempoGrant, TriggerGrant,
};
use grants::{
    any_step, collect_bleeds, grant_breakers, grant_counters, grant_gauges, grant_repeat_breaks,
    grant_spirits, grant_tempos, grant_triggers, grant_weapons,
};
#[cfg(test)]
pub(super) use grants::{counter_fold, spirit_fold, tempo_fold, trigger_fold};
pub(crate) use tuning::Tuning;

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
    /// The threat colour of the areas this attack makes: that of the enemy or boss it was
    /// compiled for (the colour is part of an enemy's attack key, so one preset used by two
    /// enemies of two colours is two attacks). `None` for every attack of an enemy that names
    /// none and for every attack of the players and companions. The colour decides nothing in
    /// the simulation: it is read by the snapshot only, never hashed or saved.
    pub colour: Option<ThreatColourIndex>,
    /// Whether a phase after the first has a hit box or a `spawn`: a channelled attack, whose
    /// ranged token a cancel cools (`attacks::cancel_runs_of`). A zone-leaving attack is not.
    pub channelled: bool,
    /// Whether any phase's `spawn` lays a zone, a line or an orb (a bare `wait` does not): what
    /// the view bar holds back off view for an elite or a companion.
    pub lays: bool,
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

/// The index of the entry `name` names, `None` for no name. A name the table lacks was refused
/// before (`threat_colours::check`), so it reads as no colour here.
pub(super) fn colour_of(
    colours: &ThreatColours,
    name: Option<&ContentId>,
) -> Option<ThreatColourIndex> {
    colours.index_of(name?.as_str())
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
    /// The run's rewards as a drop roll reads them, by place in the file: filled for a run with
    /// a setup only (a stage run alone allocates nothing for it).
    pub drops: DropTable,
    /// The equipment rules of the run's pack (ADR-CONT-3 point 8): the slot layout, the level
    /// curve, the Starlight values and the drop numbers. Filled for a run with a setup only; a
    /// stage run alone holds the empty rules, whose numbers are all zero. Not hashed: the pack
    /// fixes it.
    pub equipment_rules: EquipmentRules,
    /// The drop weight in percent for each category of the equipment rules, in the rules' order:
    /// the perk set's `drop_weight` plus the setup's `drop_weights` entry (0 where neither has
    /// one; empty when neither has any at all). It scales the weight of every star of the
    /// category a drop rolls. It is the first player's setup for every member (a `PartySetup`
    /// holds one setup per player, ADR-REPL-5; per-member weights are not built yet). Not hashed:
    /// the setup, which the replay stores, fixes it.
    pub drop_weight_pct: Vec<u32>,
    /// The styles of the run's weapon the setup unlocked (`RunSetup::unlocked_styles`): bit `n` is
    /// set when the style at place `n` of the weapon's styles is offered. `None`: every style. Not
    /// hashed: the setup, which the replay stores, fixes it.
    pub styles_offered: Option<u8>,
    /// Every attack the run can start, compiled once; found by `attack_index`.
    pub attacks: Vec<AttackDef>,
    attack_keys: AttackKeys,
    /// The threat colour of each program, by `ProgramIndex`; empty when no attack of the run
    /// has a colour (every run of a pack that names none), so the snapshot's zones cost one
    /// `is_empty` test.
    program_colours: Vec<Option<ThreatColourIndex>>,
    /// The run's attack presets (CS1b-1): what a preset reference compiles from.
    attack_presets: Arc<AttackPresets>,
    /// Some compiled attack has a phase that dashes: the tick looks for a dasher only then.
    pub has_dash: bool,
    /// Some compiled phase names an `advance` (attack magnetism): the player's tick looks for a
    /// slide only then.
    pub has_advance: bool,
    /// Some style's copy of the tree gives a step a repeat: a run queues repeats only then
    /// (false for the bundled pack).
    pub has_repeat: bool,
    /// The distinct repeat-bleed rows the style copies' repeats carry, in the order first met
    /// (derived from the pack, never hashed): `Repeat.bleed` is a row's place counted from 1.
    pub repeat_bleeds: Vec<RepeatBleed>,
    /// Some style's copy of the tree gives a step a proc factor (false for the bundled pack): a
    /// combo hit reads its step's factor only then.
    pub has_proc: bool,
    /// Some elite or boss of the run names a `stagger_max`: the stagger rules run only then.
    pub has_stagger: bool,
    /// The rules name `down_ticks`: the knockdown rules run only then.
    pub has_down: bool,
    /// Some compiled attack has a `counterable` phase: the counter rules run only then.
    pub has_counter: bool,
    /// The nodes of every phase's `spawn`, by `ProgramIndex`.
    pub emitters: Vec<Vec<Emitter>>,
    /// The attack of each on-hit rune, by `Rune::index`; none for a rune the run's file does
    /// not give an attack.
    pub rune_attacks: Vec<Option<AttackIndex>>,
    /// The main character's weapon, resolved from the run's weapons file once (WP1): none for a
    /// weapon the file does not define.
    weapon: Option<Weapon>,
    /// The base tree with a style's grants applied, one per `(style, places taken)` of a style
    /// that grants something (ADR-SIM-9 point 3), at `style * 4 + taken`; empty for a run whose
    /// weapon has no grant, which is every run of the bundled pack.
    grant_weapons: Vec<Option<Weapon>>,
    /// The weapon gauge each of those copies has (the first `gauge` kind of the style's own level
    /// and the path levels taken), at the same index; empty when no style names one.
    grant_gauges: Vec<Option<Gauge>>,
    /// Some style names a gauge: the tick looks for one only then.
    pub has_gauge: bool,
    /// The spirit each of those copies has (the first `spirit` kind of the style's own level and
    /// the path levels taken), at the same index; empty when no style names one.
    grant_spirits: Vec<Option<SpiritGrant>>,
    /// Some style names a spirit: the tick looks for one only then.
    pub has_spirit: bool,
    /// The counter bonus each of those copies has (the first `counter_bonus` kind of the style's
    /// own level and the path levels taken), at the same index; empty when no style names one.
    grant_counters: Vec<Option<CounterGrant>>,
    /// Some style names a counter bonus: the tick looks for one only then.
    pub has_counter_bonus: bool,
    /// The combo tempo each of those copies has (the first `combo_tempo_pct` kind of the style's
    /// own level and the path levels taken), at the same index; empty when no style names one.
    grant_tempos: Vec<Option<TempoGrant>>,
    /// Some style names a combo tempo: the tick looks for one only then.
    pub has_tempo: bool,
    /// The giant trigger each of those copies has (the first `giant_trigger` kind of the style's
    /// own level and the path levels taken), at the same index; empty when no style names one.
    grant_triggers: Vec<Option<TriggerGrant>>,
    /// The end ring of each of those copies whose trigger names `end_ring` and whose style has a
    /// gauge whose smash has an `active` phase, at the same index: that first active phase alone.
    /// Filled by `compile_rings`, after every other attack is compiled; empty until then.
    grant_rings: Vec<Option<AttackRef>>,
    /// Some style names a giant trigger: the tick looks for one only then.
    pub has_trigger: bool,
    /// The breaker each of those copies has (the `breaker` kinds of the style's own level and the
    /// path levels taken, folded in path order), at the same index; empty when no style names one.
    grant_breakers: Vec<Option<BreakerGrant>>,
    /// Some style names a breaker: the hit pass looks for one only then.
    pub has_breaker: bool,
    /// The orb break each of those copies has (the first `repeat_breaks_orbs` kind of the style's
    /// own level and the path levels taken), at the same index; empty when no style names one.
    grant_repeat_breaks: Vec<Option<OrbBreakGrant>>,
    /// Some style names the kind: the deal of a repeat looks for the grant only then.
    pub has_repeat_break: bool,
}

/// The places of a style's path (0 to 3 taken): the copies of one style.
const PATH_PLACES: usize = 4;

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

    /// The main character's weapon: the file's definition, or the stage's inline one.
    pub fn weapon(&self) -> Option<&Weapon> {
        self.weapon.as_ref()
    }

    /// The weapon player `me` plays now: the base tree, or the copy with the grants of the style
    /// chosen and the places of its path taken (ADR-SIM-9 point 3), read from the recorded style
    /// and path, so no state or input is added. One branch for a run with no grant.
    pub(crate) fn weapon_of(&self, s: &super::state::State, me: PlayerIdx) -> Option<&Weapon> {
        if self.grant_weapons.is_empty() {
            return self.weapon();
        }
        let granted = self
            .grant_slot(s, me)
            .and_then(|at| self.grant_weapons.get(at)?.as_ref());
        granted.or_else(|| self.weapon())
    }

    /// Where the style player `me` holds and the places of its path taken put it in the grant
    /// lists (`style * PATH_PLACES + taken`), read from the recorded style and path, so no state
    /// or input is added. Under `try_styles` the path is the log of the picks, not upgrades
    /// taken: the style's own grants only (ADR-SIM-9 point 3).
    fn grant_slot(&self, s: &super::state::State, me: PlayerIdx) -> Option<usize> {
        let g = s.players.career(me).expertise.as_ref()?;
        let style = usize::from(g.style?);
        let taken = if self.try_styles {
            0
        } else {
            g.path.len().min(PATH_PLACES - 1)
        };
        Some(style * PATH_PLACES + taken)
    }

    /// The weapon gauge player `me`'s style gives, read from the same slot as `weapon_of`; none
    /// after one bool test for a run that names no gauge.
    pub(crate) fn gauge_of(&self, s: &super::state::State, me: PlayerIdx) -> Option<&Gauge> {
        if !self.has_gauge {
            return None;
        }
        self.grant_gauges.get(self.grant_slot(s, me)?)?.as_ref()
    }

    /// The spirit player `me`'s style gives, read from the same slot as `weapon_of`; none after
    /// one bool test for a run that names no spirit.
    pub(crate) fn spirit_of(&self, s: &super::state::State, me: PlayerIdx) -> Option<&SpiritGrant> {
        if !self.has_spirit {
            return None;
        }
        self.grant_spirits.get(self.grant_slot(s, me)?)?.as_ref()
    }

    /// The counter bonus player `me`'s style gives, read from the same slot as `weapon_of`; none
    /// after one bool test for a run that names no counter bonus.
    pub(crate) fn counter_of(
        &self,
        s: &super::state::State,
        me: PlayerIdx,
    ) -> Option<&CounterGrant> {
        if !self.has_counter_bonus {
            return None;
        }
        self.grant_counters.get(self.grant_slot(s, me)?)?.as_ref()
    }

    /// The combo tempo player `me`'s style gives, read from the same slot as `weapon_of`; none
    /// after one bool test for a run that names no combo tempo.
    pub(crate) fn tempo_of(&self, s: &super::state::State, me: PlayerIdx) -> Option<&TempoGrant> {
        if !self.has_tempo {
            return None;
        }
        self.grant_tempos.get(self.grant_slot(s, me)?)?.as_ref()
    }

    /// The giant trigger player `me`'s style gives, read from the same slot as `weapon_of`; none
    /// after one bool test for a run that names no giant trigger.
    pub(crate) fn trigger_of(
        &self,
        s: &super::state::State,
        me: PlayerIdx,
    ) -> Option<&TriggerGrant> {
        if !self.has_trigger {
            return None;
        }
        self.grant_triggers.get(self.grant_slot(s, me)?)?.as_ref()
    }

    /// The end ring player `me`'s giant trigger plays when it runs out: the first active phase of
    /// the style's full-gauge smash, read from the same slot as `trigger_of`; none when the trigger
    /// does not name `end_ring` or the smash has no active phase.
    pub(crate) fn ring_of(&self, s: &super::state::State, me: PlayerIdx) -> Option<&AttackRef> {
        if !self.has_trigger {
            return None;
        }
        self.grant_rings.get(self.grant_slot(s, me)?)?.as_ref()
    }

    /// Derives and compiles each end ring: the first `active` phase of the copy's full-gauge
    /// smash as compiled (the same phases `AttackKeys::compile` takes apart), alone. Run last by
    /// `compile_run`, after the runes, so no other attack's index moves.
    pub(super) fn compile_rings(&mut self) -> Result<(), Issue> {
        if !self.grant_triggers.iter().flatten().any(|g| g.end_ring) {
            return Ok(());
        }
        let mut rings = vec![None; self.grant_triggers.len()];
        for (i, ring) in rings.iter_mut().enumerate() {
            let named = self.grant_triggers[i].as_ref().is_some_and(|g| g.end_ring);
            let gauge = self.grant_gauges.get(i).and_then(Option::as_ref);
            let (true, Some(gauge)) = (named, gauge) else {
                continue;
            };
            let crate::expertise::GaugeSpend::FullSmash { attack } = &gauge.spend;
            let first = crate::presets::phases(attack, &self.attack_presets)
                .and_then(|p| p.iter().find(|p| p.kind == PhaseKind::Active).cloned());
            *ring = first.map(|p| AttackRef::Inline(vec![p]));
        }
        for ring in rings.iter().flatten() {
            self.compile_attack(ring, None)?;
        }
        self.grant_rings = rings;
        Ok(())
    }

    /// The breaker player `me`'s style gives, read from the same slot as `weapon_of`; none after
    /// one bool test for a run that names no breaker.
    pub(crate) fn breaker_of(
        &self,
        s: &super::state::State,
        me: PlayerIdx,
    ) -> Option<&BreakerGrant> {
        if !self.has_breaker {
            return None;
        }
        self.grant_breakers.get(self.grant_slot(s, me)?)?.as_ref()
    }

    /// The orb break player `me`'s style gives its repeats, read from the same slot as
    /// `weapon_of` at the deal (a repeat queued before a pick and dealt after reads the new grant:
    /// a repeat lasts at most 120 ticks and a pick is a UI action); none after one bool test for
    /// a run that names no such kind.
    pub(crate) fn repeat_break_of(
        &self,
        s: &super::state::State,
        me: PlayerIdx,
    ) -> Option<&OrbBreakGrant> {
        if !self.has_repeat_break {
            return None;
        }
        self.grant_repeat_breaks
            .get(self.grant_slot(s, me)?)?
            .as_ref()
    }

    /// Every full-gauge attack, for the attack table.
    pub(super) fn granted_gauges(&self) -> impl Iterator<Item = &Gauge> {
        self.grant_gauges.iter().flatten()
    }

    /// Every copy of the weapon with a style's grants, for the attack table.
    pub(super) fn granted_weapons(&self) -> impl Iterator<Item = &Weapon> {
        self.grant_weapons.iter().flatten()
    }

    /// Compiles `attack` unless the run has it already, and returns its index.
    pub fn compile_attack(
        &mut self,
        attack: &AttackRef,
        colour: Option<ThreatColourIndex>,
    ) -> Result<AttackIndex, Issue> {
        let index = self.attack_keys.compile(
            &mut self.attacks,
            &mut self.emitters,
            &self.attack_presets,
            attack,
            colour,
        )?;
        self.has_dash |= self.attacks[usize::from(index.0)]
            .phases
            .iter()
            .any(|p| p.dash.is_some());
        self.has_advance |= self.attacks[usize::from(index.0)]
            .phases
            .iter()
            .any(|p| p.advance.is_some());
        Ok(index)
    }

    /// The index of `attack`, which the run compiled when it started.
    pub fn attack_index(
        &self,
        attack: &AttackRef,
        colour: Option<ThreatColourIndex>,
    ) -> AttackIndex {
        self.attack_keys
            .find(attack, colour)
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

    /// The place, counted from 1, of `bleed`'s row in `repeat_bleeds`; 0 for no bleed, and for a
    /// row the table lacks (a repeat then bleeds nothing: fail closed).
    pub(crate) fn repeat_bleed_index(&self, bleed: Option<RepeatBleed>) -> u8 {
        bleed
            .and_then(|b| self.repeat_bleeds.iter().position(|r| *r == b))
            .and_then(|i| u8::try_from(i + 1).ok())
            .unwrap_or(0)
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
    use crate::behaviour::Agent;
    use crate::rules::Rules;
    use crate::run_content::RunContent;

    #[test]
    fn a_run_with_more_than_u16_max_definitions_is_refused() {
        let stage = RunContent::bundled().trial().clone();
        let kinds = KindTable::of_stage(&stage).unwrap();
        let agent = stage.waves[0].enemy.clone();
        let mut defs = AgentDefs::default();
        for i in 0..=u16::MAX {
            assert_eq!(
                defs.push(
                    &kinds,
                    &agent,
                    AgentSource::Wave,
                    false,
                    &ThreatColours::default(),
                )
                .unwrap(),
                DefIndex(i)
            );
        }
        let refused = defs
            .push(
                &kinds,
                &agent,
                AgentSource::Wave,
                false,
                &ThreatColours::default(),
            )
            .unwrap_err();
        assert!(refused.message.contains("65535"), "{refused:?}");
    }

    #[test]
    fn the_party_damage_table_reaches_the_tuning() {
        assert_eq!(
            World::for_test(RunContent::bundled().trial().clone())
                .tuning
                .party_damage_pct,
            [100, 100, 100]
        );
        let mut v: serde_json::Value =
            serde_json::from_slice(crate::pack::ContentFiles::bundled_file("rules")).unwrap();
        v["party_damage_pct"] = serde_json::json!([100, 60, 30]);
        let rules =
            Rules::parse(v.to_string().as_bytes(), crate::version::Versions::CURRENT).unwrap();
        assert_eq!(Tuning::of(&rules).party_damage_pct, [100, 60, 30]);
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
