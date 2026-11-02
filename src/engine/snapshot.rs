//! The snapshot builder: what the frontend draws after the last tick (ADR-005).

use crate::fixed::Point;
use crate::id::{ContentId, EntityId};
use crate::ledger::Counted;
use crate::rewards::Rewards;
use crate::snapshot::{EntityKind, Snapshot};
use std::sync::Arc;

use super::Engine;

/// A run's HUD tallies after the last tick or choice: what the events of the run said, kept by the engine
/// outside the hashed state. A resumed run has the tally of the same run played straight.
#[derive(Clone, Copy, Debug)]
pub struct Tally<'a> {
    counted: &'a Counted,
    rewards: &'a Rewards,
}

impl<'a> Tally<'a> {
    /// Enemies defeated: every `Died` whose class is `Enemy` (elites included; boss, parts, companions
    /// and shrines are not), counting a mimic that dies in disguise. Saturates at `u32::MAX`.
    pub fn kills(&self) -> u32 {
        self.counted.kills
    }

    /// Each reward taken at least once, with the count its last `RewardTaken` carried, in the rewards file's order.
    pub fn taken(&self) -> impl Iterator<Item = (&'a ContentId, u8)> + 'a {
        let (rewards, counted): (&'a Rewards, &'a Counted) = (self.rewards, self.counted);
        rewards
            .rewards()
            .iter()
            .zip(&counted.taken)
            .filter(|(_, n)| **n > 0)
            .map(|(r, n)| (&r.id, *n))
    }

    /// The star names of the `RewardTaken` events, in event order, repeats kept.
    pub fn named(&self) -> &'a [Arc<str>] {
        let counted: &'a Counted = self.counted;
        &counted.named
    }

    /// Each constellation that announced a tier, with the last tier its `Constellation` event carried, in the pack's order.
    pub fn tiers(&self) -> impl Iterator<Item = (&'a ContentId, u8)> + 'a {
        let (rewards, counted): (&'a Rewards, &'a Counted) = (self.rewards, self.counted);
        rewards
            .constellations()
            .sets()
            .iter()
            .zip(&counted.tiers)
            .filter(|(_, n)| **n > 0)
            .map(|(c, n)| (&c.id, *n))
    }
}

/// One drawn entity's id, kind and position (the fields of `EntityView` a frame needs to place an event).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub id: EntityId,
    pub kind: EntityKind,
    pub at: Point,
}

/// Where the drawn things are after the last tick; filled by `Engine::places`, kept and reused by the caller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Places {
    /// As `Snapshot::entities`: the same entities, in the same order.
    pub entities: Vec<Place>,
    /// As `Snapshot::stars`, without the rarity.
    pub stars: Vec<(EntityId, Point)>,
    /// The boss on the field: the id of the `Boss` entity, if any.
    pub boss: Option<EntityId>,
    /// As `Snapshot::players`, the position of each; a player always exists, alive or dead.
    pub players: Vec<Point>,
}

impl Engine {
    /// The run's HUD tallies: kills, rewards taken, named stars and constellation tiers, as the
    /// run's events said them. Read only: it cannot change the state, the hash or the events.
    pub fn tally(&self) -> Tally<'_> {
        Tally {
            counted: &self.counted,
            rewards: self.content.rewards(),
        }
    }

    /// Clears `out` and fills it for the state after the last tick: what `snapshot()` shows for
    /// these fields (and the players' positions), without building a snapshot. Allocates only
    /// while `out` grows to the largest crowd it has held. The run's rules module fills it
    /// (`sim::snapshot`), keeping the visibility, position and kind rules of `snapshot()`;
    /// `tests/places.rs` holds the two equal.
    pub fn places(&self, out: &mut Places) {
        self.run.places(out);
    }

    /// What the frontend draws after the last tick (ADR-005), built by the run's rules module
    /// (`sim::snapshot`). Its `places` repeats the rules for the ids, kinds and positions, and
    /// the players' positions: change them together.
    pub fn snapshot(&self) -> Snapshot {
        self.run.snapshot(&self.content)
    }
}
