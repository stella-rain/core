//! The engine's run, one variant per released rules version (ADR-REPL-9 point 1, ADR-REPL-7
//! point 4): each version owns its `World`, its `State` and its snapshot view, `V1` in `sim_v1`
//! (frozen) and `V2` in `sim` (live). Every call into the rules goes through a `match` here, so
//! adding a version lists each site as a compile error. No box, no trait object: the enum is the
//! larger variant's `World` and `State` and a tag.

use crate::event::DomainEvent;
use crate::expertise::Expertise;
use crate::id::ContentId;
use crate::input::Input;
use crate::kinds::KindTable;
use crate::ledger::RunLedger;
use crate::rewards::{Rarity, Rewards};
use crate::run_content::RunContent;
use crate::run_setup::RunSetup;
use crate::snapshot::{OfferOption, Snapshot};
use crate::stage::Stage;
use crate::validate::Issue;
use crate::{sim, sim_v1};

use super::{Load, Outcome, Places};

/// A run under the rules of one `sim_version`: the compiled stage and the state it changes, both
/// of that version's types. The engine reaches them, and the snapshot view, only through here.
// The frozen state is the larger: the live one keeps its players in a list (ADR-SIM-12). One run
// is held per engine, so the size difference costs nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub(crate) enum Run {
    /// Version 1, the frozen first rules (`sim_v1`).
    V1 {
        world: sim_v1::World,
        state: sim_v1::state::State,
    },
    /// Version 2, the live rules (`sim`).
    V2 {
        world: sim::World,
        state: sim::state::State,
    },
}

/// The state of a savepoint, of the version that wrote it.
#[allow(clippy::large_enum_variant)] // as `Run`: one is held at a time
pub(crate) enum Saved {
    V1(sim_v1::state::State),
    V2(sim::state::State),
}

/// What a run is built from, whatever its version: the plain values each version builds its own
/// types from. `spec` and `horde` are the live module's types, built by `run_setup`; version 1
/// converts them field by field (plain data), until each module builds its own from the mode
/// file.
pub(crate) struct Build<'a> {
    pub stage: Stage,
    pub spec: Option<sim::HordeSpec>,
    pub horde: Option<sim::state::Horde>,
    pub kinds: KindTable,
    pub content: &'a RunContent,
    pub star_pool: Vec<u16>,
    pub extra_rerolls: u8,
    /// `None` for a stage run alone: nothing scaled, no rarity left out.
    pub difficulty: Option<Difficulty>,
    pub seed: u64,
    pub stat_priority: &'a [ContentId],
    pub pick_stats: bool,
    /// `None` for a stage run alone: no loadout.
    pub setup: Option<&'a RunSetup>,
}

/// What a run takes from its difficulty (ADR-110), as plain values.
#[derive(Clone, Copy)]
pub(crate) struct Difficulty {
    pub hp_pct: u32,
    pub damage_pct: u32,
    pub rarity: (Rarity, Rarity),
}

#[cfg(test)]
impl Saved {
    /// The version 1 state, for the tests that edit it.
    pub(crate) fn v1(self) -> sim_v1::state::State {
        match self {
            Saved::V1(state) => state,
            Saved::V2(_) => panic!("this test reads version 1 state"),
        }
    }
}

impl Run {
    /// The run of `b` under the rules of `version`; a version the game does not have is an issue.
    pub(crate) fn new(version: u32, b: Build<'_>) -> Result<Run, Issue> {
        match version {
            1 => Run::new_v1(b),
            2 => Run::new_v2(b),
            _ => Err(Issue {
                path: "$.sim_version".into(),
                message: format!("no rules for sim_version {version}"),
            }),
        }
    }

