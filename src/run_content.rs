//! The game content a run reads besides its mode's stage (ADR-067): the run stats a point
//! raises. One value, so that a run's engine, its recorder and its verification read the same
//! files, and the replay's content hash covers each of them (ADR-066 §5).

use std::sync::OnceLock;

use crate::run_stats::RunStats;

/// Content files that passed validation, each with the SHA-256 of its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunContent {
    run_stats: RunStats,
}

impl RunContent {
    pub fn new(run_stats: RunStats) -> RunContent {
        RunContent { run_stats }
    }

    /// The bundled files, the content of every run.
    pub fn bundled() -> &'static RunContent {
        static BUNDLED: OnceLock<RunContent> = OnceLock::new();
        BUNDLED.get_or_init(|| RunContent::new(RunStats::bundled().clone()))
    }

    /// What a stat point can raise.
    pub fn run_stats(&self) -> &RunStats {
        &self.run_stats
    }
}
