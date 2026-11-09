//! The simulation engine: runs a validated stage tick by tick to a clear or a fail
//! (ADR-SIM-1, ADR-SVC-7). The only external influence is one `Input` per tick and the commands
//! between ticks (ADR-REPL-5: `choose`); everything else comes from the stage and its
//! seed, or from a run setup, and from the run stats (ADR-CONT-1), so the state hash after each
//! tick is the same on every platform (ADR-SIM-3).
//!
//! Rules are dispatched by the stage's `sim_version` (ADR-REPL-1, ADR-REPL-2 points 1 to 3): versions
//! `1..=SIM_VERSION` run in `sim`, any other fails closed. Version 1 is frozen, so a change of
//! outcome after it is a gate on the new version in `sim`, a copy only for a rewrite (see `sim`
//! for what it does and does not do yet). The engine holds its run as a `Run`, one variant per
//! released rules version (`run.rs`, ADR-REPL-2) that owns that version's `World` and `State`;
//! every call into the rules goes through it, the snapshot view included (`sim::snapshot`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::event::DomainEvent;
use crate::id::{ContentId, PlayerIdx};
use crate::input::EquipAction;
use crate::input::Input;
use crate::kinds::KindTable;
use crate::ledger::{Counted, PlayerStats, RunLedger};
use crate::rewards::Rewards;
use crate::run_content::RunContent;
use crate::run_setup::{self, BuildMode, RunSetup};
use crate::run_stats::RunStats;
use crate::sim::HordeSpec;
use crate::snapshot::{OfferOption, OfferRating, SwapItem, SwapRating};
use crate::stage::Stage;
use crate::validate::{self, Error, Issue};
use crate::version::SIM_VERSION;

/// Why `Engine::choose` refused a choice; the run is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceError {
    /// `player` is not in the run's party.
    NoPlayer { player: PlayerIdx },
    /// No offer is open.
    NoOffer,
    /// `index` is not below the number of the open offer's `options`.
    OutOfRange { index: u32, options: u32 },
    /// The run's `sim_version` is not one this game runs (ADR-REPL-2 points 1 to 3). `Engine::new`
    /// refuses such a stage, so this is a fail-closed guard, not a path a valid run takes.
    Unsupported { sim_version: u32 },
}

/// Why `Engine::check_action` refused an equipment action (ADR-SIM-7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionError {
    /// `player` is not in the run's party.
    NoPlayer { player: PlayerIdx },
    /// No rule accepts this action: a run that is not in equipment mode or is `sim_version` 1
    /// (every action).
    NotBuilt,
    /// `item` is not a place of the inventory.
    NoItem { item: u32 },
    /// `slot` addresses no slot: `0..` the slots, and one past them for the Lumine slot when the
    /// run has one.
    NoSlot { slot: u32 },
    /// `Equip` into an occupied slot.
    SlotFilled { slot: u32 },
    /// `Replace` into an empty slot.
    SlotEmpty { slot: u32 },
    /// A rune in a star slot, a star in a rune slot, or a star in a slot of another category.
    WrongSlot { slot: u32 },
    /// The item is the lone random-rune reward.
    NotEquippable { item: u32 },
    /// Another slot (the Lumine slot included) holds a star sharing an option with the item.
    OptionHeld { slot: u32 },
    /// That rune is in another rune slot.
    RuneHeld { slot: u32 },
    /// `Craft`: `star` is past the setup's star pool, or its place is past the rewards file.
    NoStar { star: u32 },
    /// `Craft`: the star is the lone random-rune reward.
    NotCraftable { star: u32 },
    /// `Craft`: the star is named and the player already holds it.
    AlreadyHeld { star: u32 },
    /// `Craft`: the star costs `need` Starlight and the player has `have`.
    NotEnoughStarlight { need: u32, have: u32 },
    /// `Craft`: the inventory is at its bound.
    InventoryFull,
    /// `TreePick`: no tree point is held (none earned, all taken, no gauge or no expertise
    /// content).
    NoTreePoint,
    /// `TreePick`: `option` is not below the `options` the next pick has.
    NoTreeOption { option: u32, options: u32 },
    /// The player is downed: its equipment window is locked until a teammate revives it. Every
    /// kind of action; an equipment run only.
    Downed,
    /// The player is out of the run: its equipment window is locked for good. Every kind of
    /// action; an equipment run only.
    Out,
}

impl From<crate::sim::TreeError> for ActionError {
    fn from(e: crate::sim::TreeError) -> ActionError {
        use crate::sim::TreeError as T;
        match e {
            T::NoTreePoint => ActionError::NoTreePoint,
            T::NoTreeOption { option, options } => ActionError::NoTreeOption { option, options },
        }
    }
}

impl From<crate::sim::StarlightError> for ActionError {
    fn from(e: crate::sim::StarlightError) -> ActionError {
        use crate::sim::StarlightError as S;
        match e {
            S::NoItem { item } => ActionError::NoItem { item },
            S::NoStar { star } => ActionError::NoStar { star },
            S::NotCraftable { star } => ActionError::NotCraftable { star },
            S::AlreadyHeld { star } => ActionError::AlreadyHeld { star },
            S::NotEnoughStarlight { need, have } => ActionError::NotEnoughStarlight { need, have },
            S::InventoryFull => ActionError::InventoryFull,
        }
    }
}

