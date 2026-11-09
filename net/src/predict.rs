//! One slot's inputs by tick, and the prediction for the ticks not received yet. Crate-private:
//! the session owns one queue per slot.
// The session does not use the queue until the next commits; this allow goes with them.
#![allow(dead_code)]

use std::collections::VecDeque;

use crate::wire::WireInput;

/// A received input differs from the one already held for its tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Conflict;

/// The inputs of one slot, by tick. Ticks below `floor` were pruned and count as received.
#[derive(Clone, Debug)]
pub(crate) struct InputQueue {
    floor: u32,
    slots: VecDeque<Option<WireInput>>,
    next: u32,
    highest: Option<u32>,
}

impl InputQueue {
    /// An empty queue: nothing received.
    pub(crate) fn new() -> InputQueue {
        InputQueue {
            floor: 0,
            slots: VecDeque::new(),
            next: 0,
            highest: None,
        }
    }

    /// The first tick not received: every tick below it is held (or pruned).
    pub(crate) fn next_tick(&self) -> u32 {
        self.next
    }

    /// The highest tick received, if any.
    pub(crate) fn highest(&self) -> Option<u32> {
        self.highest
    }

    /// The input held for `tick`, if received and not pruned.
    pub(crate) fn get(&self, tick: u32) -> Option<WireInput> {
        let at = usize::try_from(tick.checked_sub(self.floor)?).ok()?;
        self.slots.get(at).copied().flatten()
    }

    /// Whether setting `input` at `tick` would conflict with the held one.
    pub(crate) fn check(&self, tick: u32, input: &WireInput) -> Result<(), Conflict> {
        match self.get(tick) {
            Some(held) if held != *input => Err(Conflict),
            _ => Ok(()),
        }
    }

    /// Holds `input` for `tick`. The same value again changes nothing; a different one is a
    /// [`Conflict`]; a tick below the floor is ignored.
    pub(crate) fn set(&mut self, tick: u32, input: WireInput) -> Result<(), Conflict> {
        let Some(at) = tick.checked_sub(self.floor) else {
            return Ok(());
        };
        self.check(tick, &input)?;
        let at = usize::try_from(at).map_err(|_| Conflict)?;
        while self.slots.len() <= at {
            self.slots.push_back(None);
        }
        self.slots[at] = Some(input);
        self.highest = Some(self.highest.map_or(tick, |h| h.max(tick)));
        while self.get(self.next).is_some() {
            self.next += 1;
        }
        Ok(())
    }

    /// The input to step at `tick`: the one received, else the last received before it with its
    /// action cleared, else neutral. An action is never predicted.
    pub(crate) fn predicted(&self, tick: u32) -> WireInput {
        if let Some(held) = self.get(tick) {
            return held;
        }
        let last = self.next.checked_sub(1).and_then(|t| self.get(t));
        last.map_or_else(WireInput::default, |held| WireInput {
            action: None,
            ..held
        })
    }

    /// Drops the ticks below `tick`, keeping the last contiguous one for the prediction.
    pub(crate) fn prune_below(&mut self, tick: u32) {
        let target = tick.min(self.next.saturating_sub(1));
        while self.floor < target {
            self.slots.pop_front();
            self.floor += 1;
        }
    }

    /// The first tick still held.
    pub(crate) fn floor(&self) -> u32 {
        self.floor
    }

    /// Up to `max` contiguous received inputs from `from`.
    pub(crate) fn run_from(&self, from: u32, max: usize) -> Vec<WireInput> {
        (from..).map_while(|t| self.get(t)).take(max).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stella_rain_core::input::EquipAction;

    fn held(main: bool) -> WireInput {
        WireInput {
            main,
            ..WireInput::default()
        }
    }

    #[test]
    fn next_tick_is_the_first_gap() {
        let mut q = InputQueue::new();
        assert_eq!(q.next_tick(), 0);
        for t in [0, 1, 3] {
            q.set(t, held(true)).unwrap();
        }
        assert_eq!(q.next_tick(), 2);
        assert_eq!(q.highest(), Some(3));
        q.set(2, held(true)).unwrap();
        assert_eq!(q.next_tick(), 4);
    }

    #[test]
    fn the_same_value_again_is_a_no_op_and_a_different_one_conflicts() {
        let mut q = InputQueue::new();
        q.set(0, held(true)).unwrap();
        q.set(1, held(false)).unwrap();
        assert_eq!(q.set(1, held(false)), Ok(()));
        assert_eq!(q.set(1, held(true)), Err(Conflict));
        assert_eq!(q.check(1, &held(true)), Err(Conflict));
        assert_eq!(q.check(1, &held(false)), Ok(()));
        assert_eq!(q.get(1), Some(held(false)));
        assert_eq!(q.next_tick(), 2);
    }

    #[test]
    fn a_prediction_repeats_the_last_input_without_its_action() {
        let mut q = InputQueue::new();
        let action = Some(EquipAction::Extract { item: 0 });
        let last = WireInput {
            main: true,
            move_x: 7,
            action,
            ..WireInput::default()
        };
        for t in 0..6 {
            q.set(t, held(false)).unwrap();
        }
        q.set(6, last).unwrap();
        let p = q.predicted(7);
        assert!(p.main);
        assert_eq!(p.move_x, 7);
        assert_eq!(p.action, None);
        assert_eq!(q.predicted(9), p, "the same input sticks");
        assert_eq!(q.predicted(6), last, "a received input keeps its action");
    }

    #[test]
    fn with_nothing_received_the_prediction_is_neutral() {
        assert_eq!(InputQueue::new().predicted(7), WireInput::default());
    }

    #[test]
    fn a_received_input_wins_over_the_prediction() {
        let mut q = InputQueue::new();
        q.set(0, held(true)).unwrap();
        q.set(7, held(false)).unwrap();
        assert_eq!(q.predicted(7), held(false));
        assert_eq!(q.predicted(3), held(true));
    }

    #[test]
    fn pruning_keeps_the_last_contiguous_tick_for_the_prediction() {
        let mut q = InputQueue::new();
        for t in 0..4 {
            q.set(t, held(t == 3)).unwrap();
        }
        q.prune_below(2);
        assert_eq!(q.floor(), 2);
        assert_eq!(q.get(1), None);
        assert_eq!(q.get(2), Some(held(false)));
        q.prune_below(100);
        assert_eq!(q.floor(), 3, "tick next_tick - 1 is kept");
        assert_eq!(q.predicted(9), held(true));
        assert_eq!(q.next_tick(), 4);
        assert_eq!(q.set(1, held(true)), Ok(()), "a pruned tick is ignored");
        assert_eq!(q.get(1), None);
    }

    #[test]
    fn run_from_returns_the_contiguous_received_inputs() {
        let mut q = InputQueue::new();
        for t in [0, 1, 2, 4] {
            q.set(t, held(true)).unwrap();
        }
        assert_eq!(q.run_from(1, 10).len(), 2);
        assert_eq!(q.run_from(0, 2).len(), 2);
        assert!(q.run_from(3, 10).is_empty());
    }
}
