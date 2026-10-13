//! The per-tick input record (ADR-016, ADR-050): the only external influence on a run, and all
//! a replay stores. Touch, keyboard and later a gamepad produce the same record; it never says
//! which device made it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// Movement direction and strength, each −127 to 127 (−128 counts as −127). The simulation
    /// clamps the vector to length 127 and scales it to the character's speed (ADR-050).
    pub move_x: i8,
    pub move_y: i8,
    /// Skill 1 pressed on this tick: a ready evade dashes (ADR-050).
    pub evade: bool,
    /// Skill 2 held on this tick: a press while ready starts the charge, letting go releases it
    /// (ADR-050).
    pub charge: bool,
    /// Skill 3 pressed on this tick: a ready Sub skill happens (ADR-050).
    pub sub: bool,
}