impl From<crate::sim::SlotError> for ActionError {
    fn from(e: crate::sim::SlotError) -> ActionError {
        use crate::sim::SlotError as S;
        match e {
            S::NoItem { item } => ActionError::NoItem { item },
            S::NoSlot { slot } => ActionError::NoSlot { slot },
            S::SlotFilled { slot } => ActionError::SlotFilled { slot },
            S::SlotEmpty { slot } => ActionError::SlotEmpty { slot },
            S::WrongSlot { slot } => ActionError::WrongSlot { slot },
            S::NotEquippable { item } => ActionError::NotEquippable { item },
            S::OptionHeld { slot } => ActionError::OptionHeld { slot },
            S::RuneHeld { slot } => ActionError::RuneHeld { slot },
        }
    }
}

/// How a run stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Running,
    /// The stage was cleared: this is what a replay has to prove (ADR-REPL-6).
    Cleared,
    /// The main character died, or the stage ran out of ticks.
    Failed,
}

/// How much of each dynamic budget (ADR-SVC-1) a tick used: the most it reached during the tick.
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

/// Why `Engine::join` or `Engine::leave` refused a change; the run is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresenceError {
    /// `slot` is already in the party (`join`).
    Occupied { slot: PlayerIdx },
    /// `slot` is not in the party (`leave`).
    Absent { slot: PlayerIdx },
    /// Slot 0 never leaves.
    FirstSlot,
    /// The run is over.
    Ended,
    /// A `sim_version` 1 run has one player for good.
    SoloRules,
    /// `slot` has left 255 times: its generation is spent (`leave`).
    Exhausted { slot: PlayerIdx },
}

/// A run of one stage.
#[derive(Debug)]
pub struct Engine {
    /// The content files the run reads (ADR-CONT-1), fixed for the run.
    content: RunContent,
    run: Run,
    /// Events of the last tick; cleared when the next one starts.
    events: Vec<DomainEvent>,
    /// What the last tick used of the dynamic budgets; zero when the tick did not run.
    load: Load,
    /// The ledger of counts and bestiary encounters (ADR-BRDG-6).
    ledger: RunLedger,
    /// The HUD tallies, counted from the events (outside the hashed state).
    counted: Counted,
    /// Damage, kills and support per player slot, folded from the events (outside the hashed
    /// state; a savepoint holds them, ADR-SIM-7, ADR-SAVE-4).
    player_stats: PlayerStats,
    /// Slot 0's player as the run started (the loadout, level 1), what a joiner is built from.
    /// Outside the hashed state; none under `sim_version` 1.
    joiner: Option<crate::sim::state::Players>,
    /// Built past the `SIM_VERSION` check by `Engine::forced`, to run a version the game does
    /// not release yet.
    #[cfg(test)]
    forced: bool,
}

impl Clone for Engine {
    /// A clone keeps the room the run reserved at load: a derived clone of a `Vec` has only its
    /// length, and the clone's first ticks would grow them (DIR1).
    fn clone(&self) -> Engine {
        let mut run = self.run.clone();
        run.reserve();
        Engine {
            content: self.content.clone(),
            events: {
                let mut events = self.events.clone();
                events.reserve(run.events_room().saturating_sub(events.len()));
                events
            },
            run,
            load: self.load,
            ledger: self.ledger.clone(),
            counted: self.counted.clone(),
            player_stats: self.player_stats.clone(),
            joiner: self.joiner.clone(),
            #[cfg(test)]
            forced: self.forced,
        }
    }
}

impl Engine {
    /// The `sim_version` whose rules the run holds.
    pub(crate) fn version(&self) -> u32 {
        self.run.version()
    }

    /// The run, for the savepoint to read the state of by its version.
    pub(crate) fn run(&self) -> &Run {
        &self.run
    }

    /// The ledger, the tallies and the player stats, for the savepoint.
    pub(crate) fn saved_parts(&self) -> (&RunLedger, &Counted, &PlayerStats) {
        (&self.ledger, &self.counted, &self.player_stats)
    }

    /// The version 1 parts of a savepoint, for the tests that read them: the world, the content,
    /// the state, the ledger and the tallies.
    #[cfg(test)]
    pub(crate) fn savepoint_parts(
        &self,
    ) -> (
        &crate::sim_v1::World,
        &RunContent,
        &crate::sim_v1::state::State,
        &RunLedger,
        &Counted,
    ) {
        (
            self.run.world(),
            &self.content,
            &self.run,
            &self.ledger,
            &self.counted,
        )
    }

    /// This engine with the state, ledger, tallies and player stats a savepoint held: a fresh
    /// engine of the same setup and content (the world and content are not saved), the saved
    /// parts swapped in. `decode_state` has reserved the state's room; no tick rule changes.
    pub(crate) fn with_saved(
        self,
        state: Saved,
        ledger: RunLedger,
        counted: Counted,
        player_stats: PlayerStats,
    ) -> Engine {
        let mut events = self.events;
        events.clear();
        Engine {
            content: self.content,
            run: self.run.with_state(state),
            events,
            load: Load::default(),
            ledger,
            counted,
            player_stats,
            joiner: self.joiner,
            #[cfg(test)]
            forced: self.forced,
        }
    }

    /// An engine of `run` and `content`, before its first tick.
    fn of(run: Run, content: &RunContent, #[allow(unused)] forced: bool) -> Engine {
        Engine {
            events: Vec::with_capacity(run.events_room()),
            ledger: run.new_ledger(),
            joiner: run.joiner_template(),
            run,
            content: content.clone(),
            load: Load::default(),
            counted: Counted::new(content.rewards()),
            player_stats: PlayerStats::default(),
            #[cfg(test)]
            forced,
        }
    }

    /// Starts a run. The stage is validated first (ADR-SVC-1), so the rules below only ever see
    /// stages inside the budgets.
    /// A stage with levels raises the bundled run stats, in their order.
    pub fn new(stage: &Stage) -> Result<Engine, Error> {
        Engine::new_with(stage, RunContent::bundled())
    }

