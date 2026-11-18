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
use crate::expertise::{
    CounterEffect, Gauge, MAX_ORB_BREAK_PCT, MAX_SPIRIT_BLEED_STACKS, SpiritEffect, StyleKind,
    TempoEffect, TriggerEffect,
};
use crate::fixed::Fx;
use crate::id::ContentId;
use crate::kinds::KindTable;
use crate::limits::{
    MAX_ADVANCE_DISTANCE, MAX_COUNTER_WINDOW_TICKS, MAX_GAUGE_DAMAGE_PCT, MAX_PCT,
    MAX_SPIRIT_DECAY_TICKS, MAX_SPIRIT_LEVEL, MAX_TEMPO_STACKS, MAX_TRIGGER_TICKS,
    MAX_WEAPON_GAUGE,
};
use crate::rewards::EquipmentRules;
use crate::run_content::RunContent;
use crate::run_stats::MAX_ATTACK_SPEED_PCT;
use crate::sim::state::AgentSource;
use crate::stage::{Boss, Stage};
use crate::threat_colours::{ThreatColourIndex, ThreatColours};
use crate::validate::Issue;
use crate::weapon::{RepeatBleed, Weapon, WeaponRef};

use super::state::PlayerIdx;

use super::attack_table::{self, AttackKeys};
use super::{VIEW_HALF_HEIGHT, VIEW_HALF_WIDTH};

mod defs;
mod tuning;
pub(crate) use defs::{AgentDef, AgentDefs, BossDef, HordeDefs, HordeSpec};
#[cfg(test)]
use defs::{leash_eligible, roams};
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

/// A style's orb break resolved once: the chance in percent and the radius round what a repeat
/// strikes, inside their caps (ADR-SVC-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OrbBreakGrant {
    pub chance_pct: u8,
    pub radius: Fx,
}

/// A style's breaker resolved once: a factor in percent on the stagger fills, on the poise damage
/// of the combo hits and on a smash's damage on a boss part (each 100: none), and the burst round
/// a target the player staggers, every number inside its cap (ADR-SVC-1, ADR-SIM-5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BreakerGrant {
    pub stagger_pct: u32,
    pub poise_pct: u32,
    pub part_break_pct: u32,
    /// A factor on the ticks of a stagger the player causes (100: none).
    pub stagger_ticks_pct: u32,
    pub burst: Option<BurstGrant>,
}

/// A stagger burst resolved once: its radius, the damage a percent of the staggering hit's (0:
/// none), a stun in ticks (0: none) and whether it knocks down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BurstGrant {
    pub radius: i32,
    pub damage_pct: u32,
    pub stun_ticks: u32,
    pub knock_down: bool,
}

/// A style's giant trigger resolved once: how long the state runs and each effect folded into its
/// field (0 where it has none, 100 for the rally factor), every number inside its cap (ADR-SVC-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TriggerGrant {
    pub ticks: u16,
    pub range_pct: u32,
    pub smash_pct: u32,
    pub rally_pct: u32,
    pub super_armour: bool,
    pub hold_move_pct: u32,
    /// The trigger names `end_ring`: its end plays the ring `World::ring_of` holds.
    pub end_ring: bool,
}

/// A style's combo tempo resolved once: the most stacks, each stack's attack speed and whether a
/// full-stack smash keeps them, every number inside its cap (ADR-SVC-1, ADR-SIM-5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TempoGrant {
    pub max_stacks: u8,
    pub speed_pct: u32,
    pub keep_on_smash: bool,
    /// Full stacks in a held hold are super armour, and only that armour shelters the stacks.
    pub hold_armour: bool,
    /// Percent added to the range of every attack the combo starts while the stacks are full, 0
    /// where no level names it.
    pub range_pct_at_full: u32,
    /// A combo that ends on a full-stack smash leaves the stacks at a reset that is not the window
    /// running out (`player::tempo_carries`).
    pub carry_at_full: bool,
}

