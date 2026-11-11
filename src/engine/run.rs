//! The engine's run, one variant per released rules version (ADR-REPL-2 points 3 and 4): each version owns its `World`, its `State` and its snapshot view, `V1` in `sim_v1`
//! (frozen) and `V2` in `sim` (live). Every call into the rules goes through a `match` here, so
//! adding a version lists each site as a compile error. No box, no trait object: the enum is the
//! larger variant's `World` and `State` and a tag. A version's events are its own types too:
//! version 1 emits `sim_v1::event::DomainEvent`, which `v1_events` adapts into the engine's list.

use std::sync::Arc;

use crate::event::DomainEvent;
use crate::expertise::Expertise;
use crate::id::{ContentId, PlayerIdx};
use crate::input::Input;
use crate::kinds::KindTable;
use crate::ledger::RunLedger;
use crate::rewards::{Rarity, Rewards};
use crate::run_content::RunContent;
use crate::run_setup::RunSetup;
use crate::snapshot::{OfferOption, Snapshot, SwapItem, SwapRating};
use crate::stage::Stage;
use crate::validate::Issue;
use crate::{sim, sim_v1};

use super::{Load, Outcome, Places};

/// A run under the rules of one `sim_version`: the compiled stage and the state it changes, both
/// of that version's types (the live world is shared behind an `Arc`, so a clone copies the state
/// alone; the frozen world stays inline). The engine reaches them, and the snapshot view, only through here.
// The frozen state is the larger: the live one keeps its players in a list (ADR-SIM-7). One run
// is held per engine, so the size difference costs nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub(crate) enum Run {
    /// Version 1, the frozen first rules (`sim_v1`).
    V1 {
        world: sim_v1::World,
        state: sim_v1::state::State,
        /// The frozen events of the tick being run: `v1_events` adapts them into the engine's
        /// list. Scratch, like `sim_v1::scratch`: empty between calls, never saved, hashed or
        /// compared.
        events: Vec<sim_v1::event::DomainEvent>,
    },
    /// Version 2, the live rules (`sim`).
    V2 {
        world: Arc<sim::World>,
        state: sim::state::State,
    },
}

impl Clone for Run {
    fn clone(&self) -> Run {
        match self {
            Run::V1 {
                world,
                state,
                events,
            } => Run::V1 {
                world: world.clone(),
                state: state.clone(),
                events: events.clone(),
            },
            Run::V2 { world, state } => Run::V2 {
                world: world.clone(),
                state: state.clone(),
            },
        }
    }

    /// The live rules copy their state into the one already held; the frozen rules, and a copy
    /// between two versions, are a clone (only the live rules are in place).
    fn clone_from(&mut self, source: &Run) {
        match (&mut *self, source) {
            (
                Run::V2 { world, state },
                Run::V2 {
                    world: s_world,
                    state: s_state,
                },
            ) => {
                world.clone_from(s_world);
                state.clone_from(s_state);
            }
            _ => *self = source.clone(),
        }
    }
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
    /// `None` for a stage run alone: no loadout.
    pub setup: Option<&'a RunSetup>,
    /// The players in the party, 1 to `MAX_PLAYERS`; version 1 has one.
    pub party: u8,
}

