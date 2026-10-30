//! The simulation engine: runs a validated stage tick by tick to a clear or a fail
//! (ADR-004, ADR-015). The only external influence is one `Input` per tick and the commands
//! between ticks (ADR-079: `choose`); everything else comes from the stage and its
//! seed, or from a run setup, and from the run stats (ADR-067), so the state hash after each
//! tick is the same on every platform (ADR-019).
//!
//! Rules are dispatched by the stage's `sim_version` (ADR-017, ADR-REPL-7 point 3): versions
//! `1..=SIM_VERSION` run in `sim`, any other fails closed. Version 1 is frozen, so a change of
//! outcome after it is a gate on the new version in `sim`, a copy only for a rewrite (see `sim`
//! for what it does and does not do yet).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::event::DomainEvent;
use crate::id::ContentId;
use crate::input::Input;
use crate::kinds::KindTable;
use crate::ledger::{Counted, RunLedger};
use crate::rewards::Rewards;
use crate::run_content::RunContent;
use crate::run_setup::{self, RunSetup};
use crate::run_stats::RunStats;
use crate::sim;
use crate::sim::{HordeSpec, World};
use crate::stage::Stage;
use crate::validate::{self, Error, Issue};
use crate::version::SIM_VERSION;

/// Why `Engine::choose` refused a choice; the run is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceError {
    /// No offer is open.
    NoOffer,
    /// `index` is not below the number of the open offer's `options`.
    OutOfRange { index: u32, options: u32 },
    /// The run's `sim_version` is not one this game runs (ADR-REPL-7 point 3). `Engine::new`
    /// refuses such a stage, so this is a fail-closed guard, not a path a valid run takes.
    Unsupported { sim_version: u32 },
}

/// How a run stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Running,
    /// The stage was cleared: this is what a replay has to prove (ADR-009).
    Cleared,
    /// The main character died, or the stage ran out of ticks.
    Failed,
}

/// How much of each dynamic budget (ADR-020) a tick used: the most it reached during the tick.
/// A read-out for tests and benchmarks, not part of the state: nothing here is hashed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Load {
    /// Enemies alive when the attacks ran, after spawns and summons.
    pub enemies: u32,
    /// Attacks being carried out, counted before any of them finished.
    pub tasks: u32,
    /// Zones alive after the attacks placed theirs, before the tick advanced any.
    pub zones: u32,
    /// Orbs alive after the attacks fired theirs, before the tick moved and removed any.
    pub orbs: u32,
    /// Emitter nodes carried out in the tick, across all attacks.
    pub emitter_steps: u32,
    /// Spawns the cap on enemies alive or on elites refused in the tick: waves, elites, summons
    /// and guardians (EC2, DS80).
    pub refused_spawns: u32,
    /// Wave spawns the spawn director held in the tick because the crowd was at its target
    /// (DIR1, DS103): horde runs only. They are not refused or lost: the wave's schedule slides.
    pub held_spawns: u32,
    /// Enemies the leash moved just outside the view, on the side opposite the view's centre, or on the enemy's own side when that one is off the map, in the tick (EC2, DS80).
    pub relocated: u32,
}

/// The kinds of a `time_limit` run: its stage's, and those of the bosses it brings later.
fn horde_kinds(stage: &Stage, horde: &HordeSpec) -> Result<KindTable, crate::validate::Issue> {
    let bosses: Vec<&crate::stage::Boss> = horde
        .bosses
        .iter()
        .chain(std::iter::once(&horde.template))
        .collect();
    let sprites: Vec<ContentId> = horde
        .pool
        .iter()
        .chain(std::iter::once(&horde.final_boss))
        .cloned()
        .collect();
    KindTable::of_stage_and(stage, &bosses, &sprites, crate::kinds::MAX_KINDS)
}

/// A run of one stage.
#[derive(Debug)]
pub struct Engine {
    stage: World,
    /// The content files the run reads (ADR-067), fixed for the run.
    content: RunContent,
    state: State,
    /// Events of the last tick; cleared when the next one starts.
    events: Vec<DomainEvent>,
    /// What the last tick used of the dynamic budgets; zero when the tick did not run.
    load: Load,
    /// The ledger of counts and bestiary encounters (ADR-BRDG-6).
    ledger: RunLedger,
    /// The HUD tallies, counted from the events (outside the hashed state).
    counted: Counted,
}

impl Clone for Engine {
    /// A clone keeps the room the run reserved at load: a derived clone of a `Vec` has only its
    /// length, and the clone's first ticks would grow them (DIR1).
    fn clone(&self) -> Engine {
        let mut state = self.state.clone();
        sim::reserve(&self.stage, &mut state);
        Engine {
            stage: self.stage.clone(),
            content: self.content.clone(),
            state,
            events: {
                let mut events = self.events.clone();
                events.reserve(sim::events_room(&self.stage).saturating_sub(events.len()));
                events
            },
            load: self.load,
            ledger: self.ledger.clone(),
            counted: self.counted.clone(),
        }
    }
}