    /// As `new`, with `content` in place of the bundled files (ADR-CONT-1).
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
        let kinds = KindTable::of_stage(stage).map_err(|i| Error::Invalid(vec![i]))?;
        let (stage, version, forced) = at_version(stage.clone());
        let seed = u64::from(stage.seed);
        let run = Run::new(
            version,
            Build {
                stage,
                spec: None,
                horde: None,
                kinds,
                content,
                star_pool: pool(None, content.rewards(), BuildMode::Offers)?,
                extra_rerolls: 0,
                difficulty: None,
                seed,
                stat_priority,
                pick_stats,
                setup: None,
                party: 1,
            },
        )
        .map_err(|i| Error::Invalid(vec![i]))?;
        Ok(Engine::of(run, content, forced))
    }

    /// Starts a run from a setup alone (ADR-REPL-5): the setup is validated, then its mode's
    /// stage runs with its weapons and seed, and the bundled content files.
    pub fn start(setup: &RunSetup) -> Result<Engine, Error> {
        Engine::start_with(setup, RunContent::bundled())
    }

    /// As `start`, with `content` in place of the bundled files (ADR-CONT-1).
    pub fn start_with(setup: &RunSetup, content: &RunContent) -> Result<Engine, Error> {
        Engine::start_party_with(setup, content, 1)
    }

    /// As `start`, for a party of `party` players (1 to `MAX_PLAYERS`) with the bundled content
    /// files. Slots 1 and 2 take the run's loadout until the per-player setup.
    pub fn start_party(setup: &RunSetup, party: u8) -> Result<Engine, Error> {
        Engine::start_party_with(setup, RunContent::bundled(), party)
    }

    /// As `start_party`, with `content` in place of the bundled files. A party outside
    /// `1..=MAX_PLAYERS`, and any party above one under `sim_version` 1, is `Error::Invalid`.
    pub fn start_party_with(
        setup: &RunSetup,
        content: &RunContent,
        party: u8,
    ) -> Result<Engine, Error> {
        if !(1..=crate::id::MAX_PLAYERS).contains(&party) {
            return Err(Error::Invalid(vec![Issue {
                path: "$.party".into(),
                message: "a party of 1 to 3 players".into(),
            }]));
        }
        run_setup::validate_in(setup, content)?;
        if setup.mode.as_str() == "time_limit" {
            return Engine::start_time_limit_party(setup, content, content.time_limit(), party);
        }
        let stage = run_setup::stage_in(setup, content)?;
        let kinds = KindTable::of_stage(&stage).map_err(|i| Error::Invalid(vec![i]))?;
        let (stage, version, forced) = at_version(stage);
        let run = Run::new(
            version,
            Build {
                stage,
                spec: None,
                horde: None,
                kinds,
                content,
                star_pool: pool(
                    setup.star_pool.as_deref(),
                    content.rewards(),
                    setup.build_mode,
                )?,
                extra_rerolls: setup.extra_rerolls,
                difficulty: Some(run_difficulty(setup, content)?),
                seed: setup.seed,
                stat_priority: &setup.stat_priority,
                pick_stats: setup.pick_stats,
                setup: Some(setup),
                party,
            },
        )
        .map_err(|i| Error::Invalid(vec![i]))?;
        Ok(Engine::of(run, content, forced))
    }

    /// For tests and benches only, under the `bench` feature: as `start_with`, on `stage` as given
    /// instead of the stage `setup.mode` derives, so a hand-built stage runs in the setup's build
    /// mode (equipment mode for `BuildMode::Equipment`, ADR-SIM-7). Nothing in the simulation
    /// differs; a replay or a savepoint of such a run is not the setup's own.
    ///
    /// The caller sets `stage.sim_version` (equipment mode needs 2); the stage keeps its own weapon
    /// and its seed is replaced by `setup.seed`, as `start` does. The setup is validated against
    /// `content` and the stage as `Engine::new_with` validates it; `setup.mode` is only checked,
    /// never read for a stage.
    #[cfg(feature = "bench")]
    #[doc(hidden)]
    pub fn new_equipped(
        stage: &Stage,
        content: &RunContent,
        setup: &RunSetup,
    ) -> Result<Engine, Error> {
        run_setup::validate_in(setup, content)?;
        validate::validate(stage)?;
        let kinds = KindTable::of_stage(stage).map_err(|i| Error::Invalid(vec![i]))?;
        let (stage, version, forced) = at_version(stage.clone());
        let run = Run::new(
            version,
            Build {
                stage,
                spec: None,
                horde: None,
                kinds,
                content,
                star_pool: pool(
                    setup.star_pool.as_deref(),
                    content.rewards(),
                    setup.build_mode,
                )?,
                extra_rerolls: setup.extra_rerolls,
                difficulty: Some(run_difficulty(setup, content)?),
                seed: setup.seed,
                stat_priority: &setup.stat_priority,
                pick_stats: setup.pick_stats,
                setup: Some(setup),
                party: 1,
            },
        )
        .map_err(|i| Error::Invalid(vec![i]))?;
        Ok(Engine::of(run, content, forced))
    }

    /// For tests and benches only, under the `bench` feature, before the first tick: pushes the
    /// stars `inventory` (reward IDs of the run's rewards file) onto `player`'s inventory in
    /// order, and gives each `(slot, level)` of `levels` (level 1 to `MAX_SLOT_LEVEL`) to an
    /// empty category slot or the empty Lumine slot. The level is the level the slot gives the
    /// star equipped into it (`Equip` keeps a level above 0), so it changes nothing until an
    /// equip. Only the live rules (`Run::V2`) hold equipment. Anything it cannot do is
    /// `Error::Invalid` and nothing changes: an unknown ID, an inventory past `MAX_INVENTORY`, a
    /// rune, held or out-of-range slot, a level outside its range.
    #[cfg(feature = "bench")]
    #[doc(hidden)]
    pub fn script_equipment(
        &mut self,
        player: PlayerIdx,
        inventory: &[ContentId],
        levels: &[(usize, u8)],
    ) -> Result<(), Error> {
        use crate::limits::{MAX_INVENTORY, MAX_SLOT_LEVEL};
        use crate::sim::equipment::{EquipSlot, Equipment, Held, SlotKind};

        let invalid = |path: &str, message: String| {
            Err(Error::Invalid(vec![Issue {
                path: path.into(),
                message,
            }]))
        };
        let Run::V2 { state, .. } = &mut self.run else {
            return invalid("$.run", "equipment needs the live rules".into());
        };
        if !state.players.is_present(player) {
            return invalid("$.player", "no such player in the party".into());
        }
        let rewards = self.content.rewards();
        let mut items = Vec::with_capacity(inventory.len());
        for id in inventory {
            let place = rewards.index_of(id).and_then(|p| u16::try_from(p).ok());
            let Some(place) = place else {
                return invalid(
                    "$.inventory",
                    format!("no reward `{id}` in the rewards file"),
                );
            };
            items.push(Held::Star(place));
        }
        let equipment = &mut state.players.career_mut(player).equipment;
        if equipment.inventory.len() + items.len() > MAX_INVENTORY {
            return invalid("$.inventory", format!("more than {MAX_INVENTORY} items"));
        }
        // The slot `slot` addresses: the layout, then the Lumine slot one past it.
        fn slot_of(equipment: &mut Equipment, slot: usize) -> Option<&mut EquipSlot> {
            let lumine_at = equipment.slots.len();
            match slot < lumine_at {
                true => equipment.slots.get_mut(slot),
                false => equipment.lumine.as_mut().filter(|_| slot == lumine_at),
            }
        }
        for (i, &(slot, level)) in levels.iter().enumerate() {
            let ok = slot_of(equipment, slot).is_some_and(|s| {
                s.held.is_none() && matches!(s.kind, SlotKind::Category(_) | SlotKind::Lumine)
            });
            if !ok
                || !(1..=MAX_SLOT_LEVEL).contains(&level)
                || levels[..i].iter().any(|l| l.0 == slot)
            {
                return invalid(
                    "$.levels",
                    format!(
                        "slot {slot} at level {level}: an empty category or Lumine slot at 1 to {MAX_SLOT_LEVEL}"
                    ),
                );
            }
        }
        equipment.inventory.extend(items);
        for &(slot, level) in levels {
            if let Some(target) = slot_of(equipment, slot) {
                target.level = level;
            }
        }
        Ok(())
    }

    /// For tests and benches only, under the `bench` feature, before the first tick: pushes the
    /// runes `runes` onto `player`'s inventory in order, as a drop would. A rune reaches the
    /// build by an `Equip` into a rune slot (a duplicate is refused there, not here). Only the
    /// live rules (`Run::V2`) hold equipment. Anything it cannot do is `Error::Invalid` and
    /// nothing changes: the old rules, no such player, an inventory past `MAX_INVENTORY`.
    #[cfg(feature = "bench")]
    #[doc(hidden)]
    pub fn script_runes(
        &mut self,
        player: PlayerIdx,
        runes: &[crate::rune::Rune],
    ) -> Result<(), Error> {
        use crate::limits::MAX_INVENTORY;
        use crate::sim::equipment::Held;

        let invalid = |path: &str, message: String| {
            Err(Error::Invalid(vec![Issue {
                path: path.into(),
                message,
            }]))
        };
        let Run::V2 { state, .. } = &mut self.run else {
            return invalid("$.run", "equipment needs the live rules".into());
        };
        if !state.players.is_present(player) {
            return invalid("$.player", "no such player in the party".into());
        }
        let equipment = &mut state.players.career_mut(player).equipment;
        if equipment.inventory.len() + runes.len() > MAX_INVENTORY {
            return invalid("$.inventory", format!("more than {MAX_INVENTORY} items"));
        }
        equipment
            .inventory
            .extend(runes.iter().map(|&r| Held::Rune(r)));
        Ok(())
    }

    /// For tests and benches only, under the `bench` feature, before the first tick: adds
    /// `points` to `player`'s weapon expertise gauge, as a kill's points would. The level rises
    /// on the next tick, so its base set and tree points arise by the rules' own path. A
    /// savepoint or replay of such a run is not the setup's own (as `new_equipped`). Only the
    /// live rules (`Run::V2`) hold equipment. Anything it cannot do is `Error::Invalid` and
    /// nothing changes: the old rules, no such player, a stage that gives no expertise points.
    #[cfg(feature = "bench")]
    #[doc(hidden)]
    pub fn script_expertise(&mut self, player: PlayerIdx, points: u32) -> Result<(), Error> {
        let invalid = |path: &str, message: String| {
            Err(Error::Invalid(vec![Issue {
                path: path.into(),
                message,
            }]))
        };
        let Run::V2 { state, .. } = &mut self.run else {
            return invalid("$.run", "equipment needs the live rules".into());
        };
        if !state.players.is_present(player) {
            return invalid("$.player", "no such player in the party".into());
        }
        let Some(gauge) = state.players.career_mut(player).expertise.as_mut() else {
            return invalid("$.expertise", "the stage gives no expertise points".into());
        };
        gauge.points = gauge.points.saturating_add(points);
        Ok(())
    }

    /// Starts a run of the `time_limit` mode from `mode` in place of the bundled file: its
    /// waves, and its bosses as they come (`TICK`'s first step).
    pub fn start_time_limit(
        setup: &RunSetup,
        content: &RunContent,
        mode: &crate::time_limit::TimeLimit,
    ) -> Result<Engine, Error> {
        Engine::start_time_limit_party(setup, content, mode, 1)
    }

    fn start_time_limit_party(
        setup: &RunSetup,
        content: &RunContent,
        mode: &crate::time_limit::TimeLimit,
        party: u8,
    ) -> Result<Engine, Error> {
        run_setup::validate_in(setup, content)?;
        let difficulty = run_setup::difficulty_of(setup, content.difficulties())?;
        let (stage, spec, horde) =
            run_setup::time_limit_run(setup, mode, difficulty, content.trial())?;
        let kinds = horde_kinds(&stage, &spec).map_err(|i| Error::Invalid(vec![i]))?;
        let (stage, version, forced) = at_version(stage);
        let run = Run::new(
            version,
            Build {
                stage,
                spec: Some(spec),
                horde: Some(horde),
                kinds,
                content,
                star_pool: pool(
                    setup.star_pool.as_deref(),
                    content.rewards(),
                    setup.build_mode,
                )?,
                extra_rerolls: setup.extra_rerolls,
                difficulty: Some(run_difficulty(setup, content)?),
                seed: setup.seed,
                stat_priority: &setup.stat_priority,
                pick_stats: setup.pick_stats,
                setup: Some(setup),
                party,
            },
        )
        .map_err(|i| Error::Invalid(vec![i]))?;
        Ok(Engine::of(run, content, forced))
    }

    /// Advances one tick with `inputs`, one entry per slot, present or not, indexed by
    /// `PlayerIdx`. The slots read are the present ones; an absent slot's entry, its `pick`
    /// included, is ignored. Once the run is over this does nothing, so a replay that is longer
    /// than the run cannot change the final hash.
    ///
    /// While an offer is open for any player the world is stopped (ADR-REPL-5): each player, in
    /// `PlayerIdx` order, whose `pick` is accepted by `check_pick` takes that option of its own
    /// offer, the events are the picks' and the tick does not advance; a player with no offer
    /// open, or a refused pick, is ignored, and with no acceptable pick nothing changes. With no
    /// offer open `pick` is not read (ADR-REPL-5). `action` is never read: an equipment
    /// action is made between steps, by `act` or a recorded ACTIONS step (`check_action`).
    pub fn step(&mut self, inputs: &[Input; crate::id::MAX_PLAYERS as usize]) {
        self.events.clear();
        self.load = Load::default();
        if self.offer_open() {
            self.take_picks(inputs);
            return;
        }
        if self.runs_its_version() {
            self.run.step(
                self.content.rewards(),
                self.content.expertise(),
                inputs,
                &mut self.events,
                &mut self.load,
            );
        } else {
            // `new` validated the version; fail closed if that ever stops being true.
            self.run.fail();
        }
        self.observe_tick();
    }

    /// Takes, for each present player in `PlayerIdx` order, the pick of its input that
    /// `check_pick` accepts (each read after the picks before it were taken); the rest are
    /// ignored.
    fn take_picks(&mut self, inputs: &[Input; crate::id::MAX_PLAYERS as usize]) {
        for (i, input) in inputs.iter().enumerate() {
            let Some(player) = u8::try_from(i).ok().and_then(PlayerIdx::new) else {
                continue;
            };
            if let Some(option) = input.pick.and_then(|k| self.pick_option(player, k).ok()) {
                self.take(player, option);
            }
        }
    }

    /// Takes only the picks of `inputs`, never ticking the world: a recorded PICKS step
    /// (ADR-REPL-5), the same in both modes. `action` is not read.
    pub(crate) fn pick_step(&mut self, inputs: &[Input; crate::id::MAX_PLAYERS as usize]) {
        self.events.clear();
        self.load = Load::default();
        self.take_picks(inputs);
    }

    /// The players in the run's party: `1..=MAX_PLAYERS` (ADR-SIM-7).
    pub fn players(&self) -> usize {
        self.run.party()
    }

    /// The slots in the party, bit `i` for slot `i`: `0b001` for a run of `sim_version` 1.
    /// Slot 0 is always set; a hole such as `0b101` arises when slot 1 leaves.
    pub fn present(&self) -> u8 {
        self.run.present()
    }

    /// Slot `slot` joins the party at the tick the engine is on (ADR-SIM-7, ADR-SIM-8). The
    /// joiner is the run's starting player at slot 0's current place, facing as it faces, at
    /// level 1 with no offer open; its totals start at zero. No event is emitted. Called between
    /// steps, like `choose`; an open offer does not block it. A join at tick 0 equals the party
    /// start of the same mask. Refused, changing nothing: a slot in the party, a run that is
    /// over, a `sim_version` 1 run.
    pub fn join(&mut self, slot: PlayerIdx) -> Result<(), PresenceError> {
        self.check_presence()?;
        if self.present() >> slot.get() & 1 == 1 {
            return Err(PresenceError::Occupied { slot });
        }
        let template = self.joiner.take().ok_or(PresenceError::SoloRules)?;
        self.run.join(slot, &template);
        self.joiner = Some(template);
        self.player_stats.reset_slot(slot);
        self.events.clear();
        Ok(())
    }

    /// Slot `slot` leaves the party at the tick the engine is on. Slot 0 never leaves. What the
    /// leaver left in flight stays and resolves through the slots present; the leaver's totals
    /// stay in `player_stats` until the slot is joined again. No event is emitted. An open offer
    /// of the leaver closes with it. Refused, changing nothing: slot 0, a slot not in the
    /// party, a slot that has already left 255 times, a run that is over, a `sim_version` 1 run.
    pub fn leave(&mut self, slot: PlayerIdx) -> Result<(), PresenceError> {
        self.check_presence()?;
        if slot == PlayerIdx::SOLO {
            return Err(PresenceError::FirstSlot);
        }
        if self.present() >> slot.get() & 1 == 0 {
            return Err(PresenceError::Absent { slot });
        }
        if self.run.generation(slot) == u8::MAX {
            return Err(PresenceError::Exhausted { slot });
        }
        self.run.leave(slot);
        self.events.clear();
        Ok(())
    }

    fn check_presence(&self) -> Result<(), PresenceError> {
        if self.outcome() != Outcome::Running {
            return Err(PresenceError::Ended);
        }
        if self.joiner.is_none() {
            return Err(PresenceError::SoloRules);
        }
        Ok(())
    }

    /// A fresh party of 3 with the slots outside `mask` cleared: the engine a savepoint of that
    /// mask restores over. `mask` holds slot 0.
    pub(crate) fn start_party_mask(
        setup: &RunSetup,
        content: &RunContent,
        mask: u8,
    ) -> Result<Engine, Error> {
        // A version 1 run has no other party than one; it starts as one.
        let party = if mask == 0b001 {
            1
        } else {
            crate::id::MAX_PLAYERS
        };
        let mut engine = Engine::start_party_with(setup, content, party)?;
        for i in 1..crate::id::MAX_PLAYERS {
            if mask >> i & 1 == 0 && party > 1 {
                if let Some(slot) = PlayerIdx::new(i) {
                    engine.run.leave(slot);
                }
            }
        }
        Ok(engine)
    }

    /// Whether `player` could take option `index` of its open offer by a pick in `step`
    /// (ADR-REPL-5). An open offer belongs to the slot that holds it, and the snapshot shows the
    /// first player's. Refused for a player outside the party, a player with no offer open, a
    /// `sim_version` this game does not run, or an index outside its options. Read-only.
    pub fn check_pick(&self, player: PlayerIdx, index: u32) -> Result<(), ChoiceError> {
        self.pick_option(player, index).map(|_| ())
    }

    /// Whether `player` could make the equipment action `action` in a step (ADR-SIM-7).
    /// Read-only. `NoPlayer` for a slot outside the party; then, in an equipment run, `Downed`
    /// for a player at 0 hit points and `Out` for one out of the run, for every kind of action
    /// (the window opens again on the revive). `Equip` and `Replace` are the
    /// equipment mode's: the refusals of `slots::check`, and `NotBuilt` in a run that is not in
    /// equipment mode or is `sim_version` 1. `Extract` and `Craft` are the same: the refusals
    /// of `starlight` (an item of the inventory; a star of the setup's pool, its price, a held
    /// named star, the inventory bound). `TreePick` is the same: `NoTreePoint` when no point is
    /// held, then `NoTreeOption` when `option` is past the next pick's choices. The match has
    /// one arm per variant so a new rule edits one line.
    pub fn check_action(&self, player: PlayerIdx, action: EquipAction) -> Result<(), ActionError> {
        if self.present() >> player.get() & 1 == 0 {
            return Err(ActionError::NoPlayer { player });
        }
        if let Some(locked) = self.run.window_lock(player) {
            return Err(locked);
        }
        let equip = |item: u32, slot: u32, replace: bool| {
            self.run
                .check_equip(self.content.rewards(), player, item, slot, replace)
                .ok_or(ActionError::NotBuilt)?
                .map_err(ActionError::from)
        };
        match action {
            EquipAction::Equip { item, slot } => equip(item, slot, false),
            EquipAction::Replace { item, slot } => equip(item, slot, true),
            EquipAction::Extract { item } => self
                .run
                .check_extract(player, item)
                .ok_or(ActionError::NotBuilt)?
                .map_err(ActionError::from),
            EquipAction::Craft { star } => self
                .run
                .check_craft(self.content.rewards(), player, star)
                .ok_or(ActionError::NotBuilt)?
                .map_err(ActionError::from),
            EquipAction::TreePick { option } => self
                .run
                .check_tree_pick(self.content.expertise(), player, option)
                .ok_or(ActionError::NotBuilt)?
                .map_err(ActionError::from),
        }
    }

    /// The step of actions: each present slot's action, in slot order, is made as `act` makes
    /// it; a refused one is ignored. No tick runs (a recorded ACTIONS step, like a PICKS step).
    pub(crate) fn action_step(
        &mut self,
        actions: &[Option<EquipAction>; crate::id::MAX_PLAYERS as usize],
    ) {
        self.events.clear();
        self.load = Load::default();
        for (slot, action) in actions.iter().enumerate() {
            let player = u8::try_from(slot).ok().and_then(PlayerIdx::new);
            if let (Some(player), Some(action)) = (player, action) {
                let _ = self.act(player, *action);
            }
        }
    }

    /// Makes `action` for `player` now, between two steps: `check_action`, then the change. A
    /// refused action changes nothing and returns what `check_action` says (ADR-SIM-7). No
    /// event is emitted and nothing is drawn.
    pub fn act(&mut self, player: PlayerIdx, action: EquipAction) -> Result<(), ActionError> {
        self.check_action(player, action)?;
        match action {
            EquipAction::Equip { item, slot } => {
                self.run
                    .equip(self.content.rewards(), player, item, slot, false)
            }
            EquipAction::Replace { item, slot } => {
                self.run
                    .equip(self.content.rewards(), player, item, slot, true)
            }
            EquipAction::Extract { item } => self.run.extract(self.content.rewards(), player, item),
            EquipAction::Craft { star } => self.run.craft(self.content.rewards(), player, star),
            EquipAction::TreePick { option } => {
                self.run.tree_pick(self.content.expertise(), player, option)
            }
        }
        Ok(())
    }

    /// Rates each candidate of `player`'s open star or boss offer, in the offer's order: its
    /// categories, the expected gain in the Main weapon's damage in basis points, the set tier
    /// the pick reaches and the stats it moves, before and after. Empty for a player outside
    /// the party, without such an offer open, with an expertise or stat offer open, or in a
    /// `sim_version` 1 run. Read-only, and a separate query from the snapshot (ADR-BRDG-4):
    /// call it when an offer opens, not every frame. The model and its limits are in
    /// `sim::offer_rating`.
    pub fn offer_ratings(&self, player: PlayerIdx) -> Vec<OfferRating> {
        if self.present() >> player.get() & 1 == 0 {
            return Vec::new();
        }
        self.run.offer_ratings(self.content.rewards(), player)
    }

    /// Rates a star against each slot it could go in, in slot order: for each, whether the swap
    /// is an equip, a replace or a same-option switch, the slot's level before and after, the
    /// expected signed change in the Main weapon's damage in basis points (the model of
    /// `offer_ratings`), the set tiers gained or lost and the stats it moves. The swap is made
    /// by the equip and replace rules on a clone, so the answer is what `act` would do. Empty
    /// for a player outside the party, a `sim_version` 1 run, a run without slots, a rune, an
    /// item or star that does not exist, or when no slot takes the star. Read-only, and a
    /// separate query from the snapshot (ADR-BRDG-4): call it when a window opens or an item
    /// arrives, not every frame; each call clones the player's equipment once per slot.
    pub fn swap_ratings(&self, player: PlayerIdx, item: SwapItem) -> Vec<SwapRating> {
        if self.present() >> player.get() & 1 == 0 {
            return Vec::new();
        }
        self.run.swap_ratings(self.content.rewards(), player, item)
    }

    fn pick_option(&self, player: PlayerIdx, index: u32) -> Result<OfferOption, ChoiceError> {
        if self.present() >> player.get() & 1 == 0 {
            return Err(ChoiceError::NoPlayer { player });
        }
        if !self.run.offer_open_for(player) {
            return Err(ChoiceError::NoOffer);
        }
        let sim_version = self.run.declared_version();
        if !self.runs_its_version() {
            // `new` validated the version; refuse, changing nothing and running no rules, if
            // that stops being true.
            return Err(ChoiceError::Unsupported { sim_version });
        }
        let options =
            self.run
                .offer_options(self.content.rewards(), self.content.expertise(), player);
        usize::try_from(index)
            .ok()
            .and_then(|i| options.get(i))
            .copied()
            .ok_or(ChoiceError::OutOfRange {
                index,
                options: u32::try_from(options.len()).unwrap_or(u32::MAX),
            })
    }

    /// Takes `option` of `player`'s open offer, already accepted by `pick_option`.
    fn take(&mut self, player: PlayerIdx, option: OfferOption) {
        self.run.choose(
            self.content.rewards(),
            self.content.expertise(),
            player,
            option,
            &mut self.events,
        );
        self.observe_choice();
    }

    /// The run's stage names a version this game runs, and it is the version of the rules the
    /// run holds. A forced engine (tests) runs the version it was built for.
    fn runs_its_version(&self) -> bool {
        let declared = self.run.declared_version();
        #[cfg(test)]
        let released = self.forced || (1..=SIM_VERSION).contains(&declared);
        #[cfg(not(test))]
        let released = (1..=SIM_VERSION).contains(&declared);
        released && declared == self.run.version()
    }

    fn observe_tick(&mut self) {
        self.run.observe_tick(&mut self.ledger, &self.events);
        self.counted
            .observe(self.content.rewards(), &self.events, &mut self.player_stats);
    }

    fn observe_choice(&mut self) {
        self.run.observe_choice(&mut self.ledger, &self.events);
        self.counted
            .observe(self.content.rewards(), &self.events, &mut self.player_stats);
    }

    /// Damage dealt, kills and support per player slot, folded from the events of every step
    /// (ADR-SIM-7). Not hashed, but in a savepoint (ADR-SAVE-4), so a resumed run's totals are
    /// those of the whole run, as a replay rebuilds them. A run of `sim_version` 1 stays zero.
    pub fn player_stats(&self) -> &PlayerStats {
        &self.player_stats
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
        self.run.kinds()
    }

    pub fn tick(&self) -> u32 {
        self.run.tick()
    }

    /// The Stardust the run has earned, times 10; 0 in a stage without a factor.
    pub fn stardust(&self) -> u32 {
        self.run.stardust()
    }

    /// The Stardust the player in `player` has earned, times 10 (ADR-SAVE-6 point 2): `None` for
    /// a slot that is not present. A version 1 run answers slot 0 with `stardust` and `None` for
    /// the others.
    pub fn stardust_of(&self, player: PlayerIdx) -> Option<u32> {
        self.run.stardust_of(player)
    }

    pub fn outcome(&self) -> Outcome {
        self.run.outcome()
    }

    /// An offer is open: `step` does nothing until `choose` closes it (ADR-REPL-5).
    pub fn offer_open(&self) -> bool {
        self.run.offer_open()
    }

    /// FNV-1a 64 over the whole state after the last tick (ADR-SIM-3).
    pub fn state_hash(&self) -> u64 {
        self.run.state_hash()
    }

    /// The domain events of the last tick, in the order they happened. A version 1 run's are the
    /// frozen rules' events adapted to the live type, with the player fields `None`.
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
        self.run.caps()
    }

    /// How much of each dynamic budget the last tick used (ADR-SVC-1).
    pub fn load(&self) -> Load {
        self.load
    }
}

