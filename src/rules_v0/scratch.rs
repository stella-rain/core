//! The buffers a tick fills and empties again: they keep their memory from tick to tick, so a
//! steady tick allocates nothing (T1b). They hold nothing between ticks that the rules read, so
//! they are neither hashed nor cloned: a cloned state starts with empty ones.

use super::grid::Grid;
use super::hit_boxes::Swing;
use super::push::Body;
use crate::id::EntityId;

/// Makes room in `v` for `capacity` items in all, whatever it holds now: a vector is reserved
/// once, at load, to the most a run can put in it (DIR1, ADR-SIM-7 decision 9).
pub(super) fn reserve_to<T>(v: &mut Vec<T>, capacity: usize) {
    v.reserve(capacity.saturating_sub(v.len()));
}

/// What a `Scratch` was reserved for, so that a clone reserves the same.
#[derive(Clone, Copy, Debug)]
struct Reserved {
    enemies: usize,
    width: i32,
    height: i32,
}

/// The attack runs that can end in one tick, each leaving its hit list behind: held to
/// `MAX_ATTACK_TASKS` lists, which is also the most that can run at once.
const HIT_LISTS: usize = super::MAX_ATTACK_TASKS as usize;

/// Room a hit list keeps beyond the enemies: the boss, its parts, the companions and the main
/// character can be hit as well (a placeholder bound; `tests/alloc.rs` checks it).
const HIT_LIST_SPARE: usize = 16;

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
    /// The shrine elites alive at the start of the shrines step, by id.
    pub(super) shrine_ids: Vec<EntityId>,
    /// Spawns the caps refused so far in the tick; `step` hands it to `Load` and empties it.
    pub(super) refused_spawns: u32,
    /// Wave spawns the director held so far in the tick; `step` hands it to `Load` too.
    pub(super) held_spawns: u32,
    /// What `reserve` made room for; `None` for a scratch that was never reserved.
    reserved: Option<Reserved>,
}

impl Clone for Scratch {
    /// An empty scratch with the same room reserved: it holds nothing between ticks, but a
    /// cloned run must not grow a buffer on its first tick either.
    fn clone(&self) -> Scratch {
        let mut clone = Scratch::default();
        if let Some(r) = self.reserved {
            clone.reserve(r.enemies, r.width, r.height);
        }
        clone
    }
}

impl std::fmt::Debug for Scratch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scratch").finish_non_exhaustive()
    }
}

impl Scratch {
    /// Reserves every buffer for a run of up to `enemies` enemies alive on a map `width` by
    /// `height` (1/256 pixels), once, when the run is built (DIR1, ADR-SIM-7 decision 9): a tick
    /// then never grows one. The bounds, per buffer:
    /// - `bodies`, `offsets`, `found`: one entry per enemy; `found_bits`: one bit per enemy.
    /// - the grid: one circle, cell and member per enemy, and the cells of the map.
    /// - `hit_lists`: `MAX_ATTACK_TASKS` lists of one entry per enemy and a few more.
    /// - `swings`: a hit box per run and per zone; `consumed`: one per orb;
    ///   `ranged_grants`: `MAX_RANGED_ATTACKERS` ids in each pool;
    ///   `shrine_ids`: one id per enemy.
    pub(super) fn reserve(&mut self, enemies: usize, width: i32, height: i32) {
        self.reserved = Some(Reserved {
            enemies,
            width,
            height,
        });
        reserve_to(&mut self.bodies, enemies);
        reserve_to(&mut self.offsets, enemies);
        reserve_to(&mut self.found, enemies);
        reserve_to(&mut self.shrine_ids, enemies);
        reserve_to(&mut self.found_bits, enemies.div_ceil(64));
        self.grid.reserve(enemies, width, height);
        while self.hit_lists.len() < HIT_LISTS {
            self.hit_lists
                .push(Vec::with_capacity(enemies + HIT_LIST_SPARE));
        }
        reserve_to(
            &mut self.swings,
            enemies + super::MAX_ZONES_ALIVE as usize + HIT_LIST_SPARE,
        );
        reserve_to(&mut self.consumed, super::MAX_ORBS_ALIVE as usize);
        for pool in &mut self.ranged_grants {
            reserve_to(pool, super::MAX_RANGED_ATTACKERS as usize);
        }
    }

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
