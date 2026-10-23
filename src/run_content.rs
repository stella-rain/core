//! The game content a run reads besides its mode's stage (ADR-067): the run stats a point
//! raises, the rewards an offer holds and the stage skills those name (ADR-098), the
//! expertise levels (ADR-078), the match-up table (ADR-100), the runes (ADR-099) and the perk
//! sets (ADR-105) and the difficulties (ADR-110). One value, so
//! that a run's engine, its recorder and its verification read the same files, and the replay's
//! content hash covers each of them (ADR-079 §5).

use crate::constellations::Constellations;
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
use crate::stardust::Stardust;
use crate::starting_unlocks::StartingUnlocks;
use crate::time_limit::TimeLimit;
use crate::weapons::Weapons;

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
    /// The weapons' combo trees (WP1).
    weapons: Weapons,
    /// What a new save begins with, if the pack holds the file (a pack released before it does
    /// not). Not part of any run: nothing in the simulation reads it, and the pack's hash covers
    /// it with the rest of the pack.
    starting_unlocks: Option<StartingUnlocks>,
    /// The rules the app pays a failed run's Stardust by (ADR-101 point 4): the pack's file, or
    /// the bundled one for a pack released without it. Outside what a run reads, as
    /// `starting_unlocks`: the pack's hash covers it, no derived content hash does.
    stardust: Stardust,
    time_limit: TimeLimit,
    /// The `trial` mode's stage, and its file's bytes as they are in the pack.
    trial: Stage,
    trial_bytes: Vec<u8>,
    /// The `tutorial` mode's stage, and its file's bytes as they are in the pack (ADR-086).
    tutorial: Stage,
    tutorial_bytes: Vec<u8>,
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
            weapons: bundled.weapons.clone(),
            starting_unlocks: bundled.starting_unlocks.clone(),
            stardust: bundled.stardust.clone(),
            time_limit: TimeLimit::bundled().clone(),
            trial: bundled.trial.clone(),
            trial_bytes: bundled.trial_bytes.clone(),
            tutorial: bundled.tutorial.clone(),
            tutorial_bytes: bundled.tutorial_bytes.clone(),
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
        weapons: Weapons,
        starting_unlocks: Option<StartingUnlocks>,
        stardust: Stardust,
        time_limit: TimeLimit,
        trial: Stage,
        trial_bytes: Vec<u8>,
        tutorial: Stage,
        tutorial_bytes: Vec<u8>,
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
            weapons,
            starting_unlocks,
            stardust,
            time_limit,
            trial,
            trial_bytes,
            tutorial,
            tutorial_bytes,
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

    /// The same content with `weapons` in place of its weapons.
    pub fn with_weapons(self, weapons: Weapons) -> RunContent {
        RunContent {
            weapons,
            pack_sha256: None,
            ..self
        }
    }

    /// The same content with `run_stats` in place of its run stats.
    pub fn with_run_stats(self, run_stats: RunStats) -> RunContent {
        RunContent {
            run_stats,
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

    /// The `tutorial` mode's stage (ADR-086).
    pub fn tutorial(&self) -> &Stage {
        &self.tutorial
    }

    /// The `tutorial` mode's file as it has been in the pack.
    pub(crate) fn tutorial_bytes(&self) -> &[u8] {
        &self.tutorial_bytes
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

    /// The weapons' combo trees (WP1).
    pub fn weapons(&self) -> &Weapons {
        &self.weapons
    }

    /// What a new save begins with (DS5 Q10), `None` when the pack was released without the file:
    /// unknown, never a guessed list.
    pub fn starting_unlocks(&self) -> Option<&StartingUnlocks> {
        self.starting_unlocks.as_ref()
    }

    /// The Stardust rules (ADR-101 point 4): the pack's file, or the bundled one for content
    /// from a pack released without it.
    pub fn stardust(&self) -> &Stardust {
        &self.stardust
    }

    /// The sets of named stars (core#NS3): those of the pack the rewards were read from, none
    /// for content without the file.
    pub fn constellations(&self) -> &Constellations {
        self.rewards.constellations()
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
