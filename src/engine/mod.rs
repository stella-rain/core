//! The simulation engine: runs a validated stage tick by tick to a clear or a fail
//! (ADR-004, ADR-015). The only external influence is one `Input` per tick and the commands
//! between ticks (ADR-079: `choose` and `spend`); everything else comes from the stage and its
//! seed, or from a run setup, and from the run stats (ADR-067), so the state hash after each
//! tick is the same on every platform (ADR-019).
//!
//! Rules are dispatched by the stage's `sim_version` (ADR-017). Only version 0 exists, and it
//! changes in place until the freeze (ADR-035): see `rules_v0` for what it does and does not
//! do yet.

use crate::event::DomainEvent;
use crate::id::ContentId;
use crate::input::Input;
use crate::kinds::KindTable;
use crate::rewards::Rewards;
use crate::rules_v0;
use crate::rules_v0::{HordeSpec, World};
use crate::run_content::RunContent;
use crate::run_setup::{self, RunSetup};
use crate::run_stats::RunStats;
use crate::stage::Stage;
use crate::validate::{self, Error, Issue};

/// Why `Engine::choose` refused a choice; the run is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceError {
    /// No offer is open.
    NoOffer,
    /// `index` is not below the number of the open offer's `options`.
    OutOfRange { index: u32, options: u32 },
}

/// Why `Engine::spend` refused to spend a stat point; the run is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpendError {
    /// The run has ended: its unspent points are lost.
    RunOver,
    /// An offer is open: it is answered first (ADR-079 §4).
    OfferOpen,
    /// No point is left (or the stage has no levels).
    NoPoint,
    /// `index` is not below the number of run `stats`.
    OutOfRange { index: u32, stats: u32 },
    /// The stat at `index` already has its `limit` of points.
    AtLimit { index: u32, limit: u8 },
}

/// How a run stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// Bullets alive after the attacks fired, before the tick moved and removed any.
    pub bullets: u32,
    /// Zones alive after the attacks placed theirs, before the tick advanced any.
    pub zones: u32,
    /// Bullets made in the tick.
    pub bullet_spawns: u32,
    /// Emitter nodes carried out in the tick, across all attacks.
    pub emitter_steps: u32,
}

impl Load {
    /// The caps of the rules (`rules_v0`): a tick that reaches all of them is a worst-case tick.
    pub const CAPS: Load = Load {
        enemies: rules_v0::MAX_ENEMIES_ALIVE,
        tasks: rules_v0::MAX_ATTACK_TASKS,
        bullets: rules_v0::MAX_BULLETS_ALIVE,
        zones: rules_v0::MAX_ZONES_ALIVE,
        bullet_spawns: rules_v0::MAX_BULLET_SPAWNS_PER_TICK,
        emitter_steps: rules_v0::MAX_EMITTER_STEPS_PER_TICK,
    };
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
#[derive(Clone, Debug)]
pub struct Engine {
    stage: World,
    /// The content files the run reads (ADR-067), fixed for the run.
    content: RunContent,
    state: State,
    /// Events of the last tick; cleared when the next one starts.
    events: Vec<DomainEvent>,
    /// What the last tick used of the dynamic budgets; zero when the tick did not run.
    load: Load,
}

impl Engine {
    /// Starts a run. The stage is validated first (ADR-020), so the rules below only ever see
    /// stages inside the budgets.
    /// A stage with levels spends its points on the bundled run stats.
    pub fn new(stage: &Stage) -> Result<Engine, Error> {
        Engine::new_with(stage, RunContent::bundled())
    }

