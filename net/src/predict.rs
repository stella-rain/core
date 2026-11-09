//! One slot's inputs by tick, and the prediction for the ticks not received yet. Crate-private:
//! the session owns one queue per slot.
// The skeleton has no bodies yet; this allow goes when they are written.
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
        todo!()
    }

    /// The highest tick received, if any.
    pub(crate) fn highest(&self) -> Option<u32> {
        todo!()
    }

    /// The input held for `tick`, if received and not pruned.
    pub(crate) fn get(&self, _tick: u32) -> Option<WireInput> {
        todo!()
    }

    /// Whether setting `input` at `tick` would conflict with the held one.
    pub(crate) fn check(&self, _tick: u32, _input: &WireInput) -> Result<(), Conflict> {
        todo!()
    }

    /// Holds `input` for `tick`. The same value again changes nothing; a different one is a
    /// [`Conflict`]; a tick below the floor is ignored.
    pub(crate) fn set(&mut self, _tick: u32, _input: WireInput) -> Result<(), Conflict> {
        todo!()
    }

    /// The input to step at `tick`: the one received, else the last received before it with its
    /// action cleared, else neutral. An action is never predicted.
    pub(crate) fn predicted(&self, _tick: u32) -> WireInput {
        todo!()
    }

    /// Drops the ticks below `tick`, keeping the last contiguous one for the prediction.
    pub(crate) fn prune_below(&mut self, _tick: u32) {
        todo!()
    }

    /// The first tick still held.
    pub(crate) fn floor(&self) -> u32 {
        self.floor
    }

    /// Up to `max` contiguous received inputs from `from`.
    pub(crate) fn run_from(&self, _from: u32, _max: usize) -> Vec<WireInput> {
        todo!()
    }
}
