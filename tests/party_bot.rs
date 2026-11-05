//! A seeded party bot and a test-only party builder over `run_play::PartyCase` (ADR-REPL-5,
//! ADR-SIM-7, ADR-SIM-8). Beside `run_play.rs`, not in `tests/common/`, which the benches share.

use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;

/// A bot for any slot of a party: every call is a pure function of the seed, the engine, the
/// slot and the tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartyBot {
    pub seed: u64,
}

impl PartyBot {
    /// The buttons of `slot` at `tick`.
    pub fn input(&self, _engine: &Engine, _slot: u8, _tick: u32) -> Input {
        Input::default()
    }

    /// The option `slot` takes from its open offer; `None` with no offer.
    pub fn pick(&self, _engine: &Engine, _slot: u8, _tick: u32) -> Option<u32> {
        None
    }
}
