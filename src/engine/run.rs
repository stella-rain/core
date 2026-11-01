//! The engine's run, one variant per released rules version (ADR-REPL-9 point 1, ADR-REPL-7
//! point 4): each version owns its `World`, its `State` and its snapshot view. Every call into
//! the rules goes through a `match` here, so adding a version lists each site as a compile
//! error. No box, no trait object: a one-variant enum is its `World` and `State`, with no tag.

use crate::event::DomainEvent;
use crate::expertise::Expertise;
use crate::input::Input;
use crate::rewards::Rewards;
use crate::run_content::RunContent;
use crate::sim::{self, World};
use crate::snapshot::{OfferOption, Snapshot};

use super::{Load, Places, State};

/// A run under the rules of one `sim_version`: the compiled stage and the state it changes, both
/// of that version's types. The engine reaches them, and the snapshot view, only through here.
#[derive(Debug, Clone)]
pub(crate) enum Run {
    /// Version 1, the frozen first rules.
    V1 { world: World, state: State },
}

impl Run {
    /// The `sim_version` whose rules this state belongs to.
    pub(crate) fn version(&self) -> u32 {
        match self {
            Run::V1 { .. } => 1,
        }
    }

    /// The `sim_version` the run's stage says: what `Engine` checks against the released
    /// versions before it runs a rule.
    pub(crate) fn declared_version(&self) -> u32 {
        match self {
            Run::V1 { world, .. } => world.sim_version,
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
                sim::choose(version, world, rewards, expertise, state, option, events);
            }
        }
    }

    pub(crate) fn offer_open(&self) -> bool {
        match self {
            Run::V1 { state, .. } => sim::offer_open(state),
        }
    }

    pub(crate) fn offer_options(
        &self,
        rewards: &Rewards,
        expertise: &Expertise,
    ) -> Vec<OfferOption> {
        match self {
            Run::V1 { world, state } => sim::offer_options(world, rewards, expertise, state),
        }
    }

    pub(crate) fn state_hash(&self) -> u64 {
        match self {
            Run::V1 { state, .. } => sim::state_hash(state),
        }
    }

    /// What the frontend draws after the last tick, built by this version's rules.
    pub(crate) fn snapshot(&self, content: &RunContent) -> Snapshot {
        match self {
            Run::V1 { world, state } => sim::snapshot::build(world, content, state),
        }
    }

    /// Fills `out` with where the drawn things are, by this version's rules.
    pub(crate) fn places(&self, out: &mut Places) {
        match self {
            Run::V1 { state, .. } => sim::snapshot::places(state, out),
        }
    }

    /// Reserves the room a run holds from load, so a clone's first ticks do not grow it (DIR1).
    pub(crate) fn reserve(&mut self) {
        match self {
            Run::V1 { world, state } => sim::reserve(world, state),
        }
    }
}

/// Tests read and edit the one variant's fields directly; the rules never go through this.
#[cfg(test)]
impl std::ops::Deref for Run {
    type Target = State;
    fn deref(&self) -> &State {
        let Run::V1 { state, .. } = self;
        state
    }
}

#[cfg(test)]
impl std::ops::DerefMut for Run {
    fn deref_mut(&mut self) -> &mut State {
        let Run::V1 { state, .. } = self;
        state
    }
}

#[cfg(test)]
impl Run {
    pub(crate) fn world(&self) -> &World {
        let Run::V1 { world, .. } = self;
        world
    }

    pub(crate) fn world_mut(&mut self) -> &mut World {
        let Run::V1 { world, .. } = self;
        world
    }

    pub(crate) fn set_state(&mut self, new: State) {
        let Run::V1 { state, .. } = self;
        *state = new;
    }
}

/// The version's types, by one path for the tests; a new version repoints this in one place.
#[cfg(test)]
pub(crate) mod v1 {
    pub(crate) use crate::sim::World;
    pub(crate) use crate::sim::state::*;
}