impl Engine {
    /// What a savepoint writes and a resume compares with: the world and content (fixed for the
    /// run), the state, the ledger and the tallies.
    pub(crate) fn savepoint_parts(&self) -> (&World, &RunContent, &State, &RunLedger, &Counted) {
        (
            &self.stage,
            &self.content,
            &self.state,
            &self.ledger,
            &self.counted,
        )
    }

    /// This engine with the state, ledger and tallies a savepoint held: a fresh engine of the
    /// same setup and content (the world and content are not saved), the saved parts swapped in.
    /// `decode_state` has reserved the state's room; no tick rule changes.
    pub(crate) fn with_saved(self, state: State, ledger: RunLedger, counted: Counted) -> Engine {
        let mut events = self.events;
        events.clear();
        Engine {
            stage: self.stage,
            content: self.content,
            state,
            events,
            load: Load::default(),
            ledger,
            counted,
        }
    }

    /// Starts a run. The stage is validated first (ADR-020), so the rules below only ever see
    /// stages inside the budgets.
    /// A stage with levels raises the bundled run stats, in their order.
    pub fn new(stage: &Stage) -> Result<Engine, Error> {
        Engine::new_with(stage, RunContent::bundled())
    }

    /// As `new`, with `content` in place of the bundled files (ADR-067).
    pub fn new_with(stage: &Stage, content: &RunContent) -> Result<Engine, Error> {
        Engine::new_with_stats(stage, content, &[], false)
    }

    /// As `new_with`, with the level-ups spent as a setup's `stat_priority` and `pick_stats` say
    /// (DS18): for a stage that has no setup. The IDs are those of `content`'s run stats.
    pub fn new_with_stats(
        stage: &Stage,
        content: &RunContent,
        stat_priority: &[ContentId],
        pick_stats: bool,
    ) -> Result<Engine, Error> {
        validate::validate(stage)?;
        let pool = sim::RunPool {
            rewards: pool(None, content.rewards())?,
            extra_rerolls: 0,
            difficulty: sim::RunDifficulty::NONE,
        };
        let kinds = KindTable::of_stage(stage).map_err(|i| Error::Invalid(vec![i]))?;
        let stage = World::new(stage.clone(), None, &kinds, content)
            .map_err(|i| Error::Invalid(vec![i]))?;
        let mut state = sim::initial_state(&stage, content, pool, u64::from(stage.seed), kinds);
        sim::set_policy(&mut state, content.run_stats(), stat_priority, pick_stats);
        let weapon = Self::weapon_str(&stage);
        let ledger = RunLedger::for_engine(&state.kinds, weapon);
        Ok(Engine {
            events: Vec::with_capacity(sim::events_room(&stage)),
            state,
            stage,
            content: content.clone(),
            load: Load::default(),
            ledger,
            counted: Counted::new(content.rewards()),
        })
    }

    /// Starts a run from a setup alone (ADR-079 §1): the setup is validated, then its mode's
    /// stage runs with its weapons and seed, and the bundled content files.
    pub fn start(setup: &RunSetup) -> Result<Engine, Error> {
        Engine::start_with(setup, RunContent::bundled())
    }

    /// As `start`, with `content` in place of the bundled files (ADR-067).
    pub fn start_with(setup: &RunSetup, content: &RunContent) -> Result<Engine, Error> {
        run_setup::validate_in(setup, content)?;
        if setup.mode.as_str() == "time_limit" {
            return Engine::start_time_limit(setup, content, content.time_limit());
        }
        let stage = run_setup::stage_in(setup, content)?;
        let pool = sim::RunPool {
            rewards: pool(setup.star_pool.as_deref(), content.rewards())?,
            extra_rerolls: setup.extra_rerolls,
            difficulty: run_difficulty(setup, content)?,
        };
        let kinds = KindTable::of_stage(&stage).map_err(|i| Error::Invalid(vec![i]))?;
        let stage = World::new(stage.clone(), None, &kinds, content)
            .map_err(|i| Error::Invalid(vec![i]))?;
        let mut state = sim::initial_state(&stage, content, pool, setup.seed, kinds);
        sim::set_policy(
            &mut state,
            content.run_stats(),
            &setup.stat_priority,
            setup.pick_stats,
        );
        let loadout = sim::Loadout::of(setup, content);
        sim::apply_loadout(&stage, content.rewards(), &mut state, &loadout);
        let weapon = Self::weapon_str(&stage);
        let ledger = RunLedger::for_engine(&state.kinds, weapon);
        Ok(Engine {
            events: Vec::with_capacity(sim::events_room(&stage)),
            state,
            stage,
            content: content.clone(),
            load: Load::default(),
            ledger,
            counted: Counted::new(content.rewards()),
        })
    }

