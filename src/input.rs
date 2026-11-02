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
    /// The smash button (B) is held on this tick (ADR-070).
    pub smash: bool,
    /// The evade button (X) is held on this tick: a press dashes when an evade is ready.
    pub evade: bool,
    /// The stage skill button (Y) is held on this tick: a press uses the skill in the slot when
    /// it is ready (ADR-075).
    pub skill: bool,
    /// Aim assist is on for this tick, a switch (ADR-072): a combo step faces the target.
    pub assist: bool,
    /// Auto mode is on for this tick, a switch (ADR-073): the simulation plays the auto combo
    /// at the target. Its presses come from the state, never from this record.
    pub auto: bool,
    /// On a tick where an offer is open: the option to take, an index into the offer's `options`
    /// (ADR-REPL-3). Read only then; ignored otherwise. Not part of the schema 1 replay stream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pick: Option<u32>,
    /// The interact button is held on this tick: the revive hold (ADR-SIM-13). Read by the down
    /// and revive rules only; the simulation ignores it until they exist. Schema 2 only: the
    /// schema 1 stream cannot carry it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interact: bool,
}

impl Input {
    /// An input that takes option `index` of the open offer and presses nothing.
    pub const fn picking(index: u32) -> Input {
        Input {
            move_x: 0,
            move_y: 0,
            main: false,
            smash: false,
            evade: false,
            skill: false,
            assist: false,
            auto: false,
            pick: Some(index),
            interact: false,
        }
    }
}

/// The buttons of an input as bits, for the state to keep from one tick to the next: `main`,
/// `smash`, `evade` and `skill` from bit 0. The switches are not buttons and are not kept.
pub(crate) fn held_buttons(input: Input) -> u8 {
    u8::from(input.main)
        | u8::from(input.smash) << 1
        | u8::from(input.evade) << 2
        | u8::from(input.skill) << 3
}

pub(crate) const MAIN: u8 = 0b1;
pub(crate) const SMASH: u8 = 0b10;
pub(crate) const EVADE: u8 = 0b100;
pub(crate) const SKILL: u8 = 0b1000;
