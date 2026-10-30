//! `encode_state` and `decode_state`: the whole run state as savepoint bytes, and back.

use crate::engine::Engine;
use crate::ledger::{Counted, RunLedger};
use crate::sim::state::State;

use super::DecodeError;

/// The state, the ledger and the tallies of `engine` as bytes: no header, no trailing bytes.
#[allow(dead_code)] // the savepoint file calls it next
pub(crate) fn encode_state(engine: &Engine) -> Vec<u8> {
    let _ = engine;
    unimplemented!("SNAP-CODEC-2 step 5")
}

/// The state, the ledger and the tallies in `bytes`, for a run of `fresh`'s content. `fresh` is
/// an engine started from the same setup and content, untouched: everything the content and the
/// setup decide comes from it, and every length and index in the bytes is checked against it.
#[allow(dead_code)] // the savepoint file calls it next
pub(crate) fn decode_state(
    bytes: &[u8],
    fresh: &Engine,
) -> Result<(State, RunLedger, Counted), DecodeError> {
    let _ = (bytes, fresh);
    unimplemented!("SNAP-CODEC-2 step 5")
}