    /// Starts a run of the `time_limit` mode from `mode` in place of the bundled file: its
    /// waves, and its bosses as they come (`TICK`'s first step).
    pub fn start_time_limit(
        setup: &RunSetup,
        content: &RunContent,
        mode: &crate::time_limit::TimeLimit,
    ) -> Result<Engine, Error> {
        run_setup::validate_in(setup, content)?;
        let difficulty = run_setup::difficulty_of(setup, content.difficulties())?;
        let (stage, spec, horde) =
            run_setup::time_limit_run(setup, mode, difficulty, content.trial())?;
        let pool = sim::RunPool {
            rewards: pool(setup.star_pool.as_deref(), content.rewards())?,
            extra_rerolls: setup.extra_rerolls,
            difficulty: run_difficulty(setup, content)?,
        };
        let kinds = horde_kinds(&stage, &spec).map_err(|i| Error::Invalid(vec![i]))?;
        let stage =
            World::new(stage, Some(spec), &kinds, content).map_err(|i| Error::Invalid(vec![i]))?;
        let mut state = sim::initial_state(&stage, content, pool, setup.seed, kinds);
        state.horde = Some(horde);
        sim::set_policy(
            &mut state,
            content.run_stats(),
            &setup.stat_priority,
            setup.pick_stats,
        );
        let loadout = sim::Loadout::of(setup, content);
        sim::apply_loadout(&stage, content.rewards(), &mut state, &loadout);
        let weapon = Self::weapon_str(&stage);
        let ledger = RunLedger::for_engine(&state.kinds, weapon);
        Ok(Engine {
            events: Vec::with_capacity(sim::events_room(&stage)),
            state,
            stage,
            content: content.clone(),
            load: Load::default(),
            ledger,
            counted: Counted::new(content.rewards()),
        })
    }

    /// Advances one tick with `input`. Once the run is over this does nothing, so a replay that
    /// is longer than the run cannot change the final hash; nor while an offer is open, when
    /// the world is stopped (ADR-079 §3).
    pub fn step(&mut self, input: Input) {
        self.events.clear();
        self.load = Load::default();
        if self.offer_open() {
            return;
        }
        match self.stage.sim_version {
            v @ 1..=SIM_VERSION => sim::step(
                v,
                &self.stage,
                self.content.rewards(),
                self.content.expertise(),
                &mut self.state,
                input,
                &mut self.events,
                &mut self.load,
            ),
            // `new` validated the version; fail closed if that ever stops being true.
            _ => self.state.outcome = Outcome::Failed,
        }
        self.observe_tick();
    }

    /// Chooses option `index` of the open offer, between ticks (ADR-079 §3): the snapshot's
    /// `offer.options` lists them. It applies at once, and the events are then this choice's.
    /// Refused, changing nothing, with no offer open, a `sim_version` this game does not run, or
    /// an index outside its options.
    pub fn choose(&mut self, index: u32) -> Result<(), ChoiceError> {
        if !self.offer_open() {
            return Err(ChoiceError::NoOffer);
        }
        let sim_version = self.stage.sim_version;
        if !(1..=SIM_VERSION).contains(&sim_version) {
            // `new` validated the version; refuse, changing nothing and running no rules, if
            // that stops being true.
            return Err(ChoiceError::Unsupported { sim_version });
        }
        let (rewards, expertise) = (self.content.rewards(), self.content.expertise());
        let options = sim::offer_options(&self.stage, rewards, expertise, &self.state);
        let Some(option) = usize::try_from(index)
            .ok()
            .and_then(|i| options.get(i))
            .copied()
        else {
            return Err(ChoiceError::OutOfRange {
                index,
                options: u32::try_from(options.len()).unwrap_or(u32::MAX),
            });
        };
        self.events.clear();
        sim::choose(
            sim_version,
            &self.stage,
            rewards,
            expertise,
            &mut self.state,
            option,
            &mut self.events,
        );
        self.observe_choice();
        Ok(())
    }

    fn weapon_str(stage: &World) -> &str {
        match &stage.stage.player.weapon {
            crate::weapon::WeaponRef::Preset { id, .. } => id.as_str(),
            crate::weapon::WeaponRef::Inline(_) => "inline",
        }
    }

    fn observe_tick(&mut self) {
        let weapon = Self::weapon_str(&self.stage);
        self.ledger.observe_tick(
            &self.state.kinds,
            weapon,
            &self.state.enemies,
            self.state.boss.as_ref(),
            &self.events,
        );
        self.counted.observe(self.content.rewards(), &self.events);
    }