/// A style's counter bonus resolved once: the window and each effect folded into its field, 0
/// where it has none (100 for the stagger factor), every number inside its cap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CounterGrant {
    pub window: u16,
    pub damage_pct: u32,
    pub stagger_pct: u32,
    pub stun_ticks: u32,
    pub advance: Fx,
    /// Give back one spent evade charge at the counter's first landed hit.
    pub refund_evade: bool,
    /// Percent of the weapon gauge's bar filled at that hit, 0 for none.
    pub gauge_fill_pct: u32,
}

/// A style's spirit resolved once: the countdown and, for each level, the effects folded into
/// one flat entry, every number clamped to its cap again (ADR-SVC-1, ADR-SIM-5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SpiritGrant {
    /// Ticks a level lasts without a smash.
    pub decay_ticks: u16,
    /// Entry `i` is level `i + 1`'s own effects.
    pub levels: Vec<SpiritFx>,
}

/// One spirit level's effects, 0 where it has none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SpiritFx {
    /// Added to the attack-up percent of a combo step.
    pub damage_pct: u32,
    /// Added to the range percent of a combo step.
    pub range_pct: u32,
    /// Bleed stacks a smash hit adds on top of its own.
    pub bleed_stacks: u32,
}

impl SpiritGrant {
    /// The effects of `level` (1 to the top), none for level 0.
    pub fn fx(&self, level: u8) -> Option<&SpiritFx> {
        self.levels.get(usize::from(level.checked_sub(1)?))
    }
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

/// The copies of `base`, the run's weapon, for each style of the weapon's expertise that grants
/// a kind: one per number of places of its path taken (0 to 3). Empty when the weapon is not a
/// preset with expertise content or no style grants anything.
fn grant_weapons(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<Weapon>> {
    let (WeaponRef::Preset { id, .. }, Some(base)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let grants = |s: &crate::expertise::Style| {
        !s.kinds.is_empty()
            || crate::expertise::style_levels(s)
                .into_iter()
                .any(|(_, o)| o.is_some_and(|o| !o.kinds.is_empty()))
    };
    if !expertise.styles.iter().any(grants) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(
                grants(style).then(|| crate::expertise::weapon_with_style(base, style, taken)),
            );
        }
    }
    out
}

/// Adds each distinct repeat-bleed row of the tree to `rows`, in the order first met.
fn collect_bleeds(branches: &crate::weapon::Branches, rows: &mut Vec<RepeatBleed>) {
    for step in [false, true]
        .into_iter()
        .filter_map(|smash| branches.get(smash))
    {
        for bleed in step.repeats.iter().filter_map(|r| r.bleed) {
            if !rows.contains(&bleed) {
                rows.push(bleed);
            }
        }
        collect_bleeds(&step.next, rows);
    }
}

/// Whether `pred` holds for some step of the tree under `branches`.
fn any_step(
    branches: &crate::weapon::Branches,
    pred: &dyn Fn(&crate::weapon::Step) -> bool,
) -> bool {
    [false, true]
        .into_iter()
        .filter_map(|smash| branches.get(smash))
        .any(|step| pred(step) || any_step(&step.next, pred))
}

/// The gauge of each style and number of places of its path taken (the same index as
/// `grant_weapons`), numbers clamped to their caps again (ADR-SVC-1). Empty when no style of the
/// weapon names one.
fn grant_gauges(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<Gauge>> {
    let (WeaponRef::Preset { id, .. }, Some(_)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let gauge_of = |s: &crate::expertise::Style, taken: usize| {
        let places = crate::expertise::style_levels(s)
            .into_iter()
            .take(taken)
            .filter_map(|(_, o)| o.map(|o| o.kinds.as_slice()));
        std::iter::once(s.kinds.as_slice())
            .chain(places)
            .flatten()
            .find_map(|k| match k {
                StyleKind::Gauge(g) => Some(clamped(g)),
                _ => None,
            })
    };
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(gauge_of(style, taken));
        }
    }
    if out.iter().all(Option::is_none) {
        out.clear();
    }
    out
}

/// The kinds of a style's own level, then of each of the first `taken` places of its path, in
/// path order (ADR-SIM-9 point 3).
fn path_kinds(s: &crate::expertise::Style, taken: usize) -> impl Iterator<Item = &StyleKind> {
    let places = crate::expertise::style_levels(s)
        .into_iter()
        .take(taken)
        .filter_map(|(_, o)| o.map(|o| o.kinds.as_slice()));
    std::iter::once(s.kinds.as_slice()).chain(places).flatten()
}

/// The spirit of each style and number of places of its path taken (the same index as
/// `grant_weapons`): the `spirit` kinds of the style's own level, then of each place taken, in
/// path order, a later level's countdown and level effects replacing the earlier ones (see
/// `spirit_fold`), clamped (ADR-SVC-1). Empty when no style of the weapon names one.
fn grant_spirits(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<SpiritGrant>> {
    let (WeaponRef::Preset { id, .. }, Some(_)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let spirit_of = |s: &crate::expertise::Style, taken: usize| {
        let mut grant: Option<SpiritGrant> = None;
        for k in path_kinds(s, taken) {
            if let StyleKind::Spirit(sp) = k {
                spirit_fold(grant.get_or_insert_with(SpiritGrant::none), sp);
            }
        }
        grant
    };
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(spirit_of(style, taken));
        }
    }
    if out.iter().all(Option::is_none) {
        out.clear();
    }
    out
}

/// The counter bonus of each style and number of places of its path taken (the same index as
/// `grant_weapons`): the `counter_bonus` kinds of the style's own level, then of each place
/// taken, in path order, a later level's window and effects replacing the earlier ones (see
/// `counter_fold`), clamped (ADR-SVC-1). Empty when no style of the weapon names one.
fn grant_counters(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<CounterGrant>> {
    let (WeaponRef::Preset { id, .. }, Some(_)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let counter_of = |s: &crate::expertise::Style, taken: usize| {
        let mut grant: Option<CounterGrant> = None;
        for k in path_kinds(s, taken) {
            if let StyleKind::CounterBonus(c) = k {
                counter_fold(grant.get_or_insert_with(CounterGrant::none), c);
            }
        }
        grant
    };
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(counter_of(style, taken));
        }
    }
    if out.iter().all(Option::is_none) {
        out.clear();
    }
    out
}

/// The combo tempo of each style and number of places of its path taken (the same index as
/// `grant_weapons`): the `combo_tempo_pct` kinds of the style's own level, then of each place
/// taken, in path order, a later level's cap and effects replacing the earlier ones (see
/// `tempo_fold`), clamped (ADR-SVC-1). Empty when no style of the weapon names one.
fn grant_tempos(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<TempoGrant>> {
    let (WeaponRef::Preset { id, .. }, Some(_)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let tempo_of = |s: &crate::expertise::Style, taken: usize| {
        let mut grant: Option<TempoGrant> = None;
        for k in path_kinds(s, taken) {
            if let StyleKind::ComboTempoPct(t) = k {
                tempo_fold(grant.get_or_insert_with(TempoGrant::none), t);
            }
        }
        grant
    };
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(tempo_of(style, taken));
        }
    }
    if out.iter().all(Option::is_none) {
        out.clear();
    }
    out
}

/// The giant trigger of each style and number of places of its path taken (the same index as
/// `grant_weapons`): the `giant_trigger` kinds of the style's own level, then of each place
/// taken, in path order, a later level's length and effects replacing the earlier ones (see
/// `trigger_fold`), clamped (ADR-SVC-1). Empty when no style of the weapon names one.
fn grant_triggers(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<TriggerGrant>> {
    let (WeaponRef::Preset { id, .. }, Some(_)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let trigger_of = |s: &crate::expertise::Style, taken: usize| {
        let mut grant: Option<TriggerGrant> = None;
        for k in path_kinds(s, taken) {
            if let StyleKind::GiantTrigger(t) = k {
                trigger_fold(grant.get_or_insert_with(TriggerGrant::none), t);
            }
        }
        grant
    };
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(trigger_of(style, taken));
        }
    }
    if out.iter().all(Option::is_none) {
        out.clear();
    }
    out
}

/// The breaker of each style and number of places of its path taken (the same index as
/// `grant_weapons`): the `breaker` kinds of the style's own level, then of each place taken, in
/// path order, each effect folded into its field and a later level's same effect replacing the
/// earlier one (ADR-SIM-9), clamped (ADR-SVC-1). Empty when no style of the weapon names one.
fn grant_breakers(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<BreakerGrant>> {
    let (WeaponRef::Preset { id, .. }, Some(_)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let breaker_of = |s: &crate::expertise::Style, taken: usize| {
        let places = crate::expertise::style_levels(s)
            .into_iter()
            .take(taken)
            .filter_map(|(_, o)| o.map(|o| o.kinds.as_slice()));
        let mut grant: Option<BreakerGrant> = None;
        for kind in std::iter::once(s.kinds.as_slice()).chain(places).flatten() {
            if let StyleKind::Breaker(b) = kind {
                breaker_folded(grant.get_or_insert(BreakerGrant::NONE), b);
            }
        }
        grant
    };
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(breaker_of(style, taken));
        }
    }
    if out.iter().all(Option::is_none) {
        out.clear();
    }
    out
}

/// The orb break of each style and number of places of its path taken (the same index as
/// `grant_weapons`): the `repeat_breaks_orbs` kind of the style's own level or of a place taken
/// (one per path), clamped again (ADR-SVC-1). Empty when no style of the weapon names one.
fn grant_repeat_breaks(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<OrbBreakGrant>> {
    let (WeaponRef::Preset { id, .. }, Some(_)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let grant_of = |s: &crate::expertise::Style, taken: usize| {
        let places = crate::expertise::style_levels(s)
            .into_iter()
            .take(taken)
            .filter_map(|(_, o)| o.map(|o| o.kinds.as_slice()));
        std::iter::once(s.kinds.as_slice())
            .chain(places)
            .flatten()
            .find_map(|kind| match kind {
                StyleKind::RepeatBreaksOrbs {
                    chance_pct,
                    radius_px,
                } => Some(OrbBreakGrant {
                    chance_pct: u8::try_from((*chance_pct).clamp(1, MAX_ORB_BREAK_PCT))
                        .unwrap_or(100),
                    radius: Fx::px(
                        i32::try_from((*radius_px).clamp(1, crate::rewards::MAX_BURST_RADIUS_PX))
                            .unwrap_or(1),
                    ),
                }),
                _ => None,
            })
    };
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(grant_of(style, taken));
        }
    }
    if out.iter().all(Option::is_none) {
        out.clear();
    }
    out
}

impl BreakerGrant {
    /// A breaker that does nothing: every factor 100, no burst.
    const NONE: BreakerGrant = BreakerGrant {
        stagger_pct: 100,
        poise_pct: 100,
        part_break_pct: 100,
        stagger_ticks_pct: 100,
        burst: None,
    };
}

/// `b`'s effects laid over `g`, every number inside its cap.
fn breaker_folded(g: &mut BreakerGrant, b: &crate::expertise::Breaker) {
    use crate::expertise::{BreakerEffect, BurstEffect};
    for e in b.effects.iter().take(5) {
        match e {
            BreakerEffect::StaggerPct { pct } => g.stagger_pct = (*pct).clamp(100, MAX_PCT),
            BreakerEffect::PoisePct { pct } => g.poise_pct = (*pct).clamp(100, MAX_PCT),
            BreakerEffect::PartBreakPct { pct } => g.part_break_pct = (*pct).clamp(100, MAX_PCT),
            BreakerEffect::StaggerTicksPct { pct } => {
                g.stagger_ticks_pct = (*pct).clamp(100, MAX_PCT);
            }
            BreakerEffect::OnStaggerBurst { radius_px, effects } => {
                let radius = (*radius_px).clamp(1, crate::rewards::MAX_BURST_RADIUS_PX);
                let mut burst = BurstGrant {
                    radius: i32::try_from(i64::from(radius) * i64::from(Fx::px(1).0))
                        .unwrap_or(i32::MAX),
                    damage_pct: 0,
                    stun_ticks: 0,
                    knock_down: false,
                };
                for effect in effects.iter().take(3) {
                    match effect {
                        BurstEffect::DamagePct { pct } => {
                            burst.damage_pct = (*pct).clamp(1, MAX_PCT)
                        }
                        BurstEffect::StunTicks { ticks } => {
                            burst.stun_ticks =
                                (*ticks).clamp(1, crate::validate::limits::MAX_STUN_TICKS);
                        }
                        BurstEffect::KnockDown {} => burst.knock_down = true,
                    }
                }
                g.burst = Some(burst);
            }
        }
    }
}

impl TriggerGrant {
    /// A giant trigger that does nothing yet: the longest length, no effect.
    pub(super) fn none() -> TriggerGrant {
        TriggerGrant {
            ticks: MAX_TRIGGER_TICKS,
            range_pct: 0,
            smash_pct: 0,
            rally_pct: 100,
            super_armour: false,
            hold_move_pct: 0,
            end_ring: false,
        }
    }
}

/// `t` laid over `g`, every number inside its cap: its length replaces the earlier one and each
/// effect it lists replaces the same effect; an effect it does not list stays, and the armour is
/// true once any level names it.
pub(super) fn trigger_fold(g: &mut TriggerGrant, t: &crate::expertise::GiantTrigger) {
    g.ticks = u16::try_from(t.ticks)
        .unwrap_or(MAX_TRIGGER_TICKS)
        .clamp(1, MAX_TRIGGER_TICKS);
    for e in t.effects.iter().take(crate::expertise::MAX_TRIGGER_EFFECTS) {
        match e {
            TriggerEffect::HoldMovePct { pct } => {
                g.hold_move_pct = (*pct).min(crate::expertise::MAX_HOLD_MOVE_PCT);
            }
            TriggerEffect::RangePct { pct } => {
                g.range_pct = (*pct).min(crate::rewards::MAX_RANGE_PCT);
            }
            TriggerEffect::SmashPct { pct } => g.smash_pct = (*pct).min(MAX_PCT),
            TriggerEffect::SuperArmour {} => g.super_armour = true,
            // The ring itself is derived at world build (the compiled smash's first active
            // phase, `compile_rings`); the grant keeps only that it is named.
            TriggerEffect::EndRing {} => g.end_ring = true,
            TriggerEffect::RallyPct { pct } => g.rally_pct = (*pct).clamp(100, MAX_PCT),
        }
    }
}

impl TempoGrant {
    /// A combo tempo that does nothing yet: the most stacks, no speed, no flag.
    pub(super) fn none() -> TempoGrant {
        TempoGrant {
            max_stacks: MAX_TEMPO_STACKS,
            speed_pct: 0,
            keep_on_smash: false,
            hold_armour: false,
            range_pct_at_full: 0,
            carry_at_full: false,
        }
    }
}

/// `t` laid over `g`, every number inside its cap: its `max_stacks` replaces the earlier one,
/// `speed_pct` replaces the earlier speed and the flags are true once any level names them.
pub(super) fn tempo_fold(g: &mut TempoGrant, t: &crate::expertise::ComboTempo) {
    g.max_stacks = u8::try_from(t.max_stacks)
        .unwrap_or(MAX_TEMPO_STACKS)
        .clamp(1, MAX_TEMPO_STACKS);
    for e in t.effects.iter().take(crate::expertise::MAX_TEMPO_EFFECTS) {
        match e {
            TempoEffect::SpeedPct { pct } => g.speed_pct = (*pct).min(MAX_ATTACK_SPEED_PCT),
            TempoEffect::KeepOnSmash {} => g.keep_on_smash = true,
            TempoEffect::SuperArmourAtFull {} => g.hold_armour = true,
            TempoEffect::RangePctAtFull { pct } => {
                g.range_pct_at_full = (*pct).min(crate::rewards::MAX_RANGE_PCT)
            }
            // Read by `player::tempo_carries`.
            TempoEffect::CarryAtFull {} => g.carry_at_full = true,
        }
    }
}

impl CounterGrant {
    /// A counter bonus that does nothing yet: the longest window, no effect.
    pub(super) fn none() -> CounterGrant {
        CounterGrant {
            window: MAX_COUNTER_WINDOW_TICKS,
            damage_pct: 0,
            stagger_pct: 100,
            stun_ticks: 0,
            advance: Fx(0),
            refund_evade: false,
            gauge_fill_pct: 0,
        }
    }
}

/// `c` laid over `g`, every number inside its cap: its window replaces the earlier one and each
/// effect it lists replaces the same effect; an effect it does not list stays.
pub(super) fn counter_fold(g: &mut CounterGrant, c: &crate::expertise::CounterBonus) {
    g.window = u16::try_from(c.window_ticks)
        .unwrap_or(MAX_COUNTER_WINDOW_TICKS)
        .clamp(1, MAX_COUNTER_WINDOW_TICKS);
    for e in c.effects.iter().take(6) {
        match e {
            CounterEffect::DamagePct { pct } => g.damage_pct = (*pct).min(MAX_PCT),
            CounterEffect::StaggerPct { pct } => g.stagger_pct = (*pct).clamp(100, MAX_PCT),
            CounterEffect::StunTicks { ticks } => {
                g.stun_ticks = (*ticks).min(crate::validate::limits::MAX_STUN_TICKS);
            }
            CounterEffect::Advance { distance } => {
                g.advance = Fx(distance.0.clamp(0, Fx::px(MAX_ADVANCE_DISTANCE).0));
            }
            CounterEffect::RefundEvade {} => g.refund_evade = true,
            CounterEffect::GaugeFillPct { pct } => g.gauge_fill_pct = (*pct).min(100),
        }
    }
}

impl SpiritGrant {
    /// A spirit that does nothing yet: the longest countdown and no level.
    pub(super) fn none() -> SpiritGrant {
        SpiritGrant {
            decay_ticks: MAX_SPIRIT_DECAY_TICKS,
            levels: Vec::new(),
        }
    }
}

/// `sp` laid over `g`, every number inside its cap: its countdown replaces the earlier one and
/// each effect of entry `i` replaces the same effect of level `i + 1`, an entry past the table
/// extending it up to `MAX_SPIRIT_LEVEL`; an empty entry changes nothing.
pub(super) fn spirit_fold(g: &mut SpiritGrant, sp: &crate::expertise::Spirit) {
    g.decay_ticks = u16::try_from(sp.decay_ticks)
        .unwrap_or(MAX_SPIRIT_DECAY_TICKS)
        .clamp(1, MAX_SPIRIT_DECAY_TICKS);
    for (i, effects) in sp
        .levels
        .iter()
        .take(usize::from(MAX_SPIRIT_LEVEL))
        .enumerate()
    {
        if g.levels.len() <= i {
            g.levels.push(SpiritFx::default());
        }
        let fx = &mut g.levels[i];
        for e in effects {
            match e {
                SpiritEffect::DamagePct { pct } => fx.damage_pct = (*pct).min(MAX_PCT),
                SpiritEffect::RangePct { pct } => {
                    fx.range_pct = (*pct).min(crate::rewards::MAX_RANGE_PCT);
                }
                SpiritEffect::SmashBleedStacks { stacks } => {
                    fx.bleed_stacks = (*stacks).min(MAX_SPIRIT_BLEED_STACKS);
                }
            }
        }
    }
}

/// `g` with every number inside its cap.
fn clamped(g: &Gauge) -> Gauge {
    let max = g.max.clamp(1, MAX_WEAPON_GAUGE);
    Gauge {
        max,
        per_step: g.per_step.min(max),
        per_hit: g.per_hit.min(max),
        from_damage_pct: g.from_damage_pct.min(MAX_GAUGE_DAMAGE_PCT),
        step_cap: g.step_cap.clamp(1, max),
        spend: g.spend.clone(),
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
