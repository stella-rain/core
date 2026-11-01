//! The engine's run state, one variant per released rules version (ADR-REPL-9 point 1,
//! ADR-REPL-7 point 4): each version owns its state types. Every call into the rules goes
//! through a `match` here, so adding a version lists each site as a compile error. No box, no
//! trait object: a one-variant enum is its `State`, with no tag.

use crate::event::DomainEvent;
use crate::expertise::Expertise;
use crate::input::Input;
use crate::rewards::Rewards;
use crate::sim::{self, World};
use crate::snapshot::OfferOption;

use super::{Load, State};

/// The state of a run under the rules of one `sim_version`.
#[derive(Debug, Clone)]
pub(crate) enum Run {
    /// Version 1, the frozen first rules.
    V1(State),
}

impl Run {
    /// The `sim_version` whose rules this state belongs to.
    pub(crate) fn version(&self) -> u32 {
        match self {
            Run::V1(_) => 1,
        }
    }

    pub(crate) fn step(
        &mut self,
        stage: &World,
        rewards: &Rewards,
        expertise: &Expertise,
        input: Input,
        events: &mut Vec<DomainEvent>,
        load: &mut Load,
    ) {
        let version = self.version();
        match self {
            Run::V1(s) => sim::step(version, stage, rewards, expertise, s, input, events, load),
        }
    }

    pub(crate) fn choose(
        &mut self,
        stage: &World,
        rewards: &Rewards,
        expertise: &Expertise,
        option: OfferOption,
        events: &mut Vec<DomainEvent>,
    ) {
        let version = self.version();
        match self {
            Run::V1(s) => sim::choose(version, stage, rewards, expertise, s, option, events),
        }
    }

    pub(crate) fn offer_open(&self) -> bool {
        match self {
            Run::V1(s) => sim::offer_open(s),
        }
    }

    pub(crate) fn offer_options(
        &self,
        stage: &World,
        rewards: &Rewards,
        expertise: &Expertise,
    ) -> Vec<OfferOption> {
        match self {
            Run::V1(s) => sim::offer_options(stage, rewards, expertise, s),
        }
    }

    pub(crate) fn state_hash(&self) -> u64 {
        match self {
            Run::V1(s) => sim::state_hash(s),
        }
    }

    /// Reserves the room a run holds from load, so a clone's first ticks do not grow it (DIR1).
    pub(crate) fn reserve(&mut self, stage: &World) {
        match self {
            Run::V1(s) => sim::reserve(stage, s),
        }
    }
}

/// Tests read and edit the one variant's fields directly; the rules never go through this.
#[cfg(test)]
impl std::ops::Deref for Run {
    type Target = State;
    fn deref(&self) -> &State {
        let Run::V1(s) = self;
        s
    }
}

#[cfg(test)]
impl std::ops::DerefMut for Run {
    fn deref_mut(&mut self) -> &mut State {
        let Run::V1(s) = self;
        s
    }
}