    fn observe_choice(&mut self) {
        let weapon = Self::weapon_str(&self.stage);
        self.ledger
            .observe_choice(&self.state.kinds, weapon, &self.events);
        self.counted.observe(self.content.rewards(), &self.events);
    }

    /// The run stats this run raises with its level-ups.
    pub fn run_stats(&self) -> &RunStats {
        self.content.run_stats()
    }

    /// The content files this run reads.
    pub fn content(&self) -> &RunContent {
        &self.content
    }

    /// The run's kind table: what every `KindIndex` in its events and snapshots names. Fixed for
    /// the run, so read it once; key anything kept past the run on the ID, not the index.
    pub fn kinds(&self) -> &KindTable {
        &self.state.kinds
    }

    pub fn tick(&self) -> u32 {
        self.state.tick
    }

    /// The Stardust the run has earned, times 10 (ADR-109); 0 in a stage without a factor.
    pub fn stardust(&self) -> u32 {
        self.state.stardust.unwrap_or(0)
    }

    pub fn outcome(&self) -> Outcome {
        self.state.outcome
    }

    /// An offer is open: `step` does nothing until `choose` closes it (ADR-079 §3).
    pub fn offer_open(&self) -> bool {
        sim::offer_open(&self.state)
    }

    /// FNV-1a 64 over the whole state after the last tick (ADR-019).
    pub fn state_hash(&self) -> u64 {
        sim::state_hash(&self.state)
    }

    /// The domain events of the last tick, in the order they happened.
    pub fn events(&self) -> &[DomainEvent] {
        &self.events
    }

    /// The ledger of counts and bestiary encounters (ADR-BRDG-6).
    pub fn ledger(&self) -> &RunLedger {
        &self.ledger
    }

    /// The caps of the run's rules: a tick that reaches all of them is a worst-case tick. The
    /// cap on enemies is the run's own, derived from its crowd target and the pack's headroom
    /// (DIR1); the others are the rules' constants.
    pub fn caps(&self) -> Load {
        Load {
            enemies: self.stage.enemy_cap,
            tasks: sim::MAX_ATTACK_TASKS,
            zones: sim::MAX_ZONES_ALIVE,
            orbs: sim::MAX_ORBS_ALIVE,
            emitter_steps: sim::MAX_EMITTER_STEPS_PER_TICK,
            // Not caps: what a tick did, not what it may hold.
            refused_spawns: 0,
            held_spawns: 0,
            relocated: 0,
        }
    }

    /// How much of each dynamic budget the last tick used (ADR-020).
    pub fn load(&self) -> Load {
        self.load
    }
}

/// What the setup's difficulty brings to the run (ADR-110): its percents, and its rarity range
/// clamped to the rarities the run's rewards have.
fn run_difficulty(setup: &RunSetup, content: &RunContent) -> Result<sim::RunDifficulty, Error> {
    let d = run_setup::difficulty_of(setup, content.difficulties())?;
    Ok(sim::RunDifficulty {
        hp_pct: d.hp_pct,
        damage_pct: d.damage_pct,
        rarity: d.rarity_range(content.rewards()),
    })
}

/// The run's star pool: the places in `rewards` of the rewards `ids` names, or of every reward
/// when it names none. A name the file does not hold fails closed, one issue per name.
fn pool(ids: Option<&[ContentId]>, rewards: &Rewards) -> Result<Vec<u16>, Error> {
    let place = |i: usize| u16::try_from(i).unwrap_or(u16::MAX);
    let Some(ids) = ids else {
        return Ok((0..rewards.rewards().len()).map(place).collect());
    };
    let mut issues = Vec::new();
    let mut out = Vec::with_capacity(ids.len());
    for (i, id) in ids.iter().enumerate() {
        match rewards.index_of(id) {
            Some(at) => out.push(place(at)),
            None => issues.push(Issue {
                path: format!("$.star_pool[{i}]"),
                message: "no such reward in the run's rewards file".into(),
            }),
        }
    }
    if issues.is_empty() {
        Ok(out)
    } else {
        Err(Error::Invalid(issues))
    }
}

mod snapshot;
pub use snapshot::{Place, Places, Tally};

#[cfg(test)]
mod codec_tests;
#[cfg(test)]
mod dispatch_tests;
#[cfg(test)]
mod hash_coverage_tests;
#[cfg(test)]
mod horde_tests;
#[cfg(test)]
mod move_index_tests;
#[cfg(test)]
mod savepoint_tests;

// Every `State` type lives in `sim::state`; these paths keep working.
pub(crate) use crate::sim::state::*;