    fn new_v1(b: Build<'_>) -> Result<Run, Issue> {
        let spec = b.spec.map(|s| sim_v1::HordeSpec {
            pool: s.pool,
            final_boss: s.final_boss,
            template: s.template,
            bosses: s.bosses,
            length_ticks: s.length_ticks,
            elite_table: s.elite_table,
            elite_waves: s.elite_waves,
        });
        let horde = b.horde.map(|h| sim_v1::state::Horde {
            boss_ticks: h.boss_ticks,
            no_call_ticks: h.no_call_ticks,
            mid_boss_hp_pct: h.mid_boss_hp_pct,
            spawned: h.spawned,
            used: h.used,
            held: h.held,
        });
        let world = sim_v1::World::new(b.stage, spec, &b.kinds, b.content)?;
        let difficulty =
            b.difficulty
                .map_or(sim_v1::RunDifficulty::NONE, |d| sim_v1::RunDifficulty {
                    hp_pct: d.hp_pct,
                    damage_pct: d.damage_pct,
                    rarity: d.rarity,
                });
        let pool = sim_v1::RunPool {
            rewards: b.star_pool,
            extra_rerolls: b.extra_rerolls,
            difficulty,
        };
        let mut state = sim_v1::initial_state(&world, b.content, pool, b.seed, b.kinds);
        state.horde = horde;
        sim_v1::set_policy(
            &mut state,
            b.content.run_stats(),
            b.stat_priority,
            b.pick_stats,
        );
        if let Some(setup) = b.setup {
            let loadout = sim_v1::Loadout::of(setup, b.content);
            sim_v1::apply_loadout(&world, b.content.rewards(), &mut state, &loadout);
        }
        Ok(Run::V1 { world, state })
    }

    fn new_v2(b: Build<'_>) -> Result<Run, Issue> {
        let world = sim::World::new(b.stage, b.spec, &b.kinds, b.content)?;
        let difficulty = b
            .difficulty
            .map_or(sim::RunDifficulty::NONE, |d| sim::RunDifficulty {
                hp_pct: d.hp_pct,
                damage_pct: d.damage_pct,
                rarity: d.rarity,
            });
        let pool = sim::RunPool {
            rewards: b.star_pool,
            extra_rerolls: b.extra_rerolls,
            difficulty,
        };
        let mut state = sim::initial_state(&world, b.content, pool, b.seed, b.kinds);
        state.horde = b.horde;
        sim::set_policy(
            &mut state,
            b.content.run_stats(),
            b.stat_priority,
            b.pick_stats,
        );
        if let Some(setup) = b.setup {
            let loadout = sim::Loadout::of(setup, b.content);
            sim::apply_loadout(
                &world,
                b.content.rewards(),
                &mut state,
                sim::state::PlayerIdx::SOLO,
                &loadout,
            );
        }
        Ok(Run::V2 { world, state })
    }

    /// The `sim_version` whose rules this state belongs to.
    pub(crate) fn version(&self) -> u32 {
        match self {
            Run::V1 { .. } => 1,
            Run::V2 { .. } => 2,
        }
    }

