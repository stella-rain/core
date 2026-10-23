//! The buffers a tick fills and empties again: they keep their memory from tick to tick, so a
//! steady tick allocates nothing (T1b). They hold nothing between ticks that the rules read, so
//! they are neither hashed nor cloned: a cloned state starts with empty ones.

use super::grid::Grid;
use super::push::Body;

#[derive(Debug, Default)]
pub(crate) struct Scratch {
    /// The enemy grid of the push-apart or of the tick's collisions.
    pub(super) grid: Grid,
    /// What the push-apart knows of each enemy, and the move it works out for it.
    pub(super) bodies: Vec<Body>,
    pub(super) offsets: Vec<(i32, i32, bool)>,
    /// The candidates the grid names for one enemy.
    pub(super) near: Vec<u32>,
    /// Whether each bullet of the tick was used up.
    pub(super) consumed: Vec<bool>,
}

impl Clone for Scratch {
    fn clone(&self) -> Scratch {
        Scratch::default()
    }
}
