//! The desync report: what a peer kept when its state and a peer's state stopped agreeing, and
//! the replay it rebuilds on demand.

use std::fmt;

use stella_rain_core::id::PlayerIdx;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{RunRecorder, RunReplay, RunReplayError};
use stella_rain_core::run_setup::RunSetup;

use super::ConfirmedTick;

/// Why a [`DesyncReport::replay`] or [`EndedRun::replay`] failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportError {
    /// The recorder refused to start from the setup, content or party.
    Setup(RunReplayError),
    /// The recorder refused a logged leave or tick.
    Recorder(RunReplayError),
    /// From a [`DesyncReport`] only: the rebuilt run ends on another hash than the one the
    /// session held: either the setup is not the one the engine started on, or the engine is not
    /// deterministic.
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
        rebuild(setup, content, self.party, &self.ticks, self.local_hash)
    }
}

/// How a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EndKind {
    /// The leader cancelled the run: the session stopped.
    Cancel,
    /// The leader restarted the run: the session went on to the next one.
    Restart,
    /// The party cleared the stage.
    Cleared,
    /// The party failed the stage: every player down, or the stage's time ran out.
    Failed,
}

/// One run that ended: the confirmed ticks it stepped, from which its own replay is rebuilt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndedRun {
    /// The run's number, as the run byte on the wire (ADR-NET-6). Data only: `replay` ignores it.
    pub run: u8,
    /// The tick the run ended at: it has stepped ticks 0 to `tick` - 1.
    pub tick: u32,
    /// The starting party size.
    pub party: u8,
    /// How the run ended. Data only: `replay` ignores it.
    pub kind: EndKind,
    /// The confirmed ticks of the run, below `tick`.
    pub ticks: Vec<ConfirmedTick>,
}

impl EndedRun {
    /// An ended run; `ticks` is cut to `tick` entries, so what a session drained past the end
    /// never reaches the replay.
    pub fn new(
        run: u8,
        tick: u32,
        party: u8,
        kind: EndKind,
        mut ticks: Vec<ConfirmedTick>,
    ) -> EndedRun {
        ticks.truncate(usize::try_from(tick).unwrap_or(usize::MAX));
        EndedRun {
            run,
            tick,
            party,
            kind,
            ticks,
        }
    }

    /// Rebuilds the run's replay as `DesyncReport::replay` does, cut at `tick` (a log shorter
    /// than `tick` is replayed as held; the log is not checked for gaps). An ended run
    /// holds no expected hash, so this never returns `ReportError::HashDiffers`; check the
    /// bytes with `verify_run`.
    pub fn replay(&self, setup: &RunSetup, content: &RunContent) -> Result<RunReplay, ReportError> {
        let end = self
            .ticks
            .len()
            .min(usize::try_from(self.tick).unwrap_or(usize::MAX));
        rebuild(setup, content, self.party, &self.ticks[..end], None)
    }
}

/// Replays `ticks` through a recorder started on `party`; with `expect`, the rebuilt run must
/// end on that hash.
fn rebuild(
    setup: &RunSetup,
    content: &RunContent,
    party: u8,
    ticks: &[ConfirmedTick],
    expect: Option<u64>,
) -> Result<RunReplay, ReportError> {
    let mut recorder =
        RunRecorder::start_party_with(setup, content, party).map_err(ReportError::Setup)?;
    for logged in ticks {
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
    if expect.is_some_and(|hash| replay.final_hash != format!("{hash:016x}")) {
        return Err(ReportError::HashDiffers);
    }
    Ok(replay)
}
