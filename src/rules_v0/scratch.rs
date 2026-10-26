//! The buffers a tick fills and empties again: they keep their memory from tick to tick, so a
//! steady tick allocates nothing (T1b). They hold nothing between ticks that the rules read, so
//! they are neither hashed nor cloned: a cloned state starts with empty ones.

use super::grid::Grid;
use super::hit_boxes::Swing;
use super::push::Body;
use crate::id::EntityId;

#[derive(Default)]
pub(crate) struct Scratch {
    /// The enemy grid of the push-apart or of the tick's collisions.
    pub(super) grid: Grid,
    /// What the push-apart knows of each enemy, and the move it works out for it.
    pub(super) bodies: Vec<Body>,
    pub(super) offsets: Vec<(i32, i32, bool)>,
    /// The hit boxes and zones active this tick, and the enemies one of them covers.
    pub(super) swings: Vec<Swing>,
    pub(super) found: Vec<usize>,
    /// One bit per enemy, for the grid to collect and order the enemies one swing reaches.
    pub(super) found_bits: Vec<u64>,
    /// Emptied hit lists of ended attack runs (`AttackRun::hits`), for new runs to take.
    pub(super) hit_lists: Vec<Vec<(EntityId, u32)>>,
    /// Whether each orb of the tick was used up.
    pub(super) consumed: Vec<bool>,
    /// The enemies granted a ranged token this tick, by `RangedPool` (RA1): worked out from the
    /// state at the start of the act step.
    pub(super) ranged_grants: [Vec<EntityId>; 2],
    /// Spawns the caps refused so far in the tick; `step` hands it to `Load` and empties it.
    pub(super) refused_spawns: u32,
}

impl Clone for Scratch {
    fn clone(&self) -> Scratch {
        Scratch::default()
    }
}

impl std::fmt::Debug for Scratch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scratch").finish_non_exhaustive()
    }
}

impl Scratch {
    /// Keeps the memory of an ended run's hit list for a later run.
    pub(super) fn recycle_hits(
        pool: &mut Vec<Vec<(EntityId, u32)>>,
        mut hits: Vec<(EntityId, u32)>,
    ) {
        if hits.capacity() > 0 {
            hits.clear();
            pool.push(hits);
        }
    }

    /// An emptied hit list for a new attack run: the roomiest of the ones waiting, so that the
    /// lists settle at the sizes the runs need and stop growing.
    pub(super) fn take_hit_list(&mut self) -> Vec<(EntityId, u32)> {
        let roomiest = (0..self.hit_lists.len()).max_by_key(|&i| self.hit_lists[i].capacity());
        roomiest.map_or_else(Vec::new, |i| self.hit_lists.swap_remove(i))
    }
}
