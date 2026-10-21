//! The game content a run reads besides its mode's stage (ADR-067): the run stats a point
//! raises, the rewards an offer holds and the stage skills those name (ADR-098), the
//! expertise levels (ADR-078), the match-up table (ADR-100), the runes (ADR-099) and the perk
//! sets (ADR-105) and the difficulties (ADR-110). One value, so
//! that a run's engine, its recorder and its verification read the same files, and the replay's
//! content hash covers each of them (ADR-079 §5).

use crate::difficulties::Difficulties;
use crate::expertise::Expertise;
use crate::match_ups::MatchUps;
use crate::pack::Pack;
use crate::perk_sets::PerkSets;
use crate::rewards::Rewards;
use crate::run_stats::RunStats;
use crate::runes::Runes;
use crate::stage::Stage;
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
    perk_sets: PerkSets,
    difficulties: Difficulties,
    time_limit: TimeLimit,
    /// The `trial` mode's stage, and its file's bytes as they are in the pack.
    trial: Stage,
    trial_bytes: Vec<u8>,
    /// The SHA-256 of the pack these files were read from: the content hash of a run
    /// (ADR-079 §5, ADR-080). `None` once any file is replaced: the run's hash is then derived
    /// from the files (`run_replay::content_sha256_with`).
    pack_sha256: Option<[u8; 32]>,
}

impl RunContent {
    /// The run stats and rewards given, with the bundled expertise, match-ups and runes.
    pub fn new(run_stats: RunStats, rewards: Rewards) -> RunContent {
        let bundled = RunContent::bundled();
        RunContent {
            run_stats,
            rewards,
            expertise: Expertise::bundled().clone(),
            match_ups: *MatchUps::bundled(),
            runes: Runes::bundled().clone(),
            perk_sets: PerkSets::bundled().clone(),
            difficulties: Difficulties::bundled().clone(),
            time_limit: TimeLimit::bundled().clone(),
            trial: bundled.trial.clone(),
            trial_bytes: bundled.trial_bytes.clone(),
            pack_sha256: None,
        }
    }

    /// The content of a pack: every file read from it, none from the bundled ones.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        run_stats: RunStats,
        rewards: Rewards,
        expertise: Expertise,
        match_ups: MatchUps,
        runes: Runes,
        perk_sets: PerkSets,
        difficulties: Difficulties,
        time_limit: TimeLimit,
        trial: Stage,
        trial_bytes: Vec<u8>,
        pack_sha256: Option<[u8; 32]>,
    ) -> RunContent {
        RunContent {
            run_stats,
            rewards,
            expertise,
            match_ups,
            runes,
            perk_sets,
            difficulties,
            time_limit,
            trial,
            trial_bytes,
            pack_sha256,
        }
    }

    /// The same content with `expertise` in place of its expertise.
    pub fn with_expertise(self, expertise: Expertise) -> RunContent {
        RunContent {
            expertise,
            pack_sha256: None,
            ..self
        }
    }

    /// The same content with `match_ups` in place of its match-up table.
    pub fn with_match_ups(self, match_ups: MatchUps) -> RunContent {
        RunContent {
            match_ups,
            pack_sha256: None,
            ..self
        }
    }

    /// The same content with `runes` in place of its runes.
    pub fn with_runes(self, runes: Runes) -> RunContent {
        RunContent {
            runes,
            pack_sha256: None,
            ..self
        }
    }

    /// The same content with `perk_sets` in place of its perk sets.
    pub fn with_perk_sets(self, perk_sets: PerkSets) -> RunContent {
        RunContent {
            perk_sets,
            pack_sha256: None,
            ..self
        }
    }

    /// The same content with `difficulties` in place of its difficulties.
    pub fn with_difficulties(self, difficulties: Difficulties) -> RunContent {
        RunContent {
            difficulties,
            pack_sha256: None,
            ..self
        }
    }

    /// The same content with `time_limit` in place of the `time_limit` mode's file: what its
    /// runs and its replays' bounds are read from, and part of their content hash.
    pub fn with_time_limit(self, time_limit: TimeLimit) -> RunContent {
        RunContent {
            time_limit,
            pack_sha256: None,
            ..self
        }
    }

    /// The bundled pack's content, the content of every run.
    pub fn bundled() -> &'static RunContent {
        Pack::bundled().content()
    }

    /// The `trial` mode's stage: also what the `time_limit` mode takes its placeholder content
    /// from.
    pub fn trial(&self) -> &Stage {
        &self.trial
    }

    /// The `trial` mode's file as it has been in the pack, and a final newline.
    pub(crate) fn trial_bytes(&self) -> &[u8] {
        &self.trial_bytes
    }

    /// The SHA-256 of the pack these files were read from, `None` for files replaced one by one.
    pub fn pack_sha256(&self) -> Option<[u8; 32]> {
        self.pack_sha256
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

    /// The perk sets a setup picks from (ADR-105, ADR-107).
    pub fn perk_sets(&self) -> &PerkSets {
        &self.perk_sets
    }

    /// The difficulties a setup picks from (ADR-110).
    pub fn difficulties(&self) -> &Difficulties {
        &self.difficulties
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
