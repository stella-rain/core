//! The save seam: how a session keeps the engine's state at the start of recent ticks so it can
//! go back to one. [`CloneRing`] keeps clones of the engine; a cheaper save can replace it behind
//! [`Save`] without touching the session.

use std::collections::VecDeque;

use stella_rain_core::engine::Engine;

/// Keeps the engine's state at the start of recent ticks.
///
/// `tick` is the number of ticks the engine had run when the state was taken, so the state saved
/// as `tick` is the one that steps tick `tick` next.
pub trait Save {
    /// Keeps `engine` as the state after `tick` ticks; saving a tick again replaces it.
    fn save(&mut self, tick: u32, engine: &Engine);

    /// Puts the state kept for `tick` into `engine` and returns true, or returns false and leaves
    /// `engine` untouched when `tick` is not held.
    fn load(&mut self, tick: u32, engine: &mut Engine) -> bool;

    /// Drops every state kept for a tick below `tick`.
    fn forget_before(&mut self, tick: u32);
}

/// A [`Save`] that holds the newest `capacity` clones of the engine, oldest first.
#[derive(Debug)]
pub struct CloneRing {
    capacity: usize,
    saved: VecDeque<(u32, Engine)>,
}

impl CloneRing {
    /// A ring that keeps the newest `capacity` saves; a capacity of 0 keeps 1.
    pub fn new(capacity: usize) -> CloneRing {
        CloneRing {
            capacity: capacity.max(1),
            saved: VecDeque::new(),
        }
    }

    /// How many saves the ring holds now.
    pub fn held(&self) -> usize {
        self.saved.len()
    }
}

impl Save for CloneRing {
    fn save(&mut self, tick: u32, engine: &Engine) {
        if let Some(slot) = self.saved.iter_mut().find(|(t, _)| *t == tick) {
            slot.1 = engine.clone();
            return;
        }
        self.saved.push_back((tick, engine.clone()));
        while self.saved.len() > self.capacity {
            self.saved.pop_front();
        }
    }

    fn load(&mut self, tick: u32, engine: &mut Engine) -> bool {
        match self.saved.iter().find(|(t, _)| *t == tick) {
            Some((_, saved)) => {
                *engine = saved.clone();
                true
            }
            None => false,
        }
    }

    fn forget_before(&mut self, tick: u32) {
        self.saved.retain(|(t, _)| *t >= tick);
    }
}
