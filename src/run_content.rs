//! The game content a run reads besides its mode's stage (ADR-067): the run stats a point
//! raises, the rewards an offer holds and the stage skills those name (ADR-098), the
//! expertise levels (ADR-078), the match-up table (ADR-100) and the runes (ADR-099). One value, so
//! that a run's engine, its recorder and its verification read the same files, and the replay's
//! content hash covers each of them (ADR-079 §5).

use crate::expertise::Expertise;
use crate::match_ups::MatchUps;
use crate::pack::Pack;
use crate::rewards::Rewards;
use crate::run_stats::RunStats;
use crate::runes::Runes;
use crate::stage_skill::StageSkills;
use crate::time_limit::TimeLimit;

/// Content files that passed validation, each with the SHA-256 of its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunContent {
    run_stats: RunStats,
    rewards: Rewards,
    expertise: Expertise,
    match_ups: MatchUps,
    runes: Runes,
    time_limit: TimeLimit,
}

impl RunContent {
    /// The run stats and rewards given, with the bundled expertise, match-ups and runes.
    pub fn new(run_stats: RunStats, rewards: Rewards) -> RunContent {
        RunContent {
            run_stats,
            rewards,
            expertise: Expertise::bundled().clone(),
            match_ups: *MatchUps::bundled(),
            runes: Runes::bundled().clone(),
            time_limit: TimeLimit::bundled().clone(),
        }
    }

    /// The content of a pack: every file read from it, none from the bundled ones.
    pub(crate) fn from_parts(
        run_stats: RunStats,
        rewards: Rewards,
        expertise: Expertise,
        match_ups: MatchUps,
        runes: Runes,
        time_limit: TimeLimit,
    ) -> RunContent {
        RunContent {
            run_stats,
            rewards,
            expertise,
            match_ups,
            runes,
            time_limit,
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

    /// The same content with `runes` in place of its runes.
    pub fn with_runes(self, runes: Runes) -> RunContent {
        RunContent { runes, ..self }
    }

    /// The same content with `time_limit` in place of the `time_limit` mode's file: what its
    /// runs and its replays' bounds are read from, and part of their content hash.
    pub fn with_time_limit(self, time_limit: TimeLimit) -> RunContent {
        RunContent { time_limit, ..self }
    }

    /// The bundled pack's content, the content of every run.
    pub fn bundled() -> &'static RunContent {
        Pack::bundled().content()
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

    /// What each on-hit rune does when it fires.
    pub fn runes(&self) -> &Runes {
        &self.runes
    }

    /// The `time_limit` mode's file.
    pub fn time_limit(&self) -> &TimeLimit {
        &self.time_limit
    }

    /// The stage skills the rewards give: those the rewards were read with.
    pub fn stage_skills(&self) -> &StageSkills {
        self.rewards.stage_skills()
    }
}
