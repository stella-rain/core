//! The per-tick input record (ADR-016, ADR-032, ADR-038): the only external influence on a
//! run, and all a replay stores. Touch and keyboard produce the same record; it never says
//! which device made it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// Movement this tick in 1/256 pixel units, before the simulation's speed cap (ADR-038).
    pub dx: i16,
    pub dy: i16,
    pub focus: bool,
    /// The main shot fires only on ticks with `fire` (ADR-038).
    pub fire: bool,
    /// The skill slot held this tick: 0 for none, 1 to 3 (ADR-032). Press, hold and release
    /// are read from how this changes between ticks.
    pub held: u8,
}