/// What a run takes from its difficulty, as plain values.
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

    /// The version 2 state, for the tests that edit it.
    #[allow(dead_code)] // read by the savepoint port that follows
    pub(crate) fn v2(self) -> sim::state::State {
        match self {
            Saved::V2(state) => state,
            Saved::V1(_) => panic!("this test reads version 2 state"),
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
        if b.party != 1 {
            return Err(Issue {
                path: "$.party".into(),
                message: "sim_version 1 has one player".into(),
            });
        }
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
        sim_v1::set_policy(&mut state, b.content.run_stats(), b.stat_priority, false);
        if let Some(setup) = b.setup {
            let loadout = sim_v1::Loadout::of(setup, b.content);
            sim_v1::apply_loadout(&world, b.content.rewards(), &mut state, &loadout);
        }
        let events = Vec::with_capacity(sim_v1::events_room(&world));
        Ok(Run::V1 {
            world,
            state,
            events,
        })
    }

    fn new_v2(b: Build<'_>) -> Result<Run, Issue> {
        let mut world = sim::World::new(b.stage, b.spec, &b.kinds, b.content)?;
        // sim_version >= 2: the equipment mode (ADR-SIM-7). A run with a setup loads the pack's
        // rules and drop table; a stage run alone keeps them empty, so its equipment branches run
        // with zero numbers (no boss bonus, no drops).
        if b.setup.is_some() {
            // `run_setup::validate_in` refused a run whose pack lacks the rules.
            if let Some(rules) = b.content.rewards().equipment() {
                world.equipment_rules = rules.clone();
            }
            world.drops = sim::DropTable::of(b.content.rewards(), &world.equipment_rules);
        }
        let loadout = b.setup.map(|setup| sim::Loadout::of(setup, b.content));
        world.drop_weight_pct = loadout
            .as_ref()
            .map_or_else(Vec::new, |l| l.drop_weight_pct(&world.equipment_rules));
        let difficulty = b
            .difficulty
            .map_or(sim::RunDifficulty::NONE, |d| sim::RunDifficulty {
                hp_pct: d.hp_pct,
                damage_pct: d.damage_pct,
                rarity: d.rarity,
            });
        let pool = sim::RunPool {
            rewards: b.star_pool,
            difficulty,
        };
        let mut state = sim::initial_state(&world, b.content, pool, b.seed, b.kinds);
        state.horde = b.horde;
        // A run's setup loads the first player; per-player setup comes with recording parties.
        sim::set_policy(
            &mut state,
            PlayerIdx::SOLO,
            b.content.run_stats(),
            b.stat_priority,
        );
        if let Some(loadout) = loadout {
            // A run's setup loads the first player; the party grows with the per-player setup.
            sim::apply_loadout(
                &world,
                b.content.rewards(),
                &mut state,
                PlayerIdx::SOLO,
                &loadout,
            );
        }
        if b.party > 1 {
            // Slots 1 and 2 take the run's loadout until the per-player setup.
            state.players = sim::state::Players::party(b.party, &state.players, state.crit_seed);
            sim::reserve(&world, &mut state);
        }
        Ok(Run::V2 {
            world: Arc::new(world),
            state,
        })
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
        inputs: &[Input; crate::id::MAX_PLAYERS as usize],
        events: &mut Vec<DomainEvent>,
        load: &mut Load,
    ) {
        let version = self.version();
        match self {
            Run::V1 {
                world,
                state,
                events: frozen,
            } => {
                sim_v1::step(
                    version, world, rewards, expertise, state, inputs[0], frozen, load,
                );
                events.extend(frozen.drain(..).filter_map(super::v1_events::adapt));
            }
            Run::V2 { world, state } => {
                sim::step(
                    version, world, rewards, expertise, state, inputs, events, load,
                );
            }
        }
    }

    /// Carries out `option` of `player`'s open offer; version 1 has one player and ignores it.
    pub(crate) fn choose(
        &mut self,
        rewards: &Rewards,
        expertise: &Expertise,
        _player: PlayerIdx,
        option: OfferOption,
        events: &mut Vec<DomainEvent>,
    ) {
        let version = self.version();
        match self {
            Run::V1 {
                world,
                state,
                events: frozen,
            } => {
                sim_v1::choose(
                    version,
                    world,
                    rewards,
                    expertise,
                    state,
                    option.into(),
                    frozen,
                );
                events.extend(frozen.drain(..).filter_map(super::v1_events::adapt));
            }
            // sim_version 2 opens no offer: there is nothing to choose.
            Run::V2 { .. } => {}
        }
    }

    /// How many players the run holds: the live rules' party, one in version 1.
    pub(crate) fn party(&self) -> usize {
        match self {
            Run::V1 { .. } => 1,
            Run::V2 { state, .. } => state.players.len(),
        }
    }

    /// The slots in the party, bit `i` for slot `i`: `0b001` under version 1.
    pub(crate) fn present(&self) -> u8 {
        match self {
            Run::V1 { .. } => 0b001,
            Run::V2 { state, .. } => state.players.mask(),
        }
    }

    /// The generation of slot `slot`'s current or last occupant: 0 under version 1.
    pub(crate) fn generation(&self, slot: PlayerIdx) -> u8 {
        match self {
            Run::V1 { .. } => 0,
            Run::V2 { state, .. } => state.players.generation(slot),
        }
    }

    /// Slot 0's player as the run starts, for a joiner; none under version 1.
    pub(crate) fn joiner_template(&self) -> Option<sim::state::Players> {
        match self {
            Run::V1 { .. } => None,
            Run::V2 { state, .. } => Some(state.players.template()),
        }
    }

    /// Slot `me` joins, built from `template`; the room is taken at once. Live rules only.
    pub(crate) fn join(&mut self, me: PlayerIdx, template: &sim::state::Players) {
        if let Run::V2 { world, state } = self {
            let seed = state.crit_seed;
            state.players.join(me, template, seed);
            sim::reserve(world, state);
        }
    }

    /// Slot `me` leaves. Live rules only.
    pub(crate) fn leave(&mut self, me: PlayerIdx) {
        if let Run::V2 { state, .. } = self {
            state.players.leave(me);
        }
    }

    /// The refusal that locks `player`'s equipment window, if any: `Downed` at 0 hit points,
    /// `Out` once out of the run. Only an equipment run of the live rules locks (ADR-SIM-7);
    /// every other run keeps `NotBuilt` for all. `player` is in the party.
    pub(crate) fn window_lock(&self, player: PlayerIdx) -> Option<super::ActionError> {
        match self {
            Run::V2 { state, .. } => {
                let p = &state.players[player];
                if p.is_out() {
                    Some(super::ActionError::Out)
                } else if p.is_downed() {
                    Some(super::ActionError::Downed)
                } else {
                    None
                }
            }
            Run::V1 { .. } => None,
        }
    }

    /// Whether `player` could equip (or `replace`) inventory item `item` into `slot`: `None`
    /// where no rule applies, a `sim_version` 1 run.
    pub(crate) fn check_equip(
        &self,
        rewards: &Rewards,
        player: PlayerIdx,
        item: u32,
        slot: u32,
        replace: bool,
    ) -> Option<Result<(), sim::SlotError>> {
        match self {
            Run::V2 { world, state } => Some(sim::check_equip(
                &world.equipment_rules,
                rewards,
                state,
                player,
                item,
                slot,
                replace,
            )),
            Run::V1 { .. } => None,
        }
    }

    /// Makes an equip (or `replace`) already accepted by `check_equip`.
    pub(crate) fn equip(
        &mut self,
        rewards: &Rewards,
        player: PlayerIdx,
        item: u32,
        slot: u32,
        replace: bool,
    ) {
        if let Run::V2 { world, state } = self {
            let _ = sim::equip_action(world, rewards, state, player, item, slot, replace);
        }
    }

    /// Whether `player` could extract inventory item `item`: `None` where no rule applies.
    pub(crate) fn check_extract(
        &self,
        player: PlayerIdx,
        item: u32,
    ) -> Option<Result<(), sim::StarlightError>> {
        match self {
            Run::V2 { state, .. } => Some(sim::check_extract(state, player, item)),
            Run::V1 { .. } => None,
        }
    }

    /// Makes an extract already accepted by `check_extract`.
    pub(crate) fn extract(&mut self, rewards: &Rewards, player: PlayerIdx, item: u32) {
        if let Run::V2 { world, state } = self {
            let _ = sim::extract_action(&world.equipment_rules, rewards, state, player, item);
        }
    }

    /// Whether `player` could craft the star at index `star` of the star pool: `None` where no
    /// rule applies.
    pub(crate) fn check_craft(
        &self,
        rewards: &Rewards,
        player: PlayerIdx,
        star: u32,
    ) -> Option<Result<(), sim::StarlightError>> {
        match self {
            Run::V2 { world, state } => Some(sim::check_craft(
                &world.equipment_rules,
                rewards,
                state,
                player,
                star,
            )),
            Run::V1 { .. } => None,
        }
    }

    /// Makes a craft already accepted by `check_craft`.
    pub(crate) fn craft(&mut self, rewards: &Rewards, player: PlayerIdx, star: u32) {
        if let Run::V2 { world, state } = self {
            let _ = sim::craft_action(&world.equipment_rules, rewards, state, player, star);
        }
    }

    /// Whether `player` could take choice `option` of the next tree pick: `None` where no rule
    /// applies.
    pub(crate) fn check_tree_pick(
        &self,
        expertise: &Expertise,
        player: PlayerIdx,
        option: u32,
    ) -> Option<Result<(), sim::TreeError>> {
        match self {
            Run::V2 { world, state } => Some(sim::check_tree_pick(
                world, expertise, state, player, option,
            )),
            Run::V1 { .. } => None,
        }
    }

    /// Makes a tree pick already accepted by `check_tree_pick`.
    pub(crate) fn tree_pick(&mut self, expertise: &Expertise, player: PlayerIdx, option: u32) {
        if let Run::V2 { world, state } = self {
            let _ = sim::tree_pick_action(world, expertise, state, player, option);
        }
    }

    /// Whether `player` has an offer open.
    pub(crate) fn offer_open_for(&self, player: PlayerIdx) -> bool {
        match self {
            Run::V1 { state, .. } => player == PlayerIdx::SOLO && sim_v1::offer_open(state),
            // The live rules open no offer.
            Run::V2 { .. } => false,
        }
    }

    pub(crate) fn offer_open(&self) -> bool {
        match self {
            Run::V1 { state, .. } => sim_v1::offer_open(state),
            Run::V2 { .. } => false,
        }
    }

    /// The options of `player`'s open offer; version 1 has one player and ignores it.
    pub(crate) fn offer_options(
        &self,
        rewards: &Rewards,
        expertise: &Expertise,
        _player: PlayerIdx,
    ) -> Vec<OfferOption> {
        match self {
            Run::V1 { world, state, .. } => sim_v1::offer_options(world, rewards, expertise, state)
                .into_iter()
                .map(Into::into)
                .collect(),
            Run::V2 { .. } => Vec::new(),
        }
    }

    /// The ratings of `item` against each slot it could go in; version 1 is frozen and rates
    /// nothing.
    pub(crate) fn swap_ratings(
        &self,
        rewards: &Rewards,
        player: PlayerIdx,
        item: SwapItem,
    ) -> Vec<SwapRating> {
        match self {
            Run::V1 { .. } => Vec::new(),
            Run::V2 { world, state } => sim::swap_ratings(world, rewards, state, player, item),
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
            Run::V1 { world, state, .. } => sim_v1::snapshot::build(world, content, state),
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

    /// Whether this is the live rules, the ones a copy is made in place for.
    pub(crate) fn is_live(&self) -> bool {
        matches!(self, Run::V2 { .. })
    }

    /// Reserves the room a run holds from load, so a clone's first ticks do not grow it (DIR1).
    pub(crate) fn reserve(&mut self) {
        match self {
            Run::V1 {
                world,
                state,
                events,
            } => {
                sim_v1::reserve(world, state);
                events.reserve(sim_v1::events_room(world).saturating_sub(events.len()));
            }
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
            // `Engine::stardust` is one number and every present player earns the same, until a
            // player can join late (joining and leaving a run); it reads the first player's.
            Run::V2 { state, .. } => state.players.career(PlayerIdx::SOLO).stardust.unwrap_or(0),
        }
    }

    /// The Stardust of the player in `player`; `None` for a slot that is not present. Version 1
    /// has slot 0 only.
    pub(crate) fn stardust_of(&self, player: PlayerIdx) -> Option<u32> {
        match self {
            Run::V1 { .. } => (player == PlayerIdx::SOLO).then(|| self.stardust()),
            Run::V2 { state, .. } => state
                .players
                .is_present(player)
                .then(|| state.players.career(player).stardust.unwrap_or(0)),
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
            (Run::V1 { world, events, .. }, Saved::V1(state)) => Run::V1 {
                world,
                state,
                events,
            },
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

/// The live version 2 state for the tests that port the frozen ones (the `Deref` above stays
/// version 1: a type has one).
#[cfg(test)]
impl Run {
    pub(crate) fn live(&self) -> &sim::state::State {
        match self {
            Run::V2 { state, .. } => state,
            Run::V1 { .. } => panic!("this test reads version 2 state"),
        }
    }

    pub(crate) fn live_mut(&mut self) -> &mut sim::state::State {
        match self {
            Run::V2 { state, .. } => state,
            Run::V1 { .. } => panic!("this test edits version 2 state"),
        }
    }

    pub(crate) fn live_world(&self) -> &sim::World {
        match self {
            Run::V2 { world, .. } => world,
            Run::V1 { .. } => panic!("this test reads a version 2 world"),
        }
    }

    pub(crate) fn live_world_mut(&mut self) -> &mut sim::World {
        match self {
            Run::V2 { world, .. } => Arc::make_mut(world),
            Run::V1 { .. } => panic!("this test edits a version 2 world"),
        }
    }

    pub(crate) fn set_live_state(&mut self, new: sim::state::State) {
        match self {
            Run::V2 { state, .. } => *state = new,
            Run::V1 { .. } => panic!("this test sets version 2 state"),
        }
    }

    /// The player in slot 0.
    pub(crate) fn solo(&self) -> &sim::state::PlayerState {
        &self.live().players[crate::id::PlayerIdx::SOLO]
    }

    pub(crate) fn solo_mut(&mut self) -> &mut sim::state::PlayerState {
        &mut self.live_mut().players[crate::id::PlayerIdx::SOLO]
    }
}

/// The types of version 2 by one path for the tests.
#[cfg(test)]
#[allow(unused_imports)] // read by the ports that follow
pub(crate) mod live {
    pub(crate) use crate::sim::World;
    pub(crate) use crate::sim::state::*;
}

/// The types of version 1 by one path for the tests (the rules the tests run, until a version 2
/// differs).
#[cfg(test)]
pub(crate) mod v1 {
    pub(crate) use crate::sim_v1::World;
    pub(crate) use crate::sim_v1::state::*;
}