/// What the setup's difficulty brings to the run: its percents, and its rarity range
/// clamped to the rarities the run's rewards have.
fn run_difficulty(setup: &RunSetup, content: &RunContent) -> Result<Difficulty, Error> {
    let d = run_setup::difficulty_of(setup, content.difficulties())?;
    Ok(Difficulty {
        hp_pct: d.hp_pct,
        damage_pct: d.damage_pct,
        rarity: d.rarity_range(content.rewards()),
    })
}

/// The run's star pool: the places in `rewards` of the rewards `ids` names, or the mode's
/// default pool when it names none (`Rewards::default_pool`). A name the file does not hold fails
/// closed, one issue per name.
fn pool(ids: Option<&[ContentId]>, rewards: &Rewards, mode: BuildMode) -> Result<Vec<u16>, Error> {
    let place = |i: usize| u16::try_from(i).unwrap_or(u16::MAX);
    let Some(ids) = ids else {
        return Ok(rewards.default_pool(mode));
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

mod run;
mod snapshot;
mod v1_events;
pub(crate) use run::{Build, Difficulty, Run, Saved};
pub use snapshot::{Place, Places, Tally};

/// Takes option `index` of the open offer for `player` through a step, as a test plays an
/// offer: refused as `check_pick` refuses it, changing nothing.
#[cfg(test)]
pub(crate) fn pick(e: &mut Engine, player: PlayerIdx, index: u32) -> Result<(), ChoiceError> {
    e.check_pick(player, index)?;
    let mut inputs = [Input::default(); crate::id::MAX_PLAYERS as usize];
    inputs[player.get()] = Input::picking(index);
    e.step(&inputs);
    Ok(())
}

#[cfg(test)]
mod action_tests;
#[cfg(test)]
mod codec_tests;
#[cfg(test)]
mod copy_tests;
#[cfg(test)]
mod dispatch_tests;
#[cfg(test)]
mod hash_coverage_tests;
#[cfg(test)]
mod horde_tests;
#[cfg(test)]
mod move_index_tests;
#[cfg(test)]
mod party_tests;
#[cfg(test)]
mod pick_tests;
#[cfg(test)]
mod players_api_tests;
#[cfg(test)]
mod presence_tests;
#[cfg(test)]
mod savepoint_tests;
#[cfg(test)]
mod snapshot_pin_tests;
#[cfg(test)]
mod v1_events_tests;

// Every `State` type lives in its rules module's `state`; the tests name the frozen rules'
// (`sim_v1::state::*`), which they run, and the live module's (`sim::state::*`) when a version 2
// differs. The live module names its own.
#[cfg(test)]
pub(crate) use crate::sim_v1::state::*;
// The savepoint and the snapshot name no state type here; only the tests do.

/// The stage with the version the engine builds its rules for: the stage's own, or the one
/// `Engine::forced` asks for in a test; and whether it was forced.
fn at_version(#[allow(unused_mut)] mut stage: Stage) -> (Stage, u32, bool) {
    #[cfg(test)]
    if let Some(version) = FORCED.with(std::cell::Cell::get) {
        stage.sim_version = version;
        return (stage, version, true);
    }
    let version = stage.sim_version;
    (stage, version, false)
}

#[cfg(test)]
thread_local! {
    /// The version `Engine::forced` builds engines for on this thread, past the `SIM_VERSION`
    /// check: a version 2 engine before the game releases one.
    static FORCED: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
impl Engine {
    /// What `make` builds, with every engine it makes through `Engine::new*` and `start*` run
    /// under the rules of `version`, whatever the stage says: for tests of a version the game does
    /// not release yet.
    pub(crate) fn forced<T>(version: u32, make: impl FnOnce() -> T) -> T {
        /// Puts the previous setting back, also when `make` panics.
        struct Restore(Option<u32>);
        impl Drop for Restore {
            fn drop(&mut self) {
                FORCED.with(|f| f.set(self.0));
            }
        }
        let _restore = Restore(FORCED.with(|f| f.replace(Some(version))));
        make()
    }

    /// An engine for `stage` under the rules of version 2 with `n` players (1 to 3): each a copy
    /// of the first player, with a career of its own from the first player's setup and its own
    /// offer generator. Slots 1 and 2 do not move or attack by `step` yet.
    ///
    /// # Panics
    ///
    /// If `n` is not in `1..=3`, or the stage does not run.
    pub(crate) fn party(stage: &Stage, n: u8) -> Engine {
        Engine::new_forced(stage, 2)
            .expect("a stage that runs")
            .into_party(n)
    }

    /// This engine, which runs the live rules, as a party of `n`; see `party`.
    pub(crate) fn into_party(mut self, n: u8) -> Engine {
        use crate::sim::state::Players;
        assert!((1..=crate::id::MAX_PLAYERS).contains(&n), "1 to 3 players");
        if n == 1 {
            return self;
        }
        let Run::V2 { state, .. } = &mut self.run else {
            panic!("the live rules");
        };
        state.players = Players::party(n, &state.players, state.crit_seed);
        self.run.reserve();
        self
    }

    /// The run, to edit its state (tests).
    pub(crate) fn run_mut(&mut self) -> &mut Run {
        &mut self.run
    }

    /// Opens what is due to every present player, as the tick's last step does (tests).
    pub(crate) fn open_offers_for_test(&mut self) {
        let (rewards, expertise) = (self.content.rewards(), self.content.expertise());
        let Run::V2 { world, state } = &mut self.run else {
            panic!("the live rules");
        };
        crate::sim::open_offers_for_test(world, rewards, expertise, state, &mut self.events);
    }

    /// An engine for `stage` under the rules of `version`, past the `SIM_VERSION` check.
    pub(crate) fn new_forced(stage: &Stage, version: u32) -> Result<Engine, Error> {
        Engine::forced(version, || Engine::new(stage))
    }
}
