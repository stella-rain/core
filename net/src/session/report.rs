//! The desync report: what a peer kept when its state and a peer's state stopped agreeing, and
//! the replay it rebuilds on demand.

use std::fmt;

use stella_rain_core::id::PlayerIdx;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{RunRecorder, RunReplay, RunReplayError};
use stella_rain_core::run_setup::RunSetup;

use super::ConfirmedTick;

/// Why a [`DesyncReport::replay`] failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportError {
    /// The recorder refused to start from the setup, content or party.
    Setup(RunReplayError),
    /// The recorder refused a logged leave or tick.
    Recorder(RunReplayError),
    /// The rebuilt run ends on another hash than the one the session held: either the setup is
    /// not the one the engine started on, or the engine is not deterministic.
    HashDiffers,
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ReportError::Setup(_) => "the recorder refused to start",
            ReportError::Recorder(_) => "the recorder refused a logged tick",
            ReportError::HashDiffers => "the rebuilt run ends on another hash",
        })
    }
}

impl std::error::Error for ReportError {}

/// What a session keeps when it stops on a desync (ADR-SVC-4: it leaves the device only when the
/// player sends it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesyncReport {
    /// The checkpoint tick the hashes differ at.
    pub tick: u32,
    /// The peer whose hash differed (or who announced the desync).
    pub slot: PlayerIdx,
    /// This session's state hash after `tick` ticks, where it still held it.
    pub local_hash: Option<u64>,
    /// The peer's hash for `tick`, absent when the stop came from a peer's `Bye`.
    pub remote_hash: Option<u64>,
    /// The starting party size.
    pub party: u8,
    /// Every tick the engine stepped below `tick`, with its inputs and leaves.
    pub ticks: Vec<ConfirmedTick>,
}

impl DesyncReport {
    /// Rebuilds the run through a [`stella_rain_core::run_replay::RunRecorder`]: per logged
    /// tick the leaves stamped on it, then the step. Nothing runs until this is called.
    pub fn replay(&self, setup: &RunSetup, content: &RunContent) -> Result<RunReplay, ReportError> {
        let mut recorder = RunRecorder::start_party_with(setup, content, self.party)
            .map_err(ReportError::Setup)?;
        for logged in &self.ticks {
            for slot in 0..3u8 {
                if logged.leaves >> slot & 1 == 1
                    && let Some(who) = PlayerIdx::new(slot)
                {
                    recorder.leave(who).map_err(ReportError::Recorder)?;
                }
            }
            let inputs = logged.inputs.map(|wire| wire.to_input());
            recorder.step(&inputs).map_err(ReportError::Recorder)?;
        }
        let replay = recorder.replay();
        if self
            .local_hash
            .is_some_and(|hash| replay.final_hash != format!("{hash:016x}"))
        {
            return Err(ReportError::HashDiffers);
        }
        Ok(replay)
    }
}
