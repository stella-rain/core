//! The per-tick input record (ADR-016, ADR-071): the only external influence on a run, and all
//! a replay stores. Touch, keyboard and later a gamepad produce the same record; it never says
//! which device made it, and its fields are named for what they do, not for buttons or keys.
//!
//! Buttons are sent held. The simulation works out a press (held now, not on the tick before)
//! and a release (the reverse) from the buttons it kept from the last tick, so a held button
//! never repeats a press.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// Movement direction and strength, each −127 to 127 (−128 counts as −127). The simulation
    /// clamps the vector to length 127 and scales it to the character's speed (ADR-071).
    pub move_x: i8,
    pub move_y: i8,
    /// The Main attack button (A) is held on this tick (ADR-070).
    pub main: bool,
    /// The Sub attack button (B) is held on this tick (ADR-070).
    pub sub: bool,
    /// The evade button (X) is held on this tick: a press dashes when an evade is ready.
    pub evade: bool,
    /// The stage skill button (Y) is held on this tick (ADR-075; it does nothing yet).
    pub skill: bool,
    /// Aim assist is on for this tick, a switch (ADR-072; it does nothing yet).
    pub assist: bool,
    /// Auto mode is on for this tick, a switch (ADR-073; it does nothing yet).
    pub auto: bool,
}

/// The buttons of an input as bits, for the state to keep from one tick to the next: `main`,
/// `sub`, `evade` and `skill` from bit 0. The switches are not buttons and are not kept.
pub(crate) fn held_buttons(input: Input) -> u8 {
    u8::from(input.main)
        | u8::from(input.sub) << 1
        | u8::from(input.evade) << 2
        | u8::from(input.skill) << 3
}

pub(crate) const MAIN: u8 = 0b1;
pub(crate) const SUB: u8 = 0b10;
pub(crate) const EVADE: u8 = 0b100;
