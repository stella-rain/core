//! File and rule versions (ADR-017). Both stay 0 until the freeze at P3 (ADR-035): until then
//! types and rules change in place, and the replay corpus is regenerated on purpose.

/// Structure of stage, event and replay files.
pub const SCHEMA_VERSION: u32 = 0;

/// Simulation rules.
pub const SIM_VERSION: u32 = 0;