    /// The module whose rules run this state.
    #[cfg(test)]
    pub(crate) fn module(&self) -> &'static str {
        match self {
            Run::V1 { .. } => "sim_v1",
            Run::V2 { .. } => "sim",
        }
    }

    /// The `sim_version` the run's stage says: what `Engine` checks against the released
    /// versions before it runs a rule.
    pub(crate) fn declared_version(&self) -> u32 {
        match self {
            Run::V1 { world, .. } => world.sim_version,
            Run::V2 { world, .. } => world.sim_version,
        }
    }

    pub(crate) fn step(
        &mut self,
        rewards: &Rewards,
        expertise: &Expertise,
        input: Input,
        events: &mut Vec<DomainEvent>,
        load: &mut Load,
    ) {
        let version = self.version();
        match self {
            Run::V1 { world, state } => {
                sim_v1::step(
                    version, world, rewards, expertise, state, input, events, load,
                );
            }
            Run::V2 { world, state } => {
                sim::step(
                    version, world, rewards, expertise, state, input, events, load,
                );
            }
        }
    }

    pub(crate) fn choose(
        &mut self,
        rewards: &Rewards,
        expertise: &Expertise,
        option: OfferOption,
        events: &mut Vec<DomainEvent>,
    ) {
        let version = self.version();
        match self {
            Run::V1 { world, state } => {
                sim_v1::choose(version, world, rewards, expertise, state, option, events);
            }
            Run::V2 { world, state } => {
                sim::choose(version, world, rewards, expertise, state, option, events);
            }
        }
    }

    pub(crate) fn offer_open(&self) -> bool {
        match self {
            Run::V1 { state, .. } => sim_v1::offer_open(state),
            Run::V2 { state, .. } => sim::offer_open(state),
        }
    }

    pub(crate) fn offer_options(
        &self,
        rewards: &Rewards,
        expertise: &Expertise,
    ) -> Vec<OfferOption> {
        match self {
            Run::V1 { world, state } => sim_v1::offer_options(world, rewards, expertise, state),
            Run::V2 { world, state } => sim::offer_options(world, rewards, expertise, state),
        }
    }

    pub(crate) fn state_hash(&self) -> u64 {
        match self {
            Run::V1 { state, .. } => sim_v1::state_hash(state),
            Run::V2 { state, .. } => sim::state_hash(state),
        }
    }

    /// What the frontend draws after the last tick, built by this version's rules.
    pub(crate) fn snapshot(&self, content: &RunContent) -> Snapshot {
        match self {
            Run::V1 { world, state } => sim_v1::snapshot::build(world, content, state),
            Run::V2 { world, state } => sim::snapshot::build(world, content, state),
        }
    }

    /// Fills `out` with where the drawn things are, by this version's rules.
    pub(crate) fn places(&self, out: &mut Places) {
        match self {
            Run::V1 { state, .. } => sim_v1::snapshot::places(state, out),
            Run::V2 { state, .. } => sim::snapshot::places(state, out),
        }
    }

    /// Reserves the room a run holds from load, so a clone's first ticks do not grow it (DIR1).
    pub(crate) fn reserve(&mut self) {
        match self {
            Run::V1 { world, state } => sim_v1::reserve(world, state),
            Run::V2 { world, state } => sim::reserve(world, state),
        }
    }

    /// The room the events of a tick need.
    pub(crate) fn events_room(&self) -> usize {
        match self {
            Run::V1 { world, .. } => sim_v1::events_room(world),
            Run::V2 { world, .. } => sim::events_room(world),
        }
    }

    /// The caps of this version's rules: see `Engine::caps`.
    pub(crate) fn caps(&self) -> Load {
        match self {
            Run::V1 { world, .. } => Load {
                enemies: world.enemy_cap,
                tasks: sim_v1::MAX_ATTACK_TASKS,
                zones: sim_v1::MAX_ZONES_ALIVE,
                orbs: sim_v1::MAX_ORBS_ALIVE,
                emitter_steps: sim_v1::MAX_EMITTER_STEPS_PER_TICK,
                ..Load::default()
            },
            Run::V2 { world, .. } => Load {
                enemies: world.enemy_cap,
                tasks: sim::MAX_ATTACK_TASKS,
                zones: sim::MAX_ZONES_ALIVE,
                orbs: sim::MAX_ORBS_ALIVE,
                emitter_steps: sim::MAX_EMITTER_STEPS_PER_TICK,
                ..Load::default()
            },
        }
    }

    /// The run's kind table.
    pub(crate) fn kinds(&self) -> &KindTable {
        match self {
            Run::V1 { state, .. } => &state.kinds,
            Run::V2 { state, .. } => &state.kinds,
        }
    }

    pub(crate) fn tick(&self) -> u32 {
        match self {
            Run::V1 { state, .. } => state.tick,
            Run::V2 { state, .. } => state.tick,
        }
    }

    pub(crate) fn stardust(&self) -> u32 {
        match self {
            Run::V1 { state, .. } => state.stardust.unwrap_or(0),
            Run::V2 { state, .. } => state.stardust.unwrap_or(0),
        }
    }

    pub(crate) fn outcome(&self) -> Outcome {
        match self {
            Run::V1 { state, .. } => state.outcome,
            Run::V2 { state, .. } => state.outcome,
        }
    }

    /// Ends the run as failed, running no rule: the fail-closed answer to a version this game
    /// does not run.
    pub(crate) fn fail(&mut self) {
        match self {
            Run::V1 { state, .. } => state.outcome = Outcome::Failed,
            Run::V2 { state, .. } => state.outcome = Outcome::Failed,
        }
    }

    /// The content ID of the weapon the run's main character holds, "inline" for one the stage
    /// defines itself.
    fn weapon(&self) -> &str {
        use crate::weapon::WeaponRef;
        let stage = match self {
            Run::V1 { world, .. } => &world.stage,
            Run::V2 { world, .. } => &world.stage,
        };
        match &stage.player.weapon {
            WeaponRef::Preset { id, .. } => id.as_str(),
            WeaponRef::Inline(_) => "inline",
        }
    }

    /// The ledger of a new run: its kinds and its weapon.
    pub(crate) fn new_ledger(&self) -> RunLedger {
        RunLedger::for_engine(self.kinds(), self.weapon())
    }

    /// Lets the ledger see the enemies and the boss after a tick and the events of it.
    pub(crate) fn observe_tick(&self, ledger: &mut RunLedger, events: &[DomainEvent]) {
        let weapon = self.weapon();
        match self {
            Run::V1 { state, .. } => ledger.observe_tick(
                &state.kinds,
                weapon,
                &state.enemies,
                state.boss.as_ref(),
                events,
            ),
            Run::V2 { state, .. } => ledger.observe_tick(
                &state.kinds,
                weapon,
                &state.enemies,
                state.boss.as_ref(),
                events,
            ),
        }
    }

    /// Lets the ledger see the events of a choice.
    pub(crate) fn observe_choice(&self, ledger: &mut RunLedger, events: &[DomainEvent]) {
        ledger.observe_choice(self.kinds(), self.weapon(), events);
    }

    /// This run with the state a savepoint held, of the same version, the world kept.
    pub(crate) fn with_state(self, saved: Saved) -> Run {
        match (self, saved) {
            (Run::V1 { world, .. }, Saved::V1(state)) => Run::V1 { world, state },
            (Run::V2 { world, .. }, Saved::V2(state)) => Run::V2 { world, state },
            // `decode_state` reads the state with the fresh run's own module.
            (_, _) => unreachable!("a saved state is of the version of the run it is read for"),
        }
    }
}