    /// As `new`, with `content` in place of the bundled files (ADR-067).
    pub fn new_with(stage: &Stage, content: &RunContent) -> Result<Engine, Error> {
        validate::validate(stage)?;
        let pool = rules_v0::RunPool {
            rewards: pool(None, content.rewards())?,
            extra_rerolls: 0,
            difficulty: rules_v0::RunDifficulty::NONE,
        };
        let kinds = KindTable::of_stage(stage).map_err(|i| Error::Invalid(vec![i]))?;
        let stage = World::new(stage.clone(), None, &kinds, content)
            .map_err(|i| Error::Invalid(vec![i]))?;
        Ok(Engine {
            state: rules_v0::initial_state(&stage, content, pool, u64::from(stage.seed), kinds),
            stage,
            content: content.clone(),
            events: Vec::new(),
            load: Load::default(),
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
        if setup.mode.0 == "time_limit" {
            return Engine::start_time_limit(setup, content, content.time_limit());
        }
        let stage = run_setup::stage_in(setup, content)?;
        let pool = rules_v0::RunPool {
            rewards: pool(setup.star_pool.as_deref(), content.rewards())?,
            extra_rerolls: setup.extra_rerolls,
            difficulty: run_difficulty(setup, content)?,
        };
        let kinds = KindTable::of_stage(&stage).map_err(|i| Error::Invalid(vec![i]))?;
        let stage = World::new(stage.clone(), None, &kinds, content)
            .map_err(|i| Error::Invalid(vec![i]))?;
        let mut state = rules_v0::initial_state(&stage, content, pool, setup.seed, kinds);
        let loadout = rules_v0::Loadout::of(setup, content);
        rules_v0::apply_loadout(&stage, content.rewards(), &mut state, &loadout);
        Ok(Engine {
            state,
            stage,
            content: content.clone(),
            events: Vec::new(),
            load: Load::default(),
        })
    }

    /// Starts a run of the `time_limit` mode from `mode` in place of the bundled file: its
    /// waves, and its bosses as they come (`rules_v0::spawn_boss`).
    pub fn start_time_limit(
        setup: &RunSetup,
        content: &RunContent,
        mode: &crate::time_limit::TimeLimit,
    ) -> Result<Engine, Error> {
        run_setup::validate_in(setup, content)?;
        let difficulty = run_setup::difficulty_of(setup, content.difficulties())?;
        let (stage, spec, horde) =
            run_setup::time_limit_run(setup, mode, difficulty, content.trial())?;
        let pool = rules_v0::RunPool {
            rewards: pool(setup.star_pool.as_deref(), content.rewards())?,
            extra_rerolls: setup.extra_rerolls,
            difficulty: run_difficulty(setup, content)?,
        };
        let kinds = horde_kinds(&stage, &spec).map_err(|i| Error::Invalid(vec![i]))?;
        let stage =
            World::new(stage, Some(spec), &kinds, content).map_err(|i| Error::Invalid(vec![i]))?;
        let mut state = rules_v0::initial_state(&stage, content, pool, setup.seed, kinds);
        state.horde = Some(horde);
        let loadout = rules_v0::Loadout::of(setup, content);
        rules_v0::apply_loadout(&stage, content.rewards(), &mut state, &loadout);
        Ok(Engine {
            state,
            stage,
            content: content.clone(),
            events: Vec::new(),
            load: Load::default(),
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
            0 => {
                rules_v0::spawn_boss(&self.stage, &mut self.state);
                rules_v0::step(
                    &self.stage,
                    self.content.rewards(),
                    self.content.expertise(),
                    &mut self.state,
                    input,
                    &mut self.events,
                    &mut self.load,
                );
            }
            // `new` validated the version; fail closed if that ever stops being true.
            _ => self.state.outcome = Outcome::Failed,
        }
    }

    /// Chooses option `index` of the open offer, between ticks (ADR-079 §3): the snapshot's
    /// `offer.options` lists them. It applies at once, and the events are then this choice's.
    /// Refused, changing nothing, with no offer open or an index outside its options.
    pub fn choose(&mut self, index: u32) -> Result<(), ChoiceError> {
        if !self.offer_open() {
            return Err(ChoiceError::NoOffer);
        }
        let (rewards, expertise) = (self.content.rewards(), self.content.expertise());
        let options = rules_v0::offer_options(&self.stage, rewards, expertise, &self.state);
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
        match self.stage.sim_version {
            0 => rules_v0::choose(
                &self.stage,
                rewards,
                expertise,
                &mut self.state,
                option,
                &mut self.events,
            ),
            _ => self.state.outcome = Outcome::Failed,
        }
        Ok(())
    }

    /// Spends a stat point on the run stat at `index`, between ticks (ADR-079 §4): the stat
    /// rises by its step at once, and the events are then this spend's. Refused, changing
    /// nothing, once the run has ended, while an offer is open, with no point left, an index
    /// outside the run stats, or the stat at its limit.
    pub fn spend(&mut self, index: u32) -> Result<(), SpendError> {
        if self.state.outcome != Outcome::Running {
            return Err(SpendError::RunOver);
        }
        if self.offer_open() {
            return Err(SpendError::OfferOpen);
        }
        let p = self
            .state
            .progress
            .as_ref()
            .filter(|p| p.points > 0)
            .ok_or(SpendError::NoPoint)?;
        let stats = self.content.run_stats().stats();
        let Some(at) = usize::try_from(index).ok().filter(|i| *i < stats.len()) else {
            return Err(SpendError::OutOfRange {
                index,
                stats: u32::try_from(stats.len()).unwrap_or(u32::MAX),
            });
        };
        let stat = &stats[at];
        if p.spent[at] >= stat.limit {
            return Err(SpendError::AtLimit {
                index,
                limit: stat.limit,
            });
        }
        self.events.clear();
        match self.stage.sim_version {
            0 => rules_v0::spend(&mut self.state, at, stat, &mut self.events),
            _ => self.state.outcome = Outcome::Failed,
        }
        Ok(())
    }

    /// The run stats this run spends its points on.
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
        rules_v0::offer_open(&self.state)
    }

    /// FNV-1a 64 over the whole state after the last tick (ADR-019).
    pub fn state_hash(&self) -> u64 {
        rules_v0::state_hash(&self.stage, &self.state)
    }

    /// The domain events of the last tick, in the order they happened.
    pub fn events(&self) -> &[DomainEvent] {
        &self.events
    }

    /// How much of each dynamic budget the last tick used (ADR-020).
    pub fn load(&self) -> Load {
        self.load
    }
}

/// What the setup's difficulty brings to the run (ADR-110): its percents, and its rarity range
/// clamped to the rarities the run's rewards have.
fn run_difficulty(
    setup: &RunSetup,
    content: &RunContent,
) -> Result<rules_v0::RunDifficulty, Error> {
    let d = run_setup::difficulty_of(setup, content.difficulties())?;
    Ok(rules_v0::RunDifficulty {
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

#[cfg(test)]
mod horde_tests;
#[cfg(test)]
mod move_index_tests;

// Every `State` type lives in `rules_v0::state`; these paths keep working.
pub(crate) use crate::rules_v0::state::*;
