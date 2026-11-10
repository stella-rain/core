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
    /// Keeps `engine` as the state after `tick` ticks; saving a tick again replaces it. An
    /// implementation may rely on the order a session calls in: a save of the tick a `load` just
    /// restored comes with the engine untouched in between, so it may keep the state it has.
    fn save(&mut self, tick: u32, engine: &Engine);

    /// Puts the state kept for `tick` into `engine` and returns true, or returns false and leaves
    /// `engine` untouched when `tick` is not held.
    fn load(&mut self, tick: u32, engine: &mut Engine) -> bool;

    /// Drops every state kept for a tick below `tick`.
    fn forget_before(&mut self, tick: u32);
}

/// A [`Save`] that holds the newest `capacity` copies of the engine, oldest first.
///
/// A save and a load copy into an engine already held (`Clone::clone_from`), so a ring that has
/// filled once allocates little and drops no engine: a slot that falls out of the ring is the
/// next one written.
#[derive(Debug)]
pub struct CloneRing {
    capacity: usize,
    saved: VecDeque<(u32, Engine)>,
    /// Engines of slots forgotten by `forget_before`, written by the next saves.
    spare: Vec<Engine>,
    /// The tick the last call restored, until the next call of any other kind: a save of that
    /// tick right after it would copy the engine onto the slot it came from.
    loaded: Option<u32>,
    /// Engine writes (a clone or a `clone_from`) and new engines made, for the unit tests.
    #[cfg(test)]
    writes: u32,
    #[cfg(test)]
    made: u32,
}

impl CloneRing {
    /// A ring that keeps the newest `capacity` saves; a capacity of 0 keeps 1.
    pub fn new(capacity: usize) -> CloneRing {
        let capacity = capacity.max(1);
        CloneRing {
            capacity,
            saved: VecDeque::with_capacity(capacity),
            spare: Vec::with_capacity(capacity),
            loaded: None,
            #[cfg(test)]
            writes: 0,
            #[cfg(test)]
            made: 0,
        }
    }

    /// How many saves the ring holds now.
    pub fn held(&self) -> usize {
        self.saved.len()
    }
}

impl Save for CloneRing {
    fn save(&mut self, tick: u32, engine: &Engine) {
        let restored = self.loaded.take();
        if let Some(slot) = self.saved.iter_mut().find(|(t, _)| *t == tick) {
            if restored == Some(tick) {
                debug_assert_eq!(
                    slot.1.state_hash(),
                    engine.state_hash(),
                    "the engine changed between a load and the save of the same tick"
                );
                return;
            }
            slot.1.clone_from(engine);
            #[cfg(test)]
            {
                self.writes += 1;
            }
            return;
        }
        let slot = if let Some(mut slot) = self.spare.pop() {
            slot.clone_from(engine);
            slot
        } else if self.saved.len() >= self.capacity {
            let (_, mut slot) = self.saved.pop_front().expect("a full ring holds a slot");
            slot.clone_from(engine);
            slot
        } else {
            #[cfg(test)]
            {
                self.made += 1;
            }
            engine.clone()
        };
        #[cfg(test)]
        {
            self.writes += 1;
        }
        self.saved.push_back((tick, slot));
    }

    fn load(&mut self, tick: u32, engine: &mut Engine) -> bool {
        self.loaded = None;
        match self.saved.iter().find(|(t, _)| *t == tick) {
            Some((_, saved)) => {
                engine.clone_from(saved);
                self.loaded = Some(tick);
                #[cfg(test)]
                {
                    self.writes += 1;
                }
                true
            }
            None => false,
        }
    }

    fn forget_before(&mut self, tick: u32) {
        while let Some(at) = self.saved.iter().position(|(t, _)| *t < tick) {
            if let Some((_, slot)) = self.saved.remove(at) {
                self.spare.push(slot);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stella_rain_core::input::Input;
    use stella_rain_core::run_setup::RunSetup;

    fn engine() -> Engine {
        Engine::start_party(&RunSetup::default(), 2).expect("the run starts")
    }

    fn step(e: &mut Engine) {
        e.step(&[Input::default(); 3]);
    }

    #[test]
    fn a_save_right_after_a_load_of_the_same_tick_writes_nothing() {
        let mut e = engine();
        let mut ring = CloneRing::new(5);
        for t in 0..4 {
            ring.save(t, &e);
            step(&mut e);
        }
        assert!(ring.load(2, &mut e));
        let writes = ring.writes;
        ring.save(2, &e);
        assert_eq!(ring.writes, writes, "the redundant save is skipped");
        assert!(ring.load(2, &mut e));
        step(&mut e);
        ring.save(3, &e);
        assert_eq!(
            ring.writes,
            writes + 2,
            "one load, one save of the next tick"
        );
    }

    #[test]
    fn a_save_of_the_next_tick_after_a_load_writes() {
        let mut e = engine();
        let mut ring = CloneRing::new(5);
        for t in 0..3 {
            ring.save(t, &e);
            step(&mut e);
        }
        assert!(ring.load(1, &mut e));
        let writes = ring.writes;
        step(&mut e);
        ring.save(2, &e);
        assert_eq!(ring.writes, writes + 1);
        ring.save(3, &e);
        assert_eq!(ring.writes, writes + 2);
    }

    #[test]
    fn a_full_ring_makes_no_new_engine() {
        let mut e = engine();
        let mut ring = CloneRing::new(5);
        for t in 0..5 {
            ring.save(t, &e);
            step(&mut e);
        }
        assert_eq!(ring.made, 5, "the first fill makes one engine per slot");
        for t in 5..105 {
            ring.save(t, &e);
            step(&mut e);
            if t % 7 == 0 {
                ring.forget_before(t.saturating_sub(2));
            }
            assert!(ring.held() <= 5);
        }
        assert_eq!(ring.made, 5, "a full ring reuses its slots in place");
    }
}