/// Tests read and edit the version 1 fields directly; the rules never go through this. A test
/// that edits state builds a version 1 engine.
#[cfg(test)]
impl std::ops::Deref for Run {
    type Target = sim_v1::state::State;
    fn deref(&self) -> &sim_v1::state::State {
        match self {
            Run::V1 { state, .. } => state,
            Run::V2 { .. } => panic!("this test reads version 1 state"),
        }
    }
}

#[cfg(test)]
impl std::ops::DerefMut for Run {
    fn deref_mut(&mut self) -> &mut sim_v1::state::State {
        match self {
            Run::V1 { state, .. } => state,
            Run::V2 { .. } => panic!("this test edits version 1 state"),
        }
    }
}

#[cfg(test)]
impl Run {
    pub(crate) fn world(&self) -> &sim_v1::World {
        match self {
            Run::V1 { world, .. } => world,
            Run::V2 { .. } => panic!("this test reads a version 1 world"),
        }
    }

    pub(crate) fn world_mut(&mut self) -> &mut sim_v1::World {
        match self {
            Run::V1 { world, .. } => world,
            Run::V2 { .. } => panic!("this test edits a version 1 world"),
        }
    }

    pub(crate) fn set_state(&mut self, new: sim_v1::state::State) {
        match self {
            Run::V1 { state, .. } => *state = new,
            Run::V2 { .. } => panic!("this test sets version 1 state"),
        }
    }
}

/// The types of version 1 by one path for the tests (the rules the tests run, until a version 2
/// differs).
#[cfg(test)]
pub(crate) mod v1 {
    pub(crate) use crate::sim_v1::World;
    pub(crate) use crate::sim_v1::state::*;
}
