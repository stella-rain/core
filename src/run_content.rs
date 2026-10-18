//! The game content a run reads besides its mode's stage (ADR-067): the run stats a point
//! raises, the rewards an offer holds and the stage skills those name (ADR-098), the
//! expertise levels (ADR-078) and the match-up table (ADR-100). One value, so
//! that a run's engine, its recorder and its verification read the same files, and the replay's
//! content hash covers each of them (ADR-079 §5).

use std::sync::OnceLock;

use crate::expertise::Expertise;
use crate::match_ups::MatchUps;
use crate::rewards::Rewards;
use crate::run_stats::RunStats;
use crate::stage_skill::StageSkills;

/// Content files that passed validation, each with the SHA-256 of its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunContent {
    run_stats: RunStats,
    rewards: Rewards,
    expertise: Expertise,
    match_ups: MatchUps,
}

impl RunContent {
    /// The run stats and rewards given, with the bundled expertise and match-ups.
    pub fn new(run_stats: RunStats, rewards: Rewards) -> RunContent {
        RunContent {
            run_stats,
            rewards,
            expertise: Expertise::bundled().clone(),
            match_ups: *MatchUps::bundled(),
        }
    }

    /// The same content with `expertise` in place of its expertise.
    pub fn with_expertise(self, expertise: Expertise) -> RunContent {
        RunContent { expertise, ..self }
    }

    /// The same content with `match_ups` in place of its match-up table.
    pub fn with_match_ups(self, match_ups: MatchUps) -> RunContent {
        RunContent { match_ups, ..self }
    }

    /// The bundled files, the content of every run.
    pub fn bundled() -> &'static RunContent {
        static BUNDLED: OnceLock<RunContent> = OnceLock::new();
        BUNDLED.get_or_init(|| {
            RunContent::new(RunStats::bundled().clone(), Rewards::bundled().clone())
        })
    }

    /// What a stat point can raise.
    pub fn run_stats(&self) -> &RunStats {
        &self.run_stats
    }

    /// What an offer can hold.
    pub fn rewards(&self) -> &Rewards {
        &self.rewards
    }

    /// The thresholds the expertise gauge rises by.
    pub fn expertise(&self) -> &Expertise {
        &self.expertise
    }

    /// What each match-up does to a hit.
    pub fn match_ups(&self) -> &MatchUps {
        &self.match_ups
    }

    /// The stage skills the rewards give: those the rewards were read with.
    pub fn stage_skills(&self) -> &StageSkills {
        self.rewards.stage_skills()
    }
}
